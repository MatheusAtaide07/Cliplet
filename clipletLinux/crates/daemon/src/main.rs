use std::{
    hash::{DefaultHasher, Hash, Hasher},
    io::Read,
    thread,
    time::Duration,
};

use wl_clipboard_rs::paste::{ClipboardType, Error, MimeType, Seat, get_contents};

const POLL_INTERVAL: Duration = Duration::from_millis(500);

fn main() {
    let mut last: Option<u64> = None;

    loop {
        match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Any) {
            Ok((mut pipe, mime)) => {
                let mut buf = Vec::new();
                match pipe.read_to_end(&mut buf) {
                    Ok(_) => {
                        let mut hasher = DefaultHasher::new();
                        (&mime, &buf).hash(&mut hasher);
                        let hash = hasher.finish();
                        if last != Some(hash) {
                            last = Some(hash);
                            print_entry(&mime, &buf);
                        }
                    }
                    Err(e) => eprintln!("failed to read clipboard: {e}"),
                }
            }
            Err(Error::ClipboardEmpty | Error::NoMimeType) => {}
            Err(e @ Error::MissingProtocol { .. }) => {
                eprintln!("{e}");
                return;
            }
            Err(e) => eprintln!("clipboard error: {e}"),
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

fn print_entry(mime: &str, data: &[u8]) {
    if is_text(mime) {
        println!("[{mime}] {}", String::from_utf8_lossy(data));
    } else {
        println!("[{mime}] {} bytes", data.len());
    }
}
