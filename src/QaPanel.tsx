import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Copy, CornerDownLeft, RefreshCw, Trash2, Loader2 } from "lucide-react";
import { ipc } from "./ipc";
import { logger } from "./logger";

interface QaMessage {
  role: string;
  text: string;
}

interface QaState {
  action: string;
  phase: string;
  panel_visible: boolean;
  selection: string | null;
  messages: QaMessage[];
  has_cloud_key?: boolean;
}

interface QaDelta {
  gen: number;
  delta: string;
}

/** R6 划词问答浮窗：路由 index.html#qa。多轮流式回答，关窗清空。 */
export default function QaPanel() {
  const { t } = useTranslation();
  const [phase, setPhase] = useState("idle");
  const [selection, setSelection] = useState<string | null>(null);
  const [messages, setMessages] = useState<QaMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [streaming, setStreaming] = useState("");
  const [hasCloudKey, setHasCloudKey] = useState<boolean | null>(null);
  // 流式回答追加到列表尾部（不闪全量）。
  const tailRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    logger.info("qa 面板挂载");
    const unlisteners: Array<() => void> = [];
    listen<QaState>("qa://state", (e) => {
      setPhase(e.payload.phase);
      setSelection(e.payload.selection);
      setMessages(e.payload.messages ?? []);
      setBusy(e.payload.phase === "recording" || e.payload.phase === "transcribing");
      setStreaming("");
      if (typeof e.payload.has_cloud_key === "boolean") {
        setHasCloudKey(e.payload.has_cloud_key);
      }
    }).then((u) => unlisteners.push(u));
    listen<QaDelta>("qa://delta", (e) => {
      setStreaming((s) => s + e.payload.delta);
      tailRef.current?.scrollIntoView({ behavior: "smooth" });
    }).then((u) => unlisteners.push(u));
    listen<string>("qa://error", (e) => {
      setError(e.payload);
      setBusy(false);
    }).then((u) => unlisteners.push(u));
    return () => unlisteners.forEach((u) => u());
  }, []);

  const streamingActive = phase === "streaming";
  const recording = phase === "recording";

  return (
    <div className="qa-panel">
      <div className="qa-header">
        <div>
          <div className="qa-title">{t("qa.title")}</div>
          <div className="qa-subtitle">{t("qa.subtitle")}</div>
        </div>
        <div className="qa-actions">
          <button
            className="btn btn-sm"
            title={t("qa.refreshSelection")}
            onClick={() =>
              ipc.qaRefreshSelection().then((s) => {
                if (s) setSelection(s);
              })
            }
          >
            <RefreshCw size={13} />
          </button>
          <button
            className="btn btn-sm"
            title={t("qa.clear")}
            onClick={() => {
              setMessages([]);
              setStreaming("");
              setError(null);
              ipc.qaClear().catch(() => {});
            }}
          >
            <Trash2 size={13} />
          </button>
        </div>
      </div>

      <div className="qa-selection">
        {hasCloudKey === false && (
          <div className="qa-error">
            {t("qa.noKeyBanner")}
          </div>
        )}
        {selection ? (
          <>
            <div className="qa-selection-label">{t("qa.selectionLabel")}</div>
            <div className="qa-selection-text">{selection}</div>
          </>
        ) : (
          <div className="qa-selection-empty">{t("qa.noSelection")}</div>
        )}
      </div>

      <div className="qa-messages">
        {messages.map((m, i) => (
          <div key={i} className={`qa-msg qa-msg-${m.role}`}>
            {m.text}
          </div>
        ))}
        {streaming && <div className="qa-msg qa-msg-assistant">{streaming}…</div>}
        {messages.length === 0 && !streaming && !error && (
          <div className="qa-empty">{t("qa.empty")}</div>
        )}
        {error && <div className="qa-error">{error}</div>}
        <div ref={tailRef} />
      </div>

      <div className="qa-footer">
        {recording || busy ? (
          <span className="qa-status">
            <Loader2 size={13} className="spin" /> {t("qa.recording")}
          </span>
        ) : streamingActive ? (
          <button className="btn btn-sm" onClick={() => ipc.qaCancel().catch(() => {})}>
            {t("qa.cancel")}
          </button>
        ) : (
          <button
            className="btn btn-sm"
            onClick={() => ipc.qaCopyLast().catch(() => {})}
            disabled={!messages.some((m) => m.role === "assistant")}
          >
            <Copy size={13} /> {t("qa.copy")}
          </button>
        )}
        <button
          className="btn btn-sm btn-primary"
          onClick={() =>
            ipc
              .qaInsertLast()
              .then(() => {})
              .catch((e) => setError(String(e)))
          }
          disabled={!messages.some((m) => m.role === "assistant")}
        >
          <CornerDownLeft size={13} /> {t("qa.insert")}
        </button>
      </div>
    </div>
  );
}
