import { useEffect, useState, type MouseEvent as ReactMouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Settings as SettingsIcon, History as HistoryIcon, BookOpen, type LucideIcon } from "lucide-react";
import Settings from "./components/Settings";
import History from "./components/History";
import Dictionary from "./components/Dictionary";
import { ipc } from "./ipc";
import { logger } from "./logger";

type Page = "settings" | "history" | "dictionary";

/** Overlay 标题栏拖拽：data-tauri-drag-region + startDragging 双保险 */
function onDragRegionMouseDown(e: ReactMouseEvent) {
  // 只响应左键；忽略可交互控件（它们带 no-drag，一般进不来）
  if (e.button !== 0) return;
  const target = e.target as HTMLElement | null;
  if (target?.closest("button, a, input, select, textarea, .nav-item, .switch")) return;
  e.preventDefault();
  getCurrentWindow()
    .startDragging()
    .catch((err) => logger.warn("startDragging 失败:", err));
}

export default function App() {
  const [pong, setPong] = useState("");
  const [page, setPage] = useState<Page>("settings");
  // 页面保活：首次进入后保持挂载，仅用 CSS 隐藏。
  // 避免每次切回「设置」都重建整页（IPC + 设备枚举 + 事件监听），造成侧栏切换卡顿。
  const [mounted, setMounted] = useState<Record<Page, boolean>>({
    settings: true,
    history: false,
    dictionary: false,
  });

  const goTo = (id: Page) => {
    setPage(id);
    setMounted((m) => (m[id] ? m : { ...m, [id]: true }));
  };

  useEffect(() => {
    logger.info("主窗口 App 组件挂载");
    ipc
      .ping()
      .then((p) => {
        setPong(p);
        logger.info("IPC ping 成功:", p);
      })
      .catch((e) => logger.error("IPC ping 失败:", e));
    // 托盘"历史记录"菜单发 nav://goto。
    let unlisten: (() => void) | undefined;
    listen<string>("nav://goto", (e) => {
      if (e.payload === "history") goTo("history");
      if (e.payload === "settings") goTo("settings");
      if (e.payload === "dictionary") goTo("dictionary");
    }).then((u) => {
      unlisten = u;
    });
    return () => unlisten?.();
  }, []);

  const nav: { id: Page; label: string; icon: LucideIcon }[] = [
    { id: "settings", label: "设置", icon: SettingsIcon },
    { id: "history", label: "历史记录", icon: HistoryIcon },
    { id: "dictionary", label: "词典", icon: BookOpen },
  ];

  return (
    <div className="layout">
      {/* Overlay 标题栏拖拽区：顶部全宽 + 侧栏 brand */}
      <div
        className="titlebar-drag"
        data-tauri-drag-region
        onMouseDown={onDragRegionMouseDown}
      />
      <aside className="sidebar">
        <div
          className="brand"
          data-tauri-drag-region
          onMouseDown={onDragRegionMouseDown}
        >
          <div className="brand-logo" aria-label="openIME" data-tauri-drag-region>
            <svg viewBox="0 0 1024 1024" width="34" height="34" aria-hidden="true">
              <rect width="1024" height="1024" rx="226" fill="#3B4FE0" />
              <g fill="#FFFFFF">
                <rect x="300" y="422" width="56" height="180" rx="28" />
                <rect x="392" y="362" width="56" height="300" rx="28" />
                <rect x="484" y="302" width="56" height="420" rx="28" />
                <rect x="576" y="362" width="56" height="300" rx="28" />
                <rect x="668" y="422" width="56" height="180" rx="28" />
              </g>
            </svg>
          </div>
          <div className="brand-name" data-tauri-drag-region>openIME</div>
        </div>
        <nav className="nav">
          {nav.map((n) => {
            const Icon = n.icon;
            return (
              <button
                key={n.id}
                className={`nav-item ${page === n.id ? "active" : ""}`}
                onClick={() => goTo(n.id)}
              >
                <span className="nav-icon">
                  <Icon strokeWidth={2} />
                </span>
                {n.label}
              </button>
            );
          })}
        </nav>
        <div className="sidebar-footer">
          <span className={`status-dot ${pong ? "ready" : "connecting"}`} />
          {pong ? "已就绪" : "连接中"}
        </div>
      </aside>

      <main className="content">
        {mounted.settings && (
          <div
            className={page === "settings" ? "page-panel" : "page-panel page-panel-hidden"}
            aria-hidden={page !== "settings"}
          >
            <Settings />
          </div>
        )}
        {mounted.history && (
          <div
            className={page === "history" ? "page-panel" : "page-panel page-panel-hidden"}
            aria-hidden={page !== "history"}
          >
            <History />
          </div>
        )}
        {mounted.dictionary && (
          <div
            className={page === "dictionary" ? "page-panel" : "page-panel page-panel-hidden"}
            aria-hidden={page !== "dictionary"}
          >
            <Dictionary />
          </div>
        )}
      </main>
    </div>
  );
}
