use std::{io, path::PathBuf};

/// App id from `src-tauri/tauri.conf.json`; the data folder is named after it.
pub const APP_ID: &str = "cliplet";
pub const DB_FILE: &str = "cliplet.db";
pub const MEDIA_DIR: &str = "media";

/// `$CLIPLET_DATA_DIR` if set (handy for development), otherwise `$XDG_DATA_HOME/cliplet`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLIPLET_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join(APP_ID)
}

pub fn db_path() -> PathBuf {
    data_dir().join(DB_FILE)
}

/// Folder for content stored as files (`entries.file_path` is relative to [`data_dir`]).
/// Created if missing.
pub fn media_dir() -> io::Result<PathBuf> {
    let dir = data_dir().join(MEDIA_DIR);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
