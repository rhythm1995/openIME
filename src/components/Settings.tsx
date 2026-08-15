import { useEffect, useRef, useState, type ReactNode } from "react";
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
  LlmModelEntry,
  LocalAsrModelEntry,
  LocalModelStatus,
  ModelDownloadProgress,
  ModelSuiteInfo,
  PolishCloudProtocol,
  ProviderConfig,
  StylePack,
  SystemInfo,
  TranscribeResult,
  WindowsImeStatus,
} from "../types";
import { ipc, permissionLabelKey, type PermissionKind, type PermissionStatus } from "../ipc";

// 默认本地 ASR id（与 voice-core asr_catalog 对齐；未安装时不算「使用中」）。
const DEFAULT_LOCAL_ASR = "sensevoice";

// 翻译目标语言分档（与 voice-core prompts::lang_display_name / lang_english_name 对齐）：
// 基础 7 语 = 润色模型兼译可靠档；扩展集 = 云端与本地专翻（MiLMMT-46 / HY-MT）可选语种。
const TRANSLATE_LANGS_BASIC: readonly string[] = ["zh", "en", "ja", "ko", "fr", "de", "es"];
const TRANSLATE_LANGS_FULL: readonly string[] = [
  "en", "fr", "ar", "de", "th", "ko", "tr", "es", "ru",
  "pt-br", "pt-pt", "id", "hi", "vi", "pl", "uk", "fa", "uz",
  "zh-hant", "yue",
];
const TRANSLATE_LANG_I18N_KEYS: Record<string, string> = {
  zh: "lang_zh", en: "lang_en", ja: "lang_ja", ko: "lang_ko", fr: "lang_fr",
  de: "lang_de", es: "lang_es", ar: "lang_ar", th: "lang_th", tr: "lang_tr",
  ru: "lang_ru", "pt-br": "lang_pt_br", "pt-pt": "lang_pt_pt", id: "lang_id",
  hi: "lang_hi", vi: "lang_vi", pl: "lang_pl", uk: "lang_uk", fa: "lang_fa",
  uz: "lang_uz", "zh-hant": "lang_zh_hant", yue: "lang_yue",
};

// R9：「Hold 下短按 Fn 补发 🌐」仅 macOS 渲染（非 macOS 隐藏，不灰字占位）。
const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
// 系统权限提示文案随平台切换（Windows 无辅助功能授权概念，且深链走 ms-settings 设置页）。
const IS_WIN = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

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

/// JS KeyboardEvent.key → 后端 parse_shortcut 接受的键名（无法表达返回 null）。
function shortcutKeyLabel(key: string): string | null {
  if (key === " ") return "Space";
  if (/^[a-zA-Z]$/.test(key)) return key.toUpperCase();
  if (/^[0-9]$/.test(key)) return key;
  if (/^F([1-9]|1[0-2])$/.test(key)) return key;
  switch (key) {
    case "Enter":
      return "Enter";
    case "Tab":
      return "Tab";
    case "Backspace":
      return "Backspace";
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    case ";":
    case ",":
    case ".":
    case "/":
    case "'":
    case "[":
    case "]":
    case "=":
    case "-":
      return key;
    default:
      return null;
  }
}

/// 按键直接捕获的快捷键设置（替代手填录入框）：
/// 点按钮进入捕获态 → 按下组合键/单键即写入；Esc 取消；捕获期间后端挂起
/// 录音键与全局快捷键，避免旧快捷键被误触发。
function HotkeyCaptureInput({
  value,
  onChange,
  allowSingle = false,
  optional = false,
  presets = [],
}: {
  value: string | null | undefined;
  onChange: (v: string | null) => void;
  allowSingle?: boolean;
  optional?: boolean;
  presets?: string[];
}) {
  const { t } = useTranslation();
  const [capturing, setCapturing] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);

  useEffect(() => {
    if (!capturing) return;
    ipc.setCaptureSuspend(true).catch(() => {});
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      // Esc = 取消（值不变）。
      if (e.key === "Escape") {
        setCapturing(false);
        setPreview(null);
        ipc.setCaptureSuspend(false).catch(() => {});
        return;
      }
      const mods: string[] = [];
      if (e.ctrlKey) mods.push("Ctrl");
      if (e.altKey) mods.push("Alt");
      if (e.shiftKey) mods.push("Shift");
      if (e.metaKey) mods.push("Cmd");
      // 只按了修饰键：显示半成品，等主键。
      if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) {
        setPreview(mods.length ? `${mods.join("+")}+…` : null);
        return;
      }
      if (e.key === "CapsLock") {
        if (allowSingle) {
          onChange("CapsLock");
          setCapturing(false);
          setPreview(null);
          ipc.setCaptureSuspend(false).catch(() => {});
        }
        return; // 组合键字段不允许单键：忽略。
      }
      const label = shortcutKeyLabel(e.key);
      if (label === null) return;
      onChange([...mods, label].join("+"));
      setCapturing(false);
      setPreview(null);
      ipc.setCaptureSuspend(false).catch(() => {});
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      ipc.setCaptureSuspend(false).catch(() => {});
    };
  }, [capturing, allowSingle, onChange]);

  const current = value?.trim() ?? "";
  return (
    <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
      <button
        type="button"
        className="btn btn-sm"
        onClick={() => {
          setPreview(null);
          setCapturing(true);
        }}
      >
        {capturing
          ? (preview ?? t("settings.hotkey.capturing"))
          : current || t("settings.hotkey.unset")}
      </button>
      {presets.map((p) => (
        <button
          key={p}
          type="button"
          className="btn btn-sm btn-ghost"
          title={t("settings.hotkey.presetTitle", { key: p })}
          onClick={() => {
            setCapturing(false);
            setPreview(null);
            onChange(p);
          }}
        >
          {p}
        </button>
      ))}
      {optional && current && (
        <button
          type="button"
          className="btn btn-sm btn-ghost"
          title={t("settings.hotkey.clear")}
          onClick={() => onChange(null)}
        >
          ×
        </button>
      )}
    </div>
  );
}

