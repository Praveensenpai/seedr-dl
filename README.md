# ⚡ seedr-dl — Seedr.cc Automation CLI & Rust Library

> **High-speed Seedr.cc automation engine, multi-threaded parallel chunk downloader, and webhook notifier with Gemini AI Jellyfin ingestion.**

[![GitHub release](https://img.shields.io/github/v/release/Praveensenpai/seedr-dl?style=flat-square&color=388bfd)](https://github.com/Praveensenpai/seedr-dl/releases/latest)
[![Crates.io](https://img.shields.io/badge/crates.io-v0.0.12-orange?style=flat-square)](https://crates.io)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/rust-2021-purple?style=flat-square)](https://www.rust-lang.org)

`seedr-dl` operates both as a **headless CLI tool** with rich webhook notifications for external scripts/processes, and as a **modular Rust library** for direct Cargo integration into your Rust applications.

---

## 🏗️ Architecture & Data Flow

```text
┌────────────────────────────────┐     ┌────────────────────────────────┐
│        CLI Execution           │     │    External Rust Application   │
│   (seedr-dl download ...)      │     │  (seedr_dl::download_magnet)   │
└───────────────┬────────────────┘     └───────────────┬────────────────┘
                │                                      │
                ▼                                      ▼
    ┌──────────────────────────────────────────────────────────────┐
    │                 seedr-dl Core Engine                         │
    ├──────────────────────────────┬───────────────────────────────┤
    │  Seedr API (OAuth & Caching) │  Multi-Chunk Range Downloader │
    │  Gemini AI Renamer           │  Jellyfin File Organizer      │
    └───────────────┬──────────────┴───────────────┬───────────────┘
                    │                              │
                    ▼                              ▼
    ┌──────────────────────────────┐   ┌──────────────────────────────┐
    │    Webhook Notifier          │   │      Target Media Library    │
    │  • HTTP POST (JSON Payload)  │   │  • Movies/Title (Year)/...   │
    │  • stdout streaming (ndjson) │   │  • TV/Show/Season 01/...     │
    │  • In-Memory Rust Callback   │   │                              │
    └──────────────────────────────┘   └──────────────────────────────┘
```

---

## 🚀 Installation

### One-Liner Install
```bash
curl -LsSf https://raw.githubusercontent.com/Praveensenpai/seedr-dl/main/install.sh | bash
```

### Build from Source
```bash
git clone https://github.com/Praveensenpai/seedr-dl
cd seedr-dl
cargo build --release
cp target/release/seedr-dl ~/.local/bin/
```

---

## 🛠️ CLI Usage

### Basic Commands
```bash
# 1. Log in to your Seedr account
seedr-dl auth

# 2. Configure Jellyfin path & Gemini API key (optional)
seedr-dl config --media-dir /mnt/media --gemini-key YOUR_GEMINI_KEY

# 3. Download a magnet or torrent URL
seedr-dl download "magnet:?xt=urn:btih:..."

# 4. List items currently in Seedr cloud
seedr-dl list
```

### Webhook & Event Notifications
Pass `--callback-url` to receive real-time HTTP POST notifications for progress, completion, and failure events:

```bash
seedr-dl download "magnet:?xt=urn:btih:..." \
  --callback-url "https://api.yourdomain.com/webhooks/downloads" \
  --yes
```

### Machine-Readable Output (`--json`)
Stream events as single-line JSON (`ndjson`) directly to stdout for Unix piping:

```bash
seedr-dl download "magnet:?xt=urn:btih:..." --json
```

### Subcommands Overview
| Command | Arguments | Description |
|---|---|---|
| `download` | `<TARGET>` | Download magnet, torrent URL, or cloud folder ID |
| `list` | `[--json]` | List active torrents and folders in cloud |
| `delete` | `<ID> [-y]` | Delete a folder or cancel a torrent |
| `clean` | `[-y]` | Delete all completed cloud folders |
| `tasks` | `[--json]` | View active background ingestion workers |
| `cancel` | `<FOLDER_ID>` | Cancel a running background download worker |
| `history` | `[list\|rename\|scan]` | Manage downloaded and organized media |
| `config` | `[--show\|--gemini-key\|--media-dir]` | View and adjust configuration |

---

## 📦 Using as a Rust Library

Add `seedr-dl` to your `Cargo.toml`:

```toml
[dependencies]
seedr-dl = { git = "https://github.com/Praveensenpai/seedr-dl" }
tokio = { version = "1.43", features = ["full"] }
```

### Example: Download with In-Memory Progress Callback

```rust
use seedr_dl::{
    download_magnet, Config, DownloadOptions, NotificationEvent, SeedrClient,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = SeedrClient::new("YOUR_ACCESS_TOKEN".into());
    let config = Config::default();

    let options = DownloadOptions {
        callback_url: Some("https://example.com/webhook".into()),
        callback_fn: Some(Arc::new(|event| match event {
            NotificationEvent::Progress { percentage, speed_bps, .. } => {
                println!("Progress: {percentage:.1}% ({speed_bps} B/s)");
            }
            NotificationEvent::Completed { file_name, destination_path, .. } => {
                println!("Done: {file_name} -> {destination_path:?}");
            }
            NotificationEvent::Failed { error, .. } => {
                eprintln!("Failed: {error}");
            }
            NotificationEvent::Status { status, message, .. } => {
                println!("[{status}] {message}");
            }
        })),
        ..Default::default()
    };

    let files = download_magnet(
        &client,
        &config,
        "magnet:?xt=urn:btih:...",
        &options,
    ).await?;

    println!("Downloaded {} file(s)!", files.len());
    Ok(())
}
```

---

## 🔔 Webhook Payloads

HTTP POST requests send a JSON body matching the `NotificationEvent` schema:

### Progress Event
```json
{
  "event": "progress",
  "folder_id": 123456,
  "file_name": "Show.S01E01.1080p.mkv",
  "downloaded_bytes": 524288000,
  "total_bytes": 1048576000,
  "speed_bps": 20971520,
  "eta_seconds": 25,
  "percentage": 50.0
}
```

### Completed Event
```json
{
  "event": "completed",
  "folder_id": 123456,
  "file_name": "Show.S01E01.1080p.mkv",
  "destination_path": "/media/shows/Show/Season 01/Show - S01E01.mkv",
  "total_bytes": 1048576000
}
```

### Failed Event
```json
{
  "event": "failed",
  "folder_id": 123456,
  "file_name": null,
  "error": "Not enough space in your Seedr cloud account"
}
```

---

## 📜 License

Distributed under the [MIT License](LICENSE).
