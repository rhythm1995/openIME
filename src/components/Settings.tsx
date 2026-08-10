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
} from "lucide-react";
import type {
  AppConfig,
  LocalModelStatus,
  ModelDownloadProgress,
  ProviderConfig,
} from "../types";
import { ipc, permissionLabel, type PermissionKind, type PermissionStatus } from "../ipc";

// sherpa 流式 Paraformer（中英双语）模型目录名，与 voice-core 保持一致。
const SHERPA_MODEL = "sherpa-onnx-streaming-paraformer-bilingual-zh-en";

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
  const [dl, setDl] = useState<ModelDownloadProgress | null>(null);
  const [dlError, setDlError] = useState<string | null>(null);
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

  useEffect(() => {
    ipc.getConfig().then(setConfig).catch(() => ipc.defaultConfig().then(setConfig));
    // 以系统实际状态为准（可能被用户在「系统设置 → 登录项」里改过）。
    ipc.getLaunchAtLogin().then(setAutoStart).catch(() => {});
    ipc.getLocalModelStatus().then(setModelStatus).catch(() => {});
    ipc.listAudioDevices().then((d) => setDevices(Array.isArray(d) ? d : [])).catch(() => {});

    // 模型下载进度 / 完成 / 失败事件。
    const unlisteners: Array<() => void> = [];
    listen<ModelDownloadProgress>("model://download-progress", (e) => {
      setDl(e.payload);
      setDlError(null);
    }).then((u) => unlisteners.push(u));
    listen("model://download-complete", () => {
      setDl(null);
      ipc.getLocalModelStatus().then(setModelStatus).catch(() => {});
    }).then((u) => unlisteners.push(u));
    listen<string>("model://download-error", (e) => {
      setDl(null);
      setDlError(e.payload);
      ipc.getLocalModelStatus().then(setModelStatus).catch(() => {});
    }).then((u) => unlisteners.push(u));

    // Fn 键事件（功能测试模块）。
    listen<boolean>("fn://edge", (e) => {
      setFnCount((c) => (e.payload ? c + 1 : c));
      setFnState(e.payload ? "down" : "idle");
    }).then((u) => unlisteners.push(u));

    // 录音状态变化：实时更新测试框。
    listen<string>("recording://partial", (e) => {
      setTestText(e.payload);
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://stopped", (e) => {
      setRecording(false);
      setTestText((prev) => prev + (e.payload ? e.payload : ""));
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://error", (e) => {
      setRecording(false);
      setMsg({ ok: false, text: `录音错误：${e.payload}` });
    }).then((u) => unlisteners.push(u));

    // 权限轮询：用户可能在系统设置里随时变更，勾选后这里自动更新。
    let cancelled = false;
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
    tick();
    return () => {
      cancelled = true;
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
      <p className="page-subtitle">配置语音识别引擎与快捷键</p>

      {/* 引擎 */}
      <div className="card">
        <h2 className="card-title">识别引擎</h2>
        <div className="field">
          <label className="field-label">引擎类型</label>
          <select
            value={active.kind}
            onChange={(e) => {
              const kind = e.target.value as ProviderConfig["kind"];
              // 切到本地引擎时自动带上默认模型目录名（下载/加载依赖它）。
              if (kind === "sherpa") {
                setActive({ kind, model: SHERPA_MODEL });
              } else {
                setActive({ kind });
              }
            }}
          >
            <option value="sherpa">sherpa-onnx（本地，离线，推荐）</option>
            <option value="bailian">通用流式 ASR（云端）</option>
          </select>
        </div>
        <div className="field">
          <label className="field-label">模型</label>
          <input
            value={active.model}
            onChange={(e) => setActive({ model: e.target.value })}
            placeholder="fun-asr-realtime"
          />
          {active.kind === "bailian" && (
            <span className="field-hint">填写服务商支持的模型 ID（如 fun-asr-realtime、paraformer-realtime-v2 等）</span>
          )}
        </div>

        {active.kind === "bailian" && (
          <>
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
            {/* 连接测试 */}
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
          <div className="field">
            <label className="field-label">本地识别模式</label>
            <select
              value={config.local_mode || "offline"}
              onChange={(e) => setConfig({ ...config, local_mode: e.target.value })}
            >
              <option value="offline">离线模式（Fn按下录音、松开后整段解码，精度更高）</option>
              <option value="realtime">实时模式（流式逐字显示）</option>
            </select>
            <span className="field-hint">
              离线模式使用 SenseVoice（中英日韩粤，~240MB）；实时模式使用流式 Paraformer（中英，~227MB）
            </span>
          </div>
        )}
        {active.kind === "sherpa" && (
          <div className="local-model-card">
            <div className="row-between" style={{ marginBottom: 8, alignItems: "flex-start" }}>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontWeight: 600, marginBottom: 2 }}>本地模型（离线识别）</div>
                <div className="field-hint" style={{ marginBottom: 0 }}>
                  sherpa-onnx 流式 Paraformer（中英）+ Silero VAD，完全离线、音频不出本机。
                </div>
              </div>
              {modelStatus?.installed ? (
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

            {/* 下载进度条 */}
            {dl && dl.phase !== "done" && (
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
                      width: `${dl.total_size > 0 ? Math.min(100, (dl.total_downloaded / dl.total_size) * 100) : 0}%`,
                      transition: "width 0.2s",
                    }}
                  />
                </div>
              </div>
            )}

            {dlError && (
              <div style={{ display: "flex", alignItems: "center", gap: 5, fontSize: 12, color: "var(--danger)", marginBottom: 8 }}>
                <StatusIcon />
                {dlError}
              </div>
            )}

            {/* 后台下载中（切走页面再回来时 dl 为空，用状态兜底提示） */}
            {!modelStatus?.installed && !dl && modelStatus?.downloading && (
              <div className="field-hint" style={{ marginBottom: 0, display: "flex", alignItems: "center", gap: 5 }}>
                <StatusIcon spin /> 正在后台下载中，请稍候…
              </div>
            )}
            {!modelStatus?.installed && !dl && !modelStatus?.downloading && (
              <div className="row-between">
                <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                  约需下载 {fmtSize(modelStatus?.missing_size ?? 0)}
                </span>
                <button
                  className="btn btn-sm"
                  onClick={async () => {
                    setDlError(null);
                    try {
                      await ipc.installLocalModel(config.local_mode || "offline");
                    } catch (e) {
                      setDlError(String(e));
                    }
                  }}
                >
                  下载并安装模型
                </button>
              </div>
            )}
            {modelStatus?.installed && (
              <div className="field-hint" style={{ marginBottom: 0, display: "flex", alignItems: "center", gap: 5 }}>
                <StatusIcon ok /> 模型就绪，保存后即可离线使用。
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

      {/* 系统权限 */}
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
                  请求授权
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
                  去授权
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
        {ax?.state !== "granted" && (
          <span className="field-hint" style={{ display: "block", marginTop: 4 }}>
            之前给 openIME 授过权？重新打包/安装后旧条目会失效：在系统设置里移除旧的
            openIME，再重新添加 /Applications/openIME.app。
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
      <div className="row-between" style={{ marginTop: 8 }}>
        <button className="btn" onClick={onSave} disabled={saving}>
          {saving ? "保存中…" : "保存设置"}
        </button>
        {msg && (
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5, color: msg.ok ? "var(--success)" : "var(--danger)" }}>
            <StatusIcon ok={msg.ok} />
            {msg.text}
          </span>
        )}
      </div>
    </div>
  );
}
