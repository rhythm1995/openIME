//! SQLite 历史记录存储：实现 [`HistoryStore`]。
//!
//! 设计：
//! - 用 r2d2 连接池（rusqlite backend），单文件 DB，放 app_data_dir。
//! - 迁移走 `user_version` PRAGMA，按版本号顺序应用 SQL（含二期预留空表）。
//! - 所有 DB 错误归并到 [`crate::Error::Store`]。
//! - 时间戳用 RFC3339 文本存（chrono 序列化），避免跨平台整数时区坑。

use std::path::Path;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::traits::{HistoryStore, SessionSummary, UtteranceRecord};
use crate::{Error, Result};

/// 迁移 SQL 列表。下标+1 即 user_version。
/// 一期建 sessions/utterances/settings；二期 hotwords 词表。
const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE IF NOT EXISTS sessions (
        id          TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        started_at  TEXT NOT NULL,
        ended_at    TEXT,
        engine      TEXT NOT NULL,
        provider    TEXT NOT NULL,
        model       TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS utterances (
        id          TEXT PRIMARY KEY,
        session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        seq         INTEGER NOT NULL,
        final_text  TEXT NOT NULL,
        audio_path  TEXT,
        created_at  TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_utterances_session ON utterances(session_id, seq);

    CREATE TABLE IF NOT EXISTS settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // v2（二期 hotwords 词表）
    r#"
    CREATE TABLE IF NOT EXISTS hotwords (
        id    TEXT PRIMARY KEY,
        word  TEXT NOT NULL UNIQUE,
        weight INTEGER NOT NULL DEFAULT 1
    );
    "#,
    // v3（风格包：用户自定义输出风格 prompt，F1）
    r#"
    CREATE TABLE IF NOT EXISTS style_packs (
        id            TEXT PRIMARY KEY,
        name          TEXT NOT NULL,
        system_prompt TEXT NOT NULL,
        is_builtin    INTEGER NOT NULL DEFAULT 0,
        ord           INTEGER NOT NULL DEFAULT 0
    );
    "#,
    // v4（R5：前缀角色——风格包加 match_prefix / provider / model / role_kind / output_mode）
    r#"
    ALTER TABLE style_packs ADD COLUMN match_prefix TEXT;
    ALTER TABLE style_packs ADD COLUMN provider TEXT;
    ALTER TABLE style_packs ADD COLUMN model TEXT;
    ALTER TABLE style_packs ADD COLUMN role_kind TEXT NOT NULL DEFAULT 'default';
    ALTER TABLE style_packs ADD COLUMN output_mode TEXT NOT NULL DEFAULT 'insert';
    "#,
];

/// SQLite HistoryStore 实现。
pub struct SqliteStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteStore {
    /// 打开/创建位于 `path` 的库并跑迁移。线程安全（池）。
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.execute_batch(
                "PRAGMA journal_mode = WAL; \
                 PRAGMA foreign_keys = ON;",
            )
        });
        let pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .map_err(|e| Error::Store(format!("连接池创建失败: {e}")))?;

        let store = Self { pool };
        store.run_migrations()?;
        Ok(store)
    }

    /// 用内存库构造（测试专用，单连接）。
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory()
            .with_init(|c| c.execute_batch("PRAGMA foreign_keys = ON;"));
        // 内存库每连接独立，必须只保留 1 连接，否则迁移/数据对不上。
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(|e| Error::Store(format!("内存连接池失败: {e}")))?;
        let store = Self { pool };
        store.run_migrations()?;
        Ok(store)
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn()?;
        let current: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| Error::Store(format!("读取 user_version 失败: {e}")))?;
        let target = MIGRATIONS.len() as u32;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let v = (i + 1) as u32;
            if v <= current {
                continue;
            }
            conn.execute_batch(sql)
                .map_err(|e| Error::Store(format!("迁移到 v{v} 失败: {e}")))?;
            conn.execute_batch(&format!("PRAGMA user_version = {v};"))
                .map_err(|e| Error::Store(format!("写入 user_version={v} 失败: {e}")))?;
        }
        if current == 0 && target == 0 {
            // no-op
        }
        Ok(())
    }

    fn conn(&self) -> Result<r2d2::PooledConnection<SqliteConnectionManager>> {
        self.pool
            .get()
            .map_err(|e| Error::Store(format!("获取连接失败: {e}")))
    }

    /// 取一个连接（应用层用于 settings 表等通用读写）。
    /// 返回的连接归还后即回到池中，可短时持有。
    pub fn conn_for_app(
        &self,
    ) -> std::result::Result<r2d2::PooledConnection<SqliteConnectionManager>, Error> {
        self.conn()
    }

    /// 迁移后的目标 user_version（测试可断言）。
    pub fn migration_version(&self) -> Result<u32> {
        let conn = self.conn()?;
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| Error::Store(format!("读取 user_version 失败: {e}")))
    }

    /// 通用 KV 读（settings 表）。
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let row: rusqlite::Result<String> =
            conn.query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                r.get(0)
            });
        match row {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::Store(format!("get_setting 失败: {e}"))),
        }
    }

    /// 通用 KV 写（upsert）。
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO settings(key,value) VALUES(?,?) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(|e| Error::Store(format!("set_setting 失败: {e}")))?;
        Ok(())
    }
}

fn ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}
fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Store(format!("时间戳解析失败: {e}")))
}

