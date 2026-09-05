# ⚡ seedr-dl

**Pure Rust Seedr.cc Downloader with Gemini AI Renaming for Jellyfin.**

`seedr-dl` bridges your **Seedr.cc** cloud torrent storage and your **Jellyfin** media server:
1. Adds magnet links / torrents to Seedr and waits for cloud caching.
2. Downloads files using a pure Rust streaming engine with live `indicatif` progress bars.
3. Automatically deciphers messy scene torrent filenames into canonical titles, release years, seasons, and episodes using **Google Gemini AI** (with an offline regex fallback).
4. Moves and organizes files directly into `~/jellyfin/media/movies` or `~/jellyfin/media/shows`.

---

## 🚀 Quick Usage

### 1. One-Time Setup
Authenticate your Seedr account:
```bash
seedr-dl auth
```

(Optional) Configure your Google Gemini API key for AI-powered recognition:
```bash
seedr-dl config --gemini-key "YOUR_GEMINI_API_KEY"
# Or export in your shell:
export GEMINI_API_KEY="YOUR_GEMINI_API_KEY"
```

### 2. Download a Magnet Link
```bash
seedr-dl "magnet:?xt=urn:btih:..."
```

### 3. List and Download Existing Cloud Files
```bash
seedr-dl list
# Or simply run without arguments:
seedr-dl
```

### 4. Non-Interactive / Scripting Mode
```bash
seedr-dl -y "magnet:?xt=urn:btih:..."
```

---

## 📁 Jellyfin Media Hierarchy

`seedr-dl` places files directly into your Jellyfin library according to official conventions:

- **Movies**:
  ```text
  ~/jellyfin/media/movies/Oppenheimer (2023)/Oppenheimer (2023).mkv
  ```
- **TV Shows**:
  ```text
  ~/jellyfin/media/shows/Breaking Bad/Season 01/Breaking Bad - S01E05.mkv
  ```

---

## 🛠️ Commands & Flags

```text
Usage: seedr-dl [OPTIONS] [MAGNET_OR_URL] [COMMAND]

Commands:
  auth    Authenticate with Seedr.cc
  list    List files and folders currently in your Seedr cloud
  config  Configure Jellyfin path or Gemini API key
  help    Print help message

Arguments:
  [MAGNET_OR_URL]  Magnet link or torrent URL to download

Options:
  -y, --yes        Skip confirmation prompts
  -h, --help       Print help
  -V, --version    Print version
```

---

## 🛡️ Code Quality
- Strictly `<300` lines per file.
- Strictly `<40` lines per function.
- Pure Rust TLS (`rustls`), zero OpenSSL dependencies.
- Zero unhandled `unwrap()` calls.
- Clippy enforced under `#![deny(clippy::all)]`.
