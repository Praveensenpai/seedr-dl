//! Multi-threaded downloader with range support, single-file offset streaming, and resume.

pub mod chunk;
pub mod coordinator;
pub mod single;
pub mod state;

use self::chunk::{download_chunk_with_retry, ChunkJob};
use self::coordinator::{spawn_coordinator, CoordinatorConfig};
use self::single::download_single;
use self::state::{cleanup_legacy_parts, ChunkRange, DownloadState};
use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::RANGE;
use reqwest::Client;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::{self, OpenOptions};
use tokio::sync::Mutex;

/// Progress callback receiving (`downloaded_bytes`, `total_bytes`, `speed_bps`, `eta_seconds`).
pub type ProgressCallback = Arc<dyn Fn(u64, u64, u64, u64) + Send + Sync>;

/// Downloader supporting chunked concurrent downloads and atomic resumes.
pub struct Downloader {
    client: Client,
    num_threads: usize,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new(8)
    }
}

/// Specifications for a download task.
pub struct DownloadTarget<'a> {
    pub url: &'a str,
    pub target_dir: &'a Path,
    pub file_name: &'a str,
    pub total_size: u64,
}

impl Downloader {
    /// Creates a new Downloader with the configured thread count.
    #[must_use]
    pub fn new(threads: usize) -> Self {
        let seedr_addr = SocketAddr::from(([95, 211, 204, 172], 443));
        let client = Client::builder()
            .resolve("www.seedr.cc", seedr_addr)
            .resolve("seedr.cc", seedr_addr)
            .resolve("stream.seedr.cc", seedr_addr)
            .resolve("direct.seedr.cc", seedr_addr)
            .timeout(Duration::from_hours(1))
            .build()
            .unwrap_or_default();

        Self {
            client,
            num_threads: threads.max(1),
        }
    }

    /// Downloads file displaying a CLI progress bar.
    pub async fn download(&self, url: &str, target_dir: &Path, file_name: &str) -> Result<PathBuf> {
        let (total_size, _) = self.probe_download(url).await?;
        let pb = create_progress_bar(total_size, file_name)?;
        let pb_clone = pb.clone();

        let res = self
            .download_with_callback(url, target_dir, file_name, move |dl, tot, _spd, _eta| {
                if tot > 0 {
                    pb_clone.set_length(tot);
                }
                pb_clone.set_position(dl);
            })
            .await;

        match res {
            Ok(path) => {
                pb.finish_with_message(format!("{} Download complete!", "✔".green().bold()));
                Ok(path)
            }
            Err(e) => {
                pb.abandon();
                Err(e)
            }
        }
    }

    /// Downloads file streaming progress to the supplied callback.
    pub async fn download_with_callback<F>(
        &self,
        url: &str,
        target_dir: &Path,
        file_name: &str,
        callback: F,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64, u64, u64) + Send + Sync + 'static,
    {
        let (total_size, supports_range) = self.probe_download(url).await?;
        let cb: ProgressCallback = Arc::new(callback);
        let target = DownloadTarget {
            url,
            target_dir,
            file_name,
            total_size,
        };

        if supports_range && total_size >= 5 * 1024 * 1024 {
            self.download_parallel(&target, cb).await
        } else {
            download_single(&self.client, &target, cb).await
        }
    }

    async fn probe_download(&self, url: &str) -> Result<(u64, bool)> {
        let res = self
            .client
            .get(url)
            .header(RANGE, "bytes=0-0")
            .send()
            .await
            .context("Failed to connect to download stream")?;

        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("Server returned HTTP status: {status}");
        }

