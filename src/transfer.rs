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

/// Individual file item within a cloud folder hierarchy.
#[derive(Debug, Clone)]
pub struct CloudFileItem {
    pub file: SeedrFile,
    pub relative_path: PathBuf,
}

/// Recursively discovers all files in a folder and its subfolders.
///
/// # Errors
/// Returns an error if API queries fail.
pub async fn collect_folder_files(
    client: &SeedrClient,
    folder_id: u64,
) -> Result<Vec<CloudFileItem>> {
    let mut items = Vec::new();
    let mut queue = vec![(folder_id, PathBuf::new())];

    while let Some((id, rel_dir)) = queue.pop() {
        let contents = client.list_folder(id).await?;
        for file in contents.files {
            let relative_path = if rel_dir.as_os_str().is_empty() {
                PathBuf::from(&file.name)
            } else {
                rel_dir.join(&file.name)
            };
            items.push(CloudFileItem {
                file,
                relative_path,
            });
        }
        for sub in contents.folders {
            let next_rel = if rel_dir.as_os_str().is_empty() {
                PathBuf::from(&sub.name)
            } else {
                rel_dir.join(&sub.name)
            };
            queue.push((sub.id, next_rel));
        }
    }

    items.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(items)
}

/// Determines the destination path for a file item.
#[must_use]
pub fn resolve_destination(
    output_dir: &Path,
    folder_name: &str,
    item: &CloudFileItem,
    is_multi_file: bool,
) -> PathBuf {
    if is_multi_file {
        output_dir.join(folder_name).join(&item.relative_path)
    } else {
        output_dir.join(&item.file.name)
    }
}

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

/// Context for single item download within a multi-file transfer.
pub struct ItemDownloadCtx<'a> {
    pub folder_id: u64,
    pub downloader: &'a Downloader,
    pub temp_dir: &'a Path,
    pub dest_path: &'a Path,
    pub current_num: usize,
    pub total_files: usize,
    pub prior_bytes: u64,
    pub total_bytes: u64,
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
    let items = collect_folder_files(client, folder.id).await?;
    if items.is_empty() {
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

    let is_multi = items.len() > 1
        || items
            .iter()
            .any(|i| i.relative_path != Path::new(&i.file.name));

    fs::create_dir_all(&opts.output_dir)?;
    let mut results = Vec::new();
    let temp_dir = downloads_dir();
    let downloader = Downloader::new(cfg.download_threads);

    let total_bytes: u64 = items.iter().map(|i| i.file.size).sum();
    let mut prior_bytes: u64 = 0;
    let total_files = items.len();

    for (idx, item) in items.iter().enumerate() {
        let current_num = idx + 1;
        let dest_path = resolve_destination(&opts.output_dir, &folder.name, item, is_multi);
        let ctx = ItemDownloadCtx {
            folder_id: folder.id,
            downloader: &downloader,
            temp_dir: &temp_dir,
            dest_path: &dest_path,
            current_num,
            total_files,
            prior_bytes,
            total_bytes,
        };

        let res = download_single_item(client, item, &ctx, opts).await?;
        results.push(res);
        prior_bytes += item.file.size;
    }

    if let Some(n) = opts.notifier {
        let final_path = if is_multi {
            opts.output_dir.join(&folder.name)
        } else {
            results
                .first()
                .cloned()
                .unwrap_or_else(|| opts.output_dir.clone())
        };
        n.notify(NotificationEvent::Completed {
            folder_id: folder.id,
            file_name: folder.name.clone(),
            destination_path: Some(final_path.to_string_lossy().to_string()),
            total_bytes,
        });
    }

    if opts.delete_cloud {
        let _ = client.delete_folder(folder.id).await;
    }

    Ok(results)
}

async fn download_single_item(
    client: &SeedrClient,
    item: &CloudFileItem,
    ctx: &ItemDownloadCtx<'_>,
    opts: &TransferOptions<'_>,
) -> Result<PathBuf> {
    let file = &item.file;
    let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
    let download_url = client.get_download_url(file_id).await?;

    let file_label = if ctx.total_files > 1 {
        format!("{}/{} {}", ctx.current_num, ctx.total_files, file.name)
    } else {
        file.name.clone()
    };

    if let Some(n) = opts.notifier {
        n.notify(NotificationEvent::Status {
            folder_id: ctx.folder_id,
            file_name: Some(file_label.clone()),
            status: "downloading".into(),
            message: format!("Downloading {file_label}"),
        });
    }

    let pb = if opts.notifier.is_some_and(Notifier::is_active) {
        ProgressBar::hidden()
    } else {
        create_progress_bar(file.size, &file_label)
    };

    let pb_clone = pb.clone();
    let notifier_clone = opts.notifier.cloned();
    let folder_id = ctx.folder_id;
    let file_name_clone = file_label;

    let prior_bytes = ctx.prior_bytes;
    let total_bytes = ctx.total_bytes;

    let downloaded_temp = ctx
        .downloader
        .download_with_callback(
            &download_url,
            ctx.temp_dir,
            &file.name,
            move |dl, _tot, spd, eta| {
                pb_clone.set_position(dl);
                if let Some(ref n) = notifier_clone {
                    let cumulative_dl = prior_bytes + dl;
                    #[allow(clippy::cast_precision_loss)]
                    let percentage = if total_bytes > 0 {
                        (cumulative_dl as f64 / total_bytes as f64) * 100.0
                    } else {
                        0.0
                    };
                    n.notify(NotificationEvent::Progress {
                        folder_id,
                        file_name: file_name_clone.clone(),
                        downloaded_bytes: cumulative_dl,
                        total_bytes,
                        speed_bps: spd,
                        eta_seconds: eta,
                        percentage,
                    });
                }
            },
        )
        .await?;

    pb.finish_with_message(format!("{} Download complete!", "✔".green().bold()));

    if let Some(parent) = ctx.dest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::rename(&downloaded_temp, ctx.dest_path).or_else(|_| {
        fs::copy(&downloaded_temp, ctx.dest_path)?;
        let _ = fs::remove_file(&downloaded_temp);
        Ok::<(), std::io::Error>(())
    })?;

    let _ = add_history_entry(HistoryEntry {
        id: crate::history::generate_id(),
        original_name: file.name.clone(),
        file_path: ctx.dest_path.to_path_buf(),
        file_size: file.size,
        downloaded_at: chrono::Local::now().to_rfc3339(),
    });

    Ok(ctx.dest_path.to_path_buf())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_destination_single_file() {
        let item = CloudFileItem {
            file: SeedrFile {
                id: Some(1),
                folder_file_id: Some(1),
                name: "movie.mkv".to_string(),
                size: 100,
            },
            relative_path: PathBuf::from("movie.mkv"),
        };
        let dest = resolve_destination(Path::new("/downloads"), "movie", &item, false);
        assert_eq!(dest, PathBuf::from("/downloads/movie.mkv"));
    }

    #[test]
    fn test_resolve_destination_series_subfolder() {
        let item = CloudFileItem {
            file: SeedrFile {
                id: Some(2),
                folder_file_id: Some(2),
                name: "sp1.mkv".to_string(),
                size: 200,
            },
            relative_path: PathBuf::from("specials/sp1.mkv"),
        };
        let dest = resolve_destination(Path::new("/downloads"), "yuru_camp", &item, true);
        assert_eq!(dest, PathBuf::from("/downloads/yuru_camp/specials/sp1.mkv"));
    }
}
