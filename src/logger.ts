// 前端日志：转发到后端 logging 模块，统一落盘到日志文件。
// 设计要点：
// - fire-and-forget，任何情况下都不抛异常（日志不能反过来把应用搞崩）。
// - installErrorHooks() 捕获 window.onerror / unhandledrejection / console.error，
//   保证 JS 崩溃也能留痕。
// - 降级路径使用「原始 console」引用，避免与包装后的 console.error 无限递归。

import { invoke } from "@tauri-apps/api/core";

export type LogLevel = "debug" | "info" | "warn" | "error";

// 模块加载时先保存原始 console，供降级与包装内部使用。
const rawLog = console.log.bind(console);
const rawWarn = console.warn.bind(console);
const rawError = console.error.bind(console);

function send(level: LogLevel, message: string) {
  try {
    invoke("frontend_log", { level, message }).catch(() => {});
  } catch {
    // invoke 本身不可用时（如后端未就绪）静默降级到原始 console。
    const text = `[openIME:${level}] ${message}`;
    if (level === "error") rawError(text);
    else if (level === "warn") rawWarn(text);
    else rawLog(text);
  }
}

function stringify(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) return `${value.name}: ${value.message}\n${value.stack ?? ""}`;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function fmt(args: unknown[]): string {
  return args.map(stringify).join(" ");
}

export const logger = {
  debug: (...args: unknown[]) => send("debug", fmt(args)),
  info: (...args: unknown[]) => send("info", fmt(args)),
  warn: (...args: unknown[]) => send("warn", fmt(args)),
  error: (...args: unknown[]) => send("error", fmt(args)),
};

/** 安装全局错误钩子，把 JS 崩溃/未捕获异常写入后端日志。只需调用一次。 */
export function installErrorHooks() {
  window.addEventListener("error", (e) => {
    send(
      "error",
      `[window.onerror] ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`
    );
  });

  window.addEventListener("unhandledrejection", (e) => {
    send("error", `[unhandledrejection] ${stringify(e.reason)}`);
  });

  // 包装 console.error：React 渲染错误等只走 console 的场景也能落盘。
  console.error = (...args: unknown[]) => {
    send("error", `[console.error] ${fmt(args)}`);
    rawError(...args);
  };
}