        if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let total = res
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|cr| cr.to_str().ok())
                .and_then(|s| s.rsplit('/').next())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            Ok((total, true))
        } else {
            let total = res.content_length().unwrap_or(0);
            Ok((total, false))
        }
    }

    async fn prepare_parallel_file(target: &DownloadTarget<'_>) -> Result<()> {
        fs::create_dir_all(target.target_dir).await?;
        cleanup_legacy_parts(target.target_dir, target.file_name).await;

        let part_path = DownloadState::part_path(target.target_dir, target.file_name);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&part_path)
            .await
            .context("Failed to open part file for preallocation")?;
        file.set_len(target.total_size)
            .await
            .context("Failed to preallocate part file")?;
        Ok(())
    }

    async fn download_parallel(
        &self,
        target: &DownloadTarget<'_>,
        callback: ProgressCallback,
    ) -> Result<PathBuf> {
        Self::prepare_parallel_file(target).await?;
        let state = DownloadState::load_or_init(
            target.target_dir,
            target.file_name,
            target.total_size,
            self.num_threads,
        )
        .await?;

        if state.chunks.iter().all(ChunkRange::is_complete) {
            callback(target.total_size, target.total_size, 0, 0);
            return finish_download(target).await;
        }

        let state_arc = Arc::new(Mutex::new(state));
        let done_flag = Arc::new(AtomicBool::new(false));
        let failed_flag = Arc::new(AtomicBool::new(false));

        let coord = spawn_coordinator(CoordinatorConfig {
            state: Arc::clone(&state_arc),
            target_dir: target.target_dir.to_path_buf(),
            file_name: target.file_name.to_string(),
            total_size: target.total_size,
            callback: Arc::clone(&callback),
            done_flag: Arc::clone(&done_flag),
        });

        let handles = spawn_chunk_jobs(&self.client, target, &state_arc, &failed_flag).await;
        let chunk_err = wait_for_chunks(handles).await;

        done_flag.store(true, Ordering::Relaxed);
        let _ = coord.await;

        if let Some(err) = chunk_err {
            let s = state_arc.lock().await;
            let _ = s.save(target.target_dir, target.file_name).await;
            return Err(err);
        }

        callback(target.total_size, target.total_size, 0, 0);
        finish_download(target).await
    }
}

async fn spawn_chunk_jobs(
    client: &Client,
    target: &DownloadTarget<'_>,
    state_arc: &Arc<Mutex<DownloadState>>,
    failed_flag: &Arc<AtomicBool>,
) -> Vec<tokio::task::JoinHandle<Result<()>>> {
    let part_path = DownloadState::part_path(target.target_dir, target.file_name);
    let chunks_to_run = {
        let s = state_arc.lock().await;
        s.chunks.clone()
    };

    let mut handles = Vec::new();
    for chunk in chunks_to_run {
        if chunk.is_complete() {
            continue;
        }
        let job = ChunkJob {
            client: client.clone(),
            url: target.url.to_string(),
            part_path: part_path.clone(),
            chunk_index: chunk.index,
        };
        let s_clone = Arc::clone(state_arc);
        let f_clone = Arc::clone(failed_flag);
        handles.push(tokio::spawn(async move {
            download_chunk_with_retry(job, s_clone, f_clone).await
        }));
    }
    handles
}

async fn wait_for_chunks(
    handles: Vec<tokio::task::JoinHandle<Result<()>>>,
) -> Option<anyhow::Error> {
    let mut chunk_err = None;
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if chunk_err.is_none() {
                    chunk_err = Some(e);
                }
            }
            Err(e) => {
                if chunk_err.is_none() {
                    chunk_err = Some(anyhow::anyhow!("Task join error: {e}"));
                }
            }
        }
    }
    chunk_err
}

async fn finish_download(target: &DownloadTarget<'_>) -> Result<PathBuf> {
    let part_path = DownloadState::part_path(target.target_dir, target.file_name);
    let final_path = target.target_dir.join(target.file_name);
    fs::rename(&part_path, &final_path).await?;
    DownloadState::cleanup(target.target_dir, target.file_name).await;
    Ok(final_path)
}

fn create_progress_bar(total_size: u64, file_name: &str) -> Result<ProgressBar> {
    let pb = if total_size > 0 {
        ProgressBar::new(total_size)
    } else {
        ProgressBar::new_spinner()
    };

    let template = if total_size > 0 {
        format!(
            "  {{spinner:.cyan}} {file_name}\n  [{{elapsed_precise}}] [{{bar:35.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{bytes_per_sec}}, ETA {{eta}})"
        )
    } else {
        format!(
            "  {{spinner:.cyan}} {file_name} [{{elapsed_precise}}] {{bytes}} ({{bytes_per_sec}})"
        )
    };

    pb.set_style(
        ProgressStyle::default_bar()
            .template(&template)?
            .progress_chars("━╸─"),
    );
    Ok(pb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_range_header_parsing() {
        let header = "bytes 0-0/987654321";
        let total = header
            .rsplit('/')
            .next()
            .and_then(|s| s.trim().parse::<u64>().ok());
        assert_eq!(total, Some(987_654_321));
    }

    #[test]
    fn test_downloader_thread_default() {
        let d = Downloader::default();
        assert_eq!(d.num_threads, 8);
    }
}
