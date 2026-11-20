//! Local SQLite store. The schema is `crates/core/schema.sql`; both the capture daemon and
//! the Tauri app open the same file in WAL mode.

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::classify::Kind;

pub use rusqlite::{Connection, Error, Result, Transaction, TransactionBehavior};

const SCHEMA_SQL: &str = include_str!("../schema.sql");
const SCHEMA_VERSION: i64 = 1;

/// Opens (and creates, the first time) the database, with the per-connection pragmas set.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// In-memory database with the schema, for tests.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // journal_mode returns a row, so it goes through query_row.
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Checked inside the write lock, so the daemon and the app can't both create the schema.
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        // The PRAGMAs at the top of the file are per connection and can't run inside a
        // transaction; `configure` already applied them.
        let ddl: String = SCHEMA_SQL
            .lines()
            .filter(|l| !l.trim_start().to_ascii_uppercase().starts_with("PRAGMA"))
            .collect::<Vec<_>>()
            .join("\n");
        tx.execute_batch(&ddl)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    tx.commit()
}

/// Reads an integer from the `settings` table, with a fallback if missing or not a number.
pub fn setting_i64(conn: &Connection, key: &str, default: i64) -> i64 {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Id of this device (the row with `is_current = 1`), created on first use.
pub fn current_device(conn: &Connection) -> Result<String> {
    let existing = conn
        .query_row("SELECT id FROM devices WHERE is_current = 1", [], |r| r.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO devices (id, name, platform, is_current) VALUES (?1, ?2, 'linux', 1)",
        params![id, hostname()],
    )?;
    Ok(id)
}

fn hostname() -> String {
    ["/etc/hostname", "/proc/sys/kernel/hostname"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "Linux".to_string())
}

/// An entry already in the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Existing {
    pub id: String,
    pub kind: String,
}

/// The newest entry with this content hash (entries about to be deleted remotely don't count).
pub fn find_by_hash(conn: &Connection, content_hash: &str) -> Result<Option<Existing>> {
    conn.query_row(
        "SELECT id, kind FROM entries
          WHERE content_hash = ?1 AND sync_status <> 'pending_remote_delete'
          ORDER BY last_used_at DESC LIMIT 1",
        [content_hash],
        |r| Ok(Existing { id: r.get(0)?, kind: r.get(1)? }),
    )
    .optional()
}

/// Marks an entry as used now: it moves to the top of the history and into the date filters.
pub fn touch(conn: &Connection, id: &str, now_ms: i64) -> Result<()> {
    conn.execute(
        "UPDATE entries SET last_used_at = max(last_used_at, ?2) WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

/// A new clipboard entry. Exactly one of `text_content` / `file_path` is set.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub id: String,
    pub device_id: String,
    pub kind: Kind,
    pub content_hash: String,
    pub preview_text: Option<String>,
    pub text_content: Option<String>,
    /// Relative to the data dir, e.g. `media/<id>.png`.
    pub file_path: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub byte_size: i64,
}

/// Inserts a captured entry: source `clipboard`, `local_only` (nothing goes to the cloud unless
/// the user sends it), created and last used now.
pub fn insert_entry(conn: &Connection, e: &NewEntry, now_ms: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO entries (id, device_id, kind, source, created_at, last_used_at, content_hash,
                              preview_text, text_content, file_path, file_name, mime_type, byte_size)
         VALUES (?1, ?2, ?3, 'clipboard', ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            e.id,
            e.device_id,
            e.kind.as_str(),
            now_ms,
            e.content_hash,
            e.preview_text,
            e.text_content,
            e.file_path,
            e.file_name,
            e.mime_type,
            e.byte_size,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(conn: &Connection, hash: &str, text: &str) -> NewEntry {
        NewEntry {
            id: uuid::Uuid::now_v7().to_string(),
            device_id: current_device(conn).unwrap(),
            kind: Kind::Text,
            content_hash: hash.into(),
            preview_text: Some(text.into()),
            text_content: Some(text.into()),
            file_path: None,
            file_name: None,
            mime_type: Some("text/plain".into()),
            byte_size: text.len() as i64,
        }
    }

    #[test]
    fn schema_and_settings() {
        let conn = open_in_memory().unwrap();
        assert_eq!(setting_i64(&conn, "inline_max_bytes", 0), 65536);
        assert_eq!(setting_i64(&conn, "missing", 7), 7);
        let a = current_device(&conn).unwrap();
        assert_eq!(current_device(&conn).unwrap(), a);
    }

    #[test]
    fn reopen_keeps_data() {
        let dir = std::env::temp_dir().join(format!("cliplet-test-{}", uuid::Uuid::now_v7()));
        let path = dir.join("cliplet.db");
        {
            let conn = open(&path).unwrap();
            let e = entry(&conn, "h1", "hello");
            insert_entry(&conn, &e, 1).unwrap();
        }
        let conn = open(&path).unwrap();
        assert!(find_by_hash(&conn, "h1").unwrap().is_some());
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dedup_touches_instead_of_inserting() {
        let conn = open_in_memory().unwrap();
        let e = entry(&conn, "abc", "ação rápida");
        insert_entry(&conn, &e, 1_000).unwrap();

        let found = find_by_hash(&conn, "abc").unwrap().unwrap();
        assert_eq!(found.id, e.id);
        touch(&conn, &found.id, 2_000).unwrap();

        let (count, created, used): (i64, i64, i64) = conn
            .query_row("SELECT count(*), created_at, last_used_at FROM entries", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((count, created, used), (1, 1_000, 2_000));
        assert!(find_by_hash(&conn, "other").unwrap().is_none());
    }

    #[test]
    fn full_text_search_ignores_accents() {
        let conn = open_in_memory().unwrap();
        insert_entry(&conn, &entry(&conn, "x", "ação rápida"), 1).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM entries_fts WHERE entries_fts MATCH 'acao'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn text_and_file_are_exclusive() {
        let conn = open_in_memory().unwrap();
        let mut e = entry(&conn, "y", "t");
        e.file_path = Some("media/x.txt".into());
        assert!(insert_entry(&conn, &e, 1).is_err());
    }
}
