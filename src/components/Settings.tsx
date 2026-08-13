import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
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
  PolishCloudProtocol,
  PolishModelStatus,
  ProviderConfig,
  StylePack,
  SystemInfo,
  TranscribeResult,
} from "../types";
import { ipc, permissionLabelKey, type PermissionKind, type PermissionStatus } from "../ipc";

// 默认本地 ASR id（与 voice-core asr_catalog 对齐；未安装时不算「使用中」）。
const DEFAULT_LOCAL_ASR = "sensevoice";

// R9：「Hold 下短按 Fn 补发 🌐」仅 macOS 渲染（非 macOS 隐藏，不灰字占位）。
const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

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
  const { t } = useTranslation();
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
  // 云端润色连接测试
  const [polishTesting, setPolishTesting] = useState(false);
  const [polishTestResult, setPolishTestResult] = useState<{ ok: boolean; text: string } | null>(null);
  // 音频设备（麦克风下拉 + 测试）
  const [devices, setDevices] = useState<string[]>([]);
  const [micTest, setMicTest] = useState<{ ok: boolean; warn: boolean; text: string } | null>(null);
  const [testingMic, setTestingMic] = useState(false);
  // 二期润色
  const [polishStatus, setPolishStatus] = useState<PolishModelStatus | null>(null);
  const [stylePacks, setStylePacks] = useState<StylePack[]>([]);
  // D3 文件转录
  const [transcribing, setTranscribing] = useState(false);
  const [transcribeResult, setTranscribeResult] = useState<TranscribeResult | null>(null);
  const [transcribeProgress, setTranscribeProgress] = useState<{ done_segs: number; total_segs: number } | null>(null);
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

    // 文件转录进度（R12）。
    listen<{ done_segs: number; total_segs: number }>("transcribe://progress", (e) => {
      setTranscribeProgress(e.payload);
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
      setMsg({ ok: false, text: t("settings.recordingError", { error: e.payload }) });
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
      setMsg({ ok: false, text: t("settings.autoStartFailed", { error: e }) });
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
          ? { ok: true, warn: false, text: t("settings.audio.micOk", { pct }) }
          : { ok: false, warn: true, text: t("settings.audio.micNoSound") }
      );
    } catch (e) {
      setMicTest({ ok: false, warn: false, text: t("settings.audio.micTestFailed", { error: e }) });
    } finally {
      setTestingMic(false);
    }
  };

  if (!config) return <p>{t("common.loading")}</p>;

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
      setMsg({ ok: true, text: t("settings.saved") });
    } catch (e) {
      setMsg({ ok: false, text: String(e) });
    } finally {
      setSaving(false);
    }
  };

  const permBadge = (s: PermissionStatus | null) => {
    if (!s) return <span className="badge badge-warning"><span className="badge-dot" />{t("perm.unknown")}</span>;
    const cls = s.state === "granted" ? "badge-success" : s.state === "denied" ? "badge-danger" : "badge-warning";
    return (
      <span className={`badge ${cls}`}>
        <span className="badge-dot" />
        {t(permissionLabelKey[s.state])}
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
      <h1 className="page-title">{t("settings.title")}</h1>
      <p className="page-subtitle">{t("settings.subtitle")}</p>

      {/* AI 润色 */}
      <div className="card">
        <h2 className="card-title">{t("settings.polish.title")}</h2>
        <div className="field">
          <label className="field-label">{t("settings.polish.modeLabel")}</label>
          <div style={{ display: "flex", gap: 12, marginTop: 8 }}>
            {([
              {
                v: "off" as const,
                t: t("settings.polish.off_t"),
                d: t("settings.polish.off_d"),
              },
              {
                v: "light" as const,
                t: t("settings.polish.light_t"),
                d: t("settings.polish.light_d"),
              },
              {
                v: "heavy" as const,
                t: t("settings.polish.heavy_t"),
                d: t("settings.polish.heavy_d"),
              },
            ]).map((opt) => {
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
                <label className="field-label">{t("settings.polish.stylePackLabel")}</label>
                <select
                  value={config.active_style_pack_id ?? ""}
                  onChange={(e) => {
                    const id = e.target.value || null;
                    setConfig({ ...config, active_style_pack_id: id });
                    ipc.setActiveStylePack(id).catch(() => {});
                  }}
                >
                  <option value="">{t("settings.polish.stylePackDefault")}</option>
                  {stylePacks.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                      {p.is_builtin ? t("settings.polish.builtin") : ""}
                    </option>
                  ))}
                </select>
                <span className="field-hint">
                  {t("settings.polish.stylePackHint")}
                </span>
              </div>
            )}

            {(config.polish_mode ?? "off") === "heavy" && (
              <div className="field" style={{ marginTop: 14 }}>
                <label className="field-label">{t("settings.polish.manageStylePacks")}</label>
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
                        {p.is_builtin ? t("settings.polish.builtin") : ""}
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
                          {t("common.delete")}
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
                    placeholder={t("settings.polish.styleNamePh")}
                    value={newStyleName}
                    onChange={(e) => setNewStyleName(e.target.value)}
                  />
                  <textarea
                    placeholder={t("settings.polish.stylePromptPh")}
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
                    {t("settings.polish.addStylePack")}
                  </button>
                </div>
              </div>
            )}

            <div className="field" style={{ marginTop: 14 }}>
              <span className="field-hint">
                {t("settings.polish.policyHint")}
              </span>
            </div>

            <div className="field">
              <label className="field-label">{t("settings.polish.cloudModelIdLabel")}</label>
              <input
                value={config.polish_cloud_model ?? "qwen-turbo"}
                onChange={(e) =>
                  setConfig({ ...config, polish_cloud_model: e.target.value })
                }
                placeholder="qwen-turbo"
              />
            </div>

            <div className="field">
              <label className="field-label">{t("settings.polish.cloudProtocolLabel")}</label>
              <select
                value={config.polish_cloud_protocol ?? "openai_chat"}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    polish_cloud_protocol: e.target.value as PolishCloudProtocol,
                  })
                }
              >
                <option value="openai_chat">{t("settings.polish.cloudProtocol_openai_chat")}</option>
                <option value="anthropic">{t("settings.polish.cloudProtocol_anthropic")}</option>
                <option value="openai_responses">{t("settings.polish.cloudProtocol_openai_responses")}</option>
              </select>
            </div>

            <div className="field">
              <label className="field-label">{t("settings.polish.cloudEndpointLabel")}</label>
              <input
                value={config.polish_cloud_endpoint ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, polish_cloud_endpoint: e.target.value })
                }
                placeholder={t("settings.polish.cloudEndpointPh")}
              />
            </div>

            <div className="field">
              <label className="field-label">{t("settings.polish.cloudApiKeyLabel")}</label>
              <input
                type="password"
                value={config.polish_cloud_api_key ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, polish_cloud_api_key: e.target.value })
                }
                placeholder={t("settings.polish.cloudApiKeyPh")}
              />
            </div>

            <div className="row-between" style={{ alignItems: "flex-end" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                {polishTestResult && msgRow(polishTestResult)}
              </div>
              <button
                className="btn btn-sm"
                disabled={polishTesting}
                onClick={async () => {
                  setPolishTesting(true);
                  setPolishTestResult(null);
                  try {
                    const m = await ipc.testCloudPolish();
                    setPolishTestResult({ ok: true, text: m });
                  } catch (e) {
                    setPolishTestResult({ ok: false, text: String(e) });
                  } finally {
                    setPolishTesting(false);
                  }
                }}
              >
                {polishTesting ? t("common.testing") : t("common.testConnection")}
              </button>
            </div>

            <div className="row-between" style={{ marginTop: 8, alignItems: "flex-start" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontWeight: 600, fontSize: 13, marginBottom: 4 }}>
                  {t("settings.polish.localPolishModelTitle")}
                </div>
                {polishStatus ? (
                  <span className="field-hint">
                    {polishStatus.installed
                      ? t("settings.polish.installedSize", { size: fmtSize(polishStatus.total_size) })
                      : t("settings.polish.notInstalledSize", { size: fmtSize(polishStatus.total_size) })}
                    {!polishStatus.llm_feature && t("settings.polish.llmFeatureOff")}
                  </span>
                ) : (
                  <span className="field-hint">{t("settings.polish.statusLoading")}</span>
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
                  ? t("common.ready")
                  : polishStatus?.downloading || (dl && dlTargetId === "polish")
                    ? t("common.downloading")
                    : t("settings.polish.downloadModel")}
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

      {/* P1：R6 划词问答 */}
      <div className="card">
        <h2 className="card-title">{t("settings.qa.title")}</h2>
        <span className="field-hint" style={{ display: "block", marginBottom: 8 }}>
          {t("settings.qa.hint")}
        </span>
        <div className="set-row">
          <div>
            <div className="set-name">{t("settings.qa.saveHistoryName")}</div>
            <div className="set-desc">{t("settings.qa.saveHistoryDesc")}</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={config.qa_save_history ?? false}
              onChange={(e) =>
                setConfig({ ...config, qa_save_history: e.target.checked })
              }
            />
            <span className="slider" />
          </label>
        </div>
      </div>

      {/* P1：R5 角色 / 风格包（不藏在 Heavy 里，任何润色模式可见） */}
      <div className="card">
        <h2 className="card-title">{t("settings.roles.title")}</h2>
        <div className="set-row">
          <div>
            <div className="set-name">{t("settings.roles.enabledName")}</div>
            <div className="set-desc">{t("settings.roles.enabledDesc")}</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={config.prefix_roles_enabled ?? true}
              onChange={(e) =>
                setConfig({ ...config, prefix_roles_enabled: e.target.checked })
              }
            />
            <span className="slider" />
          </label>
        </div>
        <span className="field-hint" style={{ display: "block", marginTop: 6 }}>
          {t("settings.roles.hint")}
        </span>
        <div style={{ display: "flex", flexDirection: "column", gap: 12, marginTop: 12 }}>
          {stylePacks.map((p) => {
            const prefix = p.match_prefix ?? "";
            const isRole = !!prefix.trim();
            const savePack = async (patch: Partial<StylePack>) => {
              await ipc.upsertStylePack({
                id: p.id,
                name: p.name,
                system_prompt: p.system_prompt,
                is_builtin: p.is_builtin,
                ord: p.ord,
                match_prefix: p.match_prefix ?? null,
                provider: p.provider ?? null,
                model: p.model ?? null,
                role_kind: p.role_kind ?? "default",
                output_mode: p.output_mode ?? "insert",
                ...patch,
              });
              ipc.listStylePacks().then(setStylePacks).catch(() => {});
            };
            return (
              <div
                key={p.id}
                style={{
                  border: "1px solid var(--border)",
                  borderRadius: 10,
                  padding: "10px 12px",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    marginBottom: 8,
                    flexWrap: "wrap",
                    gap: 6,
                  }}
                >
                  <strong style={{ fontSize: 13 }}>
                    {p.name}
                    {p.is_builtin ? t("settings.polish.builtin") : ""}
                  </strong>
                  <span style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                    {isRole && (
                      <span className="badge badge-success" style={{ fontSize: 11 }}>
                        {t("settings.roles.prefixBadge", { prefix: prefix.split("|")[0] })}
                      </span>
                    )}
                    {p.role_kind === "translate" && (
                      <span className="badge" style={{ fontSize: 11 }}>
                        {t("settings.roles.translateKind")}
                      </span>
                    )}
                  </span>
                </div>
                <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
                  <input
                    key={`${p.id}-prefix`}
                    defaultValue={prefix}
                    placeholder={t("settings.roles.prefixPh")}
                    style={{ flex: 2 }}
                    onBlur={(e) => {
                      const v = e.target.value.trim();
                      if (v !== prefix) {
                        savePack({ match_prefix: v || null });
                      }
                    }}
                  />
                  <select
                    key={`${p.id}-provider`}
                    defaultValue={p.provider ?? "cloud"}
                    style={{ flex: 1 }}
                    onChange={(e) => savePack({ provider: e.target.value || null })}
                  >
                    <option value="cloud">{t("settings.roles.providerCloud")}</option>
                    <option value="local">{t("settings.roles.providerLocal")}</option>
                  </select>
                </div>
                <textarea
                  key={`${p.id}-prompt`}
                  defaultValue={p.system_prompt}
                  rows={2}
                  style={{ fontSize: 12, width: "100%" }}
                  placeholder={t("settings.roles.promptPh")}
                  onBlur={(e) => {
                    const v = e.target.value.trim();
                    if (v && v !== p.system_prompt) {
                      savePack({ system_prompt: v });
                    }
                  }}
                />
              </div>
            );
          })}
        </div>
        {!stylePacks.some((p) => !p.is_builtin) && (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 6,
              marginTop: 12,
            }}
          >
            <input
              placeholder={t("settings.polish.styleNamePh")}
              value={newStyleName}
              onChange={(e) => setNewStyleName(e.target.value)}
            />
            <textarea
              placeholder={t("settings.polish.stylePromptPh")}
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
                  match_prefix: null,
                  provider: null,
                  model: null,
                  role_kind: "default",
                  output_mode: "insert",
                });
                setNewStyleName("");
                setNewStylePrompt("");
                ipc.listStylePacks().then(setStylePacks).catch(() => {});
              }}
            >
              {t("settings.polish.addStylePack")}
            </button>
          </div>
        )}
      </div>

      {/* 引擎 */}
      <div className="card">
        <h2 className="card-title">{t("settings.engine.title")}</h2>
        <div className="field">
          <label className="field-label">{t("settings.engine.typeLabel")}</label>
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
            <option value="sherpa">{t("settings.engine.sherpa")}</option>
            <option value="bailian">{t("settings.engine.bailian")}</option>
            <option value="openai_asr">{t("settings.engine.openai_asr")}</option>
            <option value="multimodal_asr">{t("settings.engine.multimodal_asr")}</option>
          </select>
        </div>

        {active.kind === "openai_asr" || active.kind === "multimodal_asr" ? (
          <>
            <div className="field">
              <label className="field-label">{t("settings.engine.modelLabel")}</label>
              <input
                value={active.model}
                onChange={(e) => setActive({ model: e.target.value })}
                placeholder={
                  active.kind === "openai_asr"
                    ? "qwen/qwen3-asr-flash-2026-02-10"
                    : "qwen-audio-3.0-asr-flash"
                }
              />
              <span className="field-hint">
                {active.kind === "openai_asr"
                  ? t("settings.engine.modelHintOpenai")
                  : t("settings.engine.modelHintMultimodal")}
              </span>
            </div>
            <div className="field">
              <label className="field-label">{t("settings.engine.endpointLabel")}</label>
              <input
                value={active.base_url}
                onChange={(e) => setActive({ base_url: e.target.value })}
                placeholder="https://openrouter.ai/api/v1"
              />
              <span className="field-hint">
                {active.kind === "openai_asr"
                  ? t("settings.engine.endpointHintOpenai")
                  : t("settings.engine.endpointHintMultimodal")}
              </span>
            </div>
            <div className="field">
              <label className="field-label">{t("settings.engine.apiKeyLabel")}</label>
              <input
                type="password"
                value={active.api_key}
                onChange={(e) => setActive({ api_key: e.target.value })}
                placeholder="sk-..."
              />
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
                {testing ? t("common.testing") : t("common.testConnection")}
              </button>
            </div>
          </>
        ) : null}

        {active.kind === "bailian" && (
          <>
            <div className="field">
              <label className="field-label">{t("settings.engine.modelLabel")}</label>
              <input
                value={active.model}
                onChange={(e) => setActive({ model: e.target.value })}
                placeholder="fun-asr-realtime"
              />
              <span className="field-hint">
                {t("settings.engine.modelHintBailian")}
              </span>
            </div>
            <div className="field">
              <label className="field-label">{t("settings.engine.serviceAddrLabel")}</label>
              <input
                value={active.base_url}
                onChange={(e) => setActive({ base_url: e.target.value })}
                placeholder="ws-xxx.ap-southeast-1.maas.aliyuncs.com"
              />
              <span className="field-hint">
                {t("settings.engine.serviceAddrHint")}
              </span>
            </div>
            <div className="field">
              <label className="field-label">{t("settings.engine.apiKeyLabel")}</label>
              <input
                type="password"
                value={active.api_key}
                onChange={(e) => setActive({ api_key: e.target.value })}
                placeholder="sk-..."
              />
              <span className="field-hint">{t("settings.engine.apiKeyHintBailian")}</span>
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
                {testing ? t("common.testing") : t("common.testConnection")}
              </button>
            </div>
          </>
        )}

        {active.kind === "sherpa" && (
          <div style={{ marginTop: 4 }}>
            <div className="field-label" style={{ marginBottom: 8 }}>
              {t("settings.localAsr.title")}
            </div>
            <span className="field-hint" style={{ display: "block", marginBottom: 10 }}>
              {t("settings.localAsr.hint")}
            </span>
            <div className="field" style={{ gap: 6, marginBottom: 10 }}>
              <label className="field-label" htmlFor="local-language">
                {t("settings.localAsr.defaultLangLabel")}
              </label>
              <select
                id="local-language"
                value={config.local_language || "zh"}
                onChange={(e) => setConfig({ ...config, local_language: e.target.value })}
              >
                <option value="zh">{t("settings.localAsr.lang_zh")}</option>
                <option value="en">{t("settings.localAsr.lang_en")}</option>
                <option value="yue">{t("settings.localAsr.lang_yue")}</option>
                <option value="auto">{t("settings.localAsr.lang_auto")}</option>
              </select>
              <span className="field-hint">{t("settings.localAsr.defaultLangHint")}</span>
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
                  {t("settings.localAsr.machineInfo", {
                    cpu: systemInfo.cpu_brand || t("settings.localAsr.unknownCpu"),
                    mem: fmtSize(systemInfo.total_mem),
                    availLabel: t("settings.localAsr.availLabel"),
                    avail: fmtSize(systemInfo.avail_mem),
                    os: systemInfo.os_version,
                    diskLabel: t("settings.localAsr.diskLabel"),
                    disk: fmtSize(systemInfo.disk_free),
                    silicon: systemInfo.is_apple_silicon ? t("settings.localAsr.appleSilicon") : "",
                  })}
                </span>
              ) : (
                <span style={{ flex: 1 }}>{t("settings.localAsr.collecting")}</span>
              )}
              <button
                className="btn btn-sm btn-ghost"
                style={{ fontSize: 11, flexShrink: 0 }}
                disabled={systemRefreshing}
                title={t("settings.localAsr.recollectTitle")}
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
                  t("settings.localAsr.recollect")
                )}
              </button>
            </div>

            {(() => {
              const models = asrModels.length
                ? asrModels
                : [
                    {
                      id: "firered-large",
                      title: t("settings.localAsr.models.firered_large.title"),
                      description: t("settings.localAsr.models.firered_large.desc"),
                      backend: "offline_fire_red",
                      recommended: true,
                      approx_size: 1_739_000_000,
                      installed: false,
                      active: false,
                      missing_size: 1_739_000_000,
                    },
                    {
                      id: "zipformer-zh-xlarge",
                      title: t("settings.localAsr.models.zipformer_zh_xlarge.title"),
                      description: t("settings.localAsr.models.zipformer_zh_xlarge.desc"),
                      backend: "streaming_zipformer",
                      recommended: false,
                      approx_size: 771_000_000,
                      installed: false,
                      active: false,
                      missing_size: 771_000_000,
                    },
                    {
                      id: "zipformer-zh-2025",
                      title: t("settings.localAsr.models.zipformer_zh_2025.title"),
                      description: t("settings.localAsr.models.zipformer_zh_2025.desc"),
                      backend: "streaming_zipformer",
                      recommended: false,
                      approx_size: 167_000_000,
                      installed: false,
                      active: false,
                      missing_size: 167_000_000,
                    },
                    {
                      id: "sensevoice",
                      title: t("settings.localAsr.models.sensevoice.title"),
                      description: t("settings.localAsr.models.sensevoice.desc"),
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
                              {t("settings.localAsr.recommended")}
                            </span>
                          )}
                          {selected && (
                            <span className="badge badge-success" style={{ fontSize: 11 }}>
                              {t("settings.localAsr.inUse")}
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
                          {t("settings.localAsr.installedBadge")}
                        </span>
                      ) : (
                        <span className="badge badge-warning">
                          <span className="badge-dot" />
                          {t("settings.localAsr.notInstalledBadge")}
                        </span>
                      )}
                    </div>

                    {isDownloadingThis && dl && dl.phase !== "done" && (
                      <div style={{ marginBottom: 8 }}>
                        <div className="row-between" style={{ marginBottom: 4 }}>
                          <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                            {t("settings.localAsr.dlProgress", { msg: dl.message, i: dl.file_index + 1, n: dl.file_count })}
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
                          ? t("settings.localAsr.approxSize", { size: fmtSize(m.approx_size) })
                          : t("settings.localAsr.needDownloadSize", { size: fmtSize(m.missing_size || m.approx_size) })}
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
                            {isDownloadingThis ? t("common.downloading") : t("settings.localAsr.downloadBtn")}
                          </button>
                        )}
                        {m.installed && !selected && (
                          <button
                            className="btn btn-sm btn-icon"
                            title={t("settings.localAsr.deleteModelTitle")}
                            disabled={enablingId === m.id || deletingId === m.id}
                            onClick={async () => {
                              if (!window.confirm(t("settings.localAsr.confirmDeleteModel", { title: m.title }))) return;
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
                                alert(t("settings.localAsr.deleteFailed", { error: e }));
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
                              ? t("settings.localAsr.enableTipInstallFirst")
                              : selected
                                ? t("settings.localAsr.enableTipCurrent")
                                : t("settings.localAsr.enableTipAction")
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
                              setEnableTip({ id: m.id, ok: true, text: t("settings.localAsr.enabledTip", { title: m.title }) });
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
                            t("settings.localAsr.enabled")
                          ) : (
                            t("settings.localAsr.enable")
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
        <h2 className="card-title">{t("settings.hotkey.title")}</h2>
        <div className="field" style={{ margin: 0 }}>
          <label className="field-label">{t("settings.hotkey.recordLabel")}</label>
          <input
            value={config.hotkey}
            onChange={(e) => setConfig({ ...config, hotkey: e.target.value })}
          />
          <span className="field-hint">
            {t("settings.hotkey.recordHint")}
          </span>
          <div style={{ marginTop: 10 }}>
            <label className="field-label">{t("settings.hotkey.modeLabel")}</label>
            <select
              value={config.hotkey_mode ?? "toggle"}
              onChange={(e) =>
                setConfig({
                  ...config,
                  hotkey_mode: e.target.value as "toggle" | "hold",
                })
              }
            >
              <option value="toggle">{t("settings.hotkey.mode_toggle")}</option>
              <option value="hold">{t("settings.hotkey.mode_hold")}</option>
            </select>
          </div>
          <div style={{ marginTop: 10 }}>
            <label className="field-label">{t("settings.hotkey.shortPressLabel")}</label>
            <input
              type="number"
              min={100}
              max={800}
              value={config.short_press_ms ?? 300}
              onChange={(e) =>
                setConfig({ ...config, short_press_ms: Number(e.target.value) })
              }
            />
            <span className="field-hint">{t("settings.hotkey.shortPressHint")}</span>
          </div>
          {IS_MAC && (
            <div className="set-row" style={{ marginTop: 12 }}>
              <div>
                <div className="set-name">{t("settings.hotkey.fnRepostName")}</div>
                <div className="set-desc">{t("settings.hotkey.fnRepostDesc")}</div>
              </div>
              <label className="switch">
                <input
                  type="checkbox"
                  checked={config.fn_repost_enabled ?? true}
                  onChange={(e) =>
                    setConfig({ ...config, fn_repost_enabled: e.target.checked })
                  }
                />
                <span className="slider" />
              </label>
            </div>
          )}
          <div style={{ marginTop: 10 }}>
            <label className="field-label">{t("settings.hotkey.styleSwitchLabel")}</label>
            <input
              value={config.style_switch_hotkey ?? ""}
              onChange={(e) =>
                setConfig({
                  ...config,
                  style_switch_hotkey: e.target.value || null,
                })
              }
              placeholder={t("settings.hotkey.styleSwitchPh")}
            />
          </div>
          <div style={{ marginTop: 10 }}>
            <label className="field-label">{t("settings.hotkey.translateLabel")}</label>
            <input
              value={config.translate_hotkey ?? ""}
              onChange={(e) =>
                setConfig({
                  ...config,
                  translate_hotkey: e.target.value || null,
                })
              }
              placeholder={t("settings.hotkey.translatePh")}
            />
            <span className="field-hint">{t("settings.hotkey.translateHint")}</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <label className="field-label">{t("settings.hotkey.qaLabel")}</label>
            <input
              value={config.qa_hotkey ?? ""}
              onChange={(e) =>
                setConfig({
                  ...config,
                  qa_hotkey: e.target.value || null,
                })
              }
              placeholder={t("settings.hotkey.qaPh")}
            />
            <span className="field-hint">{t("settings.hotkey.qaHint")}</span>
          </div>
          {config.hotkey.trim().toLowerCase() === "fn" && (
            <span
              className="field-hint"
              style={{ display: "flex", alignItems: "center", gap: 5, marginTop: 4, color: "var(--warning)" }}
            >
              <StatusIcon warn />
              {t("settings.hotkey.fnWarning")}
            </span>
          )}
        </div>
      </div>

      {/* P1：R4 翻译 */}
      <div className="card">
        <h2 className="card-title">{t("settings.translate.title")}</h2>
        <div className="field">
          <label className="field-label">{t("settings.translate.targetLangLabel")}</label>
          <select
            value={config.translate_target_lang ?? "en"}
            onChange={(e) =>
              setConfig({ ...config, translate_target_lang: e.target.value })
            }
          >
            <option value="zh">{t("settings.translate.lang_zh")}</option>
            <option value="en">{t("settings.translate.lang_en")}</option>
            <option value="ja">{t("settings.translate.lang_ja")}</option>
            <option value="ko">{t("settings.translate.lang_ko")}</option>
            <option value="fr">{t("settings.translate.lang_fr")}</option>
            <option value="de">{t("settings.translate.lang_de")}</option>
            <option value="es">{t("settings.translate.lang_es")}</option>
          </select>
          <span className="field-hint">{t("settings.translate.targetLangHint")}</span>
        </div>
        <div className="set-row">
          <div>
            <div className="set-name">{t("settings.translate.polishFirstName")}</div>
            <div className="set-desc">{t("settings.translate.polishFirstDesc")}</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={config.translate_with_polish ?? false}
              onChange={(e) =>
                setConfig({ ...config, translate_with_polish: e.target.checked })
              }
            />
            <span className="slider" />
          </label>
        </div>
      </div>

      {/* P1：R7 插入策略 + 剪贴板恢复 */}
      <div className="card">
        <h2 className="card-title">{t("settings.insert.title")}</h2>
        <div className="field">
          <label className="field-label">{t("settings.insert.strategyLabel")}</label>
          <select
            value={config.insert_strategy ?? "auto"}
            onChange={(e) =>
              setConfig({
                ...config,
                insert_strategy: e.target.value as "auto" | "type" | "paste",
              })
            }
          >
            <option value="auto">{t("settings.insert.strategy_auto")}</option>
            <option value="type">{t("settings.insert.strategy_type")}</option>
            <option value="paste">{t("settings.insert.strategy_paste")}</option>
          </select>
          <span className="field-hint">{t("settings.insert.strategyHint")}</span>
        </div>
        <div className="field">
          <label className="field-label">{t("settings.insert.fallbackAppsLabel")}</label>
          <input
            value={(config.paste_fallback_apps ?? []).join(", ")}
            onChange={(e) =>
              setConfig({
                ...config,
                paste_fallback_apps: e.target.value
                  .split(/[,，]/)
                  .map((s) => s.trim())
                  .filter(Boolean),
              })
            }
            placeholder={t("settings.insert.fallbackAppsPh")}
          />
          <span className="field-hint">{t("settings.insert.fallbackAppsHint")}</span>
        </div>
        <div className="set-row">
          <div>
            <div className="set-name">{t("settings.insert.restoreName")}</div>
            <div className="set-desc">{t("settings.insert.restoreDesc")}</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={config.restore_clipboard ?? true}
              onChange={(e) =>
                setConfig({ ...config, restore_clipboard: e.target.checked })
              }
            />
            <span className="slider" />
          </label>
        </div>
      </div>

      {/* D3 文件转录 */}
      <div className="card">
        <h2 className="card-title">{t("settings.transcribe.title")}</h2>
        <div className="field" style={{ display: "flex", gap: 12, marginTop: 8 }}>
          <div style={{ flex: 1 }}>
            <label className="field-label">{t("settings.transcribe.segDurationLabel")}</label>
            <input
              type="number"
              min={10}
              max={180}
              value={config.file_seg_duration_secs ?? 60}
              onChange={(e) =>
                setConfig({ ...config, file_seg_duration_secs: Number(e.target.value) })
              }
            />
          </div>
          <div style={{ flex: 1 }}>
            <label className="field-label">{t("settings.transcribe.segOverlapLabel")}</label>
            <input
              type="number"
              min={1}
              max={30}
              value={config.file_seg_overlap_secs ?? 4}
              onChange={(e) =>
                setConfig({ ...config, file_seg_overlap_secs: Number(e.target.value) })
              }
            />
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
          <button
            className="btn"
            disabled={transcribing}
            onClick={async () => {
              const selected = await open({
                multiple: false,
                filters: [{ name: t("settings.transcribe.audioFilter"), extensions: ["mp3", "wav", "flac", "ogg", "m4a"] }],
              });
              const path = typeof selected === "string" ? selected : null;
              if (!path) return;
              setTranscribing(true);
              setTranscribeResult(null);
              setTranscribeProgress(null);
              try {
                const r = await ipc.transcribeFile(path);
                setTranscribeResult(r);
              } catch (e) {
                alert(t("settings.transcribe.failed", { error: String(e) }));
              } finally {
                setTranscribing(false);
                setTranscribeProgress(null);
              }
            }}
          >
            {transcribing ? t("settings.transcribe.transcribing") : t("settings.transcribe.selectFile")}
          </button>
          {transcribing && (
            <button className="btn btn-ghost" onClick={() => ipc.cancelTranscribe()}>
              {t("settings.transcribe.cancel")}
            </button>
          )}
        </div>
        {transcribing && transcribeProgress && (
          <div className="field-hint" style={{ marginTop: 8 }}>
            {t("settings.transcribe.progress", {
              done: transcribeProgress.done_segs,
              total: transcribeProgress.total_segs,
            })}
          </div>
        )}
        {transcribeResult && (
          <>
            <textarea
              value={transcribeResult.text}
              readOnly
              rows={6}
              style={{ marginTop: 12, fontSize: 13 }}
            />
            <button
              className="btn"
              style={{ marginTop: 8 }}
              onClick={() => {
                const blob = new Blob([transcribeResult.srt], {
                  type: "text/plain",
                });
                const url = URL.createObjectURL(blob);
                const a = document.createElement("a");
                a.href = url;
                a.download = transcribeResult.file_name.replace(
                  /\.[^.]+$/,
                  ""
                ) + ".srt";
                a.click();
                URL.revokeObjectURL(url);
              }}
            >
              {t("settings.transcribe.exportSrt")}
            </button>
          </>
        )}
      </div>

      {/* App 行为 */}
      <div className="card">
        <div className="section-head"><Monitor /> {t("settings.appBehavior.title")}</div>
        <div className="set-row">
          <div>
            <div className="set-name">{t("settings.appBehavior.launchName")}</div>
            <div className="set-desc">{t("settings.appBehavior.launchDesc")}</div>
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
            <div className="set-name">{t("settings.appBehavior.muteName")}</div>
            <div className="set-desc">{t("settings.appBehavior.muteDesc")}</div>
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
        <div className="section-head"><Mic /> {t("settings.audio.title")}</div>
        <div className="set-row">
          <div>
            <div className="set-name">{t("settings.audio.micName")}</div>
            <div className="set-desc">{t("settings.audio.micDesc")}</div>
          </div>
          <div className="set-ctrl">
            <select
              style={{ width: 220 }}
              value={config.audio_device ?? ""}
              onChange={(e) =>
                setConfig({ ...config, audio_device: e.target.value || null })
              }
            >
              <option value="">{t("settings.audio.autoDetect")}</option>
              {devices.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
            <button className="btn btn-sm" onClick={testMic} disabled={testingMic}>
              {testingMic ? t("common.testing") : t("common.test")}
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
        <h2 className="card-title">{t("settings.permission.title")}</h2>
        <div className="perm-item">
          <div>
            <div className="perm-name">{t("settings.permission.micName")}</div>
            <div className="perm-desc">{t("settings.permission.micDesc")}</div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {permBadge(mic)}
            {mic?.state !== "granted" && (
              <>
                <button className="btn btn-sm" onClick={() => ipc.requestMicrophone()}>
                  {t("settings.permission.authorize")}
                </button>
                <button
                  className="btn btn-sm btn-ghost"
                  onClick={() => ipc.openPermissionSettings("microphone" as PermissionKind)}
                >
                  {t("settings.permission.openSysSettings")}
                </button>
              </>
            )}
          </div>
        </div>
        <div className="perm-item">
          <div>
            <div className="perm-name">{t("settings.permission.axName")}</div>
            <div className="perm-desc">{t("settings.permission.axDesc")}</div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {permBadge(ax)}
            {ax?.state !== "granted" && (
              <>
                <button className="btn btn-sm" onClick={() => ipc.requestAccessibility()}>
                  {t("settings.permission.authorize")}
                </button>
                <button
                  className="btn btn-sm btn-ghost"
                  onClick={() => ipc.openPermissionSettings("accessibility" as PermissionKind)}
                >
                  {t("settings.permission.openSysSettings")}
                </button>
              </>
            )}
          </div>
        </div>
        {(mic?.state !== "granted" || ax?.state !== "granted") && (
          <span className="field-hint" style={{ display: "block", marginTop: 8 }}>
            {t("settings.permission.hint")}
          </span>
        )}
      </div>

      {/* 功能测试 */}
      <div className="card">
        <h2 className="card-title">{t("settings.fnTest.title")}</h2>
        <span className="field-hint" style={{ display: "block", marginBottom: 12 }}>
          {t("settings.fnTest.hint")}
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
                    <CircleDot size={13} color="var(--danger)" /> {t("settings.fnTest.recording")}
                  </>
                ) : fnCount > 0 ? (
                  t("settings.fnTest.triggered", { count: fnCount })
                ) : (
                  t("settings.fnTest.ready")
                )}
              </div>
              <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                {fnState === "down" ? t("settings.fnTest.fnDown") : t("settings.fnTest.waitingKey")}
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
            {recording ? t("settings.fnTest.stop") : (
              <>
                <Mic2 size={13} /> {t("settings.fnTest.manualRecord")}
              </>
            )}
          </button>
        </div>

        {/* 语音录入测试框 */}
        <textarea
          value={testText}
          onChange={(e) => setTestText(e.target.value)}
          placeholder={t("settings.fnTest.resultPh")}
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
            {testText ? t("settings.fnTest.charCount", { count: testText.length }) : t("settings.fnTest.noResponseHint")}
          </span>
          {testText && (
            <button
              className="btn btn-sm btn-ghost"
              onClick={() => setTestText("")}
            >
              {t("settings.fnTest.clear")}
            </button>
          )}
        </div>
      </div>

      {/* 保存 */}
      <div className="save-bar">
        <button className="btn" onClick={onSave} disabled={saving}>
          {saving ? t("common.saving") : t("settings.saveBtn")}
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
