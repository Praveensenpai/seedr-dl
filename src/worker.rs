//! Background download worker process execution.

use crate::task::{remove_task, save_task, TaskState, TaskStatus};
use anyhow::{Context, Result};
use seedr_dl::config::{cache_dir, downloads_dir, load_auth, load_config};
use seedr_dl::downloader::Downloader;
use seedr_dl::history::{add_history_entry, HistoryEntry};
use seedr_dl::notifier::{NotificationEvent, Notifier, NotifierConfig};
use seedr_dl::seedr::SeedrClient;
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

async fn resolve_worker_folder_name(
    client: &SeedrClient,
    folder_id: u64,
    first_file: Option<&str>,
) -> String {
    let existing_task = crate::task::load_task(folder_id);
    let mut name = existing_task
        .as_ref()
        .map(|t| t.folder_name.clone())
        .unwrap_or_default();

    if name.is_empty() || name.starts_with("folder-") {
        if let Ok(root) = client.list_root().await {
            if let Some(f) = root.folders.iter().find(|f| f.id == folder_id) {
                name = f.name.clone();
            }
        }
    }
    if (name.is_empty() || name.starts_with("folder-")) && first_file.is_some() {
        if let Some(first) = first_file {
            name = first.to_string();
        }
    }
    if name.is_empty() {
        format!("folder-{folder_id}")
    } else {
        name
    }
}

struct WorkerBatchCtx<'a> {
    client: &'a SeedrClient,
    folder_id: u64,
    folder_name: &'a str,
    final_dir: &'a Path,
    temp_dir: &'a Path,
    task: &'a Arc<Mutex<TaskState>>,
    notifier: &'a Notifier,
    is_multi: bool,
    total_bytes: u64,
}

async fn download_worker_items(
    downloader: &Downloader,
    items: &[seedr_dl::transfer::CloudFileItem],
    ctx: &WorkerBatchCtx<'_>,
) -> Result<PathBuf> {
    let mut prior_bytes: u64 = 0;
    let mut last_dest = PathBuf::new();
    let total_files = items.len();

    for (idx, item) in items.iter().enumerate() {
        let dest_path = seedr_dl::transfer::resolve_destination(
            ctx.final_dir,
            ctx.folder_name,
            item,
            ctx.is_multi,
        );
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let exec_ctx = WorkerExecCtx {
            task: ctx.task,
            notifier: ctx.notifier,
            temp_dir: ctx.temp_dir,
            folder_id: ctx.folder_id,
            folder_name: ctx.folder_name,
            current_num: idx + 1,
            total_files,
            prior_bytes,
            total_bytes: ctx.total_bytes,
        };

        let temp_downloaded = execute_worker_file(ctx.client, downloader, item, &exec_ctx).await?;

        fs::rename(&temp_downloaded, &dest_path).or_else(|_| {
            fs::copy(&temp_downloaded, &dest_path)?;
            let _ = fs::remove_file(&temp_downloaded);
            Ok::<(), std::io::Error>(())
        })?;

        let _ = add_history_entry(HistoryEntry {
            id: seedr_dl::history::generate_id(),
            original_name: item.file.name.clone(),
            file_path: dest_path.clone(),
            file_size: item.file.size,
            downloaded_at: chrono::Local::now().to_rfc3339(),
        });

        prior_bytes += item.file.size;
        last_dest = dest_path;
    }
    Ok(last_dest)
}

async fn finalize_worker_completion(ctx: &WorkerBatchCtx<'_>, last_dest: &Path) -> Result<()> {
    let completion_path = if ctx.is_multi {
        ctx.final_dir.join(ctx.folder_name)
    } else {
        last_dest.to_path_buf()
    };

    let completion_name = if ctx.is_multi {
        ctx.folder_name.to_string()
    } else {
        last_dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(ctx.folder_name)
            .to_string()
    };

    ctx.notifier.notify(NotificationEvent::Completed {
        folder_id: ctx.folder_id,
        file_name: completion_name,
        destination_path: Some(completion_path.to_string_lossy().to_string()),
        total_bytes: ctx.total_bytes,
    });

    let _ = ctx.client.delete_folder(ctx.folder_id).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    remove_task(ctx.folder_id);
    Ok(())
}

