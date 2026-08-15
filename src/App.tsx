import { useEffect, useState, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { History as HistoryIcon, BookOpen, Mic, Sparkles, MessageSquarePlus, type LucideIcon } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import Settings from "./components/Settings";
import History from "./components/History";
import Dictionary from "./components/Dictionary";
import { ipc } from "./ipc";
import { logger } from "./logger";

type Page = "settings-voice" | "settings-ai" | "history" | "dictionary";

/** 两个设置入口共享同一个 Settings 挂载实例（各自实例会各持一份 config 状态，保存互相覆盖）。 */
const isSettingsPage = (p: Page) => p === "settings-voice" || p === "settings-ai";

/** 意见反馈入口：GitHub Issues，用系统默认浏览器打开（沿用浏览器里的 GitHub 登录态）。 */
const FEEDBACK_URL = "https://github.com/rhythm1995/openIME/issues";

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
  const { t, i18n } = useTranslation();
  const [pong, setPong] = useState("");
  const [page, setPage] = useState<Page>("settings-voice");
  // PR4：toast://info 事件（互斥提示 / 无 key 提示等）。
  const [toast, setToast] = useState<string | null>(null);
  // 页面保活：首次进入后保持挂载，仅用 CSS 隐藏。
  // 避免每次切回「设置」都重建整页（IPC + 设备枚举 + 事件监听），造成侧栏切换卡顿。
  const [mounted, setMounted] = useState<Record<Page, boolean>>({
    "settings-voice": true,
    "settings-ai": false,
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
      if (e.payload === "settings") goTo("settings-voice");
      if (e.payload === "dictionary") goTo("dictionary");
    }).then((u) => {
      unlisten = u;
    });
    // PR4：toast://info（翻译/QA 互斥提示、无云端 key 等）。4s 自动消失。
    let unlistenToast: (() => void) | undefined;
    let toastTimer: ReturnType<typeof setTimeout> | undefined;
    listen<string>("toast://info", (e) => {
      setToast(e.payload);
      if (toastTimer) clearTimeout(toastTimer);
      toastTimer = setTimeout(() => setToast(null), 4000);
    }).then((u) => {
      unlistenToast = u;
    });
    return () => {
      unlisten?.();
      unlistenToast?.();
      if (toastTimer) clearTimeout(toastTimer);
    };
  }, []);

  const nav: { id: Page; label: string; icon: LucideIcon }[] = [
    { id: "settings-voice", label: t("nav.settingsVoice"), icon: Mic },
    { id: "settings-ai", label: t("nav.settingsAi"), icon: Sparkles },
    { id: "history", label: t("nav.history"), icon: HistoryIcon },
    { id: "dictionary", label: t("nav.dictionary"), icon: BookOpen },
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
        {/* 左下角意见反馈：动作而非页面导航，不参与 page 状态；复用 nav-item 视觉样式 */}
        <button
          type="button"
          className="nav-item"
          onClick={() =>
            openUrl(FEEDBACK_URL).catch((e) => logger.error("打开反馈页面失败:", e))
          }
        >
          <span className="nav-icon">
            <MessageSquarePlus strokeWidth={2} />
          </span>
          {t("nav.feedback")}
        </button>
        <div className="sidebar-footer">
          <span className={`status-dot ${pong ? "ready" : "connecting"}`} />
          {pong ? t("status.ready") : t("status.connecting")}
          <button
            type="button"
            className="lang-toggle"
            onClick={() => i18n.changeLanguage(i18n.language === "zh" ? "en" : "zh")}
            title={t("lang.switchTitle")}
          >
            🌐 {i18n.language === "zh" ? t("lang.zh") : t("lang.en")}
          </button>
        </div>
      </aside>

      <main className="content">
        {(mounted["settings-voice"] || mounted["settings-ai"]) && (
          <div
            className={isSettingsPage(page) ? "page-panel" : "page-panel page-panel-hidden"}
            aria-hidden={!isSettingsPage(page)}
          >
            <Settings view={page === "settings-ai" ? "ai" : "voice"} />
          </div>
        )}{" "}        {mounted.history && (
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
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </div>
  );
}