export default function Settings({ view = "voice" }: { view?: "voice" | "ai" }) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<AppConfig | null>(null);
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
  // R11：Windows TSF 输入法状态（仅 Windows 拉取）。
  const [tsfStatus, setTsfStatus] = useState<WindowsImeStatus | null>(null);
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
  // 二期润色 / 本地三件套
  const [polishModels, setPolishModels] = useState<LlmModelEntry[]>([]);
  const [translateModels, setTranslateModels] = useState<LlmModelEntry[]>([]);
  const [suiteInfo, setSuiteInfo] = useState<ModelSuiteInfo | null>(null);
  const [stylePacks, setStylePacks] = useState<StylePack[]>([]);
  // D3 文件转录
  const [transcribing, setTranscribing] = useState(false);
  const [transcribeResult, setTranscribeResult] = useState<TranscribeResult | null>(null);
  const [transcribeProgress, setTranscribeProgress] = useState<{ done_segs: number; total_segs: number } | null>(null);
  const [newStyleName, setNewStyleName] = useState("");
  const [newStylePrompt, setNewStylePrompt] = useState("");
  // 角色 / 风格包 master-detail：列表选中项、新建草稿态、删除二次确认。
  const [selectedPackId, setSelectedPackId] = useState<string | null>(null);
  const [draftOpen, setDraftOpen] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  // 即改即存：config 变化后防抖自动保存（500ms 合并连续修改）。
  // 校验链路与原手动保存一致：validateProvider（纯内存校验）→ saveConfig
  // （后端 save_app_config 再做 endpoint/hotkey 校验，失败整单不落盘）。
  // 首次加载只记基线不回写；保存成功短暂提示，失败保留到下次成功。
  const savedSigRef = useRef<string | null>(null);
  useEffect(() => {
    if (!config) return;
    const sig = JSON.stringify(config);
    if (savedSigRef.current === null) {
      savedSigRef.current = sig;
      return;
    }
    if (sig === savedSigRef.current) return;
    const timer = setTimeout(() => {
      (async () => {
        try {
          await ipc.validateProvider(config.providers[config.active_provider]);
          await ipc.saveConfig(config);
          savedSigRef.current = sig;
          setMsg({ ok: true, text: t("settings.autoSaved") });
        } catch (e) {
          setMsg({ ok: false, text: String(e) });
        }
      })();
    }, 500);
    return () => clearTimeout(timer);
  }, [config, t]);
  useEffect(() => {
    if (!msg?.ok) return;
    const h = setTimeout(() => setMsg(null), 1800);
    return () => clearTimeout(h);
  }, [msg]);

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
    // 启动竞态兜底：前端挂载早于后端 setup（app.manage(state)），首帧 IPC 会报
    // "state not managed"。对这些「状态拉取」做有界重试，失败静默会误显示兜底目录
    // （全部未安装 + 下载按钮）。5 次 × 800ms 覆盖 setup 完成窗口。
    const fetchWithRetry = <T,>(
      fn: () => Promise<T>,
      attempts = 5,
      delay = 800,
    ): Promise<T> =>
      fn().catch((e) => {
        if (attempts <= 1) throw e;
        return new Promise<T>((resolve, reject) => {
          setTimeout(
            () => fetchWithRetry(fn, attempts - 1, delay).then(resolve, reject),
            delay,
          );
        });
      });
    const loadSecondary = () => {
      if (cancelled) return;
      ipc.getLaunchAtLogin().then((v) => {
        if (!cancelled) setAutoStart(v);
      }).catch(() => {});
      fetchWithRetry(() => ipc.listLocalAsrModels()).then((list) => {
        if (!cancelled) setAsrModels(Array.isArray(list) ? list : []);
      }).catch(() => {});
      fetchWithRetry(() => ipc.getLocalModelStatus()).then((s) => {
        if (!cancelled) setModelStatus(s);
      }).catch(() => {});
      fetchWithRetry(() => ipc.listLocalPolishModels()).then((l) => {
        if (!cancelled) setPolishModels(Array.isArray(l) ? l : []);
      }).catch(() => {});
      fetchWithRetry(() => ipc.listLocalTranslateModels()).then((l) => {
        if (!cancelled) setTranslateModels(Array.isArray(l) ? l : []);
      }).catch(() => {});
      fetchWithRetry(() => ipc.getModelSuiteInfo()).then((s) => {
        if (!cancelled) setSuiteInfo(s);
      }).catch(() => {});
      ipc.listStylePacks().then((p) => {
        if (!cancelled) setStylePacks(Array.isArray(p) ? p : []);
      }).catch(() => {});
      refreshSystemInfo(false);
      // R11：TSF 输入法状态（仅 Windows；失败静默）。
      if (IS_WIN) {
        ipc.windowsImeStatus().then(setTsfStatus).catch(() => {});
      }
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
      ipc.listLocalPolishModels().then((l) => setPolishModels(Array.isArray(l) ? l : [])).catch(() => {});
      ipc.listLocalTranslateModels().then((l) => setTranslateModels(Array.isArray(l) ? l : [])).catch(() => {});
    }).then((u) => unlisteners.push(u));
    listen<string>("model://download-error", (e) => {
      setDl(null);
      setDlTargetId(null);
      setDlError(e.payload);
      ipc.listLocalAsrModels().then(setAsrModels).catch(() => {});
      ipc.getLocalModelStatus().then(setModelStatus).catch(() => {});
      ipc.listLocalPolishModels().then((l) => setPolishModels(Array.isArray(l) ? l : [])).catch(() => {});
      ipc.listLocalTranslateModels().then((l) => setTranslateModels(Array.isArray(l) ? l : [])).catch(() => {});
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

  // 单键录音键（macOS Fn / Windows CapsLock）：补发开关、功能测试徽标按此切换文案。
  // 与后端 fn_policy::parse_watch_key 的规范化规则保持一致（空格/下划线/连字符）。
  const hotkeyNorm = (config.hotkey ?? "").trim().toLowerCase().replace(/[\s_-]+/g, "");
  const isFnHotkey = hotkeyNorm === "fn" || hotkeyNorm === "globe";
  const isCapsHotkey = hotkeyNorm === "capslock" || hotkeyNorm === "caps";
  const singleKeyLabel = isCapsHotkey ? "Caps" : "Fn";
  // 键位文案插值：提示文案里的键名跟随用户当前录音键（改键位后文案不误导）。
  const hotkeyDisplay = isFnHotkey ? "Fn" : isCapsHotkey ? "CapsLock" : (config.hotkey ?? "Fn");

  const active = config.providers[config.active_provider] ?? config.providers[0];
  const setActive = (patch: Partial<ProviderConfig>) =>
    setConfig({
      ...config,
      providers: config.providers.map((p, i) =>
        i === config.active_provider ? { ...p, ...patch } : p
      ),
    });

  // 角色 / 风格包 master-detail：当前选中项（未显式选择时取第一个）。
  const selectedPack: StylePack | null =
    stylePacks.find((p) => p.id === selectedPackId) ?? stylePacks[0] ?? null;

  const savePack = async (p: StylePack, patch: Partial<StylePack>) => {
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

  // 模型卡片排序：按机器性能的推荐档第一，其次 适合 → 可用 → 不推荐，保持目录原序。
  const perfRank = (m: LlmModelEntry) =>
    m.perf_tag?.kind === "suitable"
      ? 0
      : m.perf_tag?.kind === "usable"
        ? 1
        : m.perf_tag?.kind === "unknown"
          ? 2
          : 3;
  const sortedPolishModels = [...polishModels].sort(
    (a, b) => Number(b.recommended) - Number(a.recommended) || perfRank(a) - perfRank(b),
  );
  const sortedTranslateModels = [...translateModels].sort(
    (a, b) => Number(b.recommended) - Number(a.recommended) || perfRank(a) - perfRank(b),
  );

  // 目标语言分档：云端策略或已选本地专翻 → 扩展语种集；本地且无专翻（润色模型兼译）→ 基础 7 语。
  const translateDedicatedActive = (config.translate_local_model ?? "").trim() !== "";
  const translateLangsFull =
    translateDedicatedActive || (config.translate_policy ?? "prefer_cloud") === "prefer_cloud";
  const translateLangOptions: readonly string[] = translateLangsFull
    ? TRANSLATE_LANGS_FULL
    : TRANSLATE_LANGS_BASIC;
  const translateTargetLang = config.translate_target_lang ?? "en";
  // 现值不在当前档（如从云端切到本地兼译时仍选中扩展语种）：追加为附加项，不静默改配置。
  const translateLangExtra =
    !translateLangOptions.includes(translateTargetLang) &&
    TRANSLATE_LANG_I18N_KEYS[translateTargetLang]
      ? translateTargetLang
      : null;

  return (
    <div>
      <h1 className="page-title">{t("settings.title")}</h1>
      <p className="page-subtitle">{t("settings.subtitle")}</p>

      {/* 快捷键与触发（语音配置·第一张卡；触发模式是第一项） */}
      {view === "voice" && (
      <div className="card">
        <h2 className="card-title">{t("settings.hotkey.title")}</h2>
        <div className="field">
          <label className="field-label">{t("settings.hotkey.modeLabel")}</label>
          <select
            value={config.hotkey_mode ?? "hold"}
            onChange={(e) =>
              setConfig({
                ...config,
                hotkey_mode: e.target.value as "toggle" | "hold",
              })
            }
          >
            <option value="hold">{t("settings.hotkey.mode_hold")}</option>
            <option value="toggle">{t("settings.hotkey.mode_toggle")}</option>
          </select>
        </div>
        <div className="field">
          <label className="field-label">{t("settings.hotkey.recordLabel")}</label>
          <HotkeyCaptureInput
            value={config.hotkey}
            onChange={(v) => v && setConfig({ ...config, hotkey: v })}
            allowSingle
            presets={IS_MAC ? ["Fn"] : IS_WIN ? ["CapsLock"] : []}
          />
          <span className="field-hint">
            {t(IS_WIN ? "settings.hotkey.recordHintWin" : "settings.hotkey.recordHint")}
          </span>
        </div>
        <div className="field">
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
          <span className="field-hint">
            {t("settings.hotkey.shortPressHint", { key: hotkeyDisplay })}
          </span>
        </div>
        {(IS_MAC || isCapsHotkey) && (
          <div className="set-row">
            <div>
              <div className="set-name">
                {t(IS_WIN ? "settings.hotkey.fnRepostNameWin" : "settings.hotkey.fnRepostName", { key: hotkeyDisplay })}
              </div>
              <div className="set-desc">
                {t(IS_WIN ? "settings.hotkey.fnRepostDescWin" : "settings.hotkey.fnRepostDesc", { key: hotkeyDisplay })}
              </div>
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
        <div className="field">
          <label className="field-label">{t("settings.hotkey.styleSwitchLabel")}</label>
          <HotkeyCaptureInput
            value={config.style_switch_hotkey}
            onChange={(v) =>
              setConfig({
                ...config,
                style_switch_hotkey: v || null,
              })
            }
            optional
          />
          <span className="field-hint">{t("settings.hotkey.styleSwitchPh")}</span>
        </div>
        <div className="field">
          <label className="field-label">{t("settings.hotkey.translateLabel")}</label>
          <HotkeyCaptureInput
            value={config.translate_hotkey}
            onChange={(v) =>
              setConfig({
                ...config,
                translate_hotkey: v || null,
              })
            }
            optional
          />
          <span className="field-hint">{t("settings.hotkey.translateHint")}</span>
        </div>
        <div className="field">
          <label className="field-label">{t("settings.hotkey.qaLabel")}</label>
          <HotkeyCaptureInput
            value={config.qa_hotkey}
            onChange={(v) =>
              setConfig({
                ...config,
                qa_hotkey: v || null,
              })
            }
            optional
          />
          <span className="field-hint">{t("settings.hotkey.qaHint")}</span>
        </div>
        {isFnHotkey && (
          <span
            className="field-hint"
            style={{ display: "flex", alignItems: "center", gap: 5, marginTop: 4, color: "var(--warning)" }}
          >
            <StatusIcon warn />
            {t(IS_WIN ? "settings.hotkey.fnWarningWin" : "settings.hotkey.fnWarning")}
          </span>
        )}
      </div>
      )}

      {view === "ai" && (
      <>
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

            <div className="field" style={{ marginTop: 8 }}>
              <span className="field-hint">
                {t("settings.polish.localModelMovedHint")}
                {!suiteInfo?.llm_feature && t("settings.polish.llmFeatureOff")}
              </span>
            </div>
          </>
        )}
      </div>

      {/* 本地三件套：打开目录 / 预算条 / 润色三档 / 翻译两档 */}
      <div className="card">
        <h2 className="card-title">{t("settings.localLlm.title")}</h2>
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
          <button
            className="btn btn-sm btn-ghost"
            style={{ fontSize: 11, flexShrink: 0 }}
            onClick={async () => {
              try {
                const p = await ipc.openModelDirectory();
                setMsg({ ok: true, text: t("settings.localLlm.openDirOk", { path: p }) });
              } catch (e) {
                setMsg({ ok: false, text: String(e) });
              }
            }}
          >
            {t("settings.localLlm.openDir")}
          </button>
          {suiteInfo && (
            <span style={{ flexShrink: 0 }}>
              {t("settings.localLlm.budgetBar", {
                used: fmtSize(suiteInfo.used_bytes),
                budget: fmtSize(suiteInfo.budget_bytes),
              })}
            </span>
          )}
          {suiteInfo && (
            <span style={{ flex: 1, minWidth: 120, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={t("settings.localLlm.pathTitle")}>
              {suiteInfo.model_root}
            </span>
          )}
          {!suiteInfo?.llm_feature && (
            <span style={{ color: "var(--warning)", flexShrink: 0 }}>
              {t("settings.polish.llmFeatureOff")}
            </span>
          )}
        </div>

        {/* 润色三档 */}
        <div className="field-label" style={{ marginBottom: 6 }}>
          {t("settings.localLlm.polishTitle")}
        </div>
        {polishModels.length === 0 ? (
          <span className="field-hint">{t("settings.polish.statusLoading")}</span>
        ) : (
          sortedPolishModels.map((m) => {
            const selected = m.active && m.installed;
            const isDownloadingThis = (dlTargetId === m.id || dl?.target_id === m.id) && !!dl;
            return (
              <div
                key={m.id}
                data-model-id={m.id}
                className={`local-model-card${selected ? " local-model-card--active" : ""}`}
              >
                <div className="row-between" style={{ marginBottom: 6, alignItems: "flex-start" }}>
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div style={{ fontWeight: 600, marginBottom: 2, display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
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
                  <span className={`badge ${m.installed ? "badge-success" : "badge-warning"}`} style={{ flexShrink: 0 }}>
                    <span className="badge-dot" />
                    {m.installed
                      ? t("settings.localAsr.installedBadge")
                      : t("settings.localAsr.notInstalledBadge")}
                  </span>
                </div>

                {isDownloadingThis && dl && dl.phase !== "done" && (
                  <div style={{ marginBottom: 8 }}>
                    <div className="row-between" style={{ marginBottom: 4 }}>
                      <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                        {dl.message || dl.file_name} · {fmtSize(dl.total_downloaded)} / {fmtSize(dl.total_size)}
                      </span>
                      <span style={{ fontSize: 12, color: "var(--text-secondary)", flexShrink: 0 }}>
                        {dl.total_size > 0
                          ? `${Math.min(100, Math.round((dl.total_downloaded / dl.total_size) * 100))}%`
                          : ""}
                        {dl.speed_bps > 0 ? ` · ${fmtSize(dl.speed_bps)}/s` : ""}
                      </span>
                    </div>
                    <div style={{ height: 6, borderRadius: 3, background: "var(--border)", overflow: "hidden" }}>
                      <div
                        style={{
                          height: "100%",
                          borderRadius: 3,
                          background: "var(--accent)",
                          width: `${dl.total_size > 0 ? Math.min(100, (dl.total_downloaded / dl.total_size) * 100) : 0}%`,
                          transition: "width 0.2s",
                        }}
                      />
                    </div>
                  </div>
                )}

                <div className="row-between" style={{ marginTop: 6, gap: 8, flexWrap: "wrap" }}>
                  <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                    {m.installed
                      ? t("settings.localAsr.approxSize", { size: fmtSize(m.approx_size) })
                      : t("settings.localAsr.needDownloadSize", { size: fmtSize(m.missing_size || m.approx_size) })}
                  </span>
                  <div style={{ display: "flex", gap: 8, flexShrink: 0, alignItems: "center" }}>
                    {!m.installed && (
                      <button
                        className="btn btn-sm"
                        disabled={!!dl}
                        onClick={async () => {
                          setDlError(null);
                          setDlTargetId(m.id);
                          try {
                            await ipc.installLlmModel(m.id);
                          } catch (e) {
                            setDlError(String(e));
                            setDlTargetId(null);
                          }
                        }}
                      >
                        {isDownloadingThis ? t("common.downloading") : t("settings.localAsr.downloadBtn")}
                      </button>
                    )}
                    <button
                      className={`btn btn-sm${selected ? " btn-ghost" : ""}`}
                      disabled={!m.installed || selected || enablingId === m.id || !!dl}
                      title={
                        !m.installed
                          ? t("settings.localAsr.enableTipInstallFirst")
                          : selected
                            ? t("settings.localAsr.enableTipCurrent")
                            : t("settings.localAsr.enableTipAction")
                      }
                      onClick={async () => {
                        setEnablingId(m.id);
                        try {
                          await ipc.setActivePolishModel(m.id);
                          ipc.listLocalPolishModels().then((l) => setPolishModels(l)).catch(() => {});
                          ipc.getModelSuiteInfo().then(setSuiteInfo).catch(() => {});
                        } catch (e) {
                          setMsg({ ok: false, text: String(e) });
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
                    {m.installed && !selected && (
                      <button
                        className="btn btn-sm btn-icon"
                        title={t("settings.localAsr.deleteModelTitle")}
                        disabled={enablingId === m.id || deletingId === m.id || !!dl}
                        onClick={async () => {
                          if (!window.confirm(t("settings.localAsr.confirmDeleteModel", { title: m.title }))) return;
                          setDeletingId(m.id);
                          try {
                            await ipc.deleteLlmModel(m.id);
                            ipc.listLocalPolishModels().then((l) => setPolishModels(l)).catch(() => {});
                            ipc.getModelSuiteInfo().then(setSuiteInfo).catch(() => {});
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
                  </div>
                </div>
              </div>
            );
          })
        )}

        {/* 翻译两档 + 策略 + 兼译 */}
        <div className="field-label" style={{ margin: "16px 0 10px" }}>
          {t("settings.localLlm.translateTitle")}
        </div>
        <div className="field">
          <label className="field-label">{t("settings.localLlm.translatePolicyLabel")}</label>
          <select
            value={config.translate_policy ?? "prefer_cloud"}
            onChange={(e) =>
              setConfig({
                ...config,
                translate_policy: e.target.value as "prefer_cloud" | "prefer_local",
              })
            }
          >
            <option value="prefer_cloud">{t("settings.localLlm.policy_cloud")}</option>
            <option value="prefer_local">{t("settings.localLlm.policy_local")}</option>
          </select>
        </div>
        <div className="set-row">
          <div style={{ flex: 1, minWidth: 0, paddingRight: 8 }}>
            <div className="set-name">{t("settings.localLlm.fallbackName")}</div>
            <div className="set-desc">
              {t("settings.localLlm.fallbackDesc")}
            </div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={config.translate_use_llm_fallback ?? false}
              onChange={(e) =>
                setConfig({
                  ...config,
                  translate_use_llm_fallback: e.target.checked,
                })
              }
            />
            <span className="slider" />
          </label>
        </div>
        {(config.translate_use_llm_fallback || suiteInfo?.weak_machine) && (
          <span className="field-hint" style={{ display: "block", marginTop: 8 }}>
            {t("settings.localLlm.fallbackHint")}
          </span>
        )}
        <div
          data-model-id="none"
          className={`local-model-card${(config.translate_local_model ?? "") === "" ? " local-model-card--active" : ""}`}
        >
          <div className="row-between" style={{ marginBottom: 6, alignItems: "flex-start" }}>
            <div style={{ minWidth: 0, flex: 1 }}>
              <div style={{ fontWeight: 600, marginBottom: 2, display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                {t("settings.localLlm.noDedicated")}
                {suiteInfo?.weak_machine && (
                  <span className="badge badge-success" style={{ fontSize: 11 }}>
                    {t("settings.localAsr.recommended")}
                  </span>
                )}
                {(config.translate_local_model ?? "") === "" && (
                  <span className="badge badge-success" style={{ fontSize: 11 }}>
                    {t("settings.localAsr.inUse")}
                  </span>
                )}
              </div>
              <div className="field-hint" style={{ marginBottom: 0 }}>{t("settings.localLlm.noDedicatedDesc")}</div>
            </div>
            <span className="badge badge-success" style={{ flexShrink: 0 }}>
              <span className="badge-dot" />
              {t("settings.localLlm.noInstall")}
            </span>
          </div>
          <div className="row-between" style={{ marginTop: 6, gap: 8, flexWrap: "wrap" }}>
            <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
              {t("settings.localLlm.noDedicatedSize")}
            </span>
            <div style={{ display: "flex", gap: 8, flexShrink: 0, alignItems: "center" }}>
              <button
                className={`btn btn-sm${(config.translate_local_model ?? "") === "" ? " btn-ghost" : ""}`}
                disabled={(config.translate_local_model ?? "") === ""}
                title={
                  (config.translate_local_model ?? "") === ""
                    ? t("settings.localAsr.enableTipCurrent")
                    : t("settings.localLlm.noDedicatedEnableTip")
                }
                onClick={async () => {
                  try {
                    await ipc.setActiveTranslateModel("");
                    ipc.getConfig().then(setConfig).catch(() => {});
                    ipc.listLocalTranslateModels().then((l) => setTranslateModels(l)).catch(() => {});
                    ipc.getModelSuiteInfo().then(setSuiteInfo).catch(() => {});
                  } catch (e) {
                    setMsg({ ok: false, text: String(e) });
                  }
                }}
              >
                {(config.translate_local_model ?? "") === ""
                  ? t("settings.localAsr.enabled")
                  : t("settings.localAsr.enable")}
              </button>
            </div>
          </div>
        </div>
        {translateModels.length === 0 ? (
          <span className="field-hint">{t("settings.polish.statusLoading")}</span>
        ) : (
          sortedTranslateModels.map((m) => {
            const selected = m.active && m.installed;
            const isDownloadingThis = (dlTargetId === m.id || dl?.target_id === m.id) && !!dl;
            return (
              <div
                key={m.id}
                data-model-id={m.id}
                className={`local-model-card${selected ? " local-model-card--active" : ""}`}
              >
                <div className="row-between" style={{ marginBottom: 6, alignItems: "flex-start" }}>
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div style={{ fontWeight: 600, marginBottom: 2, display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
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
                                  : "rgba(255, 59, 48, 0.12)",
                            color:
                              m.perf_tag.kind === "suitable"
                                ? "var(--success)"
                                : m.perf_tag.kind === "usable"
                                  ? "var(--warning)"
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
                  <span className={`badge ${m.installed ? "badge-success" : "badge-warning"}`} style={{ flexShrink: 0 }}>
                    <span className="badge-dot" />
                    {m.installed
                      ? t("settings.localAsr.installedBadge")
                      : t("settings.localAsr.notInstalledBadge")}
                  </span>
                </div>

                {isDownloadingThis && dl && dl.phase !== "done" && (
                  <div style={{ marginBottom: 8 }}>
                    <div className="row-between" style={{ marginBottom: 4 }}>
                      <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                        {dl.message || dl.file_name} · {fmtSize(dl.total_downloaded)} / {fmtSize(dl.total_size)}
                      </span>
                      <span style={{ fontSize: 12, color: "var(--text-secondary)", flexShrink: 0 }}>
                        {dl.total_size > 0
                          ? `${Math.min(100, Math.round((dl.total_downloaded / dl.total_size) * 100))}%`
                          : ""}
                        {dl.speed_bps > 0 ? ` · ${fmtSize(dl.speed_bps)}/s` : ""}
                      </span>
                    </div>
                    <div style={{ height: 6, borderRadius: 3, background: "var(--border)", overflow: "hidden" }}>
                      <div
                        style={{
                          height: "100%",
                          borderRadius: 3,
                          background: "var(--accent)",
                          width: `${dl.total_size > 0 ? Math.min(100, (dl.total_downloaded / dl.total_size) * 100) : 0}%`,
                          transition: "width 0.2s",
                        }}
                      />
                    </div>
                  </div>
                )}

                <div className="row-between" style={{ marginTop: 6, gap: 8, flexWrap: "wrap" }}>
                  <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                    {m.installed
                      ? t("settings.localAsr.approxSize", { size: fmtSize(m.approx_size) })
                      : t("settings.localAsr.needDownloadSize", { size: fmtSize(m.missing_size || m.approx_size) })}
                  </span>
                  <div style={{ display: "flex", gap: 8, flexShrink: 0, alignItems: "center" }}>
                    {!m.installed && (
                      <button
                        className="btn btn-sm"
                        disabled={!!dl}
                        onClick={async () => {
                          setDlError(null);
                          setDlTargetId(m.id);
                          try {
                            await ipc.installLlmModel(m.id);
                          } catch (e) {
                            setDlError(String(e));
                            setDlTargetId(null);
                          }
                        }}
                      >
                        {isDownloadingThis ? t("common.downloading") : t("settings.localAsr.downloadBtn")}
                      </button>
                    )}
                    <button
                      className={`btn btn-sm${selected ? " btn-ghost" : ""}`}
                      disabled={!m.installed || selected || enablingId === m.id || !!dl}
                      title={
                        !m.installed
                          ? t("settings.localAsr.enableTipInstallFirst")
                          : selected
                            ? t("settings.localAsr.enableTipCurrent")
                            : t("settings.localAsr.enableTipAction")
                      }
                      onClick={async () => {
                        setEnablingId(m.id);
                        try {
                          await ipc.setActiveTranslateModel(m.id);
                          ipc.listLocalTranslateModels().then((l) => setTranslateModels(l)).catch(() => {});
                          ipc.getModelSuiteInfo().then(setSuiteInfo).catch(() => {});
                        } catch (e) {
                          setMsg({ ok: false, text: String(e) });
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
                    {m.installed && !selected && (
                      <button
                        className="btn btn-sm btn-icon"
                        title={t("settings.localAsr.deleteModelTitle")}
                        disabled={enablingId === m.id || deletingId === m.id || !!dl}
                        onClick={async () => {
                          if (!window.confirm(t("settings.localAsr.confirmDeleteModel", { title: m.title }))) return;
                          setDeletingId(m.id);
                          try {
                            await ipc.deleteLlmModel(m.id);
                            ipc.listLocalTranslateModels().then((l) => setTranslateModels(l)).catch(() => {});
                            ipc.getModelSuiteInfo().then(setSuiteInfo).catch(() => {});
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
                  </div>
                </div>
              </div>
            );
          })
        )}
        {dlError && dlTargetId && dlTargetId !== "sensevoice" && !asrModels.some((m) => m.id === dlTargetId) && (
          <span className="field-hint" style={{ color: "var(--danger)", display: "block", marginTop: 6 }}>
            {dlError}
          </span>
        )}
      </div>

      {/* P1：R5 角色 / 风格包（master-detail：左列表 + 右编辑） */}
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
        <div className="set-row" style={{ marginTop: 4 }}>
          <div>
            <div className="set-name">{t("settings.roles.assistantNameLabel")}</div>
            <div className="set-desc">{t("settings.roles.assistantNameDesc")}</div>
          </div>
          <input
            className="assistant-name-input"
            defaultValue={config.assistant_name ?? "小友"}
            onChange={(e) => {
              // IME 组合中不更新（避免把拼音中间态存进配置并同步进词典）。
              if ((e.nativeEvent as InputEvent).isComposing) return;
              const v = e.target.value.trim();
              if (v !== (config.assistant_name ?? "小友")) {
                setConfig({ ...config, assistant_name: v });
              }
            }}
            onBlur={(e) => {
              const v = e.target.value.trim();
              if (v !== (config.assistant_name ?? "小友")) {
                setConfig({ ...config, assistant_name: v });
              }
            }}
            placeholder={t("settings.roles.assistantNamePh")}
            style={{ width: 120, height: 30, fontSize: 13, textAlign: "center" }}
          />
        </div>
        <div className="roles-layout">
          {/* 左：角色 / 风格包列表 */}
          <div className="roles-list">
            {stylePacks.map((p) => {
              const prefix = p.match_prefix ?? "";
              return (
                <button
                  key={p.id}
                  type="button"
                  className={`roles-item${!draftOpen && selectedPack?.id === p.id ? " active" : ""}`}
                  onClick={() => {
                    setDraftOpen(false);
                    setSelectedPackId(p.id);
                    setConfirmDeleteId(null);
                  }}
                >
                  <span className="roles-item-name">
                    {p.name}
                    {p.is_builtin ? t("settings.polish.builtin") : ""}
                  </span>
                  {(prefix.trim() || p.role_kind === "translate") && (
                    <span className="roles-item-badges">
                      {prefix.trim() && (
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
                  )}
                </button>
              );
            })}
            <button
              type="button"
              className="roles-item roles-new"
              onClick={() => {
                setDraftOpen(true);
                setSelectedPackId(null);
                setConfirmDeleteId(null);
              }}
            >
              + {t("settings.roles.newBtn")}
            </button>
          </div>

          {/* 右：编辑面板 */}
          <div className="roles-editor">
            {draftOpen ? (
              <>
                <div className="set-name">{t("settings.roles.draftTitle")}</div>
                <div className="field" style={{ margin: 0 }}>
                  <label className="field-label">{t("settings.roles.nameLabel")}</label>
                  <input
                    value={newStyleName}
                    onChange={(e) => setNewStyleName(e.target.value)}
                    placeholder={t("settings.polish.styleNamePh")}
                  />
                </div>
                <div className="field" style={{ margin: 0 }}>
                  <label className="field-label">{t("settings.roles.promptLabel")}</label>
                  <textarea
                    value={newStylePrompt}
                    onChange={(e) => setNewStylePrompt(e.target.value)}
                    rows={6}
                    placeholder={t("settings.polish.stylePromptPh")}
                  />
                </div>
                <div className="roles-editor-footer">
                  <span className="field-hint">{t("settings.roles.draftHint")}</span>
                  <button
                    className="btn"
                    disabled={!newStyleName.trim() || !newStylePrompt.trim()}
                    onClick={async () => {
                      const id = `user-${Date.now()}`;
                      await ipc.upsertStylePack({
                        id,
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
                      setSelectedPackId(id);
                      setDraftOpen(false);
                      ipc.listStylePacks().then(setStylePacks).catch(() => {});
                    }}
                  >
                    {t("settings.roles.createBtn")}
                  </button>
                </div>
              </>
            ) : selectedPack ? (
              <>
                <div className="field" style={{ margin: 0 }}>
                  <label className="field-label">{t("settings.roles.nameLabel")}</label>
                  <input
                    key={`${selectedPack.id}-name`}
                    defaultValue={selectedPack.name}
                    onBlur={(e) => {
                      const v = e.target.value.trim();
                      if (v && v !== selectedPack.name) {
                        savePack(selectedPack, { name: v });
                      }
                    }}
                  />
                </div>
                <div className="field" style={{ margin: 0 }}>
                  <label className="field-label">{t("settings.roles.prefixLabel")}</label>
                  <input
                    key={`${selectedPack.id}-prefix`}
                    defaultValue={selectedPack.match_prefix ?? ""}
                    placeholder={t("settings.roles.prefixPh")}
                    onBlur={(e) => {
                      const v = e.target.value.trim();
                      if (v !== (selectedPack.match_prefix ?? "")) {
                        savePack(selectedPack, { match_prefix: v || null });
                      }
                    }}
                  />
                  <span className="field-hint">{t("settings.roles.prefixHint")}</span>
                </div>
                <div className="roles-editor-row">
                  <div className="field" style={{ margin: 0, flex: 1 }}>
                    <label className="field-label">{t("settings.roles.providerLabel")}</label>
                    <select
                      key={`${selectedPack.id}-provider`}
                      defaultValue={selectedPack.provider ?? ""}
                      onChange={(e) => savePack(selectedPack, { provider: e.target.value || null })}
                    >
                      <option value="">{t("settings.roles.providerFollow")}</option>
                      <option value="cloud">{t("settings.roles.providerCloud")}</option>
                      <option value="local">{t("settings.roles.providerLocal")}</option>
                    </select>
                  </div>
                  <div className="field" style={{ margin: 0, flex: 1 }}>
                    <label className="field-label">{t("settings.roles.modelLabel")}</label>
                    <input
                      key={`${selectedPack.id}-model`}
                      defaultValue={selectedPack.model ?? ""}
                      placeholder={t("settings.roles.modelPh")}
                      onBlur={(e) => {
                        const v = e.target.value.trim();
                        if (v !== (selectedPack.model ?? "")) {
                          savePack(selectedPack, { model: v || null });
                        }
                      }}
                    />
                  </div>
                </div>
                <div className="field" style={{ margin: 0 }}>
                  <label className="field-label">{t("settings.roles.promptLabel")}</label>
                  <textarea
                    key={`${selectedPack.id}-prompt`}
                    defaultValue={selectedPack.system_prompt}
                    rows={8}
                    placeholder={t("settings.roles.promptPh")}
                    onBlur={(e) => {
                      const v = e.target.value.trim();
                      if (v && v !== selectedPack.system_prompt) {
                        savePack(selectedPack, { system_prompt: v });
                      }
                    }}
                  />
                </div>
                <div className="roles-editor-footer">
                  <span className="field-hint">
                    {t("settings.roles.autoSaveHint")}
                  </span>
                  {!selectedPack.is_builtin &&
                    (confirmDeleteId === selectedPack.id ? (
                      <span style={{ display: "flex", gap: 6 }}>
                        <button
                          className="btn btn-sm btn-danger"
                          onClick={async () => {
                            await ipc.deleteStylePack(selectedPack.id);
                            setSelectedPackId(null);
                            setConfirmDeleteId(null);
                            ipc.listStylePacks().then(setStylePacks).catch(() => {});
                          }}
                        >
                          {t("settings.roles.deleteConfirm")}
                        </button>
                        <button
                          className="btn btn-sm btn-ghost"
                          onClick={() => setConfirmDeleteId(null)}
                        >
                          {t("settings.roles.deleteCancel")}
                        </button>
                      </span>
                    ) : (
                      <button
                        className="btn btn-sm btn-ghost"
                        style={{ color: "var(--danger)" }}
                        onClick={() => setConfirmDeleteId(selectedPack.id)}
                      >
                        {t("settings.roles.deleteBtn")}
                      </button>
                    ))}
                </div>
              </>
            ) : (
              <span className="field-hint">{t("settings.roles.emptyEditor")}</span>
            )}
          </div>
        </div>
      </div>
      </>
      )}

      {view === "voice" && (
      <>
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
                  {suiteInfo &&
                    t("settings.localLlm.budgetBar", {
                      used: fmtSize(suiteInfo.used_bytes),
                      budget: fmtSize(suiteInfo.budget_bytes),
                    })}
                </span>
              ) : (
                <span style={{ flex: 1 }}>{t("settings.localAsr.collecting")}</span>
              )}
              <button
                className="btn btn-sm btn-ghost"
                style={{ fontSize: 11, flexShrink: 0 }}
                onClick={async () => {
                  try {
                    const p = await ipc.openModelDirectory();
                    setMsg({ ok: true, text: t("settings.localLlm.openDirOk", { path: p }) });
                  } catch (e) {
                    setMsg({ ok: false, text: String(e) });
                  }
                }}
              >
                {t("settings.localLlm.openDir")}
              </button>
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
                    {
                      id: "funasr-nano-int8",
                      title: t("settings.localAsr.models.funasr_nano_int8.title"),
                      description: t("settings.localAsr.models.funasr_nano_int8.desc"),
                      backend: "offline_funasr_nano",
                      recommended: false,
                      approx_size: 993_000_000,
                      installed: false,
                      active: false,
                      missing_size: 993_000_000,
                    },
                    {
                      id: "funasr-nano-fp16",
                      title: t("settings.localAsr.models.funasr_nano_fp16.title"),
                      description: t("settings.localAsr.models.funasr_nano_fp16.desc"),
                      backend: "offline_funasr_nano",
                      recommended: false,
                      approx_size: 1_586_000_000,
                      installed: false,
                      active: false,
                      missing_size: 1_586_000_000,
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

      </>
      )}

      {view === "ai" && (
      <>
      {/* P1：R4 翻译 */}
      <div className="card">
        <h2 className="card-title">{t("settings.translate.title")}</h2>
        <div className="field">
          <label className="field-label">{t("settings.translate.targetLangLabel")}</label>
          <select
            value={translateTargetLang}
            onChange={(e) =>
              setConfig({ ...config, translate_target_lang: e.target.value })
            }
          >
            {translateLangOptions.map((v) => (
              <option key={v} value={v}>
                {t(`settings.translate.${TRANSLATE_LANG_I18N_KEYS[v] ?? v}`)}
              </option>
            ))}
            {translateLangExtra && (
              <option value={translateLangExtra}>
                {t(`settings.translate.${TRANSLATE_LANG_I18N_KEYS[translateLangExtra]}`)}
              </option>
            )}
          </select>
          <span className="field-hint">{t("settings.translate.targetLangHint")}</span>
          {!translateLangsFull && (
            <span className="field-hint" style={{ display: "block", marginTop: 4 }}>
              {t("settings.translate.polishOnlyLangsHint")}
            </span>
          )}
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

      </>
      )}

      {view === "voice" && (
      <>
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
            <div className="set-desc">
              {t(IS_WIN ? "settings.appBehavior.launchDescWin" : "settings.appBehavior.launchDesc")}
            </div>
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
        {IS_WIN && (
          <div className="set-row">
            <div>
              <div className="set-name">{t("settings.tsf.name")}</div>
              <div className="set-desc">
                {tsfStatus?.status === "installed"
                  ? t("settings.tsf.installed", { path: tsfStatus?.dllPath ?? "" })
                  : tsfStatus?.status === "registrationBroken"
                    ? t("settings.tsf.broken")
                    : t("settings.tsf.notInstalled")}
              </div>
              <button
                className="btn btn-sm"
                style={{ marginTop: 6 }}
                onClick={async () => {
                  try {
                    await ipc.windowsImeRestoreProfile();
                    setMsg({ ok: true, text: t("settings.tsf.restoreDone") });
                  } catch (e) {
                    setMsg({ ok: false, text: String(e) });
                  }
                }}
              >
                {t("settings.tsf.restoreBtn")}
              </button>
            </div>
            <label className="switch">
              <input
                type="checkbox"
                checked={config.windows_tsf_enabled ?? false}
                onChange={(e) =>
                  setConfig({ ...config, windows_tsf_enabled: e.target.checked })
                }
              />
              <span className="slider" />
            </label>
          </div>
        )}
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
            {t(IS_WIN ? "settings.permission.hintWin" : "settings.permission.hint")}
          </span>
        )}
      </div>

      {/* 功能测试 */}
      <div className="card">
        <h2 className="card-title">{t("settings.fnTest.title")}</h2>
        <span className="field-hint" style={{ display: "block", marginBottom: 12 }}>
          {t(IS_WIN ? "settings.fnTest.hintWin" : "settings.fnTest.hint", { key: hotkeyDisplay })}
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
              {singleKeyLabel}
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
                {fnState === "down"
                  ? t("settings.fnTest.fnDown", { key: singleKeyLabel })
                  : t("settings.fnTest.waitingKey")}
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

      </>
      )}

      {/* 自动保存状态：成功短暂显示；失败保留到下次修改成功 */}
      {msg && (
        <div className="save-bar">
          <span className="save-msg" style={{ color: msg.ok ? "var(--success)" : "var(--danger)" }}>
            <StatusIcon ok={msg.ok} />
            <span>{msg.text}</span>
          </span>
        </div>
      )}
    </div>
  );
}
