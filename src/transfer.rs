//! Direct file download transfer pipeline.

use crate::config::{downloads_dir, Config};
use crate::downloader::Downloader;
use crate::history::{add_history_entry, HistoryEntry};
use crate::notifier::{NotificationEvent, Notifier};
use crate::seedr::{SeedrClient, SeedrFile, SeedrFolder};
use anyhow::{bail, Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};

/// Options controlling file download transfer execution.
#[derive(Clone)]
pub struct TransferOptions<'a> {
    /// Target directory where downloaded files will be placed.
    pub output_dir: PathBuf,
    /// Automatically delete item from Seedr cloud after download.
    pub delete_cloud: bool,
    /// Optional event notifier for webhook/progress dispatch.
    pub notifier: Option<&'a Notifier>,
}

/// Downloads all files in a Seedr folder and saves them directly to the output directory.
///
/// # Errors
/// Returns an error if listing files, downloading, or file moving fails.
pub async fn download_folder(
    client: &SeedrClient,
    cfg: &Config,
    folder: &SeedrFolder,
    opts: &TransferOptions<'_>,
) -> Result<Vec<PathBuf>> {
    let contents = client.list_folder(folder.id).await?;
    if contents.files.is_empty() {
        let msg = format!("No files found inside cloud folder '{}'", folder.name);
        if let Some(n) = opts.notifier {
            n.notify(NotificationEvent::Failed {
                folder_id: folder.id,
                file_name: None,
                error: msg.clone(),
            });
        }
        bail!(msg);
    }

    fs::create_dir_all(&opts.output_dir)?;
    let mut results = Vec::new();
    let temp_dir = downloads_dir();
    let downloader = Downloader::new(cfg.download_threads);

    for file in &contents.files {
        let res = download_single_file(client, folder.id, file, &downloader, &temp_dir, opts).await?;
        results.push(res);
    }

    if opts.delete_cloud {
        let _ = client.delete_folder(folder.id).await;
    }

    Ok(results)
}

async fn download_single_file(
    client: &SeedrClient,
    folder_id: u64,
    file: &SeedrFile,
    downloader: &Downloader,
    temp_dir: &Path,
    opts: &TransferOptions<'_>,
) -> Result<PathBuf> {
    let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
    let download_url = client.get_download_url(file_id).await?;

    if let Some(n) = opts.notifier {
        n.notify(NotificationEvent::Status {
            folder_id,
            file_name: Some(file.name.clone()),
            status: "downloading".into(),
            message: format!("Starting download of {}", file.name),
        });
    }

    let pb = if opts.notifier.is_some_and(Notifier::is_active) {
        ProgressBar::hidden()
    } else {
        create_progress_bar(file.size, &file.name)
    };

    let pb_clone = pb.clone();
    let notifier_clone = opts.notifier.cloned();
    let file_name_clone = file.name.clone();

    let downloaded_temp = downloader
        .download_with_callback(
            &download_url,
            temp_dir,
            &file.name,
            move |dl, tot, spd, eta| {
                if tot > 0 {
                    pb_clone.set_length(tot);
                }
                pb_clone.set_position(dl);
                if let Some(ref n) = notifier_clone {
                    // reason: byte values safely cast to f64 for ratio calculation
                    #[allow(clippy::cast_precision_loss)]
                    let percentage = if tot > 0 {
                        (dl as f64 / tot as f64) * 100.0
                    } else {
                        0.0
                    };
                    n.notify(NotificationEvent::Progress {
                        folder_id,
                        file_name: file_name_clone.clone(),
                        downloaded_bytes: dl,
                        total_bytes: tot,
                        speed_bps: spd,
                        eta_seconds: eta,
                        percentage,
                    });
                }
            },
        )
        .await?;

    pb.finish_with_message(format!("{} Download complete!", "✔".green().bold()));

    let dest_path = opts.output_dir.join(&file.name);
    fs::rename(&downloaded_temp, &dest_path).or_else(|_| {
        fs::copy(&downloaded_temp, &dest_path)?;
        let _ = fs::remove_file(&downloaded_temp);
        Ok::<(), std::io::Error>(())
    })?;

    if let Some(n) = opts.notifier {
        n.notify(NotificationEvent::Completed {
            folder_id,
            file_name: file.name.clone(),
            destination_path: Some(dest_path.to_string_lossy().to_string()),
            total_bytes: file.size,
        });
    }

    let _ = add_history_entry(HistoryEntry {
        id: crate::history::generate_id(),
        original_name: file.name.clone(),
        file_path: dest_path.clone(),
        file_size: file.size,
        downloaded_at: chrono::Local::now().to_rfc3339(),
    });

    Ok(dest_path)
}

fn create_progress_bar(size: u64, name: &str) -> ProgressBar {
    let pb = ProgressBar::new(size);
    let style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("#>-");
    pb.set_style(style);
    pb.set_message(format!("{} {}", "Downloading".cyan(), name));
    pb
}
