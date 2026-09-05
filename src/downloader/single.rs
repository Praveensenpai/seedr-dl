//! Single-stream fallback downloader.

use crate::downloader::state::DownloadState;
use crate::downloader::{DownloadTarget, ProgressCallback};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::RANGE;
use reqwest::Client;
use std::path::PathBuf;
use std::time::Instant;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};

/// Downloads file over a single HTTP stream (resumes if supported).
pub async fn download_single(
    client: &Client,
    target: &DownloadTarget<'_>,
    callback: ProgressCallback,
) -> Result<PathBuf> {
    fs::create_dir_all(target.target_dir).await?;
    let final_path = target.target_dir.join(target.file_name);
    let temp_path = DownloadState::part_path(target.target_dir, target.file_name);

    let mut existing = fs::metadata(&temp_path).await.map_or(0, |m| m.len());
    if existing >= target.total_size && target.total_size > 0 {
        fs::rename(&temp_path, &final_path).await?;
        callback(target.total_size, target.total_size, 0, 0);
        return Ok(final_path);
    }

    let mut req = client.get(target.url);
    if existing > 0 {
        req = req.header(RANGE, format!("bytes={existing}-"));
    }

    let res = req.send().await.context("Failed to connect to stream")?;
    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("Server returned error on stream: {status}");
    }

    let (mut file, mut downloaded, eff_total) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let total = if target.total_size > 0 {
            target.total_size
        } else {
            existing + res.content_length().unwrap_or(0)
        };
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&temp_path)
            .await?;
        (f, existing, total)
    } else {
        let total = if target.total_size > 0 {
            target.total_size
        } else {
            res.content_length().unwrap_or(0)
        };
        existing = 0;
        let f = File::create(&temp_path).await?;
        (f, 0, total)
    };

    file.seek(SeekFrom::Start(existing)).await?;
    let mut stream = res.bytes_stream();
    let mut last_cb = Instant::now();
    let mut last_bytes = downloaded;

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.context("Network error during streaming")?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let elapsed_ms = u64::try_from(last_cb.elapsed().as_millis()).unwrap_or(1);
        if elapsed_ms >= 400 {
            let speed = downloaded
                .saturating_sub(last_bytes)
                .saturating_mul(1000)
                .checked_div(elapsed_ms)
                .unwrap_or(0);
            let rem = eff_total.saturating_sub(downloaded);
            let eta = rem.checked_div(speed).unwrap_or(0);
            callback(downloaded, eff_total, speed, eta);
            last_cb = Instant::now();
            last_bytes = downloaded;
        }
    }

    file.flush().await?;
    drop(file);
    fs::rename(&temp_path, &final_path).await?;
    callback(eff_total, eff_total, 0, 0);
    Ok(final_path)
}
