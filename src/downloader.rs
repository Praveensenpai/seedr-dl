use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::RANGE;
use reqwest::Client;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;

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
                .timeout(Duration::from_hours(1))
                .build()
                .unwrap_or_default(),
        }
    }

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

        if supports_range && total_size >= 5 * 1024 * 1024 {
            match self
                .download_parallel(url, target_dir, file_name, Arc::clone(&cb), total_size)
                .await
            {
                Ok(path) => Ok(path),
                Err(e) => {
                    eprintln!(
                        "  {} Parallel download failed ({e:#}), falling back to single stream...",
                        "•".yellow()
                    );
                    self.download_single(url, target_dir, file_name, cb, total_size)
                        .await
                }
            }
        } else {
            self.download_single(url, target_dir, file_name, cb, total_size)
                .await
        }
    }

    async fn probe_download(&self, url: &str) -> Result<(u64, bool)> {
        // Use GET with Range: bytes=0-0 instead of HEAD to avoid 405/403 or connection reset
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

    #[allow(clippy::cast_precision_loss)]
    async fn download_parallel(
        &self,
        url: &str,
        target_dir: &Path,
        file_name: &str,
        callback: ProgressCallback,
        total_size: u64,
    ) -> Result<PathBuf> {
        fs::create_dir_all(target_dir).await?;
        let final_path = target_dir.join(file_name);
        if fs::metadata(&final_path).await.map_or(0, |m| m.len()) == total_size && total_size > 0 {
            return Ok(final_path);
        }

        let chunk_size = total_size.div_ceil(THREADS);
        let downloaded = Arc::new(Mutex::new(0u64));
        let start_time = Instant::now();
        let last_cb = Arc::new(Mutex::new(Instant::now()));
        let last_bytes = Arc::new(Mutex::new(0u64));

        let mut initial_total: u64 = 0;
        for i in 0..THREADS {
            let chunk_start = i * chunk_size;
            if chunk_start >= total_size {
                break;
            }
            let chunk_end = ((i + 1) * chunk_size).min(total_size) - 1;
            let chunk_len = chunk_end - chunk_start + 1;
            let part_path = target_dir.join(format!("{file_name}.part.{i}"));
            let done = fs::metadata(&part_path).await.map_or(0, |m| m.len()).min(chunk_len);
            initial_total += done;
        }

        if let Ok(mut d) = downloaded.lock() {
            *d = initial_total;
        }
        if let Ok(mut b) = last_bytes.lock() {
            *b = initial_total;
        }

        #[allow(clippy::cast_possible_truncation)]
        let mut handles = Vec::with_capacity(THREADS as usize);
        for i in 0..THREADS {
            let chunk_start = i * chunk_size;
            if chunk_start >= total_size {
                break;
            }
            let chunk_end = ((i + 1) * chunk_size).min(total_size) - 1;
            let chunk_len = chunk_end - chunk_start + 1;
            let part_path = target_dir.join(format!("{file_name}.part.{i}"));
            let done = fs::metadata(&part_path).await.map_or(0, |m| m.len()).min(chunk_len);

            if done >= chunk_len {
                continue; // chunk already completely downloaded
            }

            let client = self.client.clone();
            let url = url.to_string();
            let downloaded = Arc::clone(&downloaded);
            let last_cb = Arc::clone(&last_cb);
            let last_bytes = Arc::clone(&last_bytes);
            let cb = Arc::clone(&callback);

            let resume_from = chunk_start + done;

            let handle = tokio::spawn(async move {
                download_chunk_to_part(
                    &client,
                    &url,
                    &part_path,
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

        // Assemble all part files safely
        let assemble_tmp = target_dir.join(format!("{file_name}.assembling"));
        let mut out_file = File::create(&assemble_tmp).await?;

        for i in 0..THREADS {
            let chunk_start = i * chunk_size;
            if chunk_start >= total_size {
                break;
            }
            let part_path = target_dir.join(format!("{file_name}.part.{i}"));
            let mut part_file = File::open(&part_path).await?;
            tokio::io::copy(&mut part_file, &mut out_file).await?;
            drop(part_file);
            let _ = fs::remove_file(part_path).await;
        }

        out_file.flush().await?;
        drop(out_file);

        fs::rename(&assemble_tmp, &final_path).await?;

        let elapsed = start_time.elapsed().as_secs().max(1);
        let avg_spd = total_size.checked_div(elapsed).unwrap_or(0);
        callback(total_size, total_size, avg_spd, 0);

        Ok(final_path)
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    async fn download_single(
        &self,
        url: &str,
        target_dir: &Path,
        file_name: &str,
        callback: ProgressCallback,
        total_size: u64,
    ) -> Result<PathBuf> {
        fs::create_dir_all(target_dir).await?;
        let final_path = target_dir.join(file_name);
        let temp_path = target_dir.join(format!("{file_name}.part"));

        let existing = fs::metadata(&temp_path).await.map_or(0, |m| m.len());
        if existing >= total_size && total_size > 0 {
            fs::rename(&temp_path, &final_path).await?;
            callback(total_size, total_size, 0, 0);
            return Ok(final_path);
        }

        let mut req = self.client.get(url);
        if existing > 0 {
            req = req.header(RANGE, format!("bytes={existing}-"));
        }

        let res = req.send().await.context("Failed to connect to stream")?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("Server returned error on stream: {status}");
        }

        let (mut file, mut downloaded, eff_total) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let total = if total_size > 0 {
                total_size
            } else {
                existing + res.content_length().unwrap_or(0)
            };
            let f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&temp_path)
                .await?;
            (f, existing, total)
        } else {
            let total = if total_size > 0 {
                total_size
            } else {
                res.content_length().unwrap_or(0)
            };
            let f = File::create(&temp_path).await?;
            (f, 0, total)
        };

        let mut stream = res.bytes_stream();
        let start_time = Instant::now();
        let mut last_cb = Instant::now();
        let mut last_bytes = downloaded;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.context("Network error during streaming")?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if last_cb.elapsed() >= Duration::from_millis(400) {
                let dur = last_cb.elapsed().as_secs_f64();
                let speed = if dur > 0.0 {
                    ((downloaded.saturating_sub(last_bytes)) as f64 / dur) as u64
                } else {
                    0
                };
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

        let elapsed = start_time.elapsed().as_secs().max(1);
        let avg_spd = eff_total.checked_div(elapsed).unwrap_or(0);
        callback(eff_total, eff_total, avg_spd, 0);

        Ok(final_path)
    }
}

#[allow(clippy::too_many_arguments, clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
async fn download_chunk_to_part(
    client: &Client,
    url: &str,
    part_path: &Path,
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

    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("Server returned error for chunk range {range}: {status}");
    }

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

    part_file.flush().await?;
    Ok(())
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
    fn test_chunk_partitioning_exact() {
        let total_size = 100_000_000u64;
        let chunk_size = total_size.div_ceil(THREADS);
        let mut covered = 0;

        for i in 0..THREADS {
            let chunk_start = i * chunk_size;
            if chunk_start >= total_size {
                break;
            }
            let chunk_end = ((i + 1) * chunk_size).min(total_size) - 1;
            let chunk_len = chunk_end - chunk_start + 1;
            assert_eq!(chunk_start, covered);
            covered += chunk_len;
        }
        assert_eq!(covered, total_size);
    }

    #[test]
    fn test_chunk_partitioning_small_file() {
        for total_size in [1u64, 2, 7, 8, 9, 15] {
            let chunk_size = total_size.div_ceil(THREADS);
            let mut covered = 0;

            for i in 0..THREADS {
                let chunk_start = i * chunk_size;
                if chunk_start >= total_size {
                    break;
                }
                let chunk_end = ((i + 1) * chunk_size).min(total_size) - 1;
                let chunk_len = chunk_end - chunk_start + 1;
                assert_eq!(chunk_start, covered);
                covered += chunk_len;
            }
            assert_eq!(covered, total_size);
        }
    }

    #[test]
    fn test_content_range_header_parsing() {
        let header = "bytes 0-0/987654321";
        let total = header
            .rsplit('/')
            .next()
            .and_then(|s| s.trim().parse::<u64>().ok());
        assert_eq!(total, Some(987_654_321));
    }
}

