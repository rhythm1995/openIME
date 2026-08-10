import { useEffect, useState } from "react";
import { Lightbulb, Trash2, BookOpen, Search, AlertTriangle } from "lucide-react";
import type { Hotword } from "../types";
import { ipc } from "../ipc";

// 词典：自定义术语（热词），用于提升 Fun-ASR 对特定名词的识别准确率。
// 添加人名、专业术语、产品名、缩写等，避免被误识别。
export default function Dictionary() {
  const [words, setWords] = useState<Hotword[] | null>(null);
  const [newWord, setNewWord] = useState("");
  const [query, setQuery] = useState("");

  const refresh = async () => {
    try {
      setWords(await ipc.listHotwords());
    } catch {
      setWords([]);
    }
  };
  useEffect(() => {
    refresh();
  }, []);

  const onAdd = async () => {
    const w = newWord.trim();
    if (!w) return;
    // 防重复：与已有词条完全相同（忽略大小写）则拒绝。
    if (words?.some((x) => x.word.toLowerCase() === w.toLowerCase())) {
      alert(`「${w}」已在词典中，无需重复添加。`);
      return;
    }
    try {
      await ipc.addHotword(w, 20);
      setNewWord("");
      await refresh();
    } catch (e) {
      alert(String(e));
    }
  };

  const onDelete = async (id: string) => {
    await ipc.deleteHotword(id);
    await refresh();
  };

  const trimmed = newWord.trim();
  const duplicate =
    !!trimmed && words?.some((w) => w.word.toLowerCase() === trimmed.toLowerCase());

  // 自动搜索：优先用搜索框，其次用添加输入框实时过滤下方列表。
  const filterText = (query.trim() || trimmed).toLowerCase();
  const filtered = words?.filter((w) => w.word.toLowerCase().includes(filterText)) || [];

  return (
    <div>
      <h1 className="page-title">词典</h1>
      <p className="page-subtitle">自定义术语，提升语音引擎对专有名词的识别准确率</p>

      {/* 说明卡 */}
      <div className="card" style={{ background: "var(--accent-soft)", boxShadow: "none" }}>
        <div style={{ display: "flex", gap: 12, alignItems: "flex-start" }}>
          <span style={{ display: "inline-flex", marginTop: 1, color: "var(--accent)" }}>
            <Lightbulb size={20} />
          </span>
          <div style={{ fontSize: 13, color: "var(--text-secondary)", lineHeight: 1.6 }}>
            添加你常用的<strong>人名、专业术语、产品名、缩写</strong>，语音引擎会提升这些词的识别权重。
            例如「智谱」「Paraformer」「AutoGLM」。词频越高、越专业，越值得加入词典。
          </div>
        </div>
      </div>

      {/* 添加 */}
      <div className="card">
        <h2 className="card-title">添加词条</h2>
        <div className="hotword-add">
          <input
            value={newWord}
            onChange={(e) => setNewWord(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && onAdd()}
            placeholder="输入术语，自动匹配已有词条"
            style={duplicate ? { borderColor: "var(--warning)" } : undefined}
          />
          <button className="btn" onClick={onAdd} disabled={!trimmed || duplicate}>
            添加
          </button>
        </div>
        {duplicate && (
          <span className="field-hint" style={{ display: "flex", alignItems: "center", gap: 5, color: "var(--warning)" }}>
            <AlertTriangle size={13} style={{ flexShrink: 0 }} />
            「{trimmed}」已在词典中，无需重复添加。
          </span>
        )}
        {!duplicate && trimmed && filtered.length > 0 && (
          <span className="field-hint" style={{ display: "block" }}>
            已有 {filtered.length} 个相近词条（见下方列表）。
          </span>
        )}
      </div>

      {/* 列表 */}
      <div className="card">
        <div className="row-between" style={{ marginBottom: 12 }}>
          <h2 className="card-title" style={{ margin: 0 }}>
            已有词条 {words && words.length > 0 ? `(${words.length})` : ""}
          </h2>
          {words && words.length > 0 && (
            <div style={{ position: "relative", width: 180 }}>
              <span style={{ position: "absolute", left: 10, top: "50%", transform: "translateY(-50%)", color: "var(--text-tertiary)", display: "flex", pointerEvents: "none" }}>
                <Search size={14} />
              </span>
              <input
                className="search-box"
                style={{ width: "100%", height: 30, paddingLeft: 30, fontSize: 13 }}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索…"
              />
            </div>
          )}
        </div>

        {words === null ? (
          <p style={{ color: "var(--text-tertiary)" }}>加载中…</p>
        ) : filtered.length === 0 ? (
          <div className="empty-state" style={{ padding: "32px 16px" }}>
            <div className="empty-state-icon"><BookOpen /></div>
            <div>{filterText ? "未找到匹配词条" : "词典为空"}</div>
          </div>
        ) : (
          <div className="hotword-list">
            {filtered.map((w) => (
              <div key={w.id} className="hotword-item">
                <div>
                  <span className="hotword-word">{w.word}</span>
                  <span className="hotword-weight">权重 {w.weight}</span>
                </div>
                <button className="btn-icon" onClick={() => onDelete(w.id)} title="删除">
                  <Trash2 />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
