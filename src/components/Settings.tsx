import { useEffect, useState, type ReactNode } from "react";
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
  Trash2,
} from "lucide-react";
import type {
  AppConfig,
  LocalAsrModelEntry,
  LocalModelStatus,
  ModelDownloadProgress,
  PolishModelStatus,
  PolishPolicy,
  ProviderConfig,
  StylePack,
  SystemInfo,
} from "../types";
import { ipc, permissionLabel, type PermissionKind, type PermissionStatus } from "../ipc";

// 默认本地 ASR id（与 voice-core asr_catalog 对齐；未安装时不算「使用中」）。
const DEFAULT_LOCAL_ASR = "sensevoice";

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
  // ASR 模型启用 / 删除的瞬态状态。
  const [enablingId, setEnablingId] = useState<string | null>(null);
  const [enableTip, setEnableTip] = useState<{ id: string; ok: boolean; text: string } | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  // 本机性能（给语音模型打标签）— 不显眼的"重新采集"。
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [systemRefreshing, setSystemRefreshing] = useState(false);
  const refreshSystemInfo = (force: boolean) => {
    const p = ipc.getSystemInfo(force);
    p.then(setSystemInfo).catch(() => {});
    return p;
  };
  // 功能测试：Fn 键事件 + 语音录入测试框
  const [fnCount, setFnCount] = useState(0);
  const [fnState, setFnState] = useState<"idle" | "down">("idle");
  const [testText, setTestText] = useState("");
  const [recording, setRecording] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; text: string } | null>(null);
  // 音频设备（麦克风下拉 + 测试）
  const [devices, setDevices] = useState<string[]>([]);
  const [micTest, setMicTest] = useState<{ ok: boolean; warn: boolean; text: string } | null>(null);
  const [testingMic, setTestingMic] = useState(false);
  // 二期润色
  const [polishStatus, setPolishStatus] = useState<PolishModelStatus | null>(null);
  const [stylePacks, setStylePacks] = useState<StylePack[]>([]);
  const [newStyleName, setNewStyleName] = useState("");
  const [newStylePrompt, setNewStylePrompt] = useState("");

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
      ipc.listStylePacks().then((p) => {
        if (!cancelled) setStylePacks(Array.isArray(p) ? p : []);
      }).catch(() => {});
      refreshSystemInfo(false);
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

    // 录音状态变化：只更新录音指示灯，不再直接写测试框文本。
    // 测试框的文本由 enigo 键盘模拟直接输入（设置窗口前台 + textarea 聚焦时），
    // 通过 textarea 的 onChange 自然同步到 testText。若再用 partial/stopped 事件
    // 写一遍，会和 enigo 输入重复 → 同一句出现两次。
    listen("recording://started", () => {
      setRecording(true);
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://partial", () => {
      // partial 流式增量仅用于 overlay 显示，不写测试框。
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://stopped", () => {
      setRecording(false);
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

      {/* AI 润色 */}
      <div className="card">
        <h2 className="card-title">AI 润色</h2>
        <div className="field">
          <label className="field-label">润色程度</label>
          <div style={{ display: "flex", gap: 12, marginTop: 8 }}>
            {([
              {
                v: "off",
                t: "保持原样",
                d: "不做 LLM 校对，仅本地规则清理（去口头禅/补标点/纠同音字）。",
              },
              { v: "light", t: "中度润色", d: "本地规则 + LLM 仅校对（修 ASR 错，不改措辞）。" },
              {
                v: "heavy",
                t: "高度润色",
                d: "本地规则 + LLM 改写润色（通顺化、调整语序，保留原意）。",
              },
            ] as const).map((opt) => {
              const selected = (config.polish_mode ?? "off") === opt.v;
              return (
                <div
                  key={opt.v}
                  onClick={() =>
                    setConfig({
                      ...config,
                      polish_mode: opt.v,
                      polish_enabled: opt.v !== "off",
                    })
                  }
                  style={{
                    flex: 1,
                    display: "flex",
                    flexDirection: "column",
                    gap: 6,
                    padding: "14px 12px",
                    borderRadius: 10,
                    cursor: "pointer",
                    border: selected
                      ? "2px solid var(--accent)"
                      : "1px solid var(--border)",
                    background: selected ? "var(--accent-soft)" : "transparent",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span
                      style={{
                        width: 14,
                        height: 14,
                        borderRadius: "50%",
                        border: selected
                          ? "4px solid var(--accent)"
                          : "2px solid var(--text-tertiary)",
                        flexShrink: 0,
                      }}
                    />
                    <strong style={{ fontSize: 14 }}>{opt.t}</strong>
                  </div>
                  <div
                    style={{
                      fontSize: 12,
                      color: "var(--text-secondary)",
                      lineHeight: 1.5,
                    }}
                  >
                    {opt.d}
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        {(config.polish_mode ?? "off") !== "off" && (
          <>
            {(config.polish_mode ?? "off") === "heavy" && stylePacks.length > 0 && (
              <div className="field" style={{ marginTop: 14 }}>
                <label className="field-label">风格包</label>
                <select
                  value={config.active_style_pack_id ?? ""}
                  onChange={(e) => {
                    const id = e.target.value || null;
                    setConfig({ ...config, active_style_pack_id: id });
                    ipc.setActiveStylePack(id).catch(() => {});
                  }}
                >
                  <option value="">默认 Heavy（通用润色）</option>
                  {stylePacks.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                      {p.is_builtin ? "（内置）" : ""}
                    </option>
                  ))}
                </select>
                <span className="field-hint">
                  Heavy 模式下，用所选风格包的指令替代默认润色（F1）
                </span>
              </div>
            )}

            {(config.polish_mode ?? "off") === "heavy" && (
              <div className="field" style={{ marginTop: 14 }}>
                <label className="field-label">管理风格包</label>
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {stylePacks.map((p) => (
                    <div
                      key={p.id}
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                        fontSize: 13,
                      }}
                    >
                      <span>
                        {p.name}
                        {p.is_builtin ? "（内置）" : ""}
                      </span>
                      {!p.is_builtin && (
                        <button
                          className="btn"
                          style={{ fontSize: 12, padding: "2px 8px" }}
                          onClick={async () => {
                            await ipc.deleteStylePack(p.id);
                            ipc
                              .listStylePacks()
                              .then(setStylePacks)
                              .catch(() => {});
                          }}
                        >
                          删除
                        </button>
                      )}
                    </div>
                  ))}
                </div>
                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: 6,
                    marginTop: 8,
                  }}
                >
                  <input
                    placeholder="风格包名称（如：客服回复）"
                    value={newStyleName}
                    onChange={(e) => setNewStyleName(e.target.value)}
                  />
                  <textarea
                    placeholder="system prompt 指令（如：请把内容改写成礼貌的客服回复）"
                    value={newStylePrompt}
                    onChange={(e) => setNewStylePrompt(e.target.value)}
                    rows={2}
                  />
                  <button
                    className="btn"
                    disabled={!newStyleName.trim() || !newStylePrompt.trim()}
                    onClick={async () => {
                      await ipc.upsertStylePack({
                        id: `user-${Date.now()}`,
                        name: newStyleName.trim(),
                        system_prompt: newStylePrompt.trim(),
                        is_builtin: false,
                        ord: 100,
                      });
                      setNewStyleName("");
                      setNewStylePrompt("");
                      ipc.listStylePacks().then(setStylePacks).catch(() => {});
                    }}
                  >
                    添加风格包
                  </button>
                </div>
              </div>
            )}

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
                      " · 当前构建未启用本地推理（需用 ./scripts/build.sh 重新打包）"}
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
            <div className="field" style={{ gap: 6, marginBottom: 10 }}>
              <label className="field-label" htmlFor="local-language">
                默认语言
              </label>
              <select
                id="local-language"
                value={config.local_language || "zh"}
                onChange={(e) => setConfig({ ...config, local_language: e.target.value })}
              >
                <option value="zh">中文（zh）</option>
                <option value="en">英文（en）</option>
                <option value="yue">粤语（yue）</option>
                <option value="auto">自动（auto）</option>
              </select>
              <span className="field-hint">传入各本地模型的 language 参数（SenseVoice/FunASR-Nano 直接提升识别率）</span>
            </div>

            {/* 本机信息小条 + 不显眼的"重新采集" */}
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                flexWrap: "wrap",
                marginBottom: 12,
                padding: "8px 10px",
                borderRadius: 10,
                background: "var(--accent-soft)",
                fontSize: 11,
                color: "var(--text-secondary)",
              }}
            >
              {systemInfo ? (
                <span style={{ flex: 1, minWidth: 120 }}>
                  本机：{systemInfo.cpu_brand || "未知CPU"} ·{" "}
                  {fmtSize(systemInfo.total_mem)} (可用 {fmtSize(systemInfo.avail_mem)}) ·{" "}
                  {systemInfo.os_version} · 磁盘剩余 {fmtSize(systemInfo.disk_free)}
                  {systemInfo.is_apple_silicon ? " · Apple Silicon" : ""}
                </span>
              ) : (
                <span style={{ flex: 1 }}>正在采集本机信息…</span>
              )}
              <button
                className="btn btn-sm btn-ghost"
                style={{ fontSize: 11, flexShrink: 0 }}
                disabled={systemRefreshing}
                title="重新采集本机 CPU/内存/磁盘信息并给模型打标签"
                onClick={async () => {
                  setSystemRefreshing(true);
                  try {
                    await refreshSystemInfo(true);
                    const list = await ipc.listLocalAsrModels();
                    setAsrModels(list);
                  } catch {}
                  setSystemRefreshing(false);
                }}
              >
                {systemRefreshing ? (
                  <Loader2 size={11} className="spin" />
                ) : (
                  "重新采集"
                )}
              </button>
            </div>

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
                          {m.perf_tag && (
                            <span
                              className="badge"
                              style={{
                                fontSize: 11,
                                background:
                                  m.perf_tag.kind === "suitable"
                                    ? "rgba(52, 199, 89, 0.14)"
                                    : m.perf_tag.kind === "usable"
                                      ? "rgba(255, 149, 0, 0.14)"
                                      : m.perf_tag.kind === "unknown"
                                        ? "var(--card-hover)"
                                        : "rgba(255, 59, 48, 0.12)",
                                color:
                                  m.perf_tag.kind === "suitable"
                                    ? "var(--success)"
                                    : m.perf_tag.kind === "usable"
                                      ? "var(--warning)"
                                      : m.perf_tag.kind === "unknown"
                                        ? "var(--text-tertiary)"
                                        : "var(--danger)",
                              }}
                              title={m.perf_tag.reason}
                            >
                              {m.perf_tag.tag}
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
                      <div style={{ display: "flex", gap: 8, flexShrink: 0, alignItems: "center" }}>
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
                        {m.installed && !selected && (
                          <button
                            className="btn btn-sm btn-icon"
                            title="删除该模型（释放磁盘）"
                            disabled={enablingId === m.id || deletingId === m.id}
                            onClick={async () => {
                              if (!window.confirm(`删除已安装的「${m.title}」模型文件？此操作不可撤销。`)) return;
                              setDeletingId(m.id);
                              setEnableTip(null);
                              try {
                                await ipc.deleteLocalAsrModel(m.id);
                                // 刷新列表与配置。
                                const [list, cfg] = await Promise.all([
                                  ipc.listLocalAsrModels(),
                                  ipc.getConfig(),
                                ]);
                                setAsrModels(list);
                                setConfig(cfg);
                              } catch (e) {
                                alert(`删除失败：${e}`);
                              } finally {
                                setDeletingId(null);
                              }
                            }}
                          >
                            {deletingId === m.id ? <Loader2 size={14} className="spin" /> : <Trash2 size={14} />}
                          </button>
                        )}
                        <button
                          className={`btn btn-sm${selected ? " btn-ghost" : ""}`}
                          disabled={!m.installed || selected || enablingId === m.id}
                          title={
                            !m.installed
                              ? "请先下载安装后再启用"
                              : selected
                                ? "当前已启用"
                                : "启用该模型"
                          }
                          onClick={async () => {
                            setEnablingId(m.id);
                            setEnableTip(null);
                            try {
                              await ipc.setActiveAsrModel(m.id);
                              // 后端已持久化 + 同步 local_mode/provider.model；拉回最新配置与列表。
                              const [cfg, list] = await Promise.all([ipc.getConfig(), ipc.listLocalAsrModels()]);
                              setConfig(cfg);
                              setAsrModels(list);
                              setEnableTip({ id: m.id, ok: true, text: `已启用「${m.title}」` });
                              setTimeout(() => setEnableTip((t) => (t && t.id === m.id ? null : t)), 3000);
                            } catch (e) {
                              setEnableTip({ id: m.id, ok: false, text: String(e) });
                            } finally {
                              setEnablingId(null);
                            }
                          }}
                        >
                          {enablingId === m.id ? (
                            <Loader2 size={13} className="spin" />
                          ) : selected ? (
                            "已启用"
                          ) : (
                            "启用"
                          )}
                        </button>
                      </div>
                    </div>
                    {enableTip && enableTip.id === m.id && (
                      <div
                        style={{
                          marginTop: 6,
                          fontSize: 12,
                          display: "flex",
                          alignItems: "center",
                          gap: 5,
                          color: enableTip.ok ? "var(--success)" : "var(--danger)",
                        }}
                      >
                        {enableTip.ok ? <CheckCircle2 size={13} /> : <XCircle size={13} />}
                        {enableTip.text}
                      </div>
                    )}
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
            默认 Fn（🌐 键）。也可填组合键如 Alt+Shift+D。
          </span>
          <div style={{ marginTop: 10 }}>
            <label className="field-label">触发模式（A1）</label>
            <select
              value={config.hotkey_mode ?? "toggle"}
              onChange={(e) =>
                setConfig({
                  ...config,
                  hotkey_mode: e.target.value as "toggle" | "hold",
                })
              }
            >
              <option value="toggle">切换（按一次开 / 再按一次停）</option>
              <option value="hold">按住说话（松开停止）</option>
            </select>
          </div>
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
