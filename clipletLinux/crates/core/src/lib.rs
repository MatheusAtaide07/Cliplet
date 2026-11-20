//! Shared code for the Linux capture daemon and the Tauri app:
//! where the data lives, the local SQLite schema and how copied content is classified.

pub mod classify;
pub mod db;
pub mod hash;
pub mod paths;

/// Current time in epoch milliseconds, the unit every timestamp column uses.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
