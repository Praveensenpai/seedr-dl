use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::RANGE;
use reqwest::Client;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

const THREADS: u64 = 8;

pub type ProgressCallback = Arc<dyn Fn(u64, u64, u64, u64) + Send + Sync>;

pub struct Downloader {
    client: Client,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new()
    }
}

impl Downloader {
    pub fn new() -> Self {
        let seedr_addr = SocketAddr::from(([95, 211, 204, 172], 443));
        Self {
            client: Client::builder()
                .resolve("www.seedr.cc", seedr_addr)
                .resolve("seedr.cc", seedr_addr)
                .resolve("stream.seedr.cc", seedr_addr)
                .resolve("direct.seedr.cc", seedr_addr)
                .timeout(Duration::from_secs(3600))
                .build()
                .unwrap_or_default(),
        }
    }

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
        self.download_parallel(url, target_dir, file_name, Arc::new(callback))
            .await
    }

    async fn download_parallel(
        &self,
        url: &str,
        target_dir: &Path,
        file_name: &str,
        callback: ProgressCallback,
    ) -> Result<PathBuf> {
        fs::create_dir_all(target_dir).await?;
        let final_path = target_dir.join(file_name);

        let total_size = self.fetch_content_length(url).await?;
        let chunk_size = total_size.div_ceil(THREADS);

        // Pre-allocate the output file
        let out_file = File::create(&final_path).await?;
        out_file.set_len(total_size).await?;
        drop(out_file);

        let downloaded = Arc::new(Mutex::new(0u64));
        let start_time = Instant::now();
        let last_cb = Arc::new(Mutex::new(Instant::now()));
        let last_bytes = Arc::new(Mutex::new(0u64));

        // Compute chunk done bytes from existing chunk part files
        let mut initial_total: u64 = 0;
        for i in 0..THREADS {
            let chunk_start = i * chunk_size;
            let chunk_end = ((i + 1) * chunk_size).min(total_size) - 1;
            let chunk_len = chunk_end - chunk_start + 1;
            let done = self.chunk_done_bytes(target_dir, file_name, i, chunk_len).await;
            initial_total += done;
        }
        if let Ok(mut d) = downloaded.lock() {
            *d = initial_total;
        }

        #[allow(clippy::cast_possible_truncation)]
        let mut handles = Vec::with_capacity(THREADS as usize);
        for i in 0..THREADS {
            let chunk_start = i * chunk_size;
            let chunk_end = ((i + 1) * chunk_size).min(total_size) - 1;
            let chunk_len = chunk_end - chunk_start + 1;
            let done = self.chunk_done_bytes(target_dir, file_name, i, chunk_len).await;

            if done >= chunk_len {
                continue; // already complete
            }

            let client = self.client.clone();
            let url = url.to_string();
            let path = final_path.clone();
            let part_path = target_dir.join(format!("{file_name}.part.{i}"));
            let downloaded = Arc::clone(&downloaded);
            let last_cb = Arc::clone(&last_cb);
            let last_bytes = Arc::clone(&last_bytes);
            let cb = Arc::clone(&callback);

            let resume_from = chunk_start + done;

            let handle = tokio::spawn(async move {
                download_chunk(
                    &client,
                    &url,
                    &path,
                    &part_path,
                    chunk_start,
                    resume_from,
                    chunk_end,
                    chunk_len,
                    done,
                    total_size,
                    downloaded,
                    last_cb,
                    last_bytes,
                    cb,
                )
                .await
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.context("Chunk task panicked")??;
        }

        // Clean up part tracking files
        for i in 0..THREADS {
            let part_path = target_dir.join(format!("{file_name}.part.{i}"));
            let _ = fs::remove_file(part_path).await;
        }

        // Final callback
        #[allow(clippy::cast_precision_loss)]
        let elapsed = start_time.elapsed().as_secs().max(1);
        let avg_spd = total_size.checked_div(elapsed).unwrap_or(0);
        callback(total_size, total_size, avg_spd, 0);

        Ok(final_path)
    }

    async fn fetch_content_length(&self, url: &str) -> Result<u64> {
        let res = self
            .client
            .head(url)
            .send()
            .await
            .context("HEAD request failed")?;
        res.content_length()
            .context("Server did not return Content-Length")
    }

    async fn chunk_done_bytes(&self, dir: &Path, name: &str, idx: u64, chunk_len: u64) -> u64 {
        let part = dir.join(format!("{name}.part.{idx}"));
        fs::metadata(&part)
            .await
            .map_or(0, |m| m.len().min(chunk_len))
    }
}

#[allow(clippy::too_many_arguments, clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
async fn download_chunk(
    client: &Client,
    url: &str,
    final_path: &Path,
    part_path: &Path,
    _chunk_start: u64,
    resume_from: u64,
    chunk_end: u64,
    chunk_len: u64,
    already_done: u64,
    total_size: u64,
    downloaded: Arc<Mutex<u64>>,
    last_cb: Arc<Mutex<Instant>>,
    last_bytes: Arc<Mutex<u64>>,
    callback: ProgressCallback,
) -> Result<()> {
    let range = format!("bytes={resume_from}-{chunk_end}");
    let res = client
        .get(url)
        .header(RANGE, &range)
        .send()
        .await
        .context("Chunk request failed")?;

    let out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(final_path)
        .await?;
    let mut out = tokio::io::BufWriter::new(out);
    out.seek(tokio::io::SeekFrom::Start(resume_from)).await?;

    let mut part_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(part_path)
        .await?;

    let mut stream = res.bytes_stream();
    let mut chunk_downloaded = already_done;

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.context("Network error in chunk stream")?;
        let len = chunk.len() as u64;

        out.write_all(&chunk).await?;
        part_file.write_all(&chunk).await?;
        chunk_downloaded += len;

        if let Ok(mut d) = downloaded.lock() {
            *d += len;
        }

        if let Ok(mut t) = last_cb.lock() {
            if t.elapsed() >= Duration::from_millis(400) {
                let dur = t.elapsed().as_secs_f64();
                let total_dl = downloaded.lock().map_or(0, |d| *d);
                let last = last_bytes.lock().map_or(0, |b| *b);
                let speed = if dur > 0.0 {
                    ((total_dl.saturating_sub(last)) as f64 / dur) as u64
                } else {
                    0
                };
                let rem = total_size.saturating_sub(total_dl);
                let eta = rem.checked_div(speed).unwrap_or(0);
                callback(total_dl, total_size, speed, eta);
                *t = Instant::now();
                if let Ok(mut b) = last_bytes.lock() {
                    *b = total_dl;
                }
            }
        }

        if chunk_downloaded >= chunk_len {
            break;
        }
    }

    out.flush().await?;
    Ok(())
}

