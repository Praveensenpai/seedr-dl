//! Background coordinator for progress reporting and periodic state persistence.

use crate::downloader::state::DownloadState;
use crate::downloader::ProgressCallback;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Configuration for the download progress coordinator.
pub struct CoordinatorConfig {
    pub state: Arc<Mutex<DownloadState>>,
    pub target_dir: PathBuf,
    pub file_name: String,
    pub total_size: u64,
    pub callback: ProgressCallback,
    pub done_flag: Arc<AtomicBool>,
}

/// Spawns the background progress and state saving coordinator task.
pub fn spawn_coordinator(cfg: CoordinatorConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(400));
        let mut last_save = Instant::now();
        let mut last_tick = Instant::now();
        let mut smoothed_speed: u64 = 0;
        let mut last_downloaded = {
            let s = cfg.state.lock().await;
            s.total_downloaded()
        };

        loop {
            interval.tick().await;
            if cfg.done_flag.load(Ordering::Relaxed) {
                break;
            }

            let total_dl = {
                let s = cfg.state.lock().await;
                s.total_downloaded()
            };

            let elapsed_ms = u64::try_from(last_tick.elapsed().as_millis()).unwrap_or(1);
            let instant_speed = total_dl
                .saturating_sub(last_downloaded)
                .saturating_mul(1000)
                .checked_div(elapsed_ms)
                .unwrap_or(0);

            smoothed_speed = if smoothed_speed == 0 {
                instant_speed
            } else {
                (smoothed_speed.saturating_mul(7) + instant_speed.saturating_mul(3)) / 10
            };

            let rem = cfg.total_size.saturating_sub(total_dl);
            let eta = rem.checked_div(smoothed_speed).unwrap_or(0);

            (cfg.callback)(total_dl, cfg.total_size, smoothed_speed, eta);
            last_downloaded = total_dl;
            last_tick = Instant::now();

            if last_save.elapsed() >= Duration::from_secs(2) {
                let s = cfg.state.lock().await;
                let _ = s.save(&cfg.target_dir, &cfg.file_name).await;
                last_save = Instant::now();
            }
        }
    })
}
