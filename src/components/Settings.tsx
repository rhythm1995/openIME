import { useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Monitor,
  Mic,
  Mic2,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Loader2,
  CircleDot,
} from "lucide-react";
import type {
  AppConfig,
  LocalAsrModelEntry,
  LocalModelStatus,
  ModelDownloadProgress,
  Persona,
  PolishModelStatus,
  PolishPolicy,
  ProviderConfig,
} from "../types";
import { ipc, permissionLabel, type PermissionKind, type PermissionStatus } from "../ipc";

// 默认本地 ASR id（与 voice-core asr_catalog 对齐；未安装时不算「使用中」）。
const DEFAULT_LOCAL_ASR = "sensevoice";

/** 离线整段解码类模型 id（启用时同步 local_mode=offline）。 */
function isOfflineAsrId(id: string): boolean {
  return id === "sensevoice" || id === "firered-large";
}

function fmtSize(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

// 状态前缀小图标：替代 ✅/❌/⚠️/🔴 emoji，保持文字与图标基线对齐。
function StatusIcon({ ok, warn, spin }: { ok?: boolean; warn?: boolean; spin?: boolean }) {
  const common = { size: 13, style: { flexShrink: 0 } as const };
  if (spin) return <Loader2 {...common} className="spin" />;
  if (ok) return <CheckCircle2 {...common} color="var(--success)" />;
  if (warn) return <AlertTriangle {...common} color="var(--warning)" />;
  return <XCircle {...common} color="var(--danger)" />;
}

export default function Settings() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [mic, setMic] = useState<PermissionStatus | null>(null);
  const [ax, setAx] = useState<PermissionStatus | null>(null);
  const [autoStart, setAutoStart] = useState(false);
  const [modelStatus, setModelStatus] = useState<LocalModelStatus | null>(null);
  const [asrModels, setAsrModels] = useState<LocalAsrModelEntry[]>([]);
  const [dl, setDl] = useState<ModelDownloadProgress | null>(null);
  const [dlError, setDlError] = useState<string | null>(null);
  const [dlTargetId, setDlTargetId] = useState<string | null>(null);
  // 功能测试：Fn 键事件 + 语音录入测试框
  const [fnCount, setFnCount] = useState(0);
  const [fnState, setFnState] = useState<"idle" | "down">("idle");
  const [testText, setTestText] = useState("");
  // 功能测试框：录音开始时的文本基准（支持多次录音累加，但单次录音 partial/final 不重复）。
  const testBaseRef = useRef("");
  // 保持 testText 最新值，供事件回调（闭包只注册一次）读取，避免 stale closure。
  const testTextRef = useRef("");
  testTextRef.current = testText;
  const [recording, setRecording] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; text: string } | null>(null);
  // 音频设备（麦克风下拉 + 测试）
  const [devices, setDevices] = useState<string[]>([]);
  const [micTest, setMicTest] = useState<{ ok: boolean; warn: boolean; text: string } | null>(null);
  const [testingMic, setTestingMic] = useState(false);
  // 二期润色
  const [polishStatus, setPolishStatus] = useState<PolishModelStatus | null>(null);
  const [personas, setPersonas] = useState<Persona[]>([]);

  useEffect(() => {
    let cancelled = false;
    // 首屏只拉配置，尽快结束「加载中」；较重的设备枚举延后一帧，避免切页/首屏卡顿。
    ipc
      .getConfig()
      .then((c) => {
        if (!cancelled) setConfig(c);
      })
      .catch(() =>
        ipc.defaultConfig().then((c) => {
          if (!cancelled) setConfig(c);
        })
      );
    // 非首屏必需：延后到下一帧，先让配置渲染出来，避免打开窗口时一堆 IPC 抢主线程。
    let deferredHandle: number | ReturnType<typeof setTimeout> | null = null;
    const loadSecondary = () => {
      if (cancelled) return;
      ipc.getLaunchAtLogin().then((v) => {
        if (!cancelled) setAutoStart(v);
      }).catch(() => {});
      ipc.listLocalAsrModels().then((list) => {
        if (!cancelled) setAsrModels(Array.isArray(list) ? list : []);
      }).catch(() => {});
      ipc.getLocalModelStatus().then((s) => {
        if (!cancelled) setModelStatus(s);
      }).catch(() => {});
      ipc.getPolishModelStatus().then((s) => {
        if (!cancelled) setPolishStatus(s);
      }).catch(() => {});
      ipc.listPersonas().then((p) => {
        if (!cancelled) setPersonas(p);
      }).catch(() => {});
      // 麦克风枚举最重，再往后一点。
      setTimeout(() => {
        if (cancelled) return;
        ipc.listAudioDevices()
          .then((d) => {
            if (!cancelled) setDevices(Array.isArray(d) ? d : []);
          })
          .catch(() => {});
      }, 80);
    };
    if (typeof requestAnimationFrame === "function") {
      deferredHandle = requestAnimationFrame(loadSecondary);
    } else {
      deferredHandle = setTimeout(loadSecondary, 0);
    }

    // 模型下载进度 / 完成 / 失败事件。
    const unlisteners: Array<() => void> = [];
    listen<ModelDownloadProgress>("model://download-progress", (e) => {
      setDl(e.payload);
      // 以后端 target_id 为准，保证进度条挂在正在下的那张卡上。
      if (e.payload.target_id) {
        setDlTargetId(e.payload.target_id);
      }
      setDlError(null);
    }).then((u) => unlisteners.push(u));
    listen("model://download-complete", () => {
      setDl(null);
      setDlTargetId(null);
      ipc.listLocalAsrModels().then(setAsrModels).catch(() => {});
      ipc.getLocalModelStatus().then(setModelStatus).catch(() => {});
      ipc.getPolishModelStatus().then(setPolishStatus).catch(() => {});
    }).then((u) => unlisteners.push(u));
    listen<string>("model://download-error", (e) => {
      setDl(null);
      setDlTargetId(null);
      setDlError(e.payload);
      ipc.listLocalAsrModels().then(setAsrModels).catch(() => {});
      ipc.getLocalModelStatus().then(setModelStatus).catch(() => {});
      ipc.getPolishModelStatus().then(setPolishStatus).catch(() => {});
    }).then((u) => unlisteners.push(u));

    // Fn 键事件（功能测试模块）。
    listen<boolean>("fn://edge", (e) => {
      setFnCount((c) => (e.payload ? c + 1 : c));
      setFnState(e.payload ? "down" : "idle");
    }).then((u) => unlisteners.push(u));

    // 录音状态变化：实时更新测试框。
    // 语义：录音开始记下当前文本作为基准 base；partial = base + 实时识别；
    //       stopped = base + 最终识别。这样单次录音 partial 与 final 不重复，
    //       多次录音的内容依次追加到基准后。
    listen("recording://started", () => {
      testBaseRef.current = testTextRef.current;
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://partial", (e) => {
      setTestText(testBaseRef.current + (e.payload || ""));
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://stopped", (e) => {
      setRecording(false);
      setTestText(testBaseRef.current + (e.payload ? e.payload : ""));
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://error", (e) => {
      setRecording(false);
      setMsg({ ok: false, text: `录音错误：${e.payload}` });
    }).then((u) => unlisteners.push(u));

    // 权限轮询：用户可能在系统设置里随时变更，勾选后这里自动更新。
    let timer: ReturnType<typeof setTimeout> | null = null;
    const tick = async () => {
      let m: PermissionStatus | null = null;
      let a: PermissionStatus | null = null;
      try {
        m = await ipc.checkPermission("microphone" as PermissionKind);
      } catch {
        /* ignore */
      }
      try {
        a = await ipc.checkPermission("accessibility" as PermissionKind);
      } catch {
        /* ignore */
      }
      if (cancelled) return;
      setMic(m);
      setAx(a);
      if (m?.state === "granted" && a?.state === "granted") return;
      timer = setTimeout(tick, 2500);
    };
    // 权限检查也延后，不和首屏 config 抢主线程/IPC 队列
    timer = setTimeout(tick, 120);
    return () => {
      cancelled = true;
      if (deferredHandle != null) {
        if (typeof cancelAnimationFrame === "function" && typeof deferredHandle === "number") {
          cancelAnimationFrame(deferredHandle);
        } else {
          clearTimeout(deferredHandle);
        }
      }
      if (timer) clearTimeout(timer);
      unlisteners.forEach((u) => u());
    };
  }, []);

  const onToggleAutoStart = async (checked: boolean) => {
    setAutoStart(checked);
    setConfig((c) => (c ? { ...c, launch_at_login: checked } : c));
    try {
      await ipc.setLaunchAtLogin(checked);
    } catch (e) {
      setMsg({ ok: false, text: `设置开机自启失败：${e}` });
    }
  };

  const testMic = async () => {
    setTestingMic(true);
    setMicTest(null);
    try {
      const level = await ipc.testMicrophone(config?.audio_device ?? null);
      const pct = Math.round(level * 100);
      setMicTest(
        level > 0.01
          ? { ok: true, warn: false, text: `麦克风正常，检测音量约 ${pct}%` }
          : { ok: false, warn: true, text: "未检测到声音，请检查麦克风或换一个设备" }
      );
    } catch (e) {
      setMicTest({ ok: false, warn: false, text: `测试失败：${e}` });
    } finally {
      setTestingMic(false);
    }
  };

  if (!config) return <p>加载中…</p>;

  const active = config.providers[config.active_provider] ?? config.providers[0];
  const setActive = (patch: Partial<ProviderConfig>) =>
    setConfig({
      ...config,
      providers: config.providers.map((p, i) =>
        i === config.active_provider ? { ...p, ...patch } : p
      ),
    });

  const onSave = async () => {
    setSaving(true);
    setMsg(null);
    try {
      await ipc.validateProvider(active);
      await ipc.saveConfig(config);
      setMsg({ ok: true, text: "已保存" });
    } catch (e) {
      setMsg({ ok: false, text: String(e) });
    } finally {
      setSaving(false);
    }
  };

  const permBadge = (s: PermissionStatus | null) => {
    if (!s) return <span className="badge badge-warning"><span className="badge-dot" />未知</span>;
    const cls = s.state === "granted" ? "badge-success" : s.state === "denied" ? "badge-danger" : "badge-warning";
    return (
      <span className={`badge ${cls}`}>
        <span className="badge-dot" />
        {permissionLabel[s.state]}
      </span>
    );
  };

  const msgRow = (m: { ok: boolean; text: string }): ReactNode => (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 12, color: m.ok ? "var(--success)" : "var(--danger)" }}>
      <StatusIcon ok={m.ok} />
      {m.text}
    </span>
  );

  return (
    <div>
      <h1 className="page-title">设置</h1>
      <p className="page-subtitle">配置语音识别引擎、AI 润色与快捷键</p>

      {/* 二期：AI 润色 */}
      <div className="card">
        <h2 className="card-title">AI 润色（二期）</h2>
        <div className="perm-item">
          <div>
            <div className="perm-name">启用润色</div>
            <div className="perm-desc">
              识别定稿后改写上屏：去口头禅、补标点；可选人设。默认优先本地 Qwen2.5-1.5B
            </div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={!!config.polish_enabled}
              onChange={(e) => setConfig({ ...config, polish_enabled: e.target.checked })}
            />
            <span className="slider" />
          </label>
        </div>

        {config.polish_enabled && (
          <>
            <div className="field" style={{ marginTop: 14 }}>
              <label className="field-label">路由策略</label>
              <select
                value={config.polish_policy ?? "prefer_local"}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    polish_policy: e.target.value as PolishPolicy,
                  })
                }
              >
                <option value="prefer_local">优先本地（推荐）</option>
                <option value="prefer_cloud">优先云端</option>
                <option value="local_only">仅本地</option>
                <option value="cloud_only">仅云端</option>
              </select>
              <span className="field-hint">
                本地需下载约 986MB 的 Qwen2.5-1.5B-Instruct Q4_K_M；云端复用百炼 API Key
              </span>
            </div>

            <div className="field">
              <label className="field-label">人设</label>
              <select
                value={config.active_persona_id ?? ""}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    active_persona_id: e.target.value || null,
                  })
                }
              >
                <option value="">无（仅轻量润色）</option>
                {personas.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="field">
              <label className="field-label">云端模型 ID</label>
              <input
                value={config.polish_cloud_model ?? "qwen-turbo"}
                onChange={(e) =>
                  setConfig({ ...config, polish_cloud_model: e.target.value })
                }
                placeholder="qwen-turbo"
              />
            </div>

            <div className="row-between" style={{ marginTop: 8, alignItems: "flex-start" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontWeight: 600, fontSize: 13, marginBottom: 4 }}>
                  本地模型 Qwen2.5-1.5B-Instruct Q4_K_M
                </div>
                {polishStatus ? (
                  <span className="field-hint">
                    {polishStatus.installed
                      ? `已安装 · ${fmtSize(polishStatus.total_size)}`
                      : `未安装 · 约 ${fmtSize(polishStatus.total_size)}`}
                    {!polishStatus.llm_feature &&
                      " · 当前构建未开 llm feature（装好模型后仍需 cmake + --features llm 才能本地推理）"}
                  </span>
                ) : (
                  <span className="field-hint">状态加载中…</span>
                )}
              </div>
              <button
                className="btn btn-sm"
                disabled={
                  (!!dl && dlTargetId !== "polish") ||
                  polishStatus?.installed ||
                  polishStatus?.downloading
                }
                onClick={async () => {
                  try {
                    setDlError(null);
                    setDlTargetId("polish");
                    await ipc.installPolishModel();
                  } catch (e) {
                    setDlError(String(e));
                    setDlTargetId(null);
                  }
                }}
              >
                {polishStatus?.installed
                  ? "已就绪"
                  : polishStatus?.downloading || (dl && dlTargetId === "polish")
                    ? "下载中…"
                    : "下载模型"}
              </button>
            </div>
            {dl && dlTargetId === "polish" && (
              <div style={{ marginTop: 10 }}>
                <div className="field-hint" style={{ marginBottom: 4 }}>
                  {dl.message || dl.file_name} · {fmtSize(dl.total_downloaded)} /{" "}
                  {fmtSize(dl.total_size)}
                </div>
                <div
                  style={{
                    height: 6,
                    borderRadius: 3,
                    background: "var(--border)",
                    overflow: "hidden",
                  }}
                >
                  <div
                    style={{
                      height: "100%",
                      width: `${
                        dl.total_size
                          ? Math.min(100, (100 * dl.total_downloaded) / dl.total_size)
                          : 0
                      }%`,
                      background: "var(--accent)",
                      transition: "width 0.2s",
                    }}
                  />
                </div>
              </div>
            )}
            {dlError && dlTargetId === "polish" && (
              <span className="field-hint" style={{ color: "var(--danger)", display: "block", marginTop: 6 }}>
                {dlError}
              </span>
            )}
          </>
        )}
      </div>

      {/* 引擎 */}
      <div className="card">
        <h2 className="card-title">识别引擎</h2>
        <div className="field">
          <label className="field-label">引擎类型</label>
          <select
            value={active.kind}
            onChange={(e) => {
              const kind = e.target.value as ProviderConfig["kind"];
              if (kind === "sherpa") {
                const asrId = config.local_asr_model || DEFAULT_LOCAL_ASR;
                setActive({ kind, model: asrId, base_url: "", api_key: "" });
              } else {
                setActive({ kind });
              }
            }}
          >
            <option value="sherpa">sherpa-onnx（本地，离线，推荐）</option>
            <option value="bailian">通用流式 ASR（云端）</option>
          </select>
        </div>

        {active.kind === "bailian" && (
          <>
            <div className="field">
              <label className="field-label">模型</label>
              <input
                value={active.model}
                onChange={(e) => setActive({ model: e.target.value })}
                placeholder="fun-asr-realtime"
              />
              <span className="field-hint">
                填写服务商支持的模型 ID（如 fun-asr-realtime、paraformer-realtime-v2 等）
              </span>
            </div>
            <div className="field">
              <label className="field-label">WebSocket 地址</label>
              <input
                value={active.base_url}
                onChange={(e) => setActive({ base_url: e.target.value })}
                placeholder="wss://your-asr-host/api-ws/v1/inference"
              />
            </div>
            <div className="field">
              <label className="field-label">API Key</label>
              <input
                type="password"
                value={active.api_key}
                onChange={(e) => setActive({ api_key: e.target.value })}
                placeholder="sk-..."
              />
              <span className="field-hint">在 ASR 服务商控制台获取 API Key</span>
            </div>
            <div className="row-between" style={{ alignItems: "flex-end" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                {testResult && msgRow(testResult)}
              </div>
              <button
                className="btn btn-sm"
                disabled={testing}
                onClick={async () => {
                  setTesting(true);
                  setTestResult(null);
                  try {
                    const m = await ipc.testCloudConnection(active);
                    setTestResult({ ok: true, text: m });
                  } catch (e) {
                    setTestResult({ ok: false, text: String(e) });
                  } finally {
                    setTesting(false);
                  }
                }}
              >
                {testing ? "测试中…" : "测试连接"}
              </button>
            </div>
          </>
        )}

        {active.kind === "sherpa" && (
          <div style={{ marginTop: 4 }}>
            <div className="field-label" style={{ marginBottom: 8 }}>
              本地 ASR 模型
            </div>
            <span className="field-hint" style={{ display: "block", marginBottom: 10 }}>
              下载后可启用；同时只启用一个，录音时走该模型识别。完全离线，音频不出本机。
            </span>

            {(() => {
              const models = asrModels.length
                ? asrModels
                : [
                    {
                      id: "firered-large",
                      title: "FireRedASR Large",
                      description:
                        "离线整段 · 中英高精度 · 约 1.7GB · 更准更慢，适合追求识别率",
                      backend: "offline_fire_red",
                      recommended: true,
                      approx_size: 1_739_000_000,
                      installed: false,
                      active: false,
                      missing_size: 1_739_000_000,
                    },
                    {
                      id: "zipformer-zh-xlarge",
                      title: "Zipformer 中文 xlarge",
                      description:
                        "流式 xlarge int8 · 中文大模型 · 约 735MB · 比 large 更准",
                      backend: "streaming_zipformer",
                      recommended: false,
                      approx_size: 771_000_000,
                      installed: false,
                      active: false,
                      missing_size: 771_000_000,
                    },
                    {
                      id: "zipformer-zh-2025",
                      title: "Zipformer 中文 2025",
                      description:
                        "流式 large int8 · 中文 · 约 167MB · 体积与速度折中",
                      backend: "streaming_zipformer",
                      recommended: false,
                      approx_size: 167_000_000,
                      installed: false,
                      active: false,
                      missing_size: 167_000_000,
                    },
                    {
                      id: "sensevoice",
                      title: "SenseVoice",
                      description:
                        "离线整段 · 中英日韩粤 · 约 240MB · 快、省资源",
                      backend: "offline_sense_voice",
                      recommended: false,
                      approx_size: 240_000_000,
                      installed: false,
                      active: false,
                      missing_size: 240_000_000,
                    },
                  ];

              // 仅「配置选中 + 已安装」才算使用中；配置指向未下载模型时不算启用。
              const preferredId = config.local_asr_model || DEFAULT_LOCAL_ASR;
              const preferredInstalled = models.some(
                (x) => x.id === preferredId && x.installed,
              );
              const effectiveId = preferredInstalled
                ? preferredId
                : models.find((x) => x.active && x.installed)?.id ??
                  models.find((x) => x.installed)?.id ??
                  null;

              return models.map((m) => {
                const selected = !!m.installed && m.id === effectiveId;
                const isDownloadingThis =
                  (dlTargetId === m.id || dl?.target_id === m.id) &&
                  (!!dl || !!modelStatus?.downloading);
                return (
                  <div
                    key={m.id}
                    className={`local-model-card${selected ? " local-model-card--active" : ""}`}
                    style={{ marginTop: 10 }}
                  >
                    <div className="row-between" style={{ marginBottom: 6, alignItems: "flex-start" }}>
                      <div style={{ minWidth: 0, flex: 1 }}>
                        <div
                          style={{
                            fontWeight: 600,
                            marginBottom: 2,
                            display: "flex",
                            alignItems: "center",
                            gap: 8,
                            flexWrap: "wrap",
                          }}
                        >
                          {m.title}
                          {m.recommended && (
                            <span className="badge badge-success" style={{ fontSize: 11 }}>
                              推荐
                            </span>
                          )}
                          {selected && (
                            <span className="badge badge-success" style={{ fontSize: 11 }}>
                              使用中
                            </span>
                          )}
                        </div>
                        <div className="field-hint" style={{ marginBottom: 0 }}>
                          {m.description}
                        </div>
                      </div>
                      {m.installed ? (
                        <span className="badge badge-success">
                          <span className="badge-dot" />
                          已安装
                        </span>
                      ) : (
                        <span className="badge badge-warning">
                          <span className="badge-dot" />
                          未安装
                        </span>
                      )}
                    </div>

                    {isDownloadingThis && dl && dl.phase !== "done" && (
                      <div style={{ marginBottom: 8 }}>
                        <div className="row-between" style={{ marginBottom: 4 }}>
                          <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                            {dl.message}（{dl.file_index + 1}/{dl.file_count}）
                          </span>
                          <span style={{ fontSize: 12, color: "var(--text-secondary)", flexShrink: 0 }}>
                            {dl.total_size > 0
                              ? `${Math.min(100, Math.round((dl.total_downloaded / dl.total_size) * 100))}%`
                              : ""}
                            {dl.speed_bps > 0 ? ` · ${fmtSize(dl.speed_bps)}/s` : ""}
                          </span>
                        </div>
                        <div
                          style={{
                            height: 6,
                            borderRadius: 3,
                            background: "var(--border)",
                            overflow: "hidden",
                          }}
                        >
                          <div
                            style={{
                              height: "100%",
                              borderRadius: 3,
                              background: "var(--accent)",
                              width: `${
                                dl.total_size > 0
                                  ? Math.min(100, (dl.total_downloaded / dl.total_size) * 100)
                                  : 0
                              }%`,
                              transition: "width 0.2s",
                            }}
                          />
                        </div>
                      </div>
                    )}

                    <div className="row-between" style={{ marginTop: 8, gap: 8, flexWrap: "wrap" }}>
                      <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                        {m.installed
                          ? `约 ${fmtSize(m.approx_size)}`
                          : `约需下载 ${fmtSize(m.missing_size || m.approx_size)}`}
                      </span>
                      <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
                        {!m.installed && (
                          <button
                            className="btn btn-sm"
                            disabled={!!dl || modelStatus?.downloading}
                            onClick={async () => {
                              setDlError(null);
                              setDlTargetId(m.id);
                              try {
                                await ipc.installLocalModel(m.id);
                              } catch (e) {
                                setDlError(String(e));
                                setDlTargetId(null);
                              }
                            }}
                          >
                            {isDownloadingThis ? "下载中…" : "下载"}
                          </button>
                        )}
                        <button
                          className={`btn btn-sm${selected ? " btn-ghost" : ""}`}
                          disabled={!m.installed || selected}
                          title={
                            !m.installed ? "请先下载安装后再启用" : selected ? "当前已启用" : "启用该模型"
                          }
                          onClick={() => {
                            setConfig({
                              ...config,
                              local_asr_model: m.id,
                              local_mode: isOfflineAsrId(m.id) ? "offline" : "realtime",
                            });
                            setActive({ model: m.id });
                          }}
                        >
                          {selected ? "已启用" : "启用"}
                        </button>
                      </div>
                    </div>
                  </div>
                );
              });
            })()}

            {dlError && dlTargetId && dlTargetId !== "polish" && (
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 5,
                  fontSize: 12,
                  color: "var(--danger)",
                  marginTop: 8,
                }}
              >
                <StatusIcon />
                {dlError}
              </div>
            )}
          </div>
        )}
      </div>

      {/* 快捷键 */}
      <div className="card">
        <h2 className="card-title">快捷键</h2>
        <div className="field" style={{ margin: 0 }}>
          <label className="field-label">录音快捷键</label>
          <input
            value={config.hotkey}
            onChange={(e) => setConfig({ ...config, hotkey: e.target.value })}
          />
          <span className="field-hint">
            默认 Fn（🌐 键）。也可填组合键如 Alt+Shift+D。按一次开始，松开/再按停止
          </span>
          {config.hotkey.trim().toLowerCase() === "fn" && (
            <span
              className="field-hint"
              style={{ display: "flex", alignItems: "center", gap: 5, marginTop: 4, color: "var(--warning)" }}
            >
              <StatusIcon warn />
              使用 Fn 键需在 系统设置 → 键盘 → 「按下 🌐 键时」选「不执行任何操作」，
              否则系统会拦截 Fn 事件。
            </span>
          )}
        </div>
      </div>

      {/* App 行为 */}
      <div className="card">
        <div className="section-head"><Monitor /> App 行为</div>
        <div className="set-row">
          <div>
            <div className="set-name">开机时启动应用</div>
            <div className="set-desc">登录 macOS 时自动启动，静默常驻菜单栏。</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={autoStart}
              onChange={(e) => onToggleAutoStart(e.target.checked)}
            />
            <span className="slider" />
          </label>
        </div>
        <div className="set-row">
          <div>
            <div className="set-name">录音时静音其他应用</div>
            <div className="set-desc">开启后，语音输入时暂停系统音频播放，完成后自动恢复。</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={config.mute_other_audio}
              onChange={(e) => setConfig({ ...config, mute_other_audio: e.target.checked })}
            />
            <span className="slider" />
          </label>
        </div>
      </div>

      {/* 音频 */}
      <div className="card">
        <div className="section-head"><Mic /> 音频</div>
        <div className="set-row">
          <div>
            <div className="set-name">麦克风</div>
            <div className="set-desc">切换和测试当前麦克风。</div>
          </div>
          <div className="set-ctrl">
            <select
              style={{ width: 220 }}
              value={config.audio_device ?? ""}
              onChange={(e) =>
                setConfig({ ...config, audio_device: e.target.value || null })
              }
            >
              <option value="">自动检测（默认输入）</option>
              {devices.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
            <button className="btn btn-sm" onClick={testMic} disabled={testingMic}>
              {testingMic ? "测试中…" : "测试"}
            </button>
          </div>
        </div>
        {micTest && (
          <div className="field-hint" style={{ marginTop: 8, display: "flex", alignItems: "center", gap: 5 }}>
            <StatusIcon ok={micTest.ok} warn={micTest.warn} />
            {micTest.text}
          </div>
        )}
      </div>

      {/* 系统权限（唯一入口：不再在顶部横幅重复） */}
      <div className="card">
        <h2 className="card-title">系统权限</h2>
        <div className="perm-item">
          <div>
            <div className="perm-name">麦克风</div>
            <div className="perm-desc">用于采集语音</div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {permBadge(mic)}
            {mic?.state !== "granted" && (
              <>
                <button className="btn btn-sm" onClick={() => ipc.requestMicrophone()}>
                  授权
                </button>
                <button
                  className="btn btn-sm btn-ghost"
                  onClick={() => ipc.openPermissionSettings("microphone" as PermissionKind)}
                >
                  系统设置
                </button>
              </>
            )}
          </div>
        </div>
        <div className="perm-item">
          <div>
            <div className="perm-name">辅助功能</div>
            <div className="perm-desc">用于将识别文字输入到当前光标</div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {permBadge(ax)}
            {ax?.state !== "granted" && (
              <>
                <button className="btn btn-sm" onClick={() => ipc.requestAccessibility()}>
                  授权
                </button>
                <button
                  className="btn btn-sm btn-ghost"
                  onClick={() => ipc.openPermissionSettings("accessibility" as PermissionKind)}
                >
                  系统设置
                </button>
              </>
            )}
          </div>
        </div>
        {(mic?.state !== "granted" || ax?.state !== "granted") && (
          <span className="field-hint" style={{ display: "block", marginTop: 8 }}>
            请用 ./scripts/build.sh install（固定签名 openIME Local Dev）安装后再授权；
            同一证书重装一般不必再授。若系统里有旧 openIME 条目仍无效：删掉旧条目后点上方「授权」。
            tauri dev 不走稳定签名，调试时授权可能反复失效。
          </span>
        )}
      </div>

      {/* 功能测试 */}
      <div className="card">
        <h2 className="card-title">功能测试</h2>
        <span className="field-hint" style={{ display: "block", marginBottom: 12 }}>
          按 Fn（🌐）键开始录音，对麦克风说话，识别结果实时显示在下方文本框中。再按 Fn 停止。
        </span>

        {/* 状态指示 */}
        <div className="row-between" style={{ marginBottom: 12 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <div
              style={{
                width: 44,
                height: 44,
                borderRadius: 10,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                fontSize: 18,
                fontWeight: 700,
                background: fnState === "down" ? "var(--accent)" : "var(--border)",
                color: fnState === "down" ? "var(--accent-text)" : "var(--text-secondary)",
                transition: "all 0.1s",
              }}
            >
              Fn
            </div>
            <div>
              <div style={{ fontWeight: 600, fontSize: 13, display: "flex", alignItems: "center", gap: 6 }}>
                {recording ? (
                  <>
                    <CircleDot size={13} color="var(--danger)" /> 录音中…
                  </>
                ) : fnCount > 0 ? (
                  `已触发 ${fnCount} 次`
                ) : (
                  "就绪"
                )}
              </div>
              <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                {fnState === "down" ? "● Fn 按下" : "等待按键"}
              </div>
            </div>
          </div>
          <button
            className="btn btn-sm"
            onClick={async () => {
              try {
                const r = await ipc.toggleRecording();
                setRecording(r);
              } catch (e) {
                setMsg({ ok: false, text: String(e) });
              }
            }}
          >
            {recording ? "■ 停止" : (
              <>
                <Mic2 size={13} /> 手动录音
              </>
            )}
          </button>
        </div>

        {/* 语音录入测试框 */}
        <textarea
          value={testText}
          onChange={(e) => setTestText(e.target.value)}
          placeholder="识别结果会显示在这里（也可手动输入编辑）…"
          style={{
            width: "100%",
            minHeight: 100,
            padding: 12,
            borderRadius: "var(--radius-sm)",
            border: "1px solid var(--border)",
            background: "var(--card-bg)",
            color: "var(--text)",
            fontSize: 14,
            fontFamily: "inherit",
            resize: "vertical",
            outline: "none",
            lineHeight: 1.6,
          }}
        />
        <div className="row-between" style={{ marginTop: 8 }}>
          <span className="field-hint">
            {testText ? `${testText.length} 字` : "如果没反应，请确认引擎已配置并保存"}
          </span>
          {testText && (
            <button
              className="btn btn-sm btn-ghost"
              onClick={() => setTestText("")}
            >
              清空
            </button>
          )}
        </div>
      </div>

      {/* 保存 */}
      <div className="save-bar">
        <button className="btn" onClick={onSave} disabled={saving}>
          {saving ? "保存中…" : "保存设置"}
        </button>
        {msg && (
          <span className="save-msg" style={{ color: msg.ok ? "var(--success)" : "var(--danger)" }}>
            <StatusIcon ok={msg.ok} />
            <span>{msg.text}</span>
          </span>
        )}
      </div>
    </div>
  );
}
