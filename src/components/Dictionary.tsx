import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Lightbulb, Trash2, BookOpen, Search, AlertTriangle, Upload } from "lucide-react";
import type { Hotword } from "../types";
import { ipc } from "../ipc";

// 词典：自定义术语（热词）。
// 作用：L0 同音/模糊音纠错（把识别成同音常用字的专有名词改回热词写法）
//       + 润色时让 LLM 保留这些专有名词的写法。
// 注：本地 sherpa 模型非 transducer，无解码层热词偏置；纠音发生在识别后。
export default function Dictionary() {
  const { t } = useTranslation();
  const [words, setWords] = useState<Hotword[] | null>(null);
  const [newWord, setNewWord] = useState("");
  const [query, setQuery] = useState("");
  const [importing, setImporting] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const refresh = async () => {
    try {
      const ws = await ipc.listHotwords();
      setWords(ws);
    } catch {
      setWords([]);
    }
  };

  const onImportFile = async (file: File) => {
    setImporting(true);
    try {
      const text = await file.text();
      const res = await ipc.importHotwordsCsv(text);
      await refresh();
      alert(t("dict.importDone", { imported: res.imported, total: res.total }));
    } catch (e) {
      alert(t("dict.importFailed", { error: String(e) }));
    } finally {
      setImporting(false);
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
      alert(t("dict.duplicateTip", { word: w }));
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
      <h1 className="page-title">{t("dict.title")}</h1>
      <p className="page-subtitle">{t("dict.subtitle")}</p>

      {/* 说明卡 */}
      <div className="card" style={{ background: "var(--accent-soft)", boxShadow: "none" }}>
        <div style={{ display: "flex", gap: 12, alignItems: "flex-start" }}>
          <span style={{ display: "inline-flex", marginTop: 1, color: "var(--accent)" }}>
            <Lightbulb size={20} />
          </span>
          <div style={{ fontSize: 13, color: "var(--text-secondary)", lineHeight: 1.6 }}>
            {t("dict.tipStart")}<strong>{t("dict.tipBold")}</strong>{t("dict.tipEnd")}
          </div>
        </div>
      </div>

      {/* 添加 */}
      <div className="card">
        <h2 className="card-title">{t("dict.addTitle")}</h2>
        <div className="hotword-add">
          <input
            value={newWord}
            onChange={(e) => setNewWord(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && onAdd()}
            placeholder={t("dict.inputPh")}
            style={duplicate ? { borderColor: "var(--warning)" } : undefined}
          />
          <button className="btn" onClick={onAdd} disabled={!trimmed || duplicate}>
            {t("dict.addBtn")}
          </button>
        </div>
        {duplicate && (
          <span className="field-hint" style={{ display: "flex", alignItems: "center", gap: 5, color: "var(--warning)" }}>
            <AlertTriangle size={13} style={{ flexShrink: 0 }} />
            {t("dict.duplicateTip", { word: trimmed })}
          </span>
        )}
        {!duplicate && trimmed && filtered.length > 0 && (
          <span className="field-hint" style={{ display: "block" }}>
            {t("dict.similarCount", { count: filtered.length })}
          </span>
        )}

        {/* 批量导入 + 容量标注 */}
        <div
          className="row-between"
          style={{ marginTop: 12, alignItems: "center", flexWrap: "wrap", gap: 8 }}
        >
          <button
            className="btn"
            onClick={() => fileRef.current?.click()}
            disabled={importing}
          >
            <Upload size={14} style={{ marginRight: 6, verticalAlign: "-2px" }} />
            {importing ? t("dict.importing") : t("dict.importBtn")}
          </button>
          <input
            ref={fileRef}
            type="file"
            accept=".csv,text/csv"
            style={{ display: "none" }}
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) onImportFile(f);
              e.target.value = "";
            }}
          />
          <span className="field-hint">
            {t("dict.importHint")}
          </span>
        </div>
      </div>

      {/* 列表 */}
      <div className="card">
        <div className="row-between" style={{ marginBottom: 12 }}>
          <h2 className="card-title" style={{ margin: 0 }}>
            {t("dict.listTitle")} {words && words.length > 0 ? `(${words.length})` : ""}
          </h2>
          {words && words.length > 0 && (
            <div style={{ position: "relative", width: 180 }}>
              <span style={{ position: "absolute", left: 10, top: "50%", transform: "translateY(-50%)", color: "var(--text-tertiary)", display: "flex", pointerEvents: "none" }}>
                <Search size={14} />
              </span>
              <input
                className="search-box"
                style={{ width: "100%", height: 30, padding: "0 8px 0 30px", margin: 0, fontSize: 13 }}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t("common.searchPh")}
              />
            </div>
          )}
        </div>

        {words === null ? (
          <p style={{ color: "var(--text-tertiary)" }}>{t("common.loading")}</p>
        ) : filtered.length === 0 ? (
          <div className="empty-state" style={{ padding: "32px 16px" }}>
            <div className="empty-state-icon"><BookOpen /></div>
            <div>{filterText ? t("dict.notFound") : t("dict.empty")}</div>
          </div>
        ) : (
          <div className="hotword-list">
            {filtered.map((w) => (
              <div key={w.id} className="hotword-item">
                <div>
                  <span className="hotword-word">{w.word}</span>
                  <span className="hotword-weight">{t("dict.weight", { weight: w.weight })}</span>
                </div>
                <button className="btn-icon" onClick={() => onDelete(w.id)} title={t("dict.deleteTitle")}>
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
