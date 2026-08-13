import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import RecorderOverlay from "./RecorderOverlay";
import QaPanel from "./QaPanel";
import { installErrorHooks, logger } from "./logger";
import "./styles.css";
import "./i18n";

// 最先安装错误钩子：任何 JS 崩溃都要留痕。
installErrorHooks();

// 按 hash 路由：#overlay 渲染悬浮窗，#qa 渲染划词问答浮窗，其余渲染主窗口。
const hash = window.location.hash;
const isOverlay = hash === "#overlay";
const isQa = hash === "#qa";
logger.info(
  `前端启动：route=${isOverlay ? "overlay" : isQa ? "qa" : "main"}, url=${window.location.href}, ua=${navigator.userAgent}`
);

try {
  const root = ReactDOM.createRoot(document.getElementById("root")!);
  const panel = isOverlay ? <RecorderOverlay /> : isQa ? <QaPanel /> : <App />;
  root.render(<React.StrictMode>{panel}</React.StrictMode>);
  logger.info("前端渲染已提交");
} catch (e) {
  logger.error("前端渲染异常:", e);
  throw e;
}
