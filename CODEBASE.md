# CODEBASE.md: seedr-dl Semantic Digest

> **Notice**: AI-optimized semantic index. Do not write narrative prose. Keep token density high.

## 1. System Topology & Data Flow
```text
Entrypoint ──> CLI/Parser ──> Domain Logic ──> Infra/IO
```

## 2. Global Constraints & Architecture Patterns
- **Primary Language**: Rust 2021 edition
- **Architectural Paradigm**: Role-based (domain/, infra/, api/cli/, tui/)
- **Hard Constraints**: <400 lines/file, <60 lines/fn, zero production unwrap(), 0 warnings.
- **Target Distribution**: Linux x86_64 standalone binary

## 3. Module & Interface Skeleton

### `src/config.rs` (Role: general, Lines: 170)
- **Responsibility**: Core general logic in src/config.rs
- **Imports**: use anyhow :: { Context , Result } , use serde :: { Deserialize , Serialize } , use std :: fs , use std :: path :: { Path , PathBuf } 
- **Types & Enums**:
  ```rust
  pub struct Config
  pub struct Auth
  ```
- **Public Functions & Signatures**:
  ```rust
  fn config_dir () -> PathBuf
  fn cache_dir () -> PathBuf
  fn downloads_dir () -> PathBuf
  fn load_config () -> Result < Config >
  fn save_config (config : & Config) -> Result < () >
  fn load_auth () -> Result < Option < Auth > >
  fn save_auth (auth : & Auth) -> Result < () >
  async fn interactive_auth () -> Result < Auth >
  ```

### `src/downloader/chunk.rs` (Role: general, Lines: 145)
- **Responsibility**: Core general logic in src/downloader/chunk.rs
- **Imports**: use crate :: downloader :: state :: { ChunkRange , DownloadState } , use anyhow :: { Context , Result } , use futures_util :: StreamExt , use reqwest :: header :: RANGE , use reqwest :: Client , use std :: path :: PathBuf , use std :: sync :: atomic :: { AtomicBool , Ordering } , use std :: sync :: Arc , use std :: time :: Duration , use tokio :: fs :: OpenOptions , use tokio :: io :: { AsyncSeekExt , AsyncWriteExt , SeekFrom } , use tokio :: sync :: Mutex 
- **Types & Enums**:
  ```rust
  pub struct ChunkJob
  ```
- **Public Functions & Signatures**:
  ```rust
  async fn download_chunk_with_retry (job : ChunkJob , state : Arc < Mutex < DownloadState > > , failed_flag : Arc < AtomicBool > ,) -> Result < () >
  ```

### `src/downloader/coordinator.rs` (Role: general, Lines: 72)
- **Responsibility**: Core general logic in src/downloader/coordinator.rs
- **Imports**: use crate :: downloader :: state :: DownloadState , use crate :: downloader :: ProgressCallback , use std :: path :: PathBuf , use std :: sync :: atomic :: { AtomicBool , Ordering } , use std :: sync :: Arc , use std :: time :: { Duration , Instant } , use tokio :: sync :: Mutex 
- **Types & Enums**:
  ```rust
  pub struct CoordinatorConfig
  ```
- **Public Functions & Signatures**:
  ```rust
  fn spawn_coordinator (cfg : CoordinatorConfig) -> tokio :: task :: JoinHandle < () >
  ```

### `src/downloader/single.rs` (Role: general, Lines: 99)
- **Responsibility**: Core general logic in src/downloader/single.rs
- **Imports**: use crate :: downloader :: state :: DownloadState , use crate :: downloader :: { DownloadTarget , ProgressCallback } , use anyhow :: { Context , Result } , use futures_util :: StreamExt , use reqwest :: header :: RANGE , use reqwest :: Client , use std :: path :: PathBuf , use std :: time :: Instant , use tokio :: fs :: { self , File , OpenOptions } , use tokio :: io :: { AsyncSeekExt , AsyncWriteExt , SeekFrom } 
- **Public Functions & Signatures**:
  ```rust
  async fn download_single (client : & Client , target : & DownloadTarget < '_ > , callback : ProgressCallback ,) -> Result < PathBuf >
  ```

### `src/downloader/state.rs` (Role: general, Lines: 172)
- **Responsibility**: Core general logic in src/downloader/state.rs
- **Imports**: use anyhow :: { Context , Result } , use serde :: { Deserialize , Serialize } , use std :: path :: { Path , PathBuf } , use tokio :: fs 
- **Types & Enums**:
  ```rust
  pub struct ChunkRange
  pub struct DownloadState
  ```
