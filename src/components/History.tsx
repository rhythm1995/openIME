import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, Mic, MoreHorizontal, Trash2 } from "lucide-react";
import type { SessionSummary, UtteranceRecord } from "../types";
import { ipc } from "../ipc";

// 历史记录：把所有会话的 utterance 打平，按「天」分组展示。
// - 三点菜单 → 删除（确认框）→ 右下角 toast
// - 行右侧复制按钮 → 右下角 toast
// - 虚拟滚动，长列表不卡

interface HistoryItem {
  id: string;
  text: string;
  sessionId: string;
  createdAt: Date;
}
interface DayGroup {
  key: string;
  date: Date;
  items: HistoryItem[];
  sessionIds: string[];
}

/** 虚拟列表估算高度（px） */
const HEADER_H = 40;
const ROW_BASE_H = 72;
const ROW_LINE_H = 22;
const GROUP_GAP = 22;
const OVERSCAN = 6;

function dayKey(d: Date): string {
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}
function timeLabel(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

function estimateRowHeight(text: string): number {
  // 粗估：约 36 字/行，含上下 padding + 时间行（占位用空格，与语言无关）
  const lines = Math.max(1, Math.ceil((text || " ").length / 36));
  return Math.max(ROW_BASE_H, 14 + 14 + lines * ROW_LINE_H + 4 + 18);
}

type FlatNode =
  | { kind: "header"; key: string; group: DayGroup; height: number }
  | {
      kind: "row";
      key: string;
      item: HistoryItem;
      indexInGroup: number;
      groupSize: number;
      height: number;
    }
  | { kind: "gap"; key: string; height: number };

type ToastState = { id: number; message: string } | null;

export default function History() {
  const { t, i18n } = useTranslation();
  const [groups, setGroups] = useState<DayGroup[] | null>(null);
  const [menuKey, setMenuKey] = useState<string | null>(null);
  const [confirmGroup, setConfirmGroup] = useState<DayGroup | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [toast, setToast] = useState<ToastState>(null);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportH, setViewportH] = useState(480);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<UtteranceRecord[] | null>(null);

  // 日期分组标签：渲染时按当前界面语言计算（语言切换即时生效，无需重新拉数据）。
  const dayLabel = (d: Date): string => {
    const now = new Date();
    if (dayKey(d) === dayKey(now)) return t("history.today");
    const yest = new Date(now);
    yest.setDate(now.getDate() - 1);
    if (dayKey(d) === dayKey(yest)) return t("history.yesterday");
    return t("history.monthDayWeek", {
      month: d.getMonth() + 1,
      day: d.getDate(),
      week: t(`history.week.${d.getDay()}`),
    });
  };

  // toLocaleString 的 locale：跟随界面语言。
  const dateLocale = i18n.language === "en" ? "en-US" : "zh-CN";

  const showToast = useCallback((message: string) => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
    const id = Date.now();
    setToast({ id, message });
    toastTimer.current = setTimeout(() => {
      setToast((t_state) => (t_state?.id === id ? null : t_state));
    }, 2200);
  }, []);

  useEffect(() => {
    return () => {
      if (toastTimer.current) clearTimeout(toastTimer.current);
    };
  }, []);

  // D2：搜索（query 变化调后端 LIKE）。
  useEffect(() => {
    const q = searchQuery.trim();
    if (!q) {
      setSearchResults(null);
      return;
    }
    let cancelled = false;
    ipc
      .searchUtterances(q)
      .then((r) => {
        if (!cancelled) setSearchResults(Array.isArray(r) ? r : []);
      })
      .catch(() => {
        if (!cancelled) setSearchResults([]);
      });
    return () => {
      cancelled = true;
    };
  }, [searchQuery]);

  const refresh = async () => {
    try {
      const sessions: SessionSummary[] = await ipc.listSessions();
      const byDay = new Map<string, { date: Date; items: HistoryItem[]; sids: Set<string> }>();
      for (const s of sessions) {
        let utts: UtteranceRecord[] = [];
        try {
          utts = await ipc.listUtterances(s.id);
        } catch {
          utts = [];
        }
        for (const u of utts) {
          const createdAt = new Date(u.created_at);
          const k = dayKey(createdAt);
          if (!byDay.has(k)) byDay.set(k, { date: createdAt, items: [], sids: new Set() });
          const g = byDay.get(k)!;
          g.items.push({ id: u.id, text: u.final_text, sessionId: s.id, createdAt });
          g.sids.add(s.id);
        }
      }
      const list: DayGroup[] = [...byDay.entries()]
        .map(([key, g]) => ({
          key,
          date: g.date,
          items: g.items.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime()),
          sessionIds: [...g.sids],
        }))
        .sort((a, b) => b.items[0].createdAt.getTime() - a.items[0].createdAt.getTime());
      setGroups(list);
    } catch {
      setGroups([]);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  // 点击外部关闭三点菜单
  useEffect(() => {
    if (!menuKey) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest?.("[data-day-menu]")) return;
      setMenuKey(null);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuKey]);

  // 虚拟列表：测量视口
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setViewportH(el.clientHeight || 480);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [groups]);

  const flat = useMemo(() => {
    if (!groups?.length) return [] as FlatNode[];
    const nodes: FlatNode[] = [];
    groups.forEach((g, gi) => {
      nodes.push({ kind: "header", key: `h-${g.key}`, group: g, height: HEADER_H });
      g.items.forEach((it, i) => {
        nodes.push({
          kind: "row",
          key: it.id,
          item: it,
          indexInGroup: i,
          groupSize: g.items.length,
          height: estimateRowHeight(it.text),
        });
      });
      if (gi < groups.length - 1) {
        nodes.push({ kind: "gap", key: `gap-${g.key}`, height: GROUP_GAP });
      }
    });
    return nodes;
  }, [groups]);

  const offsets = useMemo(() => {
    const offs = new Array<number>(flat.length + 1);
    offs[0] = 0;
    for (let i = 0; i < flat.length; i++) offs[i + 1] = offs[i] + flat[i].height;
    return offs;
  }, [flat]);

  const totalH = offsets[offsets.length - 1] ?? 0;

  const { start, end } = useMemo(() => {
    if (flat.length === 0) return { start: 0, end: 0 };
    const viewTop = Math.max(0, scrollTop);
    const viewBottom = viewTop + viewportH;
    let s = 0;
    while (s < flat.length && offsets[s + 1] < viewTop) s++;
    let e = s;
    while (e < flat.length && offsets[e] < viewBottom) e++;
    s = Math.max(0, s - OVERSCAN);
    e = Math.min(flat.length, e + OVERSCAN);
    return { start: s, end: e };
  }, [flat, offsets, scrollTop, viewportH]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) setScrollTop(el.scrollTop);
  };

  const onDeleteDay = async (g: DayGroup) => {
    setDeleting(true);
    try {
      for (const sid of g.sessionIds) {
        try {
          await ipc.deleteSession(sid);
        } catch {
          /* ignore */
        }
      }
      setConfirmGroup(null);
      await refresh();
      showToast(t("history.deleted"));
    } finally {
      setDeleting(false);
    }
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard?.writeText(text);
      showToast(t("history.copied"));
    } catch {
      showToast(t("history.copied"));
    }
  };

  const visible = flat.slice(start, end);
  const padTop = offsets[start] ?? 0;
  const padBottom = totalH - (offsets[end] ?? 0);

  return (
    <div className="history-page">
      <div className="history-head">
        <h1 className="page-title">{t("history.title")}</h1>
        <p className="page-subtitle">{t("history.subtitle")}</p>
      </div>

      <div className="field" style={{ marginBottom: 12 }}>
        <input
          placeholder={t("history.searchPh")}
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
        />
      </div>
      {searchQuery.trim() && searchResults !== null && (
        <div
          className="history-scroll"
          style={{ maxHeight: "50vh", marginBottom: 16 }}
        >
          {searchResults.length === 0 ? (
            <p style={{ color: "var(--text-tertiary)", padding: 16 }}>
              {t("history.noMatch")}
            </p>
          ) : (
            searchResults.map((u) => (
              <div
                key={u.id}
                className="day-row"
                style={{ minHeight: 60, padding: "10px 12px" }}
              >
                <div className="day-row-text" style={{ fontSize: 14 }}>
                  {u.final_text}
                </div>
                <div
                  className="day-row-time"
                  style={{ fontSize: 11, color: "var(--text-tertiary)" }}
                >
                  {new Date(u.created_at).toLocaleString(dateLocale)}
                </div>
              </div>
            ))
          )}
        </div>
      )}

      {groups === null ? (
        <p style={{ color: "var(--text-tertiary)" }}>{t("common.loading")}</p>
      ) : groups.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon">
            <Mic />
          </div>
          <div>{t("history.empty")}</div>
          <div className="empty-state-sub">{t("history.emptySub")}</div>
        </div>
      ) : (
        <div
          className="history-scroll"
          ref={scrollRef}
          onScroll={onScroll}
          role="list"
          aria-label={t("history.title")}
        >
          <div className="history-virtual" style={{ height: totalH, position: "relative" }}>
            <div style={{ height: padTop }} aria-hidden />
            {visible.map((node) => {
              if (node.kind === "header") {
                const g = node.group;
                const open = menuKey === g.key;
                return (
                  <div
                    key={node.key}
                    className="day-header"
                    style={{ minHeight: node.height }}
                    data-day-menu
                  >
                    <span>{dayLabel(g.date)}</span>
                    <div className="day-menu-wrap">
                      <button
                        type="button"
                        className="btn-icon btn-icon-neutral"
                        title={t("history.more")}
                        aria-haspopup="menu"
                        aria-expanded={open}
                        onClick={(e) => {
                          e.stopPropagation();
                          setMenuKey(open ? null : g.key);
                        }}
                      >
                        <MoreHorizontal />
                      </button>
                      {open && (
                        <div className="day-menu" role="menu">
                          <button
                            type="button"
                            className="day-menu-item day-menu-item-danger"
                            role="menuitem"
                            onClick={() => {
                              setMenuKey(null);
                              setConfirmGroup(g);
                            }}
                          >
                            <Trash2 size={14} />
                            {t("history.deleteBtn")}
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                );
              }
              if (node.kind === "gap") {
                return <div key={node.key} style={{ height: node.height }} aria-hidden />;
              }
              // row
              const { item, indexInGroup, groupSize } = node;
              const isFirst = indexInGroup === 0;
              const isLast = indexInGroup === groupSize - 1;
              return (
                <div
                  key={node.key}
                  className={[
                    "day-row",
                    !isFirst ? "sep" : "",
                    isFirst ? "day-row-first" : "",
                    isLast ? "day-row-last" : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  style={{ minHeight: node.height }}
                  role="listitem"
                >
                  <div className="day-row-main">
                    <div className="day-row-text">{item.text || t("history.emptyText")}</div>
                    <div className="day-row-time">{timeLabel(item.createdAt)}</div>
                  </div>
                  <button
                    type="button"
                    className="btn-icon btn-icon-neutral day-row-copy"
                    title={t("history.copyTitle")}
                    aria-label={t("history.copyTitle")}
                    onClick={() => copy(item.text || "")}
                  >
                    <Copy />
                  </button>
                </div>
              );
            })}
            <div style={{ height: padBottom }} aria-hidden />
          </div>
        </div>
      )}

      {/* 删除确认 */}
      {confirmGroup && (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget && !deleting) setConfirmGroup(null);
          }}
        >
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="hist-del-title">
            <h2 id="hist-del-title" className="modal-title">
              {t("history.deleteTitle")}
            </h2>
            <p className="modal-body">
              {t("history.confirmDelete", {
                label: dayLabel(confirmGroup.date),
                count: confirmGroup.items.length,
              })}
            </p>
            <div className="modal-actions">
              <button
                type="button"
                className="btn btn-ghost"
                disabled={deleting}
                onClick={() => setConfirmGroup(null)}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className="btn btn-danger"
                disabled={deleting}
                onClick={() => onDeleteDay(confirmGroup)}
              >
                {deleting ? t("common.delete") + "…" : t("common.delete")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 右下角 toast */}
      {toast && (
        <div className="toast toast-br" role="status" aria-live="polite">
          <Check size={15} strokeWidth={2.5} />
          <span>{toast.message}</span>
        </div>
      )}
    </div>
  );
}
