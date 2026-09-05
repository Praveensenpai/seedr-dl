use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::RANGE;
use reqwest::{Client, StatusCode};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;

pub type ProgressCallback = Box<dyn Fn(u64, u64, u64, u64) + Send>;

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

    pub async fn download(&self, url: &str, target_dir: &Path, file_name: &str) -> Result<PathBuf> {
        self.download_internal(url, target_dir, file_name, None)
            .await
    }

    pub async fn download_with_callback<F>(
        &self,
        url: &str,
        target_dir: &Path,
        file_name: &str,
        callback: F,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64, u64, u64) + Send + 'static,
    {
        self.download_internal(url, target_dir, file_name, Some(Box::new(callback)))
            .await
    }

    #[allow(clippy::cast_precision_loss)]
    async fn download_internal(
        &self,
        url: &str,
        target_dir: &Path,
        file_name: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<PathBuf> {
        fs::create_dir_all(target_dir).await?;
        let final_path = target_dir.join(file_name);
        let temp_path = target_dir.join(format!("{file_name}.part"));

        let existing = fs::metadata(&temp_path).await.map_or(0, |m| m.len());

        let mut req = self.client.get(url);
        if existing > 0 {
            req = req.header(RANGE, format!("bytes={existing}-"));
        }

        println!("  {} Connecting to download stream...", "•".dimmed());
        let res = req.send().await.context("Failed to connect to stream")?;
        let status = res.status();

        let (mut file, mut downloaded, total_size) = if status == StatusCode::PARTIAL_CONTENT {
            let total = existing + res.content_length().unwrap_or(0);
            let f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&temp_path)
                .await?;
            (f, existing, total)
        } else {
            let total = res.content_length().unwrap_or(0);
            let f = File::create(&temp_path).await?;
            (f, 0, total)
        };

        let pb = create_progress_bar(total_size, file_name)?;
        pb.set_position(downloaded);

        let mut stream = res.bytes_stream();
        let start_time = Instant::now();
        let mut last_cb = Instant::now();
        let mut last_bytes = downloaded;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.context("Network error during streaming")?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);

            if last_cb.elapsed() >= Duration::from_millis(500) {
                let dur = last_cb.elapsed().as_secs_f64();
                let speed = if dur > 0.0 {
                    // reason: rate measurement calculation fits safely in u64
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    (((downloaded - last_bytes) as f64 / dur) as u64)
                } else {
                    0
                };
                let rem = total_size.saturating_sub(downloaded);
                let eta = rem.checked_div(speed).unwrap_or(0);
                if let Some(ref cb) = callback {
                    cb(downloaded, total_size, speed, eta);
                }
                last_cb = Instant::now();
                last_bytes = downloaded;
            }
        }

        file.flush().await?;
        drop(file);

        fs::rename(&temp_path, &final_path).await?;
        pb.finish_with_message(format!("{} Download complete!", "✔".green().bold()));

        if let Some(ref cb) = callback {
            let total_dur = start_time.elapsed().as_secs();
            let avg_spd = downloaded.checked_div(total_dur).unwrap_or(0);
            cb(downloaded, total_size, avg_spd, 0);
        }

        Ok(final_path)
    }
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