- **Public Functions & Signatures**:
  ```rust
  fn len (& self) -> u64
  fn is_empty (& self) -> bool
  fn is_complete (& self) -> bool
  fn resume_offset (& self) -> u64
  fn new (total_size : u64 , num_threads : usize) -> Self
  fn state_path (target_dir : & Path , file_name : & str) -> PathBuf
  fn part_path (target_dir : & Path , file_name : & str) -> PathBuf
  async fn load_or_init (target_dir : & Path , file_name : & str , total_size : u64 , num_threads : usize ,) -> Result < Self >
  async fn save (& self , target_dir : & Path , file_name : & str) -> Result < () >
  async fn cleanup (target_dir : & Path , file_name : & str)
  fn total_downloaded (& self) -> u64
  async fn cleanup_legacy_parts (target_dir : & Path , file_name : & str)
  ```

### `src/downloader.rs` (Role: general, Lines: 343)
- **Responsibility**: Core general logic in src/downloader.rs
- **Imports**: use self :: chunk :: { download_chunk_with_retry , ChunkJob } , use self :: coordinator :: { spawn_coordinator , CoordinatorConfig } , use self :: single :: download_single , use self :: state :: { cleanup_legacy_parts , ChunkRange , DownloadState } , use anyhow :: { Context , Result } , use colored :: Colorize , use indicatif :: { ProgressBar , ProgressStyle } , use reqwest :: header :: RANGE , use reqwest :: Client , use std :: net :: SocketAddr , use std :: path :: { Path , PathBuf } , use std :: sync :: atomic :: { AtomicBool , Ordering } , use std :: sync :: Arc , use std :: time :: Duration , use tokio :: fs :: { self , OpenOptions } , use tokio :: sync :: Mutex 
- **Types & Enums**:
  ```rust
  pub struct Downloader
  pub struct DownloadTarget
  ```
- **Public Functions & Signatures**:
  ```rust
  fn new (threads : usize) -> Self
  async fn download (& self , url : & str , target_dir : & Path , file_name : & str) -> Result < PathBuf >
  async fn download_with_callback < F > (& self , url : & str , target_dir : & Path , file_name : & str , callback : F ,) -> Result < PathBuf > where F : Fn (u64 , u64 , u64 , u64) + Send + Sync + 'static ,
  ```

### `src/history.rs` (Role: general, Lines: 134)
- **Responsibility**: Core general logic in src/history.rs
- **Imports**: use crate :: config :: config_dir , use anyhow :: { Context , Result } , use chrono :: Local , use serde :: { Deserialize , Serialize } , use std :: fs , use std :: path :: { Path , PathBuf } , use std :: time :: { SystemTime , UNIX_EPOCH } 
- **Types & Enums**:
  ```rust
  pub struct HistoryEntry
  ```
- **Public Functions & Signatures**:
  ```rust
  fn history_path () -> PathBuf
  fn generate_id () -> String
  fn current_timestamp () -> String
  fn load_history () -> Result < Vec < HistoryEntry > >
  fn save_history (entries : & [HistoryEntry]) -> Result < () >
  fn clear_history () -> Result < () >
  fn add_history_entry (entry : HistoryEntry) -> Result < () >
  fn media_file_exists (path : & Path) -> bool
  ```

### `src/history_cli.rs` (Role: cli, Lines: 60)
- **Responsibility**: Core cli logic in src/history_cli.rs
- **Imports**: use anyhow :: Result , use clap :: Subcommand , use colored :: Colorize , use seedr_dl :: history :: { clear_history , load_history , media_file_exists } 
- **Types & Enums**:
  ```rust
  pub enum HistoryCommands
  ```
- **Public Functions & Signatures**:
  ```rust
  fn handle_history_cli (cmd : Option < HistoryCommands >) -> Result < () >
  ```

### `src/lib.rs` (Role: general, Lines: 151)
- **Responsibility**: Core general logic in src/lib.rs
- **Imports**: pub use config :: { interactive_auth , load_auth , load_config , save_auth , save_config , Auth , Config } , pub use downloader :: Downloader , pub use notifier :: { EventCallback , NotificationEvent , Notifier , NotifierConfig } , pub use queue :: { CacheOutcome , CachePolicy , EnsureOpts , Space } , pub use seedr :: { extract_btih_hash , extract_magnet_name , CachingQuery , ListContentsResponse , SeedrClient , SeedrFile , SeedrFolder , SeedrTorrent , } , pub use transfer :: { collect_folder_files , download_folder as transfer_download_folder , resolve_destination , CloudFileItem , TransferOptions , } , use anyhow :: { bail , Result } , use std :: path :: PathBuf 
- **Types & Enums**:
  ```rust
  pub enum CloudCleanup
  pub struct DownloadOptions
  ```
