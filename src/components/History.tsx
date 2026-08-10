import { useEffect, useState } from "react";
import { Mic, MoreHorizontal } from "lucide-react";
import type { SessionSummary, UtteranceRecord } from "../types";
import { ipc } from "../ipc";

// 历史记录：把所有会话的 utterance 打平，按「天」分组展示（参考时间线列表样式）。
// 每条显示转写文本 + 时间；同一天收进一张圆角卡片，行与行之间用细分隔线。

interface HistoryItem {
  id: string;
  text: string;
  sessionId: string;
  createdAt: Date;
}
interface DayGroup {
  key: string;
  label: string;
  items: HistoryItem[];
  sessionIds: string[];
}

const WEEK = ["日", "一", "二", "三", "四", "五", "六"];

function dayKey(d: Date): string {
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}
function dayLabel(d: Date): string {
  const now = new Date();
  if (dayKey(d) === dayKey(now)) return "今天";
  const yest = new Date(now);
  yest.setDate(now.getDate() - 1);
  if (dayKey(d) === dayKey(yest)) return "昨天";
  return `${d.getMonth() + 1}月${d.getDate()}日 星期${WEEK[d.getDay()]}`;
}
function timeLabel(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

export default function History() {
  const [groups, setGroups] = useState<DayGroup[] | null>(null);

  const refresh = async () => {
    try {
      const sessions: SessionSummary[] = await ipc.listSessions();
      const all: HistoryItem[] = [];
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
          all.push({ id: u.id, text: u.final_text, sessionId: s.id, createdAt });
        }
      }
      const list: DayGroup[] = [...byDay.entries()]
        .map(([key, g]) => ({
          key,
          label: dayLabel(g.date),
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

  const onDeleteDay = async (g: DayGroup) => {
    if (!window.confirm(`删除「${g.label}」的 ${g.items.length} 条记录？`)) return;
    for (const sid of g.sessionIds) {
      try {
        await ipc.deleteSession(sid);
      } catch {
        /* ignore */
      }
    }
    await refresh();
  };

  const copy = (text: string) => {
    navigator.clipboard?.writeText(text).catch(() => {});
  };

  return (
    <div>
      <h1 className="page-title">历史记录</h1>
      <p className="page-subtitle">所有语音转写内容</p>

      {groups === null ? (
        <p style={{ color: "var(--text-tertiary)" }}>加载中…</p>
      ) : groups.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon"><Mic /></div>
          <div>还没有录音记录</div>
          <div className="empty-state-sub">按 Fn 开始第一次语音输入</div>
        </div>
      ) : (
        groups.map((g) => (
          <div key={g.key} className="day-group">
            <div className="day-header">
              <span>{g.label}</span>
              <button
                className="btn-icon"
                title="删除当天"
                onClick={() => onDeleteDay(g)}
              >
                <MoreHorizontal />
              </button>
            </div>
            <div className="day-card">
              {g.items.map((it, i) => (
                <div
                  key={it.id}
                  className={`day-row${i > 0 ? " sep" : ""}`}
                  onClick={() => copy(it.text)}
                  title="点击复制"
                >
                  <div className="day-row-text">{it.text || "（空）"}</div>
                  <div className="day-row-time">{timeLabel(it.createdAt)}</div>
                </div>
              ))}
            </div>
          </div>
        ))
      )}
    </div>
  );
}
