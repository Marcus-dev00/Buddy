//! Local SQLite persistence (%LOCALAPPDATA%/Buddy/buddy.db) with FTS5 search,
//! cross-channel SHA-256 deduplication, and 7-day automatic expiration cleanup.

use crate::media_utils::text_sha256;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub message_count: i64,
    pub source: String, // "desktop" | "mobile"
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredChatMessage {
    pub id: i64,
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub attachments_json: Option<String>,
    pub agent_steps_json: Option<String>,
    pub thought_text: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryItem {
    pub id: i64,
    pub kind: String, // "screenshot" | "clipboard_text" | "clipboard_image" | "fact"
    pub content_text: Option<String>,
    pub file_path: Option<String>,
    pub thumb_path: Option<String>,
    pub sha256: String,
    pub app_name: String,
    pub is_starred: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserBookmark {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReminderItem {
    pub id: i64,
    pub title: String,
    pub fire_at_ms: i64,
    pub repeat_rule: Option<String>, // None | "daily" | "weekdays" | "weekly"
    pub enabled: bool,
    pub fired_count: i64,
    pub created_at: i64,
}

pub struct MemoryDb {
    conn: Mutex<Connection>,
    pub data_dir: PathBuf,
    pub screenshots_dir: PathBuf,
    pub thumbnails_dir: PathBuf,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn resolve_data_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Buddy");
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home).join(".buddy");
    }
    PathBuf::from(".buddy")
}

impl MemoryDb {
    pub fn open() -> Result<Self, String> {
        Self::open_with_dir(resolve_data_dir())
    }

    #[cfg(test)]
    pub fn open_at(data_dir: PathBuf) -> Result<Self, String> {
        Self::open_with_dir(data_dir)
    }

    fn ensure_column(conn: &Connection, table: &str, col: &str, alter_sql: &str) -> Result<(), String> {
        let pragma = format!("PRAGMA table_info({table})");
        let cols: Vec<String> = conn
            .prepare(&pragma)
            .and_then(|mut s| {
                let rows = s.query_map([], |r| r.get::<_, String>(1))?;
                Ok(rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default();
        if !cols.iter().any(|c| c == col) {
            conn.execute(alter_sql, [])
                .map_err(|e| format!("migrate {table}.{col} failed: {e}"))?;
        }
        Ok(())
    }

    fn open_with_dir(data_dir: PathBuf) -> Result<Self, String> {
        let screenshots_dir = data_dir.join("screenshots");
        let thumbnails_dir = data_dir.join("thumbnails");

        std::fs::create_dir_all(&screenshots_dir)
            .map_err(|e| format!("create screenshots dir failed: {e}"))?;
        std::fs::create_dir_all(&thumbnails_dir)
            .map_err(|e| format!("create thumbnails dir failed: {e}"))?;

        let db_path = data_dir.join("buddy.db");
        let conn = Connection::open(&db_path).map_err(|e| format!("open db failed: {e}"))?;

        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                content_text TEXT,
                file_path TEXT,
                thumb_path TEXT,
                sha256 TEXT NOT NULL,
                app_name TEXT NOT NULL,
                is_starred INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_memories_updated ON memories(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_sha ON memories(sha256);

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content_text,
                app_name,
                content='memories',
                content_rowid='id'
            );

            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content_text, app_name)
                VALUES (new.id, COALESCE(new.content_text, ''), new.app_name);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content_text, app_name)
                VALUES ('delete', old.id, COALESCE(old.content_text, ''), old.app_name);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content_text, app_name)
                VALUES ('delete', old.id, COALESCE(old.content_text, ''), old.app_name);
                INSERT INTO memories_fts(rowid, content_text, app_name)
                VALUES (new.id, COALESCE(new.content_text, ''), new.app_name);
            END;

            CREATE TABLE IF NOT EXISTS conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chat_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL DEFAULT 1,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                attachments_json TEXT,
                agent_steps_json TEXT,
                thought_text TEXT,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_chat_messages_conv_id
                ON chat_messages(conversation_id, id);
            CREATE INDEX IF NOT EXISTS idx_conversations_updated
                ON conversations(updated_at DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_cleanup
                ON memories(kind, is_starred, updated_at);

            CREATE TABLE IF NOT EXISTS browser_bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS reminders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                fire_at_ms INTEGER NOT NULL,
                repeat_rule TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                fired_count INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_reminders_due
                ON reminders(enabled, fire_at_ms ASC);
            ",
        )
        .map_err(|e| format!("init schema failed: {e}"))?;

        Self::ensure_column(
            &conn,
            "chat_messages",
            "conversation_id",
            "ALTER TABLE chat_messages ADD COLUMN conversation_id INTEGER NOT NULL DEFAULT 1",
        )?;
        Self::ensure_column(
            &conn,
            "chat_messages",
            "agent_steps_json",
            "ALTER TABLE chat_messages ADD COLUMN agent_steps_json TEXT",
        )?;
        Self::ensure_column(
            &conn,
            "chat_messages",
            "thought_text",
            "ALTER TABLE chat_messages ADD COLUMN thought_text TEXT",
        )?;
        Self::ensure_column(
            &conn,
            "conversations",
            "summary_text",
            "ALTER TABLE conversations ADD COLUMN summary_text TEXT",
        )?;
        Self::ensure_column(
            &conn,
            "conversations",
            "summarized_up_to_id",
            "ALTER TABLE conversations ADD COLUMN summarized_up_to_id INTEGER NOT NULL DEFAULT 0",
        )?;
        Self::ensure_column(
            &conn,
            "conversations",
            "source",
            "ALTER TABLE conversations ADD COLUMN source TEXT NOT NULL DEFAULT 'desktop'",
        )?;

        // Ensure orphaned legacy messages belong to an existing conversation row
        let msg_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0))
            .unwrap_or(0);
        let conv_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM conversations", [], |r| r.get(0))
            .unwrap_or(0);
        if msg_count > 0 && conv_count == 0 {
            let ts = now_ms();
            let first_title: String = conn
                .query_row(
                    "SELECT content FROM chat_messages WHERE role = 'user' ORDER BY id ASC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| "Previous Chat".to_string());
            let clean_title: String = first_title.chars().take(36).collect();
            let _ = conn.execute(
                "INSERT INTO conversations (id, title, created_at, updated_at) VALUES (1, ?1, ?2, ?2)",
                params![clean_title, ts],
            );
        }

        Ok(Self {
            conn: Mutex::new(conn),
            data_dir,
            screenshots_dir,
            thumbnails_dir,
        })
    }

    pub fn has_recent_sha(&self, sha256: &str, within_ms: i64) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return false;
        };
        let cutoff = now_ms() - within_ms;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE sha256 = ?1 AND updated_at >= ?2)",
            params![sha256, cutoff],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
            == 1
    }

    /// Inserts or merges by SHA-256. Returns (item, was_merged_duplicate).
    pub fn upsert_memory(
        &self,
        kind: &str,
        content_text: Option<&str>,
        file_path: Option<&str>,
        thumb_path: Option<&str>,
        sha256: &str,
        app_name: &str,
    ) -> Result<(MemoryItem, bool), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();

        // Merge if identical SHA-256 already exists in memory
        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM memories WHERE sha256 = ?1 ORDER BY updated_at DESC LIMIT 1",
                params![sha256],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing_id {
            conn.execute(
                "UPDATE memories SET updated_at = ?1, app_name = ?2 WHERE id = ?3",
                params![ts, app_name, id],
            )
            .map_err(|e| e.to_string())?;

            let item = Self::get_by_id_locked(&conn, id)?;
            return Ok((item, true));
        }

        conn.execute(
            "INSERT INTO memories (kind, content_text, file_path, thumb_path, sha256, app_name, is_starred, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
            params![kind, content_text, file_path, thumb_path, sha256, app_name, ts],
        )
        .map_err(|e| e.to_string())?;

        let id = conn.last_insert_rowid();
        let item = Self::get_by_id_locked(&conn, id)?;
        Ok((item, false))
    }

    fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
        Ok(MemoryItem {
            id: row.get(0)?,
            kind: row.get(1)?,
            content_text: row.get(2)?,
            file_path: row.get(3)?,
            thumb_path: row.get(4)?,
            sha256: row.get(5)?,
            app_name: row.get(6)?,
            is_starred: row.get::<_, i64>(7)? != 0,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }

    fn get_by_id_locked(conn: &Connection, id: i64) -> Result<MemoryItem, String> {
        conn.query_row(
            "SELECT id, kind, content_text, file_path, thumb_path, sha256, app_name, is_starred, created_at, updated_at
             FROM memories WHERE id = ?1",
            params![id],
            Self::row_to_item,
        )
        .map_err(|e| e.to_string())
    }

    pub fn get_memory_by_id(&self, id: i64) -> Result<MemoryItem, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        Self::get_by_id_locked(&conn, id)
    }

    pub fn list_memories(&self, query: Option<&str>, limit: usize) -> Result<Vec<MemoryItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let trimmed = query.unwrap_or("").trim();

        if trimmed.is_empty() {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, content_text, file_path, thumb_path, sha256, app_name, is_starred, created_at, updated_at
                     FROM memories
                     ORDER BY updated_at DESC
                     LIMIT ?1",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![limit as i64], Self::row_to_item)
                .map_err(|e| e.to_string())?;
            return Ok(rows.filter_map(Result::ok).collect());
        }

        // Escape FTS5 tokens safely and prefix-match each word
        let fts_terms: Vec<String> = trimmed
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
            .collect();

        if fts_terms.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = fts_terms.join(" AND ");
        let sql = "SELECT m.id, m.kind, m.content_text, m.file_path, m.thumb_path, m.sha256, m.app_name, m.is_starred, m.created_at, m.updated_at
                   FROM memories m
                   JOIN memories_fts f ON m.id = f.rowid
                   WHERE memories_fts MATCH ?1
                   ORDER BY m.updated_at DESC
                   LIMIT ?2";

        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(rows) = stmt.query_map(params![fts_query, limit as i64], Self::row_to_item) {
                return Ok(rows.filter_map(Result::ok).collect());
            }
        }

        // Fallback LIKE search if FTS query syntax fails
        let like = format!("%{trimmed}%");
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, content_text, file_path, thumb_path, sha256, app_name, is_starred, created_at, updated_at
                 FROM memories
                 WHERE COALESCE(content_text, '') LIKE ?1 OR app_name LIKE ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![like, limit as i64], Self::row_to_item)
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn toggle_star(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE memories SET is_starred = CASE WHEN is_starred = 0 THEN 1 ELSE 0 END WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        let starred: i64 = conn
            .query_row(
                "SELECT is_starred FROM memories WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(starred != 0)
    }

    pub fn delete_memory(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        if let Ok(item) = Self::get_by_id_locked(&conn, id) {
            Self::remove_files_if_present(
                item.file_path.as_deref(),
                item.thumb_path.as_deref(),
            );
        }
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Removes physical image files first, then removes expired DB rows (> 7 days & unstarred),
    /// and prunes temporary pasted chat images older than 7 days.
    pub fn cleanup_expired(&self) -> usize {
        let mut count = 0usize;
        if let Ok(conn) = self.conn.lock() {
            let cutoff = now_ms() - (7 * 24 * 60 * 60 * 1000);
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id, file_path, thumb_path FROM memories WHERE is_starred = 0 AND kind != 'fact' AND updated_at < ?1",
            ) {
                let expired: Vec<(i64, Option<String>, Option<String>)> = stmt
                    .query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .ok()
                    .map(|iter| iter.filter_map(Result::ok).collect())
                    .unwrap_or_default();

                count += expired.len();
                for (id, fp, tp) in expired {
                    Self::remove_files_if_present(fp.as_deref(), tp.as_deref());
                    let _ = conn.execute("DELETE FROM memories WHERE id = ?1", params![id]);
                }
            }
        }
        count += Self::prune_expired_pasted_files(&self.data_dir.join("pasted"), 7 * 24 * 60 * 60);
        count
    }

    pub(crate) fn prune_expired_pasted_files(pasted_dir: &Path, max_age_secs: u64) -> usize {
        let Ok(entries) = std::fs::read_dir(pasted_dir) else {
            return 0;
        };
        let now = SystemTime::now();
        let mut removed = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if let Ok(age) = now.duration_since(modified) {
                if age.as_secs() >= max_age_secs && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }

    pub fn get_conversation_title(&self, conversation_id: i64) -> Result<String, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT title FROM conversations WHERE id = ?1",
            params![conversation_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    }

    fn remove_files_if_present(file_path: Option<&str>, thumb_path: Option<&str>) {
        if let Some(p) = file_path {
            let path = Path::new(p);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
        if let Some(p) = thumb_path {
            let path = Path::new(p);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    pub fn checkpoint_wal(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }

    pub fn create_conversation(&self, title: Option<&str>) -> Result<ConversationSummary, String> {
        self.create_conversation_with_source(title, "desktop")
    }

    pub fn create_conversation_with_source(
        &self,
        title: Option<&str>,
        source: &str,
    ) -> Result<ConversationSummary, String> {
        let clean_source = if source == "mobile" { "mobile" } else { "desktop" };
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();
        if title.is_none() {
            if let Ok((id, existing_title, created_at, existing_source, count)) = conn.query_row(
                "SELECT c.id, c.title, c.created_at, COALESCE(c.source, 'desktop'),
                        (SELECT COUNT(*) FROM chat_messages m WHERE m.conversation_id = c.id) AS cnt
                 FROM conversations c
                 ORDER BY c.updated_at DESC, c.id DESC
                 LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            ) {
                if count == 0 && existing_source == clean_source {
                    let _ = conn.execute(
                        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                        params![ts, id],
                    );
                    return Ok(ConversationSummary {
                        id,
                        title: existing_title,
                        message_count: 0,
                        source: existing_source,
                        created_at,
                        updated_at: ts,
                    });
                }
            }
        }
        let t = title.unwrap_or("New Chat");
        conn.execute(
            "INSERT INTO conversations (title, source, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![t, clean_source, ts],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        Ok(ConversationSummary {
            id,
            title: t.to_string(),
            message_count: 0,
            source: clean_source.to_string(),
            created_at: ts,
            updated_at: ts,
        })
    }

    pub fn ensure_active_conversation(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        if let Ok(id) = conn.query_row(
            "SELECT id FROM conversations ORDER BY updated_at DESC, id DESC LIMIT 1",
            [],
            |r| r.get::<_, i64>(0),
        ) {
            return Ok(id);
        }
        let ts = now_ms();
        conn.execute(
            "INSERT INTO conversations (title, created_at, updated_at) VALUES ('New Chat', ?1, ?1)",
            params![ts],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    /// Starts a fresh conversation on app boot unless the latest conversation already has 0 messages.
    pub fn ensure_fresh_startup_conversation(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();
        if let Ok((id, count)) = conn.query_row(
            "SELECT c.id, (SELECT COUNT(*) FROM chat_messages m WHERE m.conversation_id = c.id) AS cnt
             FROM conversations c
             ORDER BY c.updated_at DESC, c.id DESC
             LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ) {
            if count == 0 {
                let _ = conn.execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![ts, id],
                );
                return Ok(id);
            }
        }
        conn.execute(
            "INSERT INTO conversations (title, created_at, updated_at) VALUES ('New Chat', ?1, ?1)",
            params![ts],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_conversations(&self, limit: usize) -> Result<Vec<ConversationSummary>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title,
                        (SELECT COUNT(*) FROM chat_messages m WHERE m.conversation_id = c.id) AS cnt,
                        COALESCE(c.source, 'desktop'),
                        c.created_at, c.updated_at
                 FROM conversations c
                 ORDER BY c.updated_at DESC, c.id DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(ConversationSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    message_count: r.get(2)?,
                    source: r.get(3)?,
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn delete_conversation(&self, conversation_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM chat_messages WHERE conversation_id = ?1",
            params![conversation_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM conversations WHERE id = ?1",
            params![conversation_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn rename_conversation(
        &self,
        conversation_id: i64,
        new_title: &str,
    ) -> Result<String, String> {
        let clean: String = new_title.trim().chars().take(64).collect();
        if clean.is_empty() {
            return Err("Title cannot be empty".to_string());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE conversations SET title = ?1 WHERE id = ?2",
            params![clean, conversation_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(clean)
    }

    pub fn search_conversations(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ConversationSummary>, String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.list_conversations(limit);
        }
        let escaped = trimmed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title,
                        (SELECT COUNT(*) FROM chat_messages m WHERE m.conversation_id = c.id) AS cnt,
                        COALESCE(c.source, 'desktop'),
                        c.created_at, c.updated_at
                 FROM conversations c
                 WHERE c.title LIKE ?1 ESCAPE '\\'
                    OR EXISTS (
                        SELECT 1 FROM chat_messages cm
                        WHERE cm.conversation_id = c.id
                          AND cm.content LIKE ?1 ESCAPE '\\'
                    )
                 ORDER BY c.updated_at DESC, c.id DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], |r| {
                Ok(ConversationSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    message_count: r.get(2)?,
                    source: r.get(3)?,
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn delete_last_assistant_message(&self, conversation_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let last: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, role FROM chat_messages WHERE conversation_id = ?1 ORDER BY id DESC LIMIT 1",
                params![conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        if let Some((msg_id, role)) = last {
            if role == "assistant" {
                conn.execute("DELETE FROM chat_messages WHERE id = ?1", params![msg_id])
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Truncate a conversation starting at the `user_turn_index`-th (0-based) user message,
    /// removing that user turn and every subsequent message so the turn can be cleanly edited & resent.
    pub fn truncate_conversation_by_user_index(
        &self,
        conversation_id: i64,
        user_turn_index: usize,
    ) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let cutoff_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM chat_messages
                 WHERE conversation_id = ?1 AND role = 'user'
                 ORDER BY id ASC
                 LIMIT 1 OFFSET ?2",
                params![conversation_id, user_turn_index as i64],
                |r| r.get(0),
            )
            .ok();
        let Some(cid) = cutoff_id else {
            return Ok(0);
        };
        let deleted = conn
            .execute(
                "DELETE FROM chat_messages WHERE conversation_id = ?1 AND id >= ?2",
                params![conversation_id, cid],
            )
            .map_err(|e| e.to_string())?;

        // Reset rolling summary if it covered any truncated message
        let up_to: i64 = conn
            .query_row(
                "SELECT summarized_up_to_id FROM conversations WHERE id = ?1",
                params![conversation_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let ts = now_ms();
        if up_to >= cid {
            let _ = conn.execute(
                "UPDATE conversations SET summary_text = NULL, summarized_up_to_id = 0, updated_at = ?1 WHERE id = ?2",
                params![ts, conversation_id],
            );
        } else {
            let _ = conn.execute(
                "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                params![ts, conversation_id],
            );
        }
        Ok(deleted)
    }

    // ------------------------------------------------------------------------
    // Browser Bookmarks
    // ------------------------------------------------------------------------

    pub fn list_bookmarks(&self) -> Result<Vec<BrowserBookmark>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, url, created_at FROM browser_bookmarks ORDER BY created_at DESC LIMIT 60",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BrowserBookmark {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    url: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn add_bookmark(&self, title: &str, url: &str) -> Result<BrowserBookmark, String> {
        let clean_url = url.trim();
        if clean_url.is_empty() {
            return Err("Bookmark URL cannot be empty".to_string());
        }
        let clean_title: String = {
            let t = title.trim();
            if t.is_empty() {
                clean_url.chars().take(48).collect()
            } else {
                t.chars().take(80).collect()
            }
        };
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();
        conn.execute(
            "INSERT INTO browser_bookmarks (title, url, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(url) DO UPDATE SET title = excluded.title, created_at = excluded.created_at",
            params![clean_title, clean_url, ts],
        )
        .map_err(|e| e.to_string())?;

        conn.query_row(
            "SELECT id, title, url, created_at FROM browser_bookmarks WHERE url = ?1",
            params![clean_url],
            |r| {
                Ok(BrowserBookmark {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    url: r.get(2)?,
                    created_at: r.get(3)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    }

    pub fn delete_bookmark(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM browser_bookmarks WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Persistent Scheduled Reminders
    // ------------------------------------------------------------------------

    pub fn create_reminder(
        &self,
        title: &str,
        fire_at_ms: i64,
        repeat_rule: Option<&str>,
    ) -> Result<ReminderItem, String> {
        let clean_title: String = title.trim().chars().take(180).collect();
        if clean_title.is_empty() {
            return Err("Reminder title cannot be empty".to_string());
        }
        let norm_repeat = repeat_rule.and_then(|r| match r.trim().to_ascii_lowercase().as_str() {
            "daily" | "weekdays" | "weekly" => Some(r.trim().to_ascii_lowercase()),
            _ => None,
        });
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();
        conn.execute(
            "INSERT INTO reminders (title, fire_at_ms, repeat_rule, enabled, fired_count, created_at)
             VALUES (?1, ?2, ?3, 1, 0, ?4)",
            params![clean_title, fire_at_ms, norm_repeat, ts],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        Self::get_reminder_locked(&conn, id)
    }

    pub fn list_reminders(&self, include_disabled: bool) -> Result<Vec<ReminderItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let sql = if include_disabled {
            "SELECT id, title, fire_at_ms, repeat_rule, enabled, fired_count, created_at
             FROM reminders
             ORDER BY enabled DESC, fire_at_ms ASC
             LIMIT 100"
        } else {
            "SELECT id, title, fire_at_ms, repeat_rule, enabled, fired_count, created_at
             FROM reminders
             WHERE enabled = 1
             ORDER BY fire_at_ms ASC
             LIMIT 100"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], Self::row_to_reminder)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn toggle_reminder(&self, id: i64, enabled: bool) -> Result<ReminderItem, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE reminders SET enabled = ?1 WHERE id = ?2",
            params![if enabled { 1 } else { 0 }, id],
        )
        .map_err(|e| e.to_string())?;
        Self::get_reminder_locked(&conn, id)
    }

    pub fn snooze_reminder(&self, id: i64, minutes: i64) -> Result<ReminderItem, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let next_ms = now_ms() + minutes.clamp(1, 1440) * 60_000;
        conn.execute(
            "UPDATE reminders SET fire_at_ms = ?1, enabled = 1 WHERE id = ?2",
            params![next_ms, id],
        )
        .map_err(|e| e.to_string())?;
        Self::get_reminder_locked(&conn, id)
    }

    pub fn delete_reminder(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM reminders WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Atomically returns all enabled reminders due at or before `current_ms`,
    /// advancing recurring reminders or disabling one-shot reminders.
    pub fn pop_due_reminders(
        &self,
        current_ms: i64,
        tz_offset_ms: i64,
    ) -> Result<Vec<ReminderItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, fire_at_ms, repeat_rule, enabled, fired_count, created_at
                 FROM reminders
                 WHERE enabled = 1 AND fire_at_ms <= ?1
                 ORDER BY fire_at_ms ASC",
            )
            .map_err(|e| e.to_string())?;
        let due: Vec<ReminderItem> = stmt
            .query_map(params![current_ms], Self::row_to_reminder)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        for item in &due {
            if let Some(rule) = item.repeat_rule.as_deref() {
                let next_ms = advance_repeat_fire_ms(item.fire_at_ms, rule, current_ms, tz_offset_ms);
                let _ = conn.execute(
                    "UPDATE reminders SET fire_at_ms = ?1, fired_count = fired_count + 1 WHERE id = ?2",
                    params![next_ms, item.id],
                );
            } else {
                let _ = conn.execute(
                    "UPDATE reminders SET enabled = 0, fired_count = fired_count + 1 WHERE id = ?1",
                    params![item.id],
                );
            }
        }
        Ok(due)
    }

    fn row_to_reminder(r: &rusqlite::Row<'_>) -> rusqlite::Result<ReminderItem> {
        Ok(ReminderItem {
            id: r.get(0)?,
            title: r.get(1)?,
            fire_at_ms: r.get(2)?,
            repeat_rule: r.get(3)?,
            enabled: r.get::<_, i64>(4)? != 0,
            fired_count: r.get(5)?,
            created_at: r.get(6)?,
        })
    }

    fn get_reminder_locked(conn: &Connection, id: i64) -> Result<ReminderItem, String> {
        conn.query_row(
            "SELECT id, title, fire_at_ms, repeat_rule, enabled, fired_count, created_at
             FROM reminders WHERE id = ?1",
            params![id],
            Self::row_to_reminder,
        )
        .map_err(|e| e.to_string())
    }

    pub fn append_chat_message(
        &self,
        conversation_id: i64,
        role: &str,
        content: &str,
        attachments_json: Option<&str>,
        agent_steps_json: Option<&str>,
        thought_text: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();
        conn.execute(
            "INSERT INTO chat_messages (conversation_id, role, content, attachments_json, agent_steps_json, thought_text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![conversation_id, role, content, attachments_json, agent_steps_json, thought_text, ts],
        )
        .map_err(|e| e.to_string())?;
        let msg_id = conn.last_insert_rowid();

        if role == "user" {
            let current_title: String = conn
                .query_row(
                    "SELECT title FROM conversations WHERE id = ?1",
                    params![conversation_id],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            if current_title == "New Chat" || current_title.is_empty() {
                let first_line = content.lines().next().unwrap_or(content).trim();
                let short: String = first_line.chars().take(36).collect();
                let new_title = if short.is_empty() {
                    "Untitled Chat".to_string()
                } else {
                    short
                };
                let _ = conn.execute(
                    "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
                    params![new_title, ts, conversation_id],
                );
            } else {
                let _ = conn.execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![ts, conversation_id],
                );
            }
        } else {
            let _ = conn.execute(
                "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                params![ts, conversation_id],
            );
        }

        Ok(msg_id)
    }

    pub fn list_chat_messages(
        &self,
        conversation_id: i64,
        limit: usize,
    ) -> Result<Vec<StoredChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, role, content, attachments_json, agent_steps_json, thought_text, created_at
                 FROM chat_messages
                 WHERE conversation_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let mut rows: Vec<StoredChatMessage> = stmt
            .query_map(params![conversation_id, limit as i64], |r| {
                Ok(StoredChatMessage {
                    id: r.get(0)?,
                    conversation_id: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    attachments_json: r.get(4)?,
                    agent_steps_json: r.get(5)?,
                    thought_text: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        rows.reverse();
        Ok(rows)
    }

    /// Save a durable long-term memory fact (`kind = 'fact'`, starred, immune to 7-day cleanup).
    pub fn save_fact(&self, content: &str, category: Option<&str>) -> Result<MemoryItem, String> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err("Memory fact cannot be empty".to_string());
        }
        let norm_cat = category
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .unwrap_or("general");
        let sha = format!(
            "fact:{}",
            text_sha256(&trimmed.to_ascii_lowercase())
        );

        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();

        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM memories WHERE sha256 = ?1 ORDER BY updated_at DESC LIMIT 1",
                params![sha],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing_id {
            conn.execute(
                "UPDATE memories SET content_text = ?1, app_name = ?2, is_starred = 1, updated_at = ?3 WHERE id = ?4",
                params![trimmed, norm_cat, ts, id],
            )
            .map_err(|e| e.to_string())?;
            return Self::get_by_id_locked(&conn, id);
        }

        conn.execute(
            "INSERT INTO memories (kind, content_text, file_path, thumb_path, sha256, app_name, is_starred, created_at, updated_at)
             VALUES ('fact', ?1, NULL, NULL, ?2, ?3, 1, ?4, ?4)",
            params![trimmed, sha, norm_cat, ts],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        Self::get_by_id_locked(&conn, id)
    }

    /// Return most recently updated durable facts (`kind = 'fact'`).
    pub fn list_active_facts(&self, limit: usize) -> Result<Vec<MemoryItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, content_text, file_path, thumb_path, sha256, app_name, is_starred, created_at, updated_at
                 FROM memories
                 WHERE kind = 'fact'
                 ORDER BY updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], Self::row_to_item)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    /// Fetch `(rolling_summary, verbatim_recent_messages)` for a conversation context window.
    pub fn list_chat_context(
        &self,
        conversation_id: i64,
        recent_limit: usize,
    ) -> Result<(Option<String>, Vec<StoredChatMessage>), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let (summary_opt, up_to_id): (Option<String>, i64) = conn
            .query_row(
                "SELECT summary_text, summarized_up_to_id FROM conversations WHERE id = ?1",
                params![conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((None, 0));

        let summary_clean = summary_opt.and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        });

        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, role, content, attachments_json, agent_steps_json, thought_text, created_at
                 FROM chat_messages
                 WHERE conversation_id = ?1 AND id > ?2
                 ORDER BY id DESC
                 LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        let mut rows: Vec<StoredChatMessage> = stmt
            .query_map(params![conversation_id, up_to_id, recent_limit as i64], |r| {
                Ok(StoredChatMessage {
                    id: r.get(0)?,
                    conversation_id: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    attachments_json: r.get(4)?,
                    agent_steps_json: r.get(5)?,
                    thought_text: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        rows.reverse();
        Ok((summary_clean, rows))
    }

    /// If uncompacted messages exceed `trigger_threshold`, return `(prev_summary, old_chunk_to_compact, new_summarized_up_to_id)` keeping newest `keep_recent` intact.
    pub fn list_messages_to_compact(
        &self,
        conversation_id: i64,
        trigger_threshold: usize,
        keep_recent: usize,
    ) -> Result<Option<(Option<String>, Vec<StoredChatMessage>, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let (prev_summary, up_to_id): (Option<String>, i64) = conn
            .query_row(
                "SELECT summary_text, summarized_up_to_id FROM conversations WHERE id = ?1",
                params![conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((None, 0));

        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, role, content, attachments_json, agent_steps_json, thought_text, created_at
                 FROM chat_messages
                 WHERE conversation_id = ?1 AND id > ?2
                 ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let all_unsummarized: Vec<StoredChatMessage> = stmt
            .query_map(params![conversation_id, up_to_id], |r| {
                Ok(StoredChatMessage {
                    id: r.get(0)?,
                    conversation_id: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    attachments_json: r.get(4)?,
                    agent_steps_json: r.get(5)?,
                    thought_text: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        if all_unsummarized.len() <= trigger_threshold || all_unsummarized.len() <= keep_recent {
            return Ok(None);
        }

        let compact_count = all_unsummarized.len() - keep_recent;
        let to_compact = all_unsummarized.into_iter().take(compact_count).collect::<Vec<_>>();
        let new_up_to_id = to_compact.last().map(|m| m.id).unwrap_or(up_to_id);
        let clean_prev = prev_summary.and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        });
        Ok(Some((clean_prev, to_compact, new_up_to_id)))
    }

    /// Persist compacted summary text + watermark atomically if newer than existing watermark.
    pub fn update_conversation_summary(
        &self,
        conversation_id: i64,
        up_to_id: i64,
        summary: &str,
    ) -> Result<(), String> {
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE conversations
             SET summary_text = ?1, summarized_up_to_id = ?2
             WHERE id = ?3 AND summarized_up_to_id < ?2",
            params![trimmed, up_to_id, conversation_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Computes the next fire timestamp for recurring reminders in local civil time.
/// Unix epoch (1970-01-01) was a Thursday (weekday 4 where Sun=0..Sat=6).
fn advance_repeat_fire_ms(current_fire_ms: i64, rule: &str, now_ms: i64, tz_offset_ms: i64) -> i64 {
    const DAY_MS: i64 = 86_400_000;
    let mut next = current_fire_ms;
    match rule {
        "weekly" => {
            while next <= now_ms {
                next += 7 * DAY_MS;
            }
            next
        }
        "weekdays" => {
            loop {
                next += DAY_MS;
                let local_days = (next + tz_offset_ms).div_euclid(DAY_MS);
                let weekday = (local_days + 4).rem_euclid(7); // 0=Sun, 6=Sat
                if next > now_ms && weekday != 0 && weekday != 6 {
                    return next;
                }
            }
        }
        _ => {
            // "daily"
            while next <= now_ms {
                next += DAY_MS;
            }
            next
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_dir(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "buddy_db_test_{label}_{}_{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn facts_survive_seven_day_cleanup_and_match_fts() {
        let dir = temp_db_dir("facts");
        let db = MemoryDb::open_at(dir.clone()).unwrap();

        let fact = db
            .save_fact("User prefers concise TypeScript examples", Some("preference"))
            .unwrap();
        assert_eq!(fact.kind, "fact");
        assert!(fact.is_starred);

        let (clip, _) = db
            .upsert_memory(
                "clipboard_text",
                Some("ephemeral clipboard snippet"),
                None,
                None,
                "clip_sha_old",
                "notepad.exe",
            )
            .unwrap();

        // Force both rows older than 7 days
        let stale_ts = now_ms() - (10 * 24 * 60 * 60 * 1000);
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE memories SET updated_at = ?1",
                params![stale_ts],
            )
            .unwrap();
        }

        let deleted = db.cleanup_expired();
        assert_eq!(deleted, 1, "only stale non-fact memory should expire");

        let active_facts = db.list_active_facts(10).unwrap();
        assert_eq!(active_facts.len(), 1);
        assert_eq!(active_facts[0].id, fact.id);

        let hits = db.list_memories(Some("TypeScript"), 10).unwrap();
        assert!(hits.iter().any(|m| m.id == fact.id));
        assert!(!hits.iter().any(|m| m.id == clip.id));

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compaction_splits_older_turns_while_keeping_recent_verbatim() {
        let dir = temp_db_dir("compact");
        let db = MemoryDb::open_at(dir.clone()).unwrap();
        let conv = db.create_conversation(Some("Long session")).unwrap();

        for i in 1..=25 {
            let role = if i % 2 == 1 { "user" } else { "assistant" };
            db.append_chat_message(
                conv.id,
                role,
                &format!("message #{i}"),
                None,
                None,
                None,
            )
            .unwrap();
        }

        let job = db
            .list_messages_to_compact(conv.id, 22, 12)
            .unwrap()
            .expect("25 messages should trigger compaction");
        assert_eq!(job.1.len(), 13);
        assert_eq!(job.1.first().unwrap().content, "message #1");
        assert_eq!(job.1.last().unwrap().content, "message #13");

        db.update_conversation_summary(conv.id, job.2, "User discussed steps 1..13.")
            .unwrap();

        let (summary, recent) = db.list_chat_context(conv.id, 20).unwrap();
        assert_eq!(summary.as_deref(), Some("User discussed steps 1..13."));
        assert_eq!(recent.len(), 12);
        assert_eq!(recent.first().unwrap().content, "message #14");
        assert_eq!(recent.last().unwrap().content, "message #25");

        // Second check with 12 uncompacted messages should not re-trigger
        assert!(db.list_messages_to_compact(conv.id, 22, 12).unwrap().is_none());

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fresh_startup_conversation_starts_new_only_when_prior_has_messages() {
        let dir = temp_db_dir("startup_fresh");
        let db = MemoryDb::open_at(dir.clone()).unwrap();

        let id1 = db.ensure_fresh_startup_conversation().unwrap();
        // Calling startup again while id1 is still empty reuses id1 instead of creating blank rows
        let id1_reused = db.ensure_fresh_startup_conversation().unwrap();
        assert_eq!(id1, id1_reused);

        db.append_chat_message(id1, "user", "hi buddy", None, None, None)
            .unwrap();

        // Once id1 has messages, next startup opens a brand-new conversation while id1 stays saved
        let id2 = db.ensure_fresh_startup_conversation().unwrap();
        assert_ne!(id1, id2);
        let all = db.list_conversations(10).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|c| c.id == id1 && c.message_count == 1));
        assert!(all.iter().any(|c| c.id == id2 && c.message_count == 0));

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_and_search_conversations_matches_message_content() {
        let dir = temp_db_dir("rename_search");
        let db = MemoryDb::open_at(dir.clone()).unwrap();

        let c1 = db.create_conversation(Some("Morning chat")).unwrap();
        let c2 = db.create_conversation(Some("Music hunt")).unwrap();
        db.append_chat_message(c1.id, "user", "Let's debug WebView2 DPI scaling", None, None, None)
            .unwrap();
        db.append_chat_message(c2.id, "user", "Convert Jay Chou song to MP3", None, None, None)
            .unwrap();

        let renamed = db.rename_conversation(c1.id, "  WebView2 Notes  ").unwrap();
        assert_eq!(renamed, "WebView2 Notes");

        // Search by message body text not present in title
        let hits = db.search_conversations("DPI scaling", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, c1.id);
        assert_eq!(hits[0].title, "WebView2 Notes");

        // Search by title
        let title_hits = db.search_conversations("Music", 20).unwrap();
        assert_eq!(title_hits.len(), 1);
        assert_eq!(title_hits[0].id, c2.id);
        assert_eq!(db.get_conversation_title(c1.id).unwrap(), "WebView2 Notes");

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pasted_image_pruner_removes_expired_entries_and_keeps_fresh() {
        let dir = temp_db_dir("pasted_prune");
        let pasted = dir.join("pasted");
        std::fs::create_dir_all(&pasted).unwrap();
        let sample = pasted.join("paste_test.png");
        std::fs::write(&sample, b"fake-png").unwrap();

        // With 7-day threshold, a newly written file must stay untouched
        assert_eq!(MemoryDb::prune_expired_pasted_files(&pasted, 7 * 24 * 60 * 60), 0);
        assert!(sample.exists());

        // With 0-second threshold, it is cleanly pruned
        assert_eq!(MemoryDb::prune_expired_pasted_files(&pasted, 0), 1);
        assert!(!sample.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncate_bookmarks_and_reminders_roundtrip() {
        let dir = temp_db_dir("stage123");
        let db = MemoryDb::open_at(dir.clone()).unwrap();

        // 1. Truncate conversation by 0-based user turn index
        let conv = db.create_conversation(Some("Edit test")).unwrap();
        db.append_chat_message(conv.id, "user", "first q", None, None, None).unwrap();
        db.append_chat_message(conv.id, "assistant", "first a", None, None, None).unwrap();
        db.append_chat_message(conv.id, "user", "second q", None, None, None).unwrap();
        db.append_chat_message(conv.id, "assistant", "second a", None, None, None).unwrap();
        let removed = db.truncate_conversation_by_user_index(conv.id, 1).unwrap();
        assert_eq!(removed, 2);
        let kept = db.list_chat_messages(conv.id, 10).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].content, "first q");
        assert_eq!(kept[1].content, "first a");

        // 2. Bookmarks upsert & delete
        let bm = db.add_bookmark("Local App", "http://localhost:5173").unwrap();
        let bm_upd = db.add_bookmark("Vite Dev", "http://localhost:5173").unwrap();
        assert_eq!(bm.id, bm_upd.id);
        assert_eq!(db.list_bookmarks().unwrap().len(), 1);
        db.delete_bookmark(bm.id).unwrap();
        assert!(db.list_bookmarks().unwrap().is_empty());

        // 3. One-shot & recurring reminders
        let base = 1_700_000_000_000i64;
        let r_once = db.create_reminder("Drink water", base - 1000, None).unwrap();
        let r_daily = db.create_reminder("Standup", base - 1000, Some("daily")).unwrap();
        let fired = db.pop_due_reminders(base, 0).unwrap();
        assert_eq!(fired.len(), 2);
        let after = db.list_reminders(true).unwrap();
        let once_after = after.iter().find(|r| r.id == r_once.id).unwrap();
        let daily_after = after.iter().find(|r| r.id == r_daily.id).unwrap();
        assert!(!once_after.enabled);
        assert_eq!(once_after.fired_count, 1);
        assert!(daily_after.enabled);
        assert_eq!(daily_after.fire_at_ms, base - 1000 + 86_400_000);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
