import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Settings as SettingsIcon, History as HistoryIcon, BookOpen, type LucideIcon } from "lucide-react";
import Settings from "./components/Settings";
import History from "./components/History";
import Dictionary from "./components/Dictionary";
import PermissionBanner from "./components/PermissionBanner";
import { ipc } from "./ipc";
import { logger } from "./logger";

type Page = "settings" | "history" | "dictionary";

export default function App() {
  const [pong, setPong] = useState("");
  const [page, setPage] = useState<Page>("settings");

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
    listen<string>("nav://goto", (e) => {
      if (e.payload === "history") setPage("history");
    }).then((u) => () => u());
  }, []);

  const nav: { id: Page; label: string; icon: LucideIcon }[] = [
    { id: "settings", label: "设置", icon: SettingsIcon },
    { id: "history", label: "历史记录", icon: HistoryIcon },
    { id: "dictionary", label: "词典", icon: BookOpen },
  ];

  return (
    <div className="layout">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-logo" aria-label="openIME">
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
          <div className="brand-name">openIME</div>
        </div>
        <nav className="nav">
          {nav.map((n) => {
            const Icon = n.icon;
            return (
              <button
                key={n.id}
                className={`nav-item ${page === n.id ? "active" : ""}`}
                onClick={() => setPage(n.id)}
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
        <PermissionBanner />
        {page === "settings" && <Settings />}
        {page === "history" && <History />}
        {page === "dictionary" && <Dictionary />}
      </main>
    </div>
  );
}
