//! Background download task persistence and process lifecycle tracking.

use anyhow::Result;
use seedr_dl::config::cache_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Status of an asynchronous ingestion task.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
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
