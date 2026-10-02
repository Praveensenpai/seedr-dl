# CODEBASE.md: seedr-dl Semantic Digest

> **Notice**: This file is an AI-optimized semantic index. Do not write narrative prose. Keep token density high.

## 1. System Topology & Data Flow
```text
CLI / Rust App
   │
   ├─► CLI Parser (src/main.rs)
   │     ├─► Auth / Config / History / Tasks CLI
   │     └─► Download Dispatcher
   │
   └─► Library API (src/lib.rs)
         ├─► seedr_dl::download_magnet(...)
         ├─► seedr_dl::download_folder(...)
         │     │
         │     ▼
         ├─► Notifier (src/notifier.rs) ──► HTTP Webhook POST / stdout ndjson / Rust Callbacks
         ├─► SeedrClient (src/seedr.rs) ──► Seedr REST API (caching, URLs, deletes)
         ├─► Downloader (src/downloader.rs) ──► Concurrent Multi-Range Chunk Streaming
         └─► Transfer (src/transfer.rs) ──► Direct File Placement & History Recording
```

## 2. Global Constraints & Architecture Patterns
- **Primary Language & Edition**: Rust 2021 edition.
- **Architectural Paradigm**: Modular dual-target architecture: headless CLI binary (`src/main.rs`) and reusable library crate (`src/lib.rs`).
- **Hard Constraints**: <400 lines/file (300 soft), <60 lines/fn (40 soft), max 4 params/fn, nesting depth <= 3, zero production `unwrap()`/`expect()`, zero compiler/clippy warnings (`-D warnings`).
- **Target Distribution**: Linux x86_64 standalone binary and Cargo library crate.

## 3. Module & Interface Skeleton

### `src/lib.rs` (Role: library root / public API, Lines: ~135)
- **Responsibility**: Crate entrypoint exporting public library API for other Rust applications.
- **Imports**: `anyhow::Result`, `std::path::PathBuf`, submodules `config`, `downloader`, `history`, `notifier`, `seedr`, `transfer`.
- **Types & Enums**:
  ```rust
  pub enum CloudCleanup { Delete, Keep }
  pub struct DownloadOptions {
      pub callback_url: Option<String>,
      pub callback_fn: Option<EventCallback>,
      pub json_output: bool,
      pub output_dir: Option<PathBuf>,
      pub cloud_cleanup: CloudCleanup,
  }
  ```
- **Public Functions**:
  ```rust
  pub async fn download_magnet(client: &SeedrClient, cfg: &Config, magnet: &str, opts: &DownloadOptions) -> Result<Vec<PathBuf>>
  pub async fn download_folder(client: &SeedrClient, cfg: &Config, folder: &SeedrFolder, opts: &DownloadOptions) -> Result<Vec<PathBuf>>
  ```
- **Consumers**: External Rust applications, `src/main.rs`.

### `src/main.rs` (Role: CLI entrypoint, Lines: ~390)
- **Responsibility**: Command-line interface for human users and subprocess execution by external tools.
- **Subcommands**: `download`, `list`, `delete`, `clean`, `tasks`, `cancel`, `auth`, `config`, `history`, `worker` (hidden).
- **Global Options**: `-o, --output <DIR>`, `--callback-url <URL>`, `--json`, `-y/--yes`, `--keep`.
- **Side Effects / I/O**: stdout formatting, process spawning for background workers.

### `src/notifier.rs` (Role: event notification & webhook dispatcher, Lines: ~204)
- **Responsibility**: Dispatches progress, status, completion, and failure events to HTTP webhooks, stdout ndjson, and in-memory callbacks.
- **Types & Enums**:
  ```rust
  pub enum NotificationEvent {
      Progress { folder_id: u64, file_name: String, downloaded_bytes: u64, total_bytes: u64, speed_bps: u64, eta_seconds: u64, percentage: f64 },
      Status { folder_id: u64, file_name: Option<String>, status: String, message: String },
      Completed { folder_id: u64, file_name: String, destination_path: Option<String>, total_bytes: u64 },
      Failed { folder_id: u64, file_name: Option<String>, error: String },
  }
  pub type EventCallback = Arc<dyn Fn(NotificationEvent) + Send + Sync>;
  pub struct NotifierConfig { pub callback_url: Option<String>, pub json_output: bool, pub callback_fn: Option<EventCallback>, pub progress_interval_ms: u64 }
  pub struct Notifier { ... }
  ```
- **Public Functions**:
  ```rust
  pub fn new(config: NotifierConfig) -> Self
  pub fn notify(&self, event: NotificationEvent)
  pub fn is_active(&self) -> bool
  pub async fn flush(&self)
  ```
