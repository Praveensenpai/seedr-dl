//! Event notification and webhook dispatch system for CLI and library consumers.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::Mutex;

/// Events emitted during torrent caching, file downloading, AI renaming, completion, or failure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum NotificationEvent {
    /// Download/transfer progress update.
    Progress {
        /// Cloud folder identifier.
        folder_id: u64,
        /// Name of the file being downloaded.
        file_name: String,
        /// Total bytes transferred so far.
        downloaded_bytes: u64,
        /// Total file size in bytes.
        total_bytes: u64,
        /// Current transfer speed in bytes per second.
        speed_bps: u64,
        /// Estimated time remaining in seconds.
        eta_seconds: u64,
        /// Download progress percentage (0.0 to 100.0).
        percentage: f64,
    },
    /// Status transition (e.g. caching, organizing).
    Status {
        /// Cloud folder identifier.
        folder_id: u64,
        /// Optional file name associated with the status.
        file_name: Option<String>,
        /// Status name (e.g. "caching", "organizing").
        status: String,
        /// Human-readable descriptive message.
        message: String,
    },
    /// Download and ingestion completed successfully.
    Completed {
        /// Cloud folder identifier.
        folder_id: u64,
        /// File name that completed.
        file_name: String,
        /// Final destination path on disk.
        destination_path: Option<String>,
        /// Total bytes downloaded.
        total_bytes: u64,
    },
    /// Download or processing failure.
    Failed {
        /// Cloud folder identifier.
        folder_id: u64,
        /// Optional file name that failed.
        file_name: Option<String>,
        /// Error description.
        error: String,
    },
}

/// Callback function type for in-process Rust event listeners.
pub type EventCallback = Arc<dyn Fn(NotificationEvent) + Send + Sync>;

/// Configuration for event dispatching and notification webhooks.
#[derive(Clone, Default)]
pub struct NotifierConfig {
    /// Webhook URL to send HTTP POST requests on events.
    pub callback_url: Option<String>,
    /// Whether to print events as single-line JSON (`ndjson`) to stdout.
    pub json_output: bool,
    /// In-memory Rust callback listener.
    pub callback_fn: Option<EventCallback>,
    /// Minimum interval between sending HTTP progress webhook calls in milliseconds.
    pub progress_interval_ms: u64,
}

/// Dispatches events to HTTP webhooks, stdout JSON streams, and Rust callbacks.
#[derive(Clone)]
pub struct Notifier {
    sender: UnboundedSender<NotificationEvent>,
    is_active: Arc<AtomicBool>,
}

impl Notifier {
    /// Creates a new `Notifier` instance with a background dispatcher task.
    #[must_use]
    pub fn new(config: NotifierConfig) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<NotificationEvent>();
        let is_active = Arc::new(AtomicBool::new(true));
        let active_flag = Arc::clone(&is_active);

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        let interval = Duration::from_millis(config.progress_interval_ms.max(250));
        let last_http_progress = Arc::new(Mutex::new(None::<Instant>));

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                dispatch_event(&event, &config, &http_client, &last_http_progress, interval).await;
            }
            active_flag.store(false, Ordering::SeqCst);
        });

        Self { sender, is_active }
    }

    /// Emits a notification event. Non-blocking and thread-safe.
    pub fn notify(&self, event: NotificationEvent) {
        let _ = self.sender.send(event);
    }

    /// Flushes pending notifications with a brief wait for network delivery.
    pub async fn flush(&self) {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    /// Returns true if the background dispatcher is still running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::SeqCst)
    }
}

async fn dispatch_event(
    event: &NotificationEvent,
    config: &NotifierConfig,
    http_client: &reqwest::Client,
    last_http_progress: &Arc<Mutex<Option<Instant>>>,
    min_interval: Duration,
) {
    if let Some(cb) = &config.callback_fn {
        (cb)(event.clone());
    }

    if config.json_output {
        if let Ok(json) = serde_json::to_string(event) {
            println!("{json}");
        }
    }

    if let Some(url) = &config.callback_url {
        send_webhook(event, url, http_client, last_http_progress, min_interval).await;
    }
}

async fn send_webhook(
    event: &NotificationEvent,
    url: &str,
    client: &reqwest::Client,
    last_progress: &Arc<Mutex<Option<Instant>>>,
    min_interval: Duration,
) {
    if let NotificationEvent::Progress { percentage, .. } = event {
        let mut guard = last_progress.lock().await;
        let should_send = guard.is_none_or(|last| {
            // reason: f64 comparison for final completion check
            #[allow(clippy::float_cmp)]
            let is_finished = *percentage >= 100.0;
            is_finished || last.elapsed() >= min_interval
        });

        if !should_send {
            return;
        }
        *guard = Some(Instant::now());
    }

    let _ = client.post(url).json(event).send().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_notifier_dispatch() {
        let (tx, mut rx) = mpsc::channel(1);
        let config = NotifierConfig {
            callback_url: None,
            json_output: false,
            callback_fn: Some(Arc::new(move |evt| {
                let _ = tx.try_send(evt);
            })),
            progress_interval_ms: 100,
        };

        let notifier = Notifier::new(config);
        notifier.notify(NotificationEvent::Status {
            folder_id: 42,
            file_name: None,
            status: "ready".into(),
            message: "All good".into(),
        });

        let received = rx.recv().await;
        assert!(received.is_some());
    }
}
