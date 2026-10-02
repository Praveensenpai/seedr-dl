//! `seedr-dl`: Fast Seedr.cc multi-chunk downloader and automation library with webhooks.

pub mod config;
pub mod downloader;
pub mod history;
pub mod notifier;
pub mod seedr;
pub mod transfer;

pub use config::{interactive_auth, load_auth, load_config, save_auth, save_config, Auth, Config};
pub use downloader::Downloader;
pub use notifier::{EventCallback, NotificationEvent, Notifier, NotifierConfig};
pub use seedr::{
    extract_magnet_name, CachingQuery, ListContentsResponse, SeedrClient, SeedrFile, SeedrFolder,
    SeedrTorrent,
};
pub use transfer::{
    collect_folder_files, download_folder as transfer_download_folder, resolve_destination,
    CloudFileItem, TransferOptions,
};

use anyhow::{bail, Result};
use std::path::PathBuf;

/// Strategy for cloud item cleanup after download.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloudCleanup {
    /// Delete item from Seedr cloud after successful download.
    #[default]
    Delete,
    /// Keep item in Seedr cloud.
    Keep,
}

/// Options for programmatic download operations.
#[derive(Clone, Default)]
pub struct DownloadOptions {
    /// Optional HTTP webhook URL to receive progress and lifecycle events.
    pub callback_url: Option<String>,
    /// Optional in-process callback listener.
    pub callback_fn: Option<EventCallback>,
    /// Whether to print progress events as ndjson to stdout.
    pub json_output: bool,
    /// Custom target output directory (defaults to configured `download_dir`).
    pub output_dir: Option<PathBuf>,
    /// Cloud cleanup behavior.
    pub cloud_cleanup: CloudCleanup,
}

/// Downloads a magnet link or torrent directly into the target output directory.
///
/// # Errors
/// Returns an error if magnet addition, caching, or downloading fails.
pub async fn download_magnet(
    client: &SeedrClient,
    cfg: &Config,
    magnet: &str,
    opts: &DownloadOptions,
) -> Result<Vec<PathBuf>> {
    let notifier = build_notifier(opts);

    notifier.notify(NotificationEvent::Status {
        folder_id: 0,
        file_name: None,
        status: "caching".into(),
        message: "Adding torrent to Seedr cloud and waiting for cache".into(),
    });

    let root_before = client.list_root().await?;
    let prev_ids: Vec<u64> = root_before.folders.iter().map(|f| f.id).collect();
    let name_hint = extract_magnet_name(magnet);

    if let Some(ref name) = name_hint {
        if let Some(existing) = root_before.folders.iter().find(|f| f.name == *name) {
            return download_folder(client, cfg, existing, opts).await;
        }
    }

    if !root_before.folders.is_empty() {
        let _ = client.delete_all_folders(&root_before.folders).await;
    }
    for t in &root_before.torrents {
        let _ = client.delete_torrent(t.id).await;
    }

    let torrent_id = client.add_magnet(magnet).await?;
    let query = CachingQuery {
        torrent_id,
        name: name_hint.as_deref(),
        previous_folders: &prev_ids,
    };
    let folder_opt = client.wait_for_caching(&query, None).await?;

    let Some(folder) = folder_opt else {
        let msg = "Could not find completed folder in Seedr cloud".to_string();
        notifier.notify(NotificationEvent::Failed {
            folder_id: 0,
            file_name: None,
            error: msg.clone(),
        });
        bail!(msg);
    };

    download_folder(client, cfg, &folder, opts).await
}

/// Downloads an existing folder from the Seedr cloud directly into the target directory.
///
/// # Errors
/// Returns an error if listing or downloading files fails.
pub async fn download_folder(
    client: &SeedrClient,
    cfg: &Config,
    folder: &SeedrFolder,
    opts: &DownloadOptions,
) -> Result<Vec<PathBuf>> {
    let notifier = build_notifier(opts);
    let output_dir = opts
        .output_dir
        .clone()
        .unwrap_or_else(|| cfg.download_dir.clone());

    let transfer_opts = TransferOptions {
        output_dir,
        delete_cloud: opts.cloud_cleanup == CloudCleanup::Delete,
        notifier: Some(&notifier),
    };

    match transfer_download_folder(client, cfg, folder, &transfer_opts).await {
        Ok(paths) => Ok(paths),
        Err(err) => {
            notifier.notify(NotificationEvent::Failed {
                folder_id: folder.id,
                file_name: None,
                error: format!("{err:#}"),
            });
            Err(err)
        }
    }
}

fn build_notifier(opts: &DownloadOptions) -> Notifier {
    Notifier::new(NotifierConfig {
        callback_url: opts.callback_url.clone(),
        json_output: opts.json_output,
        callback_fn: opts.callback_fn.clone(),
        progress_interval_ms: 500,
    })
}
