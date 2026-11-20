//! Media type detection for the "type" search filter (linuxre.md, "Filtros de busca").

/// The six kinds a copied item can be. `as_str` matches the CHECK on `entries.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image,
    Color,
    File,
    Link,
    Unknown,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Color => "color",
            Kind::File => "file",
            Kind::Link => "link",
            Kind::Unknown => "unknown",
        }
    }
}

/// Result of classifying copied text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextClass {
    pub kind: Kind,
    /// What the history shows: normalized hex for a color, the URL for a link,
    /// the first characters for plain text.
    pub preview: String,
}

pub const PREVIEW_CHARS: usize = 200;

/// Classifies copied text as a color, a link or plain text.
pub fn classify_text(text: &str) -> TextClass {
    let t = text.trim();
    if let Some(hex) = parse_color(t) {
        return TextClass { kind: Kind::Color, preview: hex };
    }
    if is_link(t) {
        return TextClass { kind: Kind::Link, preview: t.to_string() };
    }
    TextClass { kind: Kind::Text, preview: preview_of(text) }
}

/// The whole text must be one color: hex (#RGB, #RGBA, #RRGGBB, #RRGGBBAA),
/// rgb()/rgba(), hsl()/hsla() and the other CSS functions, or a CSS name such as `red`.
/// Returns it as lowercase hex.
pub fn parse_color(t: &str) -> Option<String> {
    if t.is_empty() || t.len() > 100 || t.contains('\n') {
        return None;
    }
    // csscolorparser also takes hex without '#'; that would turn words like
    // "bad", "cafe" or "add" into colors, so a bare hex string is not a color.
    if t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    csscolorparser::parse(t)
        .ok()
        .map(|c| c.to_css_hex().to_string())
}

/// A single http(s) URL with nothing around it.
pub fn is_link(t: &str) -> bool {
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return false;
    }
    match url::Url::parse(t) {
        Ok(u) => matches!(u.scheme(), "https" | "http") && u.host().is_some(),
        Err(_) => false,
    }
}

/// First [`PREVIEW_CHARS`] characters, whitespace runs collapsed.
pub fn preview_of(text: &str) -> String {
    let mut out = String::new();
    for (i, word) in text.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }
    out.chars().take(PREVIEW_CHARS).collect()
}

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tif", "tiff", "avif", "heic", "heif",
    "ico", "jxl",
];

pub fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

const MIME_EXT: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("image/bmp", "bmp"),
    ("image/svg+xml", "svg"),
    ("image/tiff", "tiff"),
    ("image/avif", "avif"),
    ("image/heic", "heic"),
    ("image/heif", "heif"),
    ("image/x-icon", "ico"),
    ("image/jxl", "jxl"),
    ("text/plain", "txt"),
    ("text/html", "html"),
    ("text/csv", "csv"),
    ("text/markdown", "md"),
    ("application/pdf", "pdf"),
    ("application/json", "json"),
    ("application/zip", "zip"),
    ("application/gzip", "gz"),
    ("application/xml", "xml"),
    ("audio/mpeg", "mp3"),
    ("audio/ogg", "ogg"),
    ("audio/wav", "wav"),
    ("audio/flac", "flac"),
    ("video/mp4", "mp4"),
    ("video/webm", "webm"),
    ("video/x-matroska", "mkv"),
];

/// File extension for a MIME type (`image/png` → `png`); `bin` when unknown.
pub fn ext_for_mime(mime: &str) -> &'static str {
    let base = mime.split(';').next().unwrap_or("").trim();
    MIME_EXT
        .iter()
        .find(|(m, _)| m.eq_ignore_ascii_case(base))
        .map(|(_, e)| *e)
        .unwrap_or("bin")
}

/// MIME type for a file extension (`JPEG` → `image/jpeg`); `application/octet-stream` when unknown.
pub fn mime_for_ext(ext: &str) -> &'static str {
    let ext = ext.to_ascii_lowercase();
    let ext = match ext.as_str() {
        "jpeg" => "jpg",
        "tif" => "tiff",
        other => other,
    };
    MIME_EXT
        .iter()
        .find(|(_, e)| *e == ext)
        .map(|(m, _)| *m)
        .unwrap_or("application/octet-stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(t: &str) -> Kind {
        classify_text(t).kind
    }

    #[test]
    fn colors() {
        for c in ["#fff", "#FF0000", "#ff000080", "rgb(1,2,3)", "rgba(0, 0, 0, 0.5)",
                  "hsl(120, 100%, 50%)", "hsla(120 100% 50% / 0.3)", "red", " RebeccaPurple \n"] {
            assert_eq!(kind(c), Kind::Color, "{c}");
        }
        assert_eq!(classify_text("#f00").preview, "#ff0000");
        assert_eq!(classify_text("red").preview, "#ff0000");
    }

    #[test]
    fn not_colors() {
        for t in ["bad", "cafe", "add", "hello", "#ggg", "red car", "rgb(1,2", "#ff0000 and more"] {
            assert_ne!(kind(t), Kind::Color, "{t}");
        }
    }

    #[test]
    fn links() {
        assert_eq!(kind("https://example.com"), Kind::Link);
        assert_eq!(kind("  http://example.com/a?b=c  "), Kind::Link);
        assert_eq!(kind("see https://example.com"), Kind::Text);
        assert_eq!(kind("file:///etc/hosts"), Kind::Text);
        assert_eq!(kind("https://"), Kind::Text);
    }

    #[test]
    fn text_and_preview() {
        assert_eq!(kind("hello world"), Kind::Text);
        let long = "a ".repeat(500);
        assert_eq!(classify_text(&long).preview.chars().count(), PREVIEW_CHARS);
        assert_eq!(preview_of("  a\n\n b  "), "a b");
    }

    #[test]
    fn mime_and_ext() {
        assert!(is_image_ext("PNG"));
        assert!(!is_image_ext("pdf"));
        assert_eq!(ext_for_mime("image/png"), "png");
        assert_eq!(ext_for_mime("text/plain;charset=utf-8"), "txt");
        assert_eq!(ext_for_mime("x-weird/thing"), "bin");
        assert_eq!(mime_for_ext("JPEG"), "image/jpeg");
        assert_eq!(mime_for_ext("pdf"), "application/pdf");
        assert_eq!(mime_for_ext("xyz"), "application/octet-stream");
    }
}