#[async_trait]
impl HistoryStore for SqliteStore {
    async fn create_session(&self, session: &SessionSummary) -> Result<()> {
        let s = session.clone();
        let conn = self.conn()?;
        let ended_ts: Option<String> = s.ended_at.as_ref().map(ts);
        conn.execute(
            "INSERT INTO sessions(id,title,started_at,ended_at,engine,provider,model) \
             VALUES(?,?,?,?,?,?,?)",
            params![
                s.id,
                s.title,
                ts(&s.started_at),
                ended_ts,
                s.engine,
                s.provider,
                s.model,
            ],
        )
        .map_err(|e| Error::Store(format!("create_session 失败: {e}")))?;
        Ok(())
    }

    async fn save_utterance(&self, utterance: &UtteranceRecord) -> Result<()> {
        let u = utterance.clone();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO utterances(id,session_id,seq,final_text,audio_path,created_at) \
             VALUES(?,?,?,?,?,?)",
            params![
                u.id,
                u.session_id,
                u.seq,
                u.final_text,
                u.audio_path,
                ts(&u.created_at),
            ],
        )
        .map_err(|e| Error::Store(format!("save_utterance 失败: {e}")))?;
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,title,started_at,ended_at,engine,provider,model \
                 FROM sessions ORDER BY started_at DESC",
            )
            .map_err(|e| Error::Store(format!("list_sessions prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                let ended: Option<String> = r.get(3)?;
                Ok(SessionSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    started_at: parse_ts(&r.get::<_, String>(2)?).unwrap_or_else(|_| Utc::now()),
                    ended_at: ended.and_then(|s| parse_ts(&s).ok()),
                    engine: r.get(4)?,
                    provider: r.get(5)?,
                    model: r.get(6)?,
                })
            })
            .map_err(|e| Error::Store(format!("list_sessions query 失败: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Store(format!("读取会话行失败: {e}")))?);
        }
        Ok(out)
    }

    async fn list_utterances(&self, session_id: &str) -> Result<Vec<UtteranceRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,session_id,seq,final_text,audio_path,created_at \
                 FROM utterances WHERE session_id=? ORDER BY seq ASC",
            )
            .map_err(|e| Error::Store(format!("list_utterances prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([session_id], |r| {
                let audio: Option<String> = r.get(4)?;
                Ok(UtteranceRecord {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    seq: r.get(2)?,
                    final_text: r.get(3)?,
                    audio_path: audio,
                    created_at: parse_ts(&r.get::<_, String>(5)?).unwrap_or_else(|_| Utc::now()),
                })
            })
            .map_err(|e| Error::Store(format!("list_utterances query 失败: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Store(format!("读取录音行失败: {e}")))?);
        }
        Ok(out)
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn()?;
        // foreign_keys=ON 时级联删 utterances；保险起见显式删一次。
        conn.execute("DELETE FROM sessions WHERE id=?", [session_id])
            .map_err(|e| Error::Store(format!("delete_session 失败: {e}")))?;
        Ok(())
    }
}

// ──────────────────────── 热词词典 ────────────────────────

/// 一条热词。生效路径：L0 拼音同音纠错（`correct_l0`：与热词同音的片段替换为热词）
/// 与润色 LLM 提示词（专有名词保留写法）。
/// 注意：本地 sherpa ASR（SenseVoice / FunASR Nano）不支持热词偏置——sherpa-onnx
/// 仅 transducer 系模型支持热词（且需 modified_beam_search 解码），故热词不影响
/// ASR 解码本身，只作用于识别之后的文本纠错。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hotword {
    pub id: String,
    pub word: String,
    pub weight: i32,
}

/// R5：前缀角色的角色种类。`Translate` 命中走 `translate_text`（与 R4 共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    /// 普通指令角色：用包自身 system_prompt 直连 LLM。
    #[default]
    Default,
    /// 翻译角色：命中后走云翻译（目标语言用全局 translate_target_lang）。
    Translate,
}

/// R5：输出模式。P1 仅 `Insert`（直接插入光标），`Panel` 预留。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Insert,
    Panel,
}

/// 一个风格包（F1）：用户自定义输出风格的系统提示词。
/// Heavy 润色模式下，若选中某风格包，用其 system_prompt 替代默认 Heavy prompt。
/// R5 扩展：`match_prefix` 非空时也是「前缀角色」，听写结果命中前缀则强制按本包处理。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StylePack {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
    pub is_builtin: bool,
    pub ord: i32,
    /// R5：前缀别名，`|` 分隔多别名（如 `邮件|mail|写邮件`）。None/空 = 纯风格包。
    #[serde(default)]
    pub match_prefix: Option<String>,
    /// R5：`None` = cloud（默认）、`cloud`、`local`。
    #[serde(default)]
    pub provider: Option<String>,
    /// R5：覆盖全局 cloud model（可选）。
    #[serde(default)]
    pub model: Option<String>,
    /// R5：`default` / `translate`。
    #[serde(default)]
    pub role_kind: RoleKind,
    /// R5：P1 仅 `insert`。
    #[serde(default)]
    pub output_mode: OutputMode,
}

impl StylePack {
    /// 是否可作为前缀角色（match_prefix 非空）。
    pub fn is_prefix_role(&self) -> bool {
        self.match_prefix
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }
}