- **Public Functions & Signatures**:
  ```rust
  async fn download_magnet (client : & SeedrClient , cfg : & Config , magnet : & str , opts : & DownloadOptions ,) -> Result < Vec < PathBuf > >
  async fn download_folder (client : & SeedrClient , cfg : & Config , folder : & SeedrFolder , opts : & DownloadOptions ,) -> Result < Vec < PathBuf > >
  ```

### `src/main.rs` (Role: general, Lines: 389)
- **Responsibility**: Core general logic in src/main.rs
- **Imports**: use anyhow :: { Context , Result } , use clap :: { Parser , Subcommand } , use colored :: Colorize , use seedr_dl :: config :: { load_auth , load_config , save_config , Config } , use seedr_dl :: notifier :: { NotificationEvent , Notifier , NotifierConfig } , use seedr_dl :: seedr :: { CachingQuery , ListContentsResponse , SeedrClient , SeedrFolder , SeedrTorrent } , use seedr_dl :: transfer :: TransferOptions , use std :: io :: { self , Write } , use std :: path :: PathBuf 

### `src/notifier.rs` (Role: general, Lines: 204)
- **Responsibility**: Core general logic in src/notifier.rs
- **Imports**: use serde :: { Deserialize , Serialize } , use std :: sync :: atomic :: { AtomicBool , Ordering } , use std :: sync :: Arc , use std :: time :: { Duration , Instant } , use tokio :: sync :: mpsc :: { self , UnboundedSender } , use tokio :: sync :: Mutex 
- **Types & Enums**:
  ```rust
  pub enum NotificationEvent
  pub struct NotifierConfig
  pub struct Notifier
  ```
- **Public Functions & Signatures**:
  ```rust
  fn new (config : NotifierConfig) -> Self
  fn notify (& self , event : NotificationEvent)
  async fn flush (& self)
  fn is_active (& self) -> bool
  ```

### `src/queue.rs` (Role: general, Lines: 309)
- **Responsibility**: Core general logic in src/queue.rs
- **Imports**: use crate :: seedr :: { extract_magnet_name , ListContentsResponse , SeedrClient , SeedrFolder , SeedrTorrent , } , use anyhow :: Result , use std :: time :: Duration , use tokio :: time :: Instant 
- **Types & Enums**:
  ```rust
  pub struct CachePolicy
  pub enum CacheOutcome
  pub struct Space
  pub struct EnsureOpts
  ```
- **Public Functions & Signatures**:
  ```rust
  fn from_list (list : & ListContentsResponse) -> Self
  const fn free (& self) -> u64
  async fn space (& self) -> Result < Space >
  async fn find_cached_folder (& self , name : & str) -> Result < Option < SeedrFolder > >
  async fn ensure_cached (& self , magnet : & str , opts : & EnsureOpts < '_ >) -> Result < CacheOutcome >
  async fn purge_folders (& self , ids : & [u64]) -> Result < usize >
  ```

### `src/seedr/tests.rs` (Role: general, Lines: 38)
- **Responsibility**: Core general logic in src/seedr/tests.rs
- **Imports**: use super :: * 

### `src/seedr.rs` (Role: general, Lines: 398)
- **Responsibility**: Core general logic in src/seedr.rs
- **Imports**: use crate :: config :: Auth , use anyhow :: { bail , Context , Result } , use colored :: Colorize , use indicatif :: { ProgressBar , ProgressStyle } , use reqwest :: Client , use serde :: { Deserialize , Serialize } , use std :: net :: SocketAddr , use std :: time :: Duration 
- **Types & Enums**:
  ```rust
  pub struct SeedrTorrent
  pub struct SeedrFolder
  pub struct SeedrFile
  pub struct ListContentsResponse
  pub struct CachingQuery
  pub struct SeedrClient
  ```
- **Public Functions & Signatures**:
  ```rust
  fn new (token : String) -> Self
  async fn login (email : & str , pass : & str) -> Result < Auth >
  async fn add_magnet (& self , magnet : & str) -> Result < u64 >
  async fn wait_for_caching (& self , query : & CachingQuery < '_ > , mut on_progress : Option < & mut (dyn FnMut (& SeedrTorrent) + Send) > ,) -> Result < Option < SeedrFolder > >
  async fn list_root (& self) -> Result < ListContentsResponse >
  async fn list_folder (& self , folder_id : u64) -> Result < ListContentsResponse >
  async fn get_download_url (& self , file_id : u64) -> Result < String >
  async fn delete_folder (& self , folder_id : u64) -> Result < () >
  async fn delete_torrent (& self , torrent_id : u64) -> Result < () >
  async fn delete_all_folders (& self , folders : & [SeedrFolder]) -> Result < () >
  fn extract_magnet_name (magnet_or_url : & str) -> Option < String >
  fn extract_btih_hash (magnet : & str) -> Option < String >
  ```

