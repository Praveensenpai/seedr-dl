use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

pub struct Downloader {
    client: Client,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new()
    }
}

use std::net::SocketAddr;

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
        fs::create_dir_all(target_dir).await?;
        let final_path = target_dir.join(file_name);
        let temp_path = target_dir.join(format!("{file_name}.part"));

        println!("  {} Connecting to download stream...", "•".dimmed());
        let res = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to connect to download URL: {url}"))?;

        let total_size = res.content_length().unwrap_or(0);
        let pb = create_progress_bar(total_size, file_name)?;

        let mut file = File::create(&temp_path)
            .await
            .with_context(|| format!("Failed to create temporary file: {}", temp_path.display()))?;

        let mut stream = res.bytes_stream();
        let mut downloaded: u64 = 0;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.with_context(|| "Network error during download streaming")?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }

        file.flush().await?;
        drop(file);

        fs::rename(&temp_path, &final_path)
            .await
            .with_context(|| format!("Failed to finalize file: {}", final_path.display()))?;

        pb.finish_with_message(format!("{} Download complete!", "✔".green().bold()));
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
