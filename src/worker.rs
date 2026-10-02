//! Background download worker process execution.

use crate::task::{remove_task, save_task, TaskState, TaskStatus};
use anyhow::{Context, Result};
use seedr_dl::config::{cache_dir, downloads_dir, load_auth, load_config, Config};
use seedr_dl::downloader::state::DownloadState;
use seedr_dl::downloader::Downloader;
use seedr_dl::history::{add_history_entry, HistoryEntry};
use seedr_dl::notifier::{NotificationEvent, Notifier, NotifierConfig};
use seedr_dl::seedr::{SeedrClient, SeedrFile};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Configuration for launching a background worker.
pub struct WorkerSpawnOpts<'a> {
    pub folder_id: u64,
    pub folder_name: &'a str,
    pub file_size: u64,
    pub callback_url: Option<&'a str>,
    pub output_dir: Option<&'a Path>,
}

/// Spawns a background worker process for asynchronous downloading.
///
/// # Errors
/// Returns an error if the process cannot be spawned or the log file cannot be created.
pub fn spawn_worker(opts: &WorkerSpawnOpts<'_>) -> Result<()> {
    let exe = std::env::current_exe().context("Could not get current executable")?;
    let log_dir = cache_dir().join("logs");
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{}.log", opts.folder_id));
    let log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .context("Failed to open log file")?;

    let stdout_stdio = log_file
        .try_clone()
        .map_or_else(|_| Stdio::null(), Stdio::from);

    let mut args = vec!["worker".to_string(), opts.folder_id.to_string()];
    if let Some(url) = opts.callback_url {
        args.push("--callback-url".to_string());
        args.push(url.to_string());
    }
    if let Some(dir) = opts.output_dir {
        args.push("--output-dir".to_string());
        args.push(dir.to_string_lossy().to_string());
    }

    let child = Command::new(exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(stdout_stdio)
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("Failed to spawn background download worker")?;

    let task = TaskState {
        folder_id: opts.folder_id,
        pid: child.id(),
        folder_name: opts.folder_name.to_string(),
        file_name: opts.folder_name.to_string(),
        downloaded_bytes: 0,
        total_bytes: opts.file_size,
        speed_bps: 0,
        eta_seconds: 0,
        status: TaskStatus::Downloading,
        error: None,
    };
    save_task(&task)?;
    Ok(())
}

/// Runs background worker execution, notifying on error or completion.
///
/// # Errors
/// Returns an error if the download fails.
pub async fn run_worker(
    folder_id: u64,
    callback_url: Option<String>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    let notifier = Notifier::new(NotifierConfig {
        callback_url,
        json_output: false,
        callback_fn: None,
        progress_interval_ms: 500,
    });

    match run_worker_internal(folder_id, output_dir.as_deref(), &notifier).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("Worker failed for folder {folder_id}: {e:#}");
            notifier.notify(NotificationEvent::Failed {
                folder_id,
                file_name: None,
                error: format!("{e:#}"),
            });
            if let Some(mut t) = crate::task::load_task(folder_id) {
                t.status = TaskStatus::Failed;
                t.error = Some(format!("{e:#}"));
                let _ = save_task(&t);
            }
            Err(e)
        }
    }
}

async fn run_worker_internal(
    folder_id: u64,
    output_dir: Option<&Path>,
    notifier: &Notifier,
) -> Result<()> {
    let cfg = load_config()?;
    let auth = load_auth()?.context("No auth found. Please run 'seedr-dl auth'")?;
    let client = SeedrClient::new(auth.access_token);

    let folder_resp = client.list_folder(folder_id).await?;
    let file = folder_resp
        .files
        .first()
        .context("No files found in folder")?;

    let temp_dir = downloads_dir();
    let _ = fs::create_dir_all(&temp_dir);
    let task = init_worker_task(folder_id, file, &temp_dir);

    let downloaded_path = execute_worker_download(&client, &cfg, file, &task, notifier).await?;

    let final_dir = output_dir.unwrap_or(&cfg.download_dir);
    fs::create_dir_all(final_dir)?;
    let dest_path = final_dir.join(&file.name);

    fs::rename(&downloaded_path, &dest_path).or_else(|_| {
        fs::copy(&downloaded_path, &dest_path)?;
        let _ = fs::remove_file(&downloaded_path);
        Ok::<(), std::io::Error>(())
    })?;

    if let Ok(mut t) = task.lock() {
        t.status = TaskStatus::Completed;
        let _ = save_task(&t);
    }

    notifier.notify(NotificationEvent::Completed {
        folder_id,
        file_name: file.name.clone(),
        destination_path: Some(dest_path.to_string_lossy().to_string()),
        total_bytes: file.size,
    });

    let _ = add_history_entry(HistoryEntry {
        id: seedr_dl::history::generate_id(),
        original_name: file.name.clone(),
        file_path: dest_path,
        file_size: file.size,
        downloaded_at: chrono::Local::now().to_rfc3339(),
    });

    let _ = client.delete_folder(folder_id).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    remove_task(folder_id);

    Ok(())
}

fn init_worker_task(folder_id: u64, file: &SeedrFile, temp_dir: &Path) -> Arc<Mutex<TaskState>> {
    let state_path = DownloadState::state_path(temp_dir, &file.name);
    let initial_dl = if let Ok(content) = fs::read_to_string(&state_path) {
        serde_json::from_str::<DownloadState>(&content).map_or(0, |s| s.total_downloaded())
    } else {
        0
    };

    let task = Arc::new(Mutex::new(TaskState {
        folder_id,
        pid: std::process::id(),
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
    task
}

async fn execute_worker_download(
    client: &SeedrClient,
    cfg: &Config,
    file: &SeedrFile,
    task: &Arc<Mutex<TaskState>>,
    notifier: &Notifier,
) -> Result<PathBuf> {
    let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
    let download_url = client.get_download_url(file_id).await?;
    let downloader = Downloader::new(cfg.download_threads);
    let temp_dir = downloads_dir();

    let task_cb = Arc::clone(task);
    let notifier_clone = notifier.clone();
    let file_name = file.name.clone();
    let folder_id = if let Ok(guard) = task.lock() {
        guard.folder_id
    } else {
        0
    };

    downloader
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
                // reason: byte values safely cast to f64 for ratio calculation
                #[allow(clippy::cast_precision_loss)]
                let percentage = if total > 0 {
                    (downloaded as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                notifier_clone.notify(NotificationEvent::Progress {
                    folder_id,
                    file_name: file_name.clone(),
                    downloaded_bytes: downloaded,
                    total_bytes: total,
                    speed_bps: speed,
                    eta_seconds: eta,
                    percentage,
                });
            },
        )
        .await
}
