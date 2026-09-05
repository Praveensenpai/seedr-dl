use crate::config::{cache_dir, get_gemini_key, load_auth, load_config};
use crate::downloader::Downloader;
use crate::gemini;
use crate::organizer;
use crate::seedr::SeedrClient;
use crate::task::{remove_task, save_task, TaskState, TaskStatus};
use anyhow::{Context, Result};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub fn spawn_worker(folder_id: u64) -> Result<()> {
    let exe = std::env::current_exe().context("Could not get current executable")?;
    Command::new(exe)
        .args(["__worker", &folder_id.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn background download worker")?;
    Ok(())
}

pub async fn run_worker(folder_id: u64) -> Result<()> {
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

    let task = Arc::new(Mutex::new(TaskState {
        folder_id,
        pid,
        folder_name: file.name.clone(),
        file_name: file.name.clone(),
        downloaded_bytes: 0,
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
    let downloader = Downloader::new();
    let temp_dir = cache_dir();

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
    let info = gemini::parse_media(&file.name, gemini_key.as_deref()).await;

    organizer::organize_file(&downloaded_path, &info, &cfg.jellyfin_media_dir, true)?;

    let _ = client.delete_folder(folder_id).await;
    remove_task(folder_id);

    Ok(())
}
