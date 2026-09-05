//! Per-chunk download worker with retry and offset-based file streaming.

use crate::downloader::state::{ChunkRange, DownloadState};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::RANGE;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::Mutex;

/// Parameters defining a chunk download job.
pub struct ChunkJob {
    pub client: Client,
    pub url: String,
    pub part_path: PathBuf,
    pub chunk_index: usize,
}

/// Downloads a chunk with up to 3 retries and exponential backoff.
pub async fn download_chunk_with_retry(
    job: ChunkJob,
    state: Arc<Mutex<DownloadState>>,
    failed_flag: Arc<AtomicBool>,
) -> Result<()> {
    let mut attempt = 0;
    let max_attempts = 3;

    loop {
        if failed_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Download aborted");
        }

        let current_chunk = {
            let s = state.lock().await;
            s.chunks[job.chunk_index].clone()
        };

        if current_chunk.is_complete() {
            return Ok(());
        }

        match stream_chunk(&job, &current_chunk, &state, &failed_flag).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempt += 1;
                if attempt >= max_attempts {
                    failed_flag.store(true, Ordering::Relaxed);
                    return Err(e).context(format!(
                        "Chunk {} failed after {max_attempts} attempts",
                        job.chunk_index
                    ));
                }
                let backoff_secs = 1u64 << (attempt - 1);
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            }
        }
    }
}

async fn stream_chunk(
    job: &ChunkJob,
    chunk: &ChunkRange,
    state: &Arc<Mutex<DownloadState>>,
    failed_flag: &Arc<AtomicBool>,
) -> Result<()> {
    let resume_from = chunk.resume_offset();
    let range = format!("bytes={resume_from}-{}", chunk.end);

    let res = job
        .client
        .get(&job.url)
        .header(RANGE, &range)
        .send()
        .await
        .context("Failed to connect to chunk stream")?;

    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("Server returned HTTP status {status} for chunk range {range}");
    }

    let mut file = OpenOptions::new()
        .write(true)
        .open(&job.part_path)
        .await
        .context("Failed to open part file for chunk")?;

    file.seek(SeekFrom::Start(resume_from))
        .await
        .context("Failed to seek part file to resume offset")?;

    let mut stream = res.bytes_stream();
    let mut buffered_bytes: u64 = 0;

    while let Some(chunk_res) = stream.next().await {
        if failed_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Download cancelled");
        }

        let bytes = chunk_res.context("Network error reading chunk stream")?;
        file.write_all(&bytes)
            .await
            .context("Failed writing chunk bytes")?;

        buffered_bytes += bytes.len() as u64;

        if buffered_bytes >= 256 * 1024 {
            let mut s = state.lock().await;
            let current = s.chunks[job.chunk_index].downloaded + buffered_bytes;
            s.chunks[job.chunk_index].downloaded = current.min(s.chunks[job.chunk_index].len());
            buffered_bytes = 0;
        }
    }

    if buffered_bytes > 0 {
        let mut s = state.lock().await;
        let current = s.chunks[job.chunk_index].downloaded + buffered_bytes;
        s.chunks[job.chunk_index].downloaded = current.min(s.chunks[job.chunk_index].len());
    }

    file.flush().await.context("Failed to flush chunk writes")?;
    Ok(())
}