- **Side Effects / I/O**: Non-blocking tokio mpsc channel, HTTP POST to webhook URLs with rate-limiting.

### `src/seedr.rs` (Role: Seedr API adapter, Lines: ~396)
- **Responsibility**: Authenticates with Seedr.cc, manages cloud folders/torrents, adaptive polling, and fetches download URLs.
- **Types**: `SeedrTorrent`, `SeedrFolder`, `SeedrFile`, `CachingQuery`, `ListContentsResponse`, `SeedrClient`.
- **Public Functions**:
  ```rust
  pub async fn login(email: &str, pass: &str) -> Result<Auth>
  pub async fn add_magnet(&self, magnet: &str) -> Result<u64>
  pub async fn wait_for_caching(&self, query: &CachingQuery<'_>, on_progress: Option<&mut (dyn FnMut(&SeedrTorrent) + Send)>) -> Result<Option<SeedrFolder>>
  pub async fn list_root(&self) -> Result<ListContentsResponse>
  pub async fn list_folder(&self, folder_id: u64) -> Result<ListContentsResponse>
  pub async fn get_download_url(&self, file_id: u64) -> Result<String>
  pub async fn delete_folder(&self, folder_id: u64) -> Result<()>
  pub async fn delete_torrent(&self, torrent_id: u64) -> Result<()>
  pub async fn delete_all_folders(&self, folders: &[SeedrFolder]) -> Result<()>
  pub fn extract_magnet_name(magnet_or_url: &str) -> Option<String>
  ```

### `src/transfer.rs` (Role: direct file download pipeline & series resolver, Lines: ~315)
- **Responsibility**: Recursively scans cloud folders and subfolders, preserves series directory hierarchy, streams cloud files to local destination directory with cumulative progress reporting.
- **Types**: `CloudFileItem`, `TransferOptions`, `ItemDownloadCtx`.
- **Public Functions**:
  ```rust
  pub async fn collect_folder_files(client: &SeedrClient, folder_id: u64) -> Result<Vec<CloudFileItem>>
  pub fn resolve_destination(output_dir: &Path, folder_name: &str, item: &CloudFileItem, is_multi_file: bool) -> PathBuf
  pub async fn download_folder(client: &SeedrClient, cfg: &Config, folder: &SeedrFolder, opts: &TransferOptions<'_>) -> Result<Vec<PathBuf>>
  ```

### `src/downloader.rs` (Role: HTTP multi-range streaming engine, Lines: ~336)
- **Submodules**: `coordinator.rs` (72), `chunk.rs` (145), `single.rs` (99), `state.rs` (172).
- **Public Functions**:
  ```rust
  pub fn new(threads: usize) -> Self
  pub async fn download(&self, url: &str, target_dir: &Path, file_name: &str) -> Result<PathBuf>
  pub async fn download_with_callback<F>(&self, url: &str, target_dir: &Path, file_name: &str, callback: F) -> Result<PathBuf>
  ```

### `src/config.rs` (Role: persistent configuration, Lines: ~173)
- **Types**: `Config` (`download_dir`, `delete_after_download`, `download_threads`), `Auth` (`access_token`, `refresh_token`).
- **Functions**: `load_config()`, `save_config()`, `load_auth()`, `save_auth()`, `interactive_auth()`.

### `src/history.rs` (Role: local download log, Lines: ~134)
- **Types**: `HistoryEntry` (`id`, `original_name`, `file_path`, `file_size`, `downloaded_at`).
- **Functions**: `load_history()`, `save_history()`, `add_history_entry()`, `clear_history()`.

### `src/worker.rs` (Role: background multi-file series runner, Lines: ~340)
- **Responsibility**: Manages background downloading of entire series/folders recursively with cumulative progress reporting to tasks and webhooks, resolving accurate series/file names.
- **Types**: `WorkerSpawnOpts`, `WorkerBatchCtx`, `WorkerExecCtx`.
- **Functions**: `spawn_worker(opts: &WorkerSpawnOpts)`, `run_worker(folder_id: u64, callback_url: Option<String>, output_dir: Option<PathBuf>)`.

### `src/task.rs` (Role: background task state persistence & CLI display, Lines: ~217)
- **Types**: `TaskState`, `TaskStatus`.
- **Functions**: `save_task()`, `load_task()`, `remove_task()`, `list_active_tasks()`, `cancel_task()`, `register_caching_task()`, `update_caching_progress()`, `print_active_tasks()`.

