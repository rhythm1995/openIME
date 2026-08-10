import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import RecorderOverlay from "./RecorderOverlay";
import { installErrorHooks, logger } from "./logger";
import "./styles.css";

// 最先安装错误钩子：任何 JS 崩溃都要留痕。
installErrorHooks();

// 按 hash 路由：#overlay 渲染悬浮窗，其余渲染主窗口。
const isOverlay = window.location.hash === "#overlay";
logger.info(
  `前端启动：route=${isOverlay ? "overlay" : "main"}, url=${window.location.href}, ua=${navigator.userAgent}`
);

try {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>{isOverlay ? <RecorderOverlay /> : <App />}</React.StrictMode>
  );
  logger.info("前端渲染已提交");
} catch (e) {
  logger.error("前端渲染异常:", e);
  throw e;
}
