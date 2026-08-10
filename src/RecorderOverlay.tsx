import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "./ipc";
import { logger } from "./logger";

// 悬浮窗：显示录音状态与实时转写（partial）。
// 路由 index.html#overlay 时渲染本组件。
export default function RecorderOverlay() {
  const [recording, setRecording] = useState(false);
  const [partial, setPartial] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    logger.info("overlay 组件挂载");
    ipc.getRecordingState().then(setRecording).catch((e) => {
      logger.error("get_recording_state 失败:", e);
    });

    const unlisteners: Array<() => void> = [];
    listen<string>("recording://partial", (e) => setPartial(e.payload)).then((u) =>
      unlisteners.push(u)
    );
    listen<string>("recording://stopped", () => {
      setRecording(false);
      setPartial("");
      window.close();
    }).then((u) => unlisteners.push(u));
    listen<string>("recording://error", (e) => {
      logger.error("录音错误事件:", e.payload);
      setError(e.payload);
      setRecording(false);
    }).then((u) => unlisteners.push(u));

    return () => unlisteners.forEach((u) => u());
  }, []);

  return (
    <div className={`overlay ${recording ? "rec" : "idle"}`}>
      <div className="dot" />
      <div className="overlay-text">
        {error ? (
          <span className="err">{error}</span>
        ) : recording ? (
          partial || "正在聆听…"
        ) : (
          ""
        )}
      </div>
    </div>
  );
}
