<div align="center">
  <h1>⚡ seedr-dl</h1>
  <p><em>Ratatui TUI for Seedr.cc — 8-thread parallel downloads, attach/detach, Gemini AI renaming for Jellyfin</em></p>

  [![GitHub release](https://img.shields.io/github/v/release/Praveensenpai/seedr-dl?style=flat-square)](https://github.com/Praveensenpai/seedr-dl/releases/latest)
  [![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
</div>

---

## 🚀 Install (one-liner)

```bash
curl -LsSf https://raw.githubusercontent.com/Praveensenpai/seedr-dl/main/install.sh | bash
```

Detects your OS/arch automatically and downloads the pre-built binary from the latest GitHub Release.

---

## ✨ Features

| Feature | Description |
|---|---|
| **Ratatui TUI** | Full terminal dashboard — navigate with `↑/↓` or `j/k` |
| **8-thread downloader** | Parallel chunk downloading via HTTP Range requests |
| **Resumable** | Each chunk tracked independently — restart picks up where it left off |
| **Attach / Detach** | `Enter` starts & attaches, `[d]`/`[Esc]` detaches (download keeps going), `[a]` re-attaches |
| **Gemini AI renaming** | Auto-parses media titles for clean Jellyfin library structure |
| **Jellyfin integration** | Organizes downloaded files into `Movies/` or `TV Shows/` automatically |

---

## 🎮 Keybindings

### Manager
| Key | Action |
|---|---|
| `↑/↓` or `j/k` | Select item |
| `Enter` | Download selected item & attach to live monitor |
| `a` | Attach to running download |
| `m` | Add magnet link |
| `d` | Delete selected cloud item |
| `r` | Refresh |
| `q` | Quit |

### Download Monitor (attach screen)
| Key | Action |
|---|---|
| `d` / `Esc` | **Detach** — download continues in background |
| `x` | Cancel download |
| `Enter` / `q` | Return to manager (when complete) |

---

## ⚙️ Setup

```bash
# Authenticate with Seedr.cc
seedr-dl auth

# (Optional) Set Jellyfin media path & Gemini API key
seedr-dl config --media-dir ~/Media --gemini-key YOUR_KEY

# Launch TUI
seedr-dl
```

---

## 🔨 Build from source

```bash
git clone https://github.com/Praveensenpai/seedr-dl
cd seedr-dl
cargo build --release
cp target/release/seedr-dl ~/.local/bin/
```
