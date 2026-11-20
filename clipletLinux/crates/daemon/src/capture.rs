//! Decides what to read from a clipboard offer and turns it into [`Item`]s, detecting the type
//! used by the search filter: text, image, color, file, link or unknown.

use std::{
    fs::File,
    io::{self, Cursor, Read},
    path::Path,
};

use cliplet_core::classify::{self, Kind};
use wl_clipboard_rs::watch::Offer;

use crate::store::{Content, Item, Store};

type Error = Box<dyn std::error::Error>;

const URI_LIST: &str = "text/uri-list";

/// Plain-text MIME types, best first (UTF-8 before the legacy X11 names).
const TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
    "COMPOUND_TEXT",
];

fn is_text_mime(m: &str) -> bool {
    TEXT_MIMES.contains(&m) || m.starts_with("text/plain")
}

/// Offered types that describe the selection rather than hold content.
fn is_meta_mime(m: &str) -> bool {
    matches!(m, "TARGETS" | "SAVE_TARGETS" | "MULTIPLE" | "TIMESTAMP" | "DELETE" | "INCR")
        || m.starts_with("x-kde-")
        || m.starts_with("chromium/")
        || m.starts_with("application/x-qt-")
}

/// Reads the selection. Order:
/// 1. `text/uri-list` with local files (file manager copy) → one image/file item per file;
/// 2. an image, when no plain text is offered → image;
/// 3. plain text → text, color or link;
/// 4. any other type → unknown.
pub fn read(mime_types: &[String], offer: &mut Offer<'_>, store: &Store) -> Result<Vec<Item>, Error> {
    if mime_types.iter().any(|m| m == URI_LIST) {
        let list = read_all(offer, URI_LIST)?;
        let items = files_from_uri_list(&String::from_utf8_lossy(&list), store)?;
        if !items.is_empty() {
            return Ok(items);
        }
        // No local files in it (e.g. only web URLs): the text rule below catches those.
    }

    let has_text = mime_types.iter().any(|m| is_text_mime(m));
    if !has_text {
        let image = mime_types
            .iter()
            .find(|m| m.as_str() == "image/png")
            .or_else(|| mime_types.iter().find(|m| m.starts_with("image/")));
        if let Some(mime) = image {
            let mut pipe = offer.receive(mime)?;
            let staged = store.stage(&mut pipe, classify::ext_for_mime(mime))?;
            if staged.size == 0 {
                return Ok(vec![]);
            }
            return Ok(vec![Item {
                kind: Kind::Image,
                mime_type: Some(mime.clone()),
                file_name: None,
                preview_text: None,
                content: Content::File(staged),
            }]);
        }
    }

    if let Some(mime) = TEXT_MIMES
        .iter()
        .find(|t| mime_types.iter().any(|m| m == *t))
        .map(|t| t.to_string())
        .or_else(|| mime_types.iter().find(|m| is_text_mime(m)).cloned())
    {
        let bytes = read_all(offer, &mime)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if text.trim().is_empty() {
            return Ok(vec![]);
        }
        let class = classify::classify_text(&text);
        let content = if text.len() > store.inline_max {
            Content::File(store.stage(&mut Cursor::new(text.as_bytes()), "txt")?)
        } else {
            Content::Inline(text)
        };
        return Ok(vec![Item {
            kind: class.kind,
            mime_type: Some("text/plain".into()),
            file_name: None,
            preview_text: Some(class.preview),
            content,
        }]);
    }

    if let Some(mime) = mime_types.iter().find(|m| !is_meta_mime(m)) {
        let mut pipe = offer.receive(mime)?;
        let staged = store.stage(&mut pipe, classify::ext_for_mime(mime))?;
        if staged.size == 0 {
            return Ok(vec![]);
        }
        return Ok(vec![Item {
            kind: Kind::Unknown,
            mime_type: Some(mime.clone()),
            file_name: None,
            preview_text: None,
            content: Content::File(staged),
        }]);
    }

    Ok(vec![])
}

fn read_all(offer: &mut Offer<'_>, mime: &str) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    offer.receive(mime)?.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Local files named in a `text/uri-list` (RFC 2483: one URI per line, `#` starts a comment).
/// Directories and anything that isn't a readable regular file are skipped.
fn files_from_uri_list(list: &str, store: &Store) -> Result<Vec<Item>, Error> {
    let mut items = Vec::new();
    for line in list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Ok(url) = url::Url::parse(line) else { continue };
        if url.scheme() != "file" {
            continue;
        }
        let Ok(path) = url.to_file_path() else { continue };
        match file_item(&path, store) {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(e) => eprintln!("skipping {}: {e}", path.display()),
        }
    }
    Ok(items)
}

fn file_item(path: &Path, store: &Store) -> io::Result<Option<Item>> {
    if !path.metadata()?.is_file() {
        return Ok(None);
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let kind = if classify::is_image_ext(&ext) { Kind::Image } else { Kind::File };
    let stored_ext = if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        "bin".to_string()
    } else {
        ext.clone()
    };

    let staged = store.stage(&mut File::open(path)?, &stored_ext)?;
    Ok(Some(Item {
        kind,
        mime_type: Some(classify::mime_for_ext(&ext).into()),
        file_name: name.clone(),
        preview_text: name,
        content: Content::File(staged),
    }))
}
