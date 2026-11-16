//! Local SQLite persistence (%LOCALAPPDATA%/Buddy/buddy.db) with FTS5 search,
//! cross-channel SHA-256 deduplication, and 7-day automatic expiration cleanup.

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
    pub kind: String, // "screenshot" | "clipboard_text" | "clipboard_image"
    pub content_text: Option<String>,
    pub file_path: Option<String>,
    pub thumb_path: Option<String>,
    pub sha256: String,
    pub app_name: String,
    pub is_starred: bool,
    pub created_at: i64,
    pub updated_at: i64,
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
        let data_dir = resolve_data_dir();
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
            ",
        )
        .map_err(|e| format!("init schema failed: {e}"))?;

        // Migrate legacy chat_messages columns if missing
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(chat_messages)")
            .and_then(|mut s| {
                let rows = s.query_map([], |r| r.get::<_, String>(1))?;
                Ok(rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default();

        if !cols.iter().any(|c| c == "conversation_id") {
            conn.execute(
                "ALTER TABLE chat_messages ADD COLUMN conversation_id INTEGER NOT NULL DEFAULT 1",
                [],
            )
            .map_err(|e| format!("migrate chat_messages failed: {e}"))?;
        }
        if !cols.iter().any(|c| c == "agent_steps_json") {
            conn.execute(
                "ALTER TABLE chat_messages ADD COLUMN agent_steps_json TEXT",
                [],
            )
            .map_err(|e| format!("migrate agent_steps_json failed: {e}"))?;
        }
        if !cols.iter().any(|c| c == "thought_text") {
            conn.execute(
                "ALTER TABLE chat_messages ADD COLUMN thought_text TEXT",
                [],
            )
            .map_err(|e| format!("migrate thought_text failed: {e}"))?;
        }

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

    /// Removes physical image files first, then removes expired DB rows (> 7 days & unstarred).
    pub fn cleanup_expired(&self) -> usize {
        let Ok(conn) = self.conn.lock() else {
            return 0;
        };
        let cutoff = now_ms() - (7 * 24 * 60 * 60 * 1000);
        let mut stmt = match conn.prepare(
            "SELECT id, file_path, thumb_path FROM memories WHERE is_starred = 0 AND updated_at < ?1",
        ) {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let expired: Vec<(i64, Option<String>, Option<String>)> = stmt
            .query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .ok()
            .map(|iter| iter.filter_map(Result::ok).collect())
            .unwrap_or_default();

        let count = expired.len();
        for (id, fp, tp) in expired {
            Self::remove_files_if_present(fp.as_deref(), tp.as_deref());
            let _ = conn.execute("DELETE FROM memories WHERE id = ?1", params![id]);
        }
        count
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
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let ts = now_ms();
        let t = title.unwrap_or("New Chat");
        conn.execute(
            "INSERT INTO conversations (title, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![t, ts],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        Ok(ConversationSummary {
            id,
            title: t.to_string(),
            message_count: 0,
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

    pub fn list_conversations(&self, limit: usize) -> Result<Vec<ConversationSummary>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title,
                        (SELECT COUNT(*) FROM chat_messages m WHERE m.conversation_id = c.id) AS cnt,
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
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
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
}
