use crate::config::{cache_dir, downloads_dir, get_gemini_key, load_auth, load_config};
use crate::downloader::state::DownloadState;
use crate::downloader::Downloader;
use crate::gemini;
use crate::organizer;
use crate::seedr::SeedrClient;
use crate::task::{remove_task, save_task, TaskState, TaskStatus};
use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn spawn_worker(folder_id: u64, folder_name: &str, file_size: u64) -> Result<()> {
    let exe = std::env::current_exe().context("Could not get current executable")?;
    let log_dir = cache_dir().join("logs");
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{folder_id}.log"));
    let log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .context("Failed to open log file")?;

    let stdout_stdio = log_file
        .try_clone()
        .map_or_else(|_| Stdio::null(), Stdio::from);

    let child = Command::new(exe)
        .args(["worker", &folder_id.to_string()])
        .stdin(Stdio::null())
        .stdout(stdout_stdio)
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("Failed to spawn background download worker")?;

    let task = TaskState {
        folder_id,
        pid: child.id(),
        folder_name: folder_name.to_string(),
        file_name: folder_name.to_string(),
        downloaded_bytes: 0,
        total_bytes: file_size,
        speed_bps: 0,
        eta_seconds: 0,
        status: TaskStatus::Downloading,
        error: None,
    };
    save_task(&task)?;
    Ok(())
}

pub async fn run_worker(folder_id: u64) -> Result<()> {
    match run_worker_internal(folder_id).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("Worker failed for folder {folder_id}: {e:#}");
            if let Some(mut t) = crate::task::load_task(folder_id) {
                t.status = TaskStatus::Failed;
                t.error = Some(format!("{e:#}"));
                let _ = save_task(&t);
            }
            Err(e)
        }
    }
}

async fn run_worker_internal(folder_id: u64) -> Result<()> {
    let cfg = load_config()?;
    let auth = load_auth()?.context("No auth found. Please run 'seedr-dl auth'")?;
    let client = SeedrClient::new(auth.access_token);

    let folder_resp = client.list_folder(folder_id).await?;
    let file = folder_resp
        .files
        .first()
        .context("No files found in folder")?;

    let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
    let pid = std::process::id();

    let temp_dir = downloads_dir();
    let _ = fs::create_dir_all(&temp_dir);
    let state_path = DownloadState::state_path(&temp_dir, &file.name);
    let initial_dl = if let Ok(content) = fs::read_to_string(&state_path) {
        serde_json::from_str::<DownloadState>(&content).map_or(0, |s| s.total_downloaded())
    } else {
        0
    };

    let task = Arc::new(Mutex::new(TaskState {
        folder_id,
        pid,
        folder_name: file.name.clone(),
        file_name: file.name.clone(),
        downloaded_bytes: initial_dl,
        total_bytes: file.size,
        speed_bps: 0,
        eta_seconds: 0,
        status: TaskStatus::Downloading,
        error: None,
    }));

    if let Ok(guard) = task.lock() {
        let _ = save_task(&guard);
    }

    let download_url = client.get_download_url(file_id).await?;
    let downloader = Downloader::new(cfg.download_threads);

    let task_cb = Arc::clone(&task);
    let downloaded_path = downloader
        .download_with_callback(
            &download_url,
            &temp_dir,
            &file.name,
            move |downloaded, total, speed, eta| {
                if let Ok(mut t) = task_cb.lock() {
                    t.downloaded_bytes = downloaded;
                    t.total_bytes = total;
                    t.speed_bps = speed;
                    t.eta_seconds = eta;
                    let _ = save_task(&t);
                }
            },
        )
        .await?;

    if let Ok(mut t) = task.lock() {
        t.status = TaskStatus::Organizing;
        let _ = save_task(&t);
    }

    let gemini_key = get_gemini_key(&cfg);
    let (info, status) = gemini::parse_media(&file.name, gemini_key.as_deref()).await;
    let target = organizer::IngestTarget {
        original_name: &file.name,
        rename_status: status,
        non_interactive: true,
    };

    organizer::organize_file(&downloaded_path, &info, &cfg.jellyfin_media_dir, &target)?;

    if let Ok(mut t) = task.lock() {
        t.status = TaskStatus::Completed;
        let _ = save_task(&t);
    }

    let _ = client.delete_folder(folder_id).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    remove_task(folder_id);

    Ok(())
}
