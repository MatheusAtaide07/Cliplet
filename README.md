# Cliplet

A clipboard manager that syncs with your other devices.

## Repository layout

| Directory       | Description                                              |
| --------------- | -------------------------------------------------------- |
| `clipletLinux/` | Linux app (Tauri + React UI, Rust workspace with `core` and the Wayland clipboard `daemon`) |
| `clipletMac/`   | macOS app (not started yet)                              |

## clipletLinux

```sh
cd clipletLinux
cargo run -p daemon   # Wayland clipboard watcher (needs wlr/ext data-control)
npm install && npm run tauri dev   # desktop app
```