### `src/task.rs` (Role: general, Lines: 216)
- **Responsibility**: Core general logic in src/task.rs
- **Imports**: use anyhow :: Result , use colored :: Colorize , use seedr_dl :: config :: cache_dir , use serde :: { Deserialize , Serialize } , use std :: fs , use std :: path :: PathBuf 
- **Types & Enums**:
  ```rust
  pub enum TaskStatus
  pub struct TaskState
  ```
- **Public Functions & Signatures**:
  ```rust
  fn tasks_dir () -> PathBuf
  fn list_active_tasks () -> Vec < TaskState >
  fn load_task (folder_id : u64) -> Option < TaskState >
  fn save_task (task : & TaskState) -> Result < () >
  fn remove_task (folder_id : u64)
  fn cancel_task (folder_id : u64)
  fn is_process_alive (pid : u32) -> bool
  fn register_caching_task (torrent_id : u64 , name : & str)
  fn update_caching_progress (torrent_id : u64 , torrent : & seedr_dl :: SeedrTorrent)
  fn print_active_tasks (json : bool)
  ```

### `src/transfer.rs` (Role: general, Lines: 313)
- **Responsibility**: Core general logic in src/transfer.rs
- **Imports**: use crate :: config :: { downloads_dir , Config } , use crate :: downloader :: Downloader , use crate :: history :: { add_history_entry , HistoryEntry } , use crate :: notifier :: { NotificationEvent , Notifier } , use crate :: seedr :: { SeedrClient , SeedrFile , SeedrFolder } , use anyhow :: { bail , Context , Result } , use colored :: Colorize , use indicatif :: { ProgressBar , ProgressStyle } , use std :: fs , use std :: path :: { Path , PathBuf } 
- **Types & Enums**:
  ```rust
  pub struct CloudFileItem
  pub struct TransferOptions
  pub struct ItemDownloadCtx
  ```
- **Public Functions & Signatures**:
  ```rust
  async fn collect_folder_files (client : & SeedrClient , folder_id : u64 ,) -> Result < Vec < CloudFileItem > >
  fn resolve_destination (output_dir : & Path , folder_name : & str , item : & CloudFileItem , is_multi_file : bool ,) -> PathBuf
  async fn download_folder (client : & SeedrClient , cfg : & Config , folder : & SeedrFolder , opts : & TransferOptions < '_ > ,) -> Result < Vec < PathBuf > >
  ```

### `src/worker.rs` (Role: general, Lines: 387)
- **Responsibility**: Core general logic in src/worker.rs
- **Imports**: use crate :: task :: { remove_task , save_task , TaskState , TaskStatus } , use anyhow :: { Context , Result } , use seedr_dl :: config :: { cache_dir , downloads_dir , load_auth , load_config } , use seedr_dl :: downloader :: Downloader , use seedr_dl :: history :: { add_history_entry , HistoryEntry } , use seedr_dl :: notifier :: { NotificationEvent , Notifier , NotifierConfig } , use seedr_dl :: seedr :: SeedrClient , use std :: fs :: { self , OpenOptions } , use std :: path :: { Path , PathBuf } , use std :: process :: { Command , Stdio } , use std :: sync :: { Arc , Mutex } , use std :: time :: Duration 
- **Types & Enums**:
  ```rust
  pub struct WorkerSpawnOpts
  ```
- **Public Functions & Signatures**:
  ```rust
  fn spawn_worker (opts : & WorkerSpawnOpts < '_ >) -> Result < () >
  async fn run_worker (folder_id : u64 , callback_url : Option < String > , output_dir : Option < PathBuf > ,) -> Result < () >
  ```

## 4. Execution Lifecycle Trace
1. **Startup**: Entrypoint parses CLI flags & dispatches command.
2. **Execution**: Core domain logic processes inputs and evaluates rules.
3. **Persistence / I/O**: Domain logic calls infra for disk/terminal I/O.
4. **Exit**: Graceful termination with standard exit codes.

## 5. Verification Commands
```bash
cargo build --release --target x86_64-unknown-linux-gnu
cargo test --all-targets
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```
