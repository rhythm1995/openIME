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
use serde::{Deserialize, Serialize};

use crate::traits::{HistoryStore, SessionSummary, UtteranceRecord};
use crate::{Error, Result};

/// 迁移 SQL 列表。下标+1 即 user_version。
/// 一期建 sessions/utterances/settings；二期预留 personas/hotwords 空表。
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
    // v2（二期预留，先建空表，避免后续破坏性迁移）
    r#"
    CREATE TABLE IF NOT EXISTS personas (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        prompt     TEXT NOT NULL,
        is_builtin INTEGER NOT NULL DEFAULT 0,
        ord        INTEGER NOT NULL DEFAULT 0,
        hidden     INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS hotwords (
        id    TEXT PRIMARY KEY,
        word  TEXT NOT NULL UNIQUE,
        weight INTEGER NOT NULL DEFAULT 1
    );
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

/// 一条热词。Fun-ASR/Sherpa 通过热词加权提升特定术语识别率；
/// 百炼通过 vocabulary_id 引用服务端词表。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hotword {
    pub id: String,
    pub word: String,
    pub weight: i32,
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

    pub fn delete_hotword(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM hotwords WHERE id=?", [id])
            .map_err(|e| Error::Store(format!("delete_hotword 失败: {e}")))?;
        Ok(())
    }

    /// 导出为 sherpa-onnx hotwords.txt 格式（每行：词 权重）。
    pub fn export_hotwords_sherpa(&self) -> Result<String> {
        let words = self.list_hotwords()?;
        let mut out = String::new();
        for w in words {
            out.push_str(&format!("{} {}\n", w.word, w.weight));
        }
        Ok(out)
    }

    // ── 二期：人设 ──

    pub fn list_personas(&self) -> Result<Vec<Persona>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, prompt, is_builtin, ord, hidden FROM personas \
                 WHERE hidden = 0 ORDER BY ord ASC, rowid ASC",
            )
            .map_err(|e| Error::Store(format!("list_personas prepare 失败: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Persona {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    prompt: r.get(2)?,
                    is_builtin: r.get::<_, i32>(3)? != 0,
                    ord: r.get(4)?,
                    hidden: r.get::<_, i32>(5)? != 0,
                })
            })
            .map_err(|e| Error::Store(format!("list_personas query 失败: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Store(format!("读取人设行失败: {e}")))?);
        }
        Ok(out)
    }

    pub fn upsert_persona(&self, p: &Persona) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO personas(id, name, prompt, is_builtin, ord, hidden) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               name=excluded.name, prompt=excluded.prompt, \
               is_builtin=excluded.is_builtin, ord=excluded.ord, hidden=excluded.hidden",
            params![
                p.id,
                p.name,
                p.prompt,
                if p.is_builtin { 1 } else { 0 },
                p.ord,
                if p.hidden { 1 } else { 0 },
            ],
        )
        .map_err(|e| Error::Store(format!("upsert_persona 失败: {e}")))?;
        Ok(())
    }

    pub fn delete_persona(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM personas WHERE id=? AND is_builtin=0", [id])
            .map_err(|e| Error::Store(format!("delete_persona 失败: {e}")))?;
        Ok(())
    }

    /// 写入内置人设（仅当表中无该 id 时）。
    pub fn seed_builtin_personas_if_empty(&self) -> Result<()> {
        let existing = self.list_personas()?;
        if !existing.is_empty() {
            return Ok(());
        }
        let builtins = [
            (
                "builtin-formal",
                "正式",
                "请用正式、简洁的书面语改写，适合工作消息。",
                0,
            ),
            (
                "builtin-casual",
                "口语",
                "请保持口语自然，略作通顺即可，不要过于书面。",
                1,
            ),
            (
                "builtin-email",
                "邮件",
                "请改写成适合中文商务邮件正文的语气，礼貌但不冗长。",
                2,
            ),
        ];
        for (id, name, prompt, ord) in builtins {
            self.upsert_persona(&Persona {
                id: id.into(),
                name: name.into(),
                prompt: prompt.into(),
                is_builtin: true,
                ord,
                hidden: false,
            })?;
        }
        Ok(())
    }
}

/// 二期人设。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub is_builtin: bool,
    pub ord: i32,
    pub hidden: bool,
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
}