impl SqliteStore {
    pub fn list_hotwords(&self) -> Result<Vec<Hotword>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, word, weight FROM hotwords ORDER BY rowid DESC")
            .map_err(|e| Error::Store(format!("list_hotwords prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Hotword {
                    id: r.get(0)?,
                    word: r.get(1)?,
                    weight: r.get(2)?,
                })
            })
            .map_err(|e| Error::Store(format!("list_hotwords query 失败: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Store(format!("读取热词行失败: {e}")))?);
        }
        Ok(out)
    }

    pub fn add_hotword(&self, word: &str, weight: i32) -> Result<Hotword> {
        let conn = self.conn()?;
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO hotwords(id, word, weight) VALUES(?,?,?)",
            params![id, word, weight],
        )
        .map_err(|e| Error::Store(format!("add_hotword 失败: {e}")))?;
        Ok(Hotword {
            id,
            word: word.to_string(),
            weight,
        })
    }

    /// 批量加热词（去重：忽略空行与已存在的词）。返回新增数量。
    pub fn add_hotwords_batch(&self, words: &[String]) -> Result<usize> {
        let mut existing: std::collections::HashSet<String> =
            self.list_hotwords()?.into_iter().map(|h| h.word).collect();
        let conn = self.conn()?;
        let mut added = 0usize;
        for w in words {
            let w = w.trim();
            if w.is_empty() || !existing.insert(w.to_string()) {
                continue;
            }
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO hotwords(id, word, weight) VALUES(?,?,?)",
                params![id, w, 1],
            )
            .map_err(|e| Error::Store(format!("add_hotwords_batch 失败: {e}")))?;
            added += 1;
        }
        Ok(added)
    }

    pub fn delete_hotword(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM hotwords WHERE id=?", [id])
            .map_err(|e| Error::Store(format!("delete_hotword 失败: {e}")))?;
        Ok(())
    }

    /// 跨会话搜索录音文本（D2 历史搜索）：LIKE 模糊匹配，按时间倒序，限 200 条。
    pub fn search_utterances(&self, query: &str) -> Result<Vec<UtteranceRecord>> {
        let conn = self.conn()?;
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT id,session_id,seq,final_text,audio_path,created_at \
                 FROM utterances WHERE final_text LIKE ? ORDER BY created_at DESC LIMIT 200",
            )
            .map_err(|e| Error::Store(format!("search_utterances prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([&pattern], |r| {
                let audio: Option<String> = r.get(4)?;
                Ok(UtteranceRecord {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    seq: r.get(2)?,
                    final_text: r.get(3)?,
                    audio_path: audio,
                    created_at: parse_ts(&r.get::<_, String>(5)?).unwrap_or_else(|_| Utc::now()),
                })
            })
            .map_err(|e| Error::Store(format!("search_utterances query 失败: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Store(format!("读取搜索行失败: {e}")))?);
        }
        Ok(out)
    }

    /// D1：导出所有录音为 Markdown 日记（按日期分组，每条含时间+文本）。
    pub fn export_diary_markdown(&self) -> Result<String> {
        use std::collections::BTreeMap;
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT final_text, created_at FROM utterances ORDER BY created_at ASC")
            .map_err(|e| Error::Store(format!("export_diary prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| Error::Store(format!("export_diary query 失败: {e}")))?;
        let mut by_day: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for row in rows {
            let (text, ts) = row.map_err(|e| Error::Store(format!("读取日记行失败: {e}")))?;
            let date = ts.get(..10).unwrap_or("").to_string();
            by_day.entry(date).or_default().push((text, ts));
        }
        let mut md = String::from("# openIME 日记\n\n");
        for (date, items) in &by_day {
            if !date.is_empty() {
                md.push_str(&format!("## {date}\n\n"));
            }
            for (text, ts) in items {
                let time = ts.get(11..19).unwrap_or("");
                md.push_str(&format!("- {time} {text}\n"));
            }
            md.push('\n');
        }
        Ok(md)
    }

    // ── 风格包（F1）──

    pub fn list_style_packs(&self) -> Result<Vec<StylePack>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, system_prompt, is_builtin, ord, \
                 match_prefix, provider, model, role_kind, output_mode FROM style_packs \
                 ORDER BY ord ASC, rowid ASC",
            )
            .map_err(|e| Error::Store(format!("list_style_packs prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(StylePack {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    system_prompt: r.get(2)?,
                    is_builtin: r.get::<_, i32>(3)? != 0,
                    ord: r.get(4)?,
                    match_prefix: r.get(5)?,
                    provider: r.get(6)?,
                    model: r.get(7)?,
                    role_kind: parse_role_kind(&r.get::<_, String>(8)?),
                    output_mode: parse_output_mode(&r.get::<_, String>(9)?),
                })
            })
            .map_err(|e| Error::Store(format!("list_style_packs query 失败: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Store(format!("读取风格包行失败: {e}")))?);
        }
        Ok(out)
    }

    pub fn upsert_style_pack(&self, p: &StylePack) -> Result<()> {
        // FR-5.8：拒绝相同别名（忽略大小写、逐别名比较；与其它包冲突则整单失败）。
        if let Some(spec) = p.match_prefix.as_deref() {
            let aliases: Vec<String> = spec
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect();
            if !aliases.is_empty() {
                for other in self.list_style_packs()? {
                    if other.id == p.id {
                        continue;
                    }
                    let Some(other_spec) = other.match_prefix.as_deref() else {
                        continue;
                    };
                    let clash = other_spec
                        .split('|')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_lowercase())
                        .any(|a| aliases.iter().any(|mine| mine == &a));
                    if clash {
                        return Err(Error::Store(format!(
                            "前缀别名冲突：与「{}」存在相同别名（{}）",
                            other.name, spec
                        )));
                    }
                }
            }
        }
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO style_packs(id, name, system_prompt, is_builtin, ord, \
             match_prefix, provider, model, role_kind, output_mode) \
             VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, system_prompt=excluded.system_prompt, \
             is_builtin=excluded.is_builtin, ord=excluded.ord, match_prefix=excluded.match_prefix, \
             provider=excluded.provider, model=excluded.model, role_kind=excluded.role_kind, \
             output_mode=excluded.output_mode",
            params![
                p.id,
                p.name,
                p.system_prompt,
                if p.is_builtin { 1 } else { 0 },
                p.ord,
                p.match_prefix,
                p.provider,
                p.model,
                role_kind_str(p.role_kind),
                output_mode_str(p.output_mode),
            ],
        )
        .map_err(|e| Error::Store(format!("upsert_style_pack 失败: {e}")))?;
        Ok(())
    }

    pub fn delete_style_pack(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM style_packs WHERE id=? AND is_builtin=0", [id])
            .map_err(|e| Error::Store(format!("delete_style_pack 失败: {e}")))?;
        Ok(())
    }

    /// 已下架的内置纯风格包 id（正式 / 口语 / commit message）。
    /// 本地三件套方案后不再引导「风格包」概念，内置只保留前缀角色包。
    pub const LEGACY_BUILTIN_STYLE_PACK_IDS: [&str; 3] =
        ["builtin-formal", "builtin-casual", "builtin-commit"];

    /// 一次性清理已下架的内置风格包（幂等，每次启动跑）。
    ///
    /// 内置包不受 `delete_style_pack` 的 is_builtin=0 限制，直接 SQL 删。
    /// 返回被删的 id 列表（调用方据此清 `active_style_pack_id` 悬空引用）。
    pub fn remove_legacy_builtin_style_packs(&self) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut removed = Vec::new();
        for id in Self::LEGACY_BUILTIN_STYLE_PACK_IDS {
            let n = conn
                .execute("DELETE FROM style_packs WHERE id=? AND is_builtin=1", [id])
                .map_err(|e| {
                    Error::Store(format!("remove_legacy_builtin_style_packs 失败: {e}"))
                })?;
            if n > 0 {
                removed.push(id.to_string());
            }
        }
        Ok(removed)
    }

    /// R5：内置前缀包种子——按 id 补缺失项（不清空用户对内置包的修改，
    /// 只插入不存在的 id；用户厌恶种子时把 match_prefix 清空即可，不被重新种回）。
    /// 已存在的内置包会同步 `ord`（内置排序非用户改动，保证「翻译」排第一）。
    pub fn seed_builtin_prefix_packs_if_missing(&self) -> Result<()> {
        let existing: Vec<String> = self.list_style_packs()?.into_iter().map(|p| p.id).collect();
        {
            let conn = self.conn()?;
            for (id, _, _, _, _, ord) in BUILTIN_PREFIX_PACKS {
                conn.execute(
                    "UPDATE style_packs SET ord=? WHERE id=? AND is_builtin=1",
                    params![ord, id],
                )
                .map_err(|e| Error::Store(format!("内置角色 ord 同步失败: {e}")))?;
            }
        }
        for (id, name, prefix, kind, prompt, ord) in BUILTIN_PREFIX_PACKS {
            if existing.iter().any(|e| e == id) {
                continue;
            }
            self.upsert_style_pack(&StylePack {
                id: id.into(),
                name: name.into(),
                system_prompt: prompt.into(),
                is_builtin: true,
                ord,
                match_prefix: Some(prefix.into()),
                provider: None,
                model: None,
                role_kind: kind,
                output_mode: OutputMode::Insert,
            })?;
        }
        Ok(())
    }

    /// 同步「助手名+角色别名」组合热词（每次启动执行，幂等）。
    ///
    /// 组合词（小友翻译 / 小友邮件 / 小友命令 …）写入热词表后，L0 拼音纠错把
    /// ASR 错写的同音组合（小又翻忆 等）精准纠回——前缀角色的触发锚定在自定义
    /// 非常见词上，不再依赖 ASR 标点输出。
    ///
    /// - settings 记录上次同步的助手名；**改名时删除旧组合词、写入新组合**
    ///   （仅限本机制写入的组合词，用户手动加的词不动）。
    /// - 助手名为空：删旧组合、不写入（功能关闭）。
    /// - 一次性清理旧设计的裸别名热词（翻译/邮件/写邮件/命令/指令），
    ///   避免残留误纠（「明令」→「命令」）。
    ///
    /// 热词生效路径说明：本地 ASR（SenseVoice / FunASR Nano）不支持 sherpa 原生
    /// 热词偏置（仅 transducer 系模型支持），热词走 L0 拼音同音纠错。
    pub fn sync_assistant_combo_hotwords(&self, assistant_name: &str) -> Result<()> {
        const LAST_NAME_KEY: &str = "assistant_hotwords_synced_name";
        const LEGACY_FLAG: &str = "builtin_prefix_alias_hotwords_seeded";
        let name = assistant_name.trim().to_string();
        if self.get_setting(LAST_NAME_KEY)?.as_deref() == Some(name.as_str()) {
            return Ok(()); // 助手名未变：组合词已在库。
        }
        // 改名（或迁移）：删掉旧助手名的组合词（只删本机制写入的组合）。
        let renamed_from = self.get_setting(LAST_NAME_KEY)?;
        if let Some(old) = renamed_from.as_deref() {
            self.remove_combo_hotwords(old)?;
        }
        // 一次性清理旧设计的裸别名热词。
        if self.get_setting(LEGACY_FLAG)?.is_some() {
            for word in ["翻译", "邮件", "写邮件", "命令", "指令"] {
                self.delete_hotword_by_word(word)?;
            }
            self.set_setting(LEGACY_FLAG, "cleaned")?;
        }
        // 写入新组合词（仅纯汉字组合；英文组合无拼音不参与纠错）。
        let packs = self.list_style_packs()?;
        let combos: Vec<String> = crate::polish::assistant_combo_words(&name, &packs)
            .into_iter()
            .filter(|w| is_all_hanzi(w))
            .collect();
        if !combos.is_empty() {
            self.add_hotwords_batch(&combos)?;
        }
        self.set_setting(LAST_NAME_KEY, &name)?;
        tracing::info!(
            "助手组合热词已同步：{}（新增 {} 词；原名 {:?} 的组合词已删除）",
            if name.is_empty() {
                "（空，功能关闭）"
            } else {
                &name
            },
            combos.len(),
            renamed_from.as_deref().unwrap_or("")
        );
        Ok(())
    }

    /// 删除某助手名下的全部组合热词（按词精确匹配，找不到则跳过）。
    fn remove_combo_hotwords(&self, assistant_name: &str) -> Result<()> {
        let packs = self.list_style_packs()?;
        let words = crate::polish::assistant_combo_words(assistant_name, &packs);
        for w in words {
            self.delete_hotword_by_word(&w)?;
        }
        Ok(())
    }

    /// 按词删除一条热词（词表 UI 之外的程序化清理用；不存在则静默）。
    fn delete_hotword_by_word(&self, word: &str) -> Result<()> {
        if let Some(h) = self.list_hotwords()?.into_iter().find(|h| h.word == word) {
            self.delete_hotword(&h.id)?;
        }
        Ok(())
    }
}