async fn run_worker_internal(
    folder_id: u64,
    output_dir: Option<&Path>,
    notifier: &Notifier,
) -> Result<()> {
    let cfg = load_config()?;
    let auth = load_auth()?.context("No auth found. Please run 'seedr-dl auth'")?;
    let client = SeedrClient::new(auth.access_token);

    let items = seedr_dl::transfer::collect_folder_files(&client, folder_id).await?;
    if items.is_empty() {
        anyhow::bail!("No files found in folder {folder_id}");
    }

    let folder_name = resolve_worker_folder_name(
        &client,
        folder_id,
        items.first().map(|i| i.file.name.as_str()),
    )
    .await;

    let is_multi = items.len() > 1
        || items
            .iter()
            .any(|i| i.relative_path != Path::new(&i.file.name));

    let final_dir = output_dir.unwrap_or(&cfg.download_dir);
    let total_bytes: u64 = items.iter().map(|i| i.file.size).sum();

    let temp_dir = downloads_dir();
    let _ = fs::create_dir_all(&temp_dir);

    let task = init_worker_task(folder_id, &folder_name, total_bytes);
    let downloader = Downloader::new(cfg.download_threads);

    let batch_ctx = WorkerBatchCtx {
        client: &client,
        folder_id,
        folder_name: &folder_name,
        final_dir,
        temp_dir: &temp_dir,
        task: &task,
        notifier,
        is_multi,
        total_bytes,
    };

    let last_dest = download_worker_items(&downloader, &items, &batch_ctx).await?;

    if let Ok(mut t) = task.lock() {
        t.status = TaskStatus::Completed;
        t.downloaded_bytes = total_bytes;
        let _ = save_task(&t);
    }

    finalize_worker_completion(&batch_ctx, &last_dest).await
}

fn init_worker_task(folder_id: u64, folder_name: &str, total_bytes: u64) -> Arc<Mutex<TaskState>> {
    let task = Arc::new(Mutex::new(TaskState {
        folder_id,
        pid: std::process::id(),
        folder_name: folder_name.to_string(),
        file_name: folder_name.to_string(),
        downloaded_bytes: 0,
        total_bytes,
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
struct WorkerExecCtx<'a> {
    task: &'a Arc<Mutex<TaskState>>,
    notifier: &'a Notifier,
    temp_dir: &'a Path,
    folder_id: u64,
    folder_name: &'a str,
    current_num: usize,
    total_files: usize,
    prior_bytes: u64,
    total_bytes: u64,
}

async fn execute_worker_file(
    client: &SeedrClient,
    downloader: &Downloader,
    item: &seedr_dl::transfer::CloudFileItem,
    ctx: &WorkerExecCtx<'_>,
) -> Result<PathBuf> {
    let file = &item.file;
    let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
    let download_url = client.get_download_url(file_id).await?;
    let file_label = if ctx.total_files > 1 {
        format!(
            "{} ({}/{}: {})",
            ctx.folder_name, ctx.current_num, ctx.total_files, file.name
        )
    } else {
        file.name.clone()
    };

    let task_cb = Arc::clone(ctx.task);
    let notifier_clone = ctx.notifier.clone();
    let folder_id = ctx.folder_id;
    let label_clone = file_label;
    let prior_bytes = ctx.prior_bytes;
    let total_bytes = ctx.total_bytes;

    downloader
        .download_with_callback(
            &download_url,
            ctx.temp_dir,
            &file.name,
            move |downloaded, _total, speed, eta| {
                let cumulative_dl = prior_bytes + downloaded;
                if let Ok(mut t) = task_cb.lock() {
                    t.file_name.clone_from(&label_clone);
                    t.downloaded_bytes = cumulative_dl;
                    t.total_bytes = total_bytes;
                    t.speed_bps = speed;
                    t.eta_seconds = eta;
                    let _ = save_task(&t);
                }
                #[allow(clippy::cast_precision_loss)]
                let percentage = if total_bytes > 0 {
                    (cumulative_dl as f64 / total_bytes as f64) * 100.0
                } else {
                    0.0
                };
                notifier_clone.notify(NotificationEvent::Progress {
                    folder_id,
                    file_name: label_clone.clone(),
                    downloaded_bytes: cumulative_dl,
                    total_bytes,
                    speed_bps: speed,
                    eta_seconds: eta,
                    percentage,
                });
            },
        )
        .await
}
