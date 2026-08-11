import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "./ipc";
import { logger } from "./logger";

type Phase = "idle" | "listening" | "processing" | "error";

// 悬浮窗：Fn 按下 → 文字上屏前保持可见。
// 路由 index.html#overlay 时渲染本组件。
// 注意：不要 window.close()；收起由原生 orderOut 负责，否则会和 HUD 生命周期打架。
export default function RecorderOverlay() {
  const [phase, setPhase] = useState<Phase>("listening");
  const [partial, setPartial] = useState("");
  const [status, setStatus] = useState("正在聆听…");
  const [error, setError] = useState<string | null>(null);
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    logger.info("overlay 组件挂载");
    // 窗口可能在录音已开始后才被 orderFront：读一次后端状态。
    ipc
      .getRecordingState()
      .then((r) => {
        if (r) {
          setPhase("listening");
          setStatus("正在聆听…");
        }
      })
      .catch((e) => {
        logger.error("get_recording_state 失败:", e);
      });

    const unlisteners: Array<() => void> = [];
    listen("recording://started", () => {
      setError(null);
      setLeaving(false);
      setPartial("");
      setPhase("listening");
      setStatus("正在聆听…");
    }).then((u) => unlisteners.push(u));
    // 流式逐字走输入组件直入，不再在左下角大段回显；仅保持"正在聆听"状态。
    listen<string>("recording://partial", () => {
      setPhase("listening");
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://processing", (e) => {
      setPhase("processing");
      setStatus(e.payload || "正在识别…");
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://stopped", () => {
      // 先触发淡出动画，原生侧随后 hide 窗口；这里复位文案，不 close 窗口。
      setLeaving(true);
      setPhase("idle");
      setPartial("");
      setStatus("");
      setError(null);
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://error", (e) => {
      logger.error("录音错误事件:", e.payload);
      setError(e.payload);
      setPhase("error");
    }).then((u) => unlisteners.push(u));

    return () => unlisteners.forEach((u) => u());
  }, []);

  const active = phase === "listening" || phase === "processing";
  const text = error
    ? error
    : partial || status || (active ? "正在聆听…" : "…");

  return (
    <div
      className={`overlay ${active ? "rec" : phase === "error" ? "err" : "idle"}${
        leaving ? " out" : ""
      }`}
    >
      <div className="dot" />
      <div className="overlay-text">
        {error ? <span className="err">{text}</span> : text}
      </div>
    </div>
  );
}