/// 全汉字判定（拼音非空且覆盖全部字符）。
fn is_all_hanzi(s: &str) -> bool {
    use pinyin::ToPinyin;
    let n = s.chars().count();
    n > 0 && s.to_pinyin().flatten().count() == n
}

/// 内置前缀角色包清单（与 [`SqliteStore::seed_builtin_prefix_packs_if_missing`] 共享）。
/// ord 即列表排序：翻译排第一（最高频角色），邮件、命令随后。
const BUILTIN_PREFIX_PACKS: [(&str, &str, &str, RoleKind, &str, i32); 3] = [
    (
        "builtin-role-translate",
        "翻译",
        "翻译|translate|译",
        RoleKind::Translate,
        // fallback prompt：命中 translate 角色实际走 translate_text；本 prompt 仅兜底。
        "把语音内容翻译成目标语言，只输出译文。",
        10,
    ),
    (
        "builtin-role-mail",
        "邮件",
        "邮件|mail|写邮件",
        RoleKind::Default,
        "你是中文语音输入助手。请把语音内容改写为正式得体的邮件正文，\
         只输出邮件正文本身（含称呼与落款），不要解释、不要加引号。",
        11,
    ),
    (
        "builtin-role-cmd",
        "命令",
        "命令|command|指令",
        RoleKind::Default,
        "你是命令行助手。把语音内容转换为一条可直接粘贴执行的命令，\
         只输出命令本身，不要解释、不要代码块标记。",
        12,
    ),
];

