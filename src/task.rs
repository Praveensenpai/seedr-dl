use crate::config::cache_dir;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    Downloading,
    Organizing,
    Completed,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskState {
    pub folder_id: u64,
    pub pid: u32,
    pub folder_name: String,
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub eta_seconds: u64,
    pub status: TaskStatus,
    pub error: Option<String>,
}

pub fn tasks_dir() -> PathBuf {
    cache_dir().join("tasks")
}

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

pub fn load_task(folder_id: u64) -> Option<TaskState> {
    let file_path = tasks_dir().join(format!("{folder_id}.json"));
    let content = fs::read_to_string(file_path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn save_task(task: &TaskState) -> Result<()> {
    let dir = tasks_dir();
    fs::create_dir_all(&dir)?;
    let file_path = dir.join(format!("{}.json", task.folder_id));
    let json = serde_json::to_string_pretty(task)?;
    fs::write(file_path, json)?;
    Ok(())
}

pub fn remove_task(folder_id: u64) {
    let file_path = tasks_dir().join(format!("{folder_id}.json"));
    let _ = fs::remove_file(file_path);
}

pub fn cancel_task(folder_id: u64) {
    if let Some(task) = load_task(folder_id) {
        let pid_str = task.pid.to_string();
        let _ = std::process::Command::new("kill").arg(&pid_str).status();
        remove_task(folder_id);
    }
}

pub fn is_process_alive(pid: u32) -> bool {
    let proc_path = format!("/proc/{pid}");
    std::path::Path::new(&proc_path).exists()
}
