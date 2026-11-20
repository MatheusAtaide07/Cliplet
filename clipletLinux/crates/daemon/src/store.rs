//! Writes captured items to SQLite and the media folder, without duplicates.

use std::{
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use cliplet_core::{
    classify::Kind,
    db::{self, Connection, NewEntry, TransactionBehavior},
    hash::{ContentHasher, hash_bytes},
    now_ms,
    paths::MEDIA_DIR,
};

const TMP_PREFIX: &str = ".tmp-";

/// Content written to `media/.tmp-<uuid>` and hashed on the way. The temporary file is
/// removed on drop unless it was moved to its final name.
pub struct Staged {
    tmp: Option<PathBuf>,
    pub hash: String,
    pub size: u64,
    /// Extension for the final file name, without the dot.
    pub ext: String,
}

impl Staged {
    fn persist(mut self, to: &Path) -> io::Result<()> {
        let tmp = self.tmp.take().expect("staged file already persisted");
        fs::rename(&tmp, to).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(tmp) = &self.tmp {
            let _ = fs::remove_file(tmp);
        }
    }
}

pub enum Content {
    /// Text, link or color up to `inline_max_bytes`: goes in `entries.text_content`.
    Inline(String),
    /// Everything else: goes to `media/<id>.<ext>`.
    File(Staged),
}

pub struct Item {
    pub kind: Kind,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub preview_text: Option<String>,
    pub content: Content,
}

pub enum Outcome {
    New { id: String, kind: Kind, file_path: Option<String>, size: u64 },
    Duplicate { id: String, kind: String, touched: bool },
}

pub struct Store {
    conn: Connection,
    data_dir: PathBuf,
    media_dir: PathBuf,
    device_id: String,
    /// Above this many bytes, text goes to a file (`settings.inline_max_bytes`).
    pub inline_max: usize,
}

impl Store {
    pub fn open(data_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let media_dir = data_dir.join(MEDIA_DIR);
        fs::create_dir_all(&media_dir)?;
        remove_leftover_tmp(&media_dir);

        let conn = db::open(&data_dir.join(cliplet_core::paths::DB_FILE))?;
        let device_id = db::current_device(&conn)?;
        let inline_max = db::setting_i64(&conn, "inline_max_bytes", 65536).max(0) as usize;
        Ok(Self { conn, data_dir, media_dir, device_id, inline_max })
    }

    /// Streams `reader` into a temporary file in the media folder, hashing it.
    pub fn stage(&self, reader: &mut dyn Read, ext: &str) -> io::Result<Staged> {
        let tmp = self.media_dir.join(format!("{TMP_PREFIX}{}", uuid::Uuid::now_v7()));
        // From here on, dropping `staged` deletes the file, also on error.
        let mut staged = Staged { tmp: Some(tmp.clone()), hash: String::new(), size: 0, ext: ext.into() };
        let mut out = BufWriter::new(File::create(&tmp)?);
        let mut hasher = ContentHasher::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            staged.size += n as u64;
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        staged.hash = hasher.finish();
        Ok(staged)
    }

    /// Saves an item unless the same content is already in the history. A duplicate only gets
    /// its `last_used_at` moved to now (when `touch_duplicates` is set), so it rises to the top
    /// of the history instead of appearing twice.
    pub fn ingest(&mut self, item: Item, touch_duplicates: bool) -> Result<Outcome, Box<dyn std::error::Error>> {
        let now = now_ms();
        let hash = match &item.content {
            Content::Inline(text) => hash_bytes(text.as_bytes()),
            Content::File(staged) => staged.hash.clone(),
        };

        // IMMEDIATE takes the write lock up front, so the check and the insert can't interleave
        // with another writer.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = db::find_by_hash(&tx, &hash)? {
            if touch_duplicates {
                db::touch(&tx, &existing.id, now)?;
            }
            tx.commit()?;
            // A staged file is dropped here, which deletes it.
            return Ok(Outcome::Duplicate { id: existing.id, kind: existing.kind, touched: touch_duplicates });
        }

        let id = uuid::Uuid::now_v7().to_string();
        let (text_content, file_path, size) = match item.content {
            Content::Inline(text) => {
                let size = text.len() as u64;
                (Some(text), None, size)
            }
            Content::File(staged) => {
                let rel = format!("{MEDIA_DIR}/{id}.{}", staged.ext);
                let size = staged.size;
                staged.persist(&self.data_dir.join(&rel))?;
                (None, Some(rel), size)
            }
        };

        let entry = NewEntry {
            id: id.clone(),
            device_id: self.device_id.clone(),
            kind: item.kind,
            content_hash: hash,
            preview_text: item.preview_text,
            text_content,
            file_path: file_path.clone(),
            file_name: item.file_name,
            mime_type: item.mime_type,
            byte_size: size as i64,
        };
        let saved = db::insert_entry(&tx, &entry, now).and_then(|_| tx.commit());
        if let Err(e) = saved {
            if let Some(rel) = &file_path {
                let _ = fs::remove_file(self.data_dir.join(rel));
            }
            return Err(e.into());
        }
        Ok(Outcome::New { id, kind: item.kind, file_path, size })
    }
}

/// Temporary files left behind if the daemon was killed mid-write.
fn remove_leftover_tmp(media_dir: &Path) {
    let Ok(dir) = fs::read_dir(media_dir) else { return };
    for entry in dir.flatten() {
        if entry.file_name().to_string_lossy().starts_with(TMP_PREFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}
