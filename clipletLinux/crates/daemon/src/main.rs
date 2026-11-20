//! Clipboard capture daemon for Wayland compositors with the data-control protocol
//! (KDE Plasma, wlroots: Sway, Hyprland…). Every copy is classified and saved to the local
//! SQLite database; binary content and large text go to the `media/` folder.

mod capture;
mod store;

use std::process::ExitCode;

use cliplet_core::paths;
use wl_clipboard_rs::{
    paste::{Error, Seat},
    watch::{ClipboardEvent, ClipboardType, Watcher},
};

use store::{Outcome, Store};

fn main() -> ExitCode {
    let data_dir = paths::data_dir();
    let mut store = match Store::open(data_dir.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open the database in {}: {e}", data_dir.display());
            return ExitCode::FAILURE;
        }
    };
    println!("cliplet daemon: saving to {}", data_dir.display());

    let mut watcher = match Watcher::new(ClipboardType::Regular, Seat::Unspecified) {
        Ok(w) => w,
        Err(e @ Error::MissingProtocol { .. }) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("cannot watch the clipboard: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The first event is the selection that was already there when the daemon started. If it is
    // already in the history, leave its last_used_at alone so a restart doesn't reorder history.
    let mut first = true;
    loop {
        let event = match watcher.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("clipboard error: {e}");
                return ExitCode::FAILURE;
            }
        };
        if let ClipboardEvent::Changed { mime_types, mut offer, .. } = event {
            match capture::read(&mime_types, &mut offer, &store) {
                Ok(items) => {
                    for item in items {
                        match store.ingest(item, !first) {
                            Ok(outcome) => log(&outcome),
                            Err(e) => eprintln!("failed to save clipboard entry: {e}"),
                        }
                    }
                }
                Err(e) => eprintln!("failed to read clipboard: {e}"),
            }
        }
        first = false;
    }
}

fn log(outcome: &Outcome) {
    match outcome {
        Outcome::New { kind, file_path: Some(path), size, .. } => {
            println!("new {} {path} {}", kind.as_str(), human_size(*size));
        }
        Outcome::New { id, kind, file_path: None, size } => {
            println!("new {} {id} {}", kind.as_str(), human_size(*size));
        }
        Outcome::Duplicate { id, kind, touched: true } => println!("dup {kind} → touched {id}"),
        Outcome::Duplicate { id, kind, touched: false } => println!("dup {kind} {id} (already saved)"),
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{size:.1} {}", UNITS[unit]) }
}