fn role_kind_str(k: RoleKind) -> String {
    match k {
        RoleKind::Default => "default".into(),
        RoleKind::Translate => "translate".into(),
    }
}

fn parse_role_kind(s: &str) -> RoleKind {
    match s {
        "translate" => RoleKind::Translate,
        _ => RoleKind::Default,
    }
}

fn output_mode_str(m: OutputMode) -> String {
    match m {
        OutputMode::Insert => "insert".into(),
        OutputMode::Panel => "panel".into(),
    }
}

fn parse_output_mode(s: &str) -> OutputMode {
    match s {
        "panel" => OutputMode::Panel,
        _ => OutputMode::Insert,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            title: format!("会话-{id}"),
            started_at: Utc::now(),
            ended_at: None,
            engine: "local".into(),
            provider: "sherpa".into(),
            model: "paraformer-online".into(),
        }
    }

    #[tokio::test]
    async fn migrations_apply_and_version_is_target() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.migration_version().unwrap(), MIGRATIONS.len() as u32);
    }

    #[tokio::test]
    async fn session_crud_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let s = sample_session("s1");
        store.create_session(&s).await.unwrap();
        let listed = store.list_sessions().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "s1");
        assert_eq!(listed[0].model, "paraformer-online");

        store.delete_session("s1").await.unwrap();
        assert!(store.list_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn utterances_ordered_by_seq() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.create_session(&sample_session("s1")).await.unwrap();

        for (seq, text) in [("1", "你好"), ("2", "世界")] {
            store
                .save_utterance(&UtteranceRecord {
                    id: format!("u-{seq}"),
                    session_id: "s1".into(),
                    seq: seq.parse().unwrap(),
                    final_text: text.into(),
                    audio_path: None,
                    created_at: Utc::now(),
                })
                .await
                .unwrap();
        }

        let us = store.list_utterances("s1").await.unwrap();
        assert_eq!(us.len(), 2);
        assert_eq!(us[0].seq, 1);
        assert_eq!(us[0].final_text, "你好");
        assert_eq!(us[1].seq, 2);
        assert_eq!(us[1].final_text, "世界");
    }

    #[tokio::test]
    async fn delete_session_cascades_utterances() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.create_session(&sample_session("s1")).await.unwrap();
        store
            .save_utterance(&UtteranceRecord {
                id: "u-1".into(),
                session_id: "s1".into(),
                seq: 1,
                final_text: "x".into(),
                audio_path: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        store.delete_session("s1").await.unwrap();
        assert!(store.list_utterances("s1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ended_at_optional_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut s = sample_session("s1");
        s.ended_at = Some(Utc::now());
        store.create_session(&s).await.unwrap();
        let got = store.list_sessions().await.unwrap();
        assert!(got[0].ended_at.is_some());
    }

    #[tokio::test]
    async fn file_db_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ime.db");

        {
            let store = SqliteStore::open(&path).unwrap();
            store.create_session(&sample_session("s1")).await.unwrap();
        }
        {
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.migration_version().unwrap(), MIGRATIONS.len() as u32);
            assert_eq!(store.list_sessions().await.unwrap().len(), 1);
        }
    }

    #[test]
    fn hotwords_batch_dedup_and_skip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let added = store
            .add_hotwords_batch(&[
                "智谱".into(),
                "Paraformer".into(),
                " 智谱 ".into(),
                "".into(),
            ])
            .unwrap();
        // "智谱" 与 " 智谱 "(trim 后) 去重，空行忽略 → 新增 2。
        assert_eq!(added, 2);
        assert_eq!(store.list_hotwords().unwrap().len(), 2);
        // 再次导入含重复词：只新增不重复的。
        let again = store
            .add_hotwords_batch(&["智谱".into(), "新词".into()])
            .unwrap();
        assert_eq!(again, 1);
        assert_eq!(store.list_hotwords().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn search_utterances_like_match() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.create_session(&sample_session("s1")).await.unwrap();
        store
            .save_utterance(&UtteranceRecord {
                id: "u1".into(),
                session_id: "s1".into(),
                seq: 0,
                final_text: "今天讨论智谱模型".into(),
                audio_path: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        store
            .save_utterance(&UtteranceRecord {
                id: "u2".into(),
                session_id: "s1".into(),
                seq: 1,
                final_text: "明天天气不错".into(),
                audio_path: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        let r = store.search_utterances("智谱").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].final_text, "今天讨论智谱模型");
        assert!(store.search_utterances("不存在的词xyz").unwrap().is_empty());
    }

    #[test]
    fn style_packs_crud_and_seed() {
        let store = SqliteStore::open_in_memory().unwrap();
        // 内置纯风格包（正式/口语/commit）已下架，不再种子；手动 upsert 三个做 CRUD 验证。
        for (seed, id) in ["builtin-formal", "builtin-casual", "builtin-commit"]
            .into_iter()
            .enumerate()
        {
            store
                .upsert_style_pack(&StylePack {
                    id: id.into(),
                    name: format!("包{id}"),
                    system_prompt: "test".into(),
                    is_builtin: true,
                    ord: seed as i32,
                    match_prefix: None,
                    provider: None,
                    model: None,
                    role_kind: RoleKind::Default,
                    output_mode: OutputMode::Insert,
                })
                .unwrap();
        }
        let packs = store.list_style_packs().unwrap();
        assert_eq!(packs.len(), 3);
        // upsert 自定义
        store
            .upsert_style_pack(&StylePack {
                id: "my".into(),
                name: "我的".into(),
                system_prompt: "test".into(),
                is_builtin: false,
                ord: 10,
                match_prefix: None,
                provider: None,
                model: None,
                role_kind: RoleKind::Default,
                output_mode: OutputMode::Insert,
            })
            .unwrap();
        assert_eq!(store.list_style_packs().unwrap().len(), 4);
        // 删自定义 OK，删内置无效（常规删除通道）
        store.delete_style_pack("my").unwrap();
        assert_eq!(store.list_style_packs().unwrap().len(), 3);
        store.delete_style_pack("builtin-formal").unwrap();
        assert_eq!(store.list_style_packs().unwrap().len(), 3); // 常规通道删不掉内置
    }

    #[test]
    fn legacy_builtin_style_packs_removed_and_idempotent() {
        let store = SqliteStore::open_in_memory().unwrap();
        // 造一个已下架内置包 + 一个自建包。
        for (id, builtin) in [("builtin-formal", true), ("my-own", false)] {
            store
                .upsert_style_pack(&StylePack {
                    id: id.into(),
                    name: id.into(),
                    system_prompt: "p".into(),
                    is_builtin: builtin,
                    ord: 0,
                    match_prefix: None,
                    provider: None,
                    model: None,
                    role_kind: RoleKind::Default,
                    output_mode: OutputMode::Insert,
                })
                .unwrap();
        }
        let removed = store.remove_legacy_builtin_style_packs().unwrap();
        assert_eq!(removed, vec!["builtin-formal".to_string()]);
        // 只删内置下架包，自建包不受影响。
        let ids: Vec<String> = store
            .list_style_packs()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec!["my-own".to_string()]);
        // 幂等：再跑一遍无返回。
        assert!(store
            .remove_legacy_builtin_style_packs()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn prefix_pack_seed_syncs_builtin_ord_translate_first() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.seed_builtin_prefix_packs_if_missing().unwrap();
        let packs = store.list_style_packs().unwrap();
        // 翻译排第一（ord 10），邮件 11、命令 12。
        assert_eq!(packs[0].id, "builtin-role-translate");
        assert_eq!(packs[0].ord, 10);
        assert_eq!(packs[1].id, "builtin-role-mail");
        assert_eq!(packs[2].id, "builtin-role-cmd");

        // 已有库（旧 ord：mail=10 在前）→ seed 后内置排序被同步为翻译第一，
        // 但用户改过的 match_prefix 不被覆盖。
        let store2 = SqliteStore::open_in_memory().unwrap();
        store2
            .upsert_style_pack(&StylePack {
                id: "builtin-role-mail".into(),
                name: "邮件".into(),
                system_prompt: "p".into(),
                is_builtin: true,
                ord: 10,
                match_prefix: None, // 用户清空（禁用该角色）
                provider: None,
                model: None,
                role_kind: RoleKind::Default,
                output_mode: OutputMode::Insert,
            })
            .unwrap();
        store2.seed_builtin_prefix_packs_if_missing().unwrap();
        let packs = store2.list_style_packs().unwrap();
        assert_eq!(packs[0].id, "builtin-role-translate", "翻译应排第一");
        let mail = packs.iter().find(|p| p.id == "builtin-role-mail").unwrap();
        assert_eq!(mail.ord, 11, "内置 ord 应被同步");
        assert_eq!(mail.match_prefix, None, "用户清空的前缀不得被种回");
    }

    #[test]
    fn v4_role_fields_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.migration_version().unwrap(), 4);
        let mut p = StylePack {
            id: "role-1".into(),
            name: "翻译".into(),
            system_prompt: "把内容翻译成目标语言".into(),
            is_builtin: true,
            ord: 7,
            match_prefix: Some("翻译|translate".into()),
            provider: Some("cloud".into()),
            model: Some("qwen-plus".into()),
            role_kind: RoleKind::Translate,
            output_mode: OutputMode::Insert,
        };
        store.upsert_style_pack(&p).unwrap();
        let got = store.list_style_packs().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], p);
        assert!(got[0].is_prefix_role());
        // 清空前缀 → 退回纯风格包。
        p.match_prefix = None;
        store.upsert_style_pack(&p).unwrap();
        let got = store.list_style_packs().unwrap();
        assert!(!got[0].is_prefix_role());
    }

    #[test]
    fn v4_migrates_v3_table() {
        // 从 v3 库升级：旧行 match_prefix=NULL、role_kind='default'。
        let manager = SqliteConnectionManager::memory()
            .with_init(|c| c.execute_batch("PRAGMA foreign_keys = ON;"));
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        // 手工建到 v3。
        {
            let conn = pool.get().unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY, title TEXT NOT NULL, started_at TEXT NOT NULL,
                    ended_at TEXT, engine TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS utterances (
                    id TEXT PRIMARY KEY, session_id TEXT NOT NULL, seq INTEGER NOT NULL,
                    final_text TEXT NOT NULL, audio_path TEXT, created_at TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS hotwords (id TEXT PRIMARY KEY, word TEXT NOT NULL UNIQUE, weight INTEGER NOT NULL DEFAULT 1);
                CREATE TABLE IF NOT EXISTS style_packs (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, system_prompt TEXT NOT NULL,
                    is_builtin INTEGER NOT NULL DEFAULT 0, ord INTEGER NOT NULL DEFAULT 0);
                INSERT INTO style_packs(id,name,system_prompt,is_builtin,ord)
                    VALUES('builtin-formal','正式','p',1,0);
                PRAGMA user_version = 3;
                "#,
            )
            .unwrap();
        }
        // 用迁移入口打开（SQLite v4 ALTER TABLE 直接作用在既有连接上）。
        let conn = pool.get().unwrap();
        let current: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(current, 3);
        conn.execute_batch(MIGRATIONS[3]).unwrap();
        conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        let packs: Vec<(Option<String>, String)> = {
            let mut stmt = conn
                .prepare("SELECT match_prefix, role_kind FROM style_packs")
                .unwrap();
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].0, None);
        assert_eq!(packs[0].1, "default");
    }

    #[test]
    fn seed_prefix_packs_inserts_missing_only() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.seed_builtin_prefix_packs_if_missing().unwrap();
        let packs = store.list_style_packs().unwrap();
        assert_eq!(packs.len(), 3);
        let mail = packs.iter().find(|p| p.id == "builtin-role-mail").unwrap();
        assert_eq!(mail.match_prefix.as_deref(), Some("邮件|mail|写邮件"));
        let tr = packs
            .iter()
            .find(|p| p.id == "builtin-role-translate")
            .unwrap();
        assert_eq!(tr.role_kind, RoleKind::Translate);
        // 再 seed 不重复、不清用户改动。
        store
            .upsert_style_pack(&StylePack {
                id: "builtin-role-mail".into(),
                name: "邮件".into(),
                system_prompt: "自定义".into(),
                is_builtin: true,
                ord: 10,
                match_prefix: None,
                provider: None,
                model: None,
                role_kind: RoleKind::Default,
                output_mode: OutputMode::Insert,
            })
            .unwrap();
        store.seed_builtin_prefix_packs_if_missing().unwrap();
        let packs = store.list_style_packs().unwrap();
        assert_eq!(packs.len(), 3);
        let mail = packs.iter().find(|p| p.id == "builtin-role-mail").unwrap();
        assert_eq!(mail.match_prefix, None, "种子不得覆盖用户清空的前缀");
        assert_eq!(mail.system_prompt, "自定义");
    }

    #[test]
    fn assistant_combo_hotwords_synced_and_renamed() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.seed_builtin_prefix_packs_if_missing().unwrap();
        // 首次同步：组合词入库（纯汉字组合；英文组合过滤）。
        store.sync_assistant_combo_hotwords("小友").unwrap();
        let words: Vec<String> = store
            .list_hotwords()
            .unwrap()
            .into_iter()
            .map(|h| h.word)
            .collect();
        for w in [
            "小友翻译",
            "小友译",
            "小友邮件",
            "小友写邮件",
            "小友命令",
            "小友指令",
        ] {
            assert!(words.contains(&w.to_string()), "缺少组合热词 {w}");
        }
        for w in ["小友translate", "小友mail", "小友command"] {
            assert!(!words.contains(&w.to_string()), "{w} 不应入热词（无拼音）");
        }
        // 同名再跑：幂等不重复。
        store.sync_assistant_combo_hotwords("小友").unwrap();
        assert_eq!(store.list_hotwords().unwrap().len(), 6);

        // 改名：旧组合删除、新组合写入；用户手动加的词不受影响。
        store.add_hotword("我的自定义词", 1).unwrap();
        store.sync_assistant_combo_hotwords("阿法").unwrap();
        let words: Vec<String> = store
            .list_hotwords()
            .unwrap()
            .into_iter()
            .map(|h| h.word)
            .collect();
        assert!(!words.contains(&"小友翻译".to_string()), "旧组合应删除");
        assert!(words.contains(&"阿法翻译".to_string()), "新组合应写入");
        assert!(words.contains(&"我的自定义词".to_string()), "用户词不动");

        // 助手名改空：删组合不写入（功能关闭）。
        store.sync_assistant_combo_hotwords("").unwrap();
        let words: Vec<String> = store
            .list_hotwords()
            .unwrap()
            .into_iter()
            .map(|h| h.word)
            .collect();
        assert!(!words.contains(&"阿法翻译".to_string()), "空名应删组合");
        assert!(words.contains(&"我的自定义词".to_string()));
    }

    #[test]
    fn legacy_bare_alias_hotwords_cleaned_once() {
        // 旧设计种的裸别名热词（含 flag）→ 首次同步时清理，防「明令→命令」类误纠残留。
        let store = SqliteStore::open_in_memory().unwrap();
        store.seed_builtin_prefix_packs_if_missing().unwrap();
        store
            .set_setting("builtin_prefix_alias_hotwords_seeded", "1")
            .unwrap();
        for w in ["翻译", "邮件", "写邮件", "命令", "指令"] {
            store.add_hotword(w, 1).unwrap();
        }
        store.sync_assistant_combo_hotwords("小友").unwrap();
        let words: Vec<String> = store
            .list_hotwords()
            .unwrap()
            .into_iter()
            .map(|h| h.word)
            .collect();
        assert!(!words.contains(&"翻译".to_string()), "裸别名应被清理");
        assert!(!words.contains(&"命令".to_string()), "裸别名应被清理");
        assert!(words.contains(&"小友翻译".to_string()), "组合词正常写入");
        // 用户后来手动加回的裸别名不再被清（flag 已消费）。
        store.add_hotword("翻译", 1).unwrap();
        store.sync_assistant_combo_hotwords("小友").unwrap();
        let words: Vec<String> = store
            .list_hotwords()
            .unwrap()
            .into_iter()
            .map(|h| h.word)
            .collect();
        assert!(words.contains(&"翻译".to_string()), "用户手动加回的词不动");
    }

    #[test]
    fn upsert_rejects_duplicate_prefix_alias() {
        // FR-5.8：与其它包存在相同别名 → 整单失败。
        let store = SqliteStore::open_in_memory().unwrap();
        let p = |id: &str, prefix: &str| StylePack {
            id: id.into(),
            name: id.into(),
            system_prompt: "p".into(),
            is_builtin: false,
            ord: 0,
            match_prefix: Some(prefix.into()),
            provider: None,
            model: None,
            role_kind: RoleKind::Default,
            output_mode: OutputMode::Insert,
        };
        store.upsert_style_pack(&p("a", "邮件|mail")).unwrap();
        assert!(store.upsert_style_pack(&p("b", "mail")).is_err());
        assert!(
            store.upsert_style_pack(&p("b", "MAIL")).is_err(),
            "忽略大小写"
        );
        // 同一包的自己更新不冲突。
        assert!(store.upsert_style_pack(&p("a", "邮件|mail|写邮件")).is_ok());
        // 无前缀包不参与冲突。
        assert!(store.upsert_style_pack(&p("c", "")).is_ok());
    }
}
