//! Background download task persistence and process lifecycle tracking.

use anyhow::Result;
use colored::Colorize;
use seedr_dl::config::cache_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Status of an asynchronous ingestion task.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    /// Caching torrent into Seedr cloud.
    Caching,
    /// In progress downloading chunks.
    Downloading,
    /// Organizing media into Jellyfin library.
    Organizing,
    /// Ingestion finished successfully.
    Completed,
    /// Encountered fatal error.
    Failed,
}

/// Persistent record of an active background ingestion worker.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskState {
    /// Target cloud folder ID.
    pub folder_id: u64,
    /// Worker process identifier.
    pub pid: u32,
    /// Folder display name.
    pub folder_name: String,
    /// File display name.
    pub file_name: String,
    /// Bytes downloaded.
    pub downloaded_bytes: u64,
    /// Total bytes.
    pub total_bytes: u64,
    /// Speed in bytes per second.
    pub speed_bps: u64,
    /// Estimated time to complete in seconds.
    pub eta_seconds: u64,
    /// Current task status.
    pub status: TaskStatus,
    /// Error description if failed.
    pub error: Option<String>,
}

/// Returns the cache path storing task states.
#[must_use]
pub fn tasks_dir() -> PathBuf {
    cache_dir().join("tasks")
}

/// Lists all active background download tasks.
#[must_use]
pub fn list_active_tasks() -> Vec<TaskState> {
    let dir = tasks_dir();
    if !dir.exists() {
        return Vec::new();
    }

    let mut tasks = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(task) = serde_json::from_str::<TaskState>(&content) {
                        if is_process_alive(task.pid) {
                            tasks.push(task);
                        } else {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }
    tasks.sort_by_key(|a| a.folder_id);
    tasks
}

/// Loads a task state by folder ID if it exists.
#[must_use]
pub fn load_task(folder_id: u64) -> Option<TaskState> {
    let file_path = tasks_dir().join(format!("{folder_id}.json"));
    let content = fs::read_to_string(file_path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Saves the given task state to disk.
///
/// # Errors
/// Returns an error if writing to disk fails.
pub fn save_task(task: &TaskState) -> Result<()> {
    let dir = tasks_dir();
    fs::create_dir_all(&dir)?;
    let file_path = dir.join(format!("{}.json", task.folder_id));
    let json = serde_json::to_string_pretty(task)?;
    fs::write(file_path, json)?;
    Ok(())
}

/// Deletes a task record from disk.
pub fn remove_task(folder_id: u64) {
    let file_path = tasks_dir().join(format!("{folder_id}.json"));
    let _ = fs::remove_file(file_path);
}

/// Cancels a running background download task by killing its process.
pub fn cancel_task(folder_id: u64) {
    if let Some(task) = load_task(folder_id) {
        let pid_str = task.pid.to_string();
        let _ = std::process::Command::new("kill").arg(&pid_str).status();
        remove_task(folder_id);
    }
}

/// Checks if a process with the given PID is currently active.
#[must_use]
pub fn is_process_alive(pid: u32) -> bool {
    let proc_path = format!("/proc/{pid}");
    std::path::Path::new(&proc_path).exists()
}

/// Registers an initial caching task record on disk.
pub fn register_caching_task(torrent_id: u64, name: &str) {
    let task = TaskState {
        folder_id: torrent_id,
        pid: std::process::id(),
        folder_name: name.to_string(),
        file_name: name.to_string(),
        downloaded_bytes: 0,
        total_bytes: 0,
        speed_bps: 0,
        eta_seconds: 0,
        status: TaskStatus::Caching,
        error: None,
    };
    let _ = save_task(&task);
}

/// Updates progress of an active cloud caching task.
pub fn update_caching_progress(torrent_id: u64, torrent: &seedr_dl::SeedrTorrent) {
    let total = torrent.size.unwrap_or(0);
    let pct = torrent.progress.unwrap_or(0.0);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let downloaded = if total > 0 && pct > 0.0 {
        ((pct / 100.0) * (total as f64)) as u64
    } else {
        0
    };
    let speed = torrent.download_rate.unwrap_or(0);
    let eta = if speed > 0 && total > downloaded {
        (total - downloaded) / speed
    } else {
        0
    };
    let task = TaskState {
        folder_id: torrent_id,
        pid: std::process::id(),
        folder_name: torrent.name.clone(),
        file_name: torrent.name.clone(),
        downloaded_bytes: downloaded,
        total_bytes: total,
        speed_bps: speed,
        eta_seconds: eta,
        status: TaskStatus::Caching,
        error: None,
    };
    let _ = save_task(&task);
}

/// Prints active background ingestion tasks to stdout.
pub fn print_active_tasks(json: bool) {
    let tasks = list_active_tasks();
    if json {
        if let Ok(out) = serde_json::to_string_pretty(&tasks) {
            println!("{out}");
        }
        return;
    }
    if tasks.is_empty() {
        println!("  No active background ingestion tasks.");
        return;
    }
    println!("  {}", "Active Background Ingestion Tasks:".cyan().bold());
    for t in &tasks {
        let dl_mb = t.downloaded_bytes / 1_048_576;
        let tot_mb = t.total_bytes / 1_048_576;
        let (pct_whole, pct_dec) = t
            .downloaded_bytes
            .saturating_mul(1000)
            .checked_div(t.total_bytes)
            .map_or((0, 0), |x10| (x10 / 10, x10 % 10));
        let kbytes_per_sec = t.speed_bps / 1024;
        let (mb_whole, mb_frac) = (kbytes_per_sec / 1024, (kbytes_per_sec % 1024) * 100 / 1024);
        let tag = match t.status {
            TaskStatus::Caching => colored::Colorize::yellow("Caching"),
            TaskStatus::Downloading => colored::Colorize::cyan("Downloading"),
            TaskStatus::Organizing => colored::Colorize::magenta("Organizing"),
            TaskStatus::Completed => colored::Colorize::green("Completed"),
            TaskStatus::Failed => colored::Colorize::red("Failed"),
        };
        println!(
            "  • [{}] [{tag}] {} — {dl_mb}/{tot_mb} MB ({pct_whole}.{pct_dec}%, {mb_whole}.{mb_frac:02} MB/s, ETA {}s)",
            t.folder_id, t.folder_name, t.eta_seconds
        );
    }
}
