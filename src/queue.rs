//! Idempotent cloud queue operations for multi-item automation.
//!
//! These helpers add a magnet only when it is absent, bound the time spent
//! waiting for Seedr to cache it, and purge by explicit folder ID. They never
//! delete unrelated folders, unlike the interactive `download_magnet` path.

use crate::seedr::{
    extract_magnet_name, ListContentsResponse, SeedrClient, SeedrFolder, SeedrTorrent,
};
use anyhow::Result;
use std::time::Duration;
use tokio::time::Instant;

/// Default ceiling on how long to wait for a torrent to finish caching.
const DEFAULT_MAX_WAIT_SECS: u64 = 1_800;
/// Default headroom kept free in the Seedr account before adding a magnet.
const DEFAULT_MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Default first poll interval while a torrent is actively caching.
const DEFAULT_INITIAL_POLL_SECS: u64 = 5;
/// Polls allowed after a torrent vanishes but before a folder appears.
const MAX_MISSING_POLLS: u32 = 12;
/// Poll interval used while waiting for the completed folder to materialise.
const FALLBACK_POLL_SECS: u64 = 3;

/// Tunables controlling [`SeedrClient::ensure_cached`].
#[derive(Clone, Debug)]
pub struct CachePolicy {
    /// Total time allowed for the cloud to finish caching the torrent.
    pub max_wait: Duration,
    /// Free bytes that must remain before a new magnet is accepted.
    pub min_free_bytes: u64,
    /// First poll interval, adapted afterwards from the reported rate.
    pub initial_poll_secs: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            max_wait: Duration::from_secs(DEFAULT_MAX_WAIT_SECS),
            min_free_bytes: DEFAULT_MIN_FREE_BYTES,
            initial_poll_secs: DEFAULT_INITIAL_POLL_SECS,
        }
    }
}

/// Outcome of an idempotent cache request.
#[derive(Debug, Clone)]
pub enum CacheOutcome {
    /// The completed folder is available in the cloud.
    Ready(SeedrFolder),
    /// The deadline elapsed before the torrent finished caching.
    Timeout {
        /// Seedr torrent identifier being waited on.
        torrent_id: u64,
        /// Last reported completion percentage.
        progress: f64,
    },
    /// The account lacks the free space required by the policy.
    NotEnoughSpace {
        /// Currently free bytes in the cloud account.
        free: u64,
        /// Free bytes the policy demands before adding.
        needed: u64,
    },
}

/// Cloud storage accounting reported by `list_contents`.
#[derive(Debug, Clone, Copy)]
pub struct Space {
    /// Total quota in bytes.
    pub max: u64,
    /// Used bytes.
    pub used: u64,
}

impl Space {
    /// Builds accounting from a `list_contents` response.
    #[must_use]
    pub fn from_list(list: &ListContentsResponse) -> Self {
        Self {
            max: list.space_max.unwrap_or(0),
            used: list.space_used.unwrap_or(0),
        }
    }

    /// Remaining bytes, saturating at zero when usage exceeds the quota.
    #[must_use]
    pub const fn free(&self) -> u64 {
        self.max.saturating_sub(self.used)
    }
}

/// Options for [`SeedrClient::ensure_cached`].
#[derive(Default)]
pub struct EnsureOpts<'a> {
    /// Wait and quota policy.
    pub policy: CachePolicy,
    /// Optional progress hook; when absent the wait is silent.
    pub on_progress: Option<&'a (dyn Fn(&SeedrTorrent) + Send + Sync)>,
}

impl SeedrClient {
    /// Reports current cloud quota usage.
    ///
    /// # Errors
    /// Returns an error if the account listing request fails.
    pub async fn space(&self) -> Result<Space> {
        let list = self.list_root().await?;
        Ok(Space::from_list(&list))
    }

    /// Finds a completed cloud folder whose name matches `name`.
    ///
    /// # Errors
    /// Returns an error if the account listing request fails.
    pub async fn find_cached_folder(&self, name: &str) -> Result<Option<SeedrFolder>> {
        let list = self.list_root().await?;
        Ok(find_by_name(&list.folders, name))
    }

    /// Ensures a magnet is cached in the cloud and returns the completed folder.
    ///
    /// Idempotent: an existing folder or in-flight torrent matching the magnet
    /// name is reused instead of re-added. Nothing is deleted on the way.
    ///
    /// # Errors
    /// Returns an error if listing the account or adding the magnet fails.
    pub async fn ensure_cached(&self, magnet: &str, opts: &EnsureOpts<'_>) -> Result<CacheOutcome> {
        let name = extract_magnet_name(magnet);
        let root = self.list_root().await?;

        if let Some(n) = name.as_deref() {
            if let Some(folder) = find_by_name(&root.folders, n) {
                return Ok(CacheOutcome::Ready(folder));
            }
            if let Some(torrent) = find_torrent(&root.torrents, n) {
                let previous = folder_ids(&root.folders);
                return self
                    .wait_for_caching_until(torrent.id, name.as_deref(), &previous, opts)
                    .await;
            }
        }

        let space = Space::from_list(&root);
        if space.free() < opts.policy.min_free_bytes {
            return Ok(CacheOutcome::NotEnoughSpace {
                free: space.free(),
                needed: opts.policy.min_free_bytes,
            });
        }

        let previous = folder_ids(&root.folders);
        let torrent_id = self.add_magnet(magnet).await?;
        self.wait_for_caching_until(torrent_id, name.as_deref(), &previous, opts)
            .await
    }

    /// Deletes exactly the folders named by `ids` and returns how many were sent.
    ///
    /// # Errors
    /// Returns an error if the deletion request fails.
    pub async fn purge_folders(&self, ids: &[u64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let targets: Vec<SeedrFolder> = ids
            .iter()
            .map(|&id| SeedrFolder {
                id,
                name: String::new(),
                size: None,
            })
            .collect();
        self.delete_all_folders(&targets).await?;
        Ok(targets.len())
    }

    async fn wait_for_caching_until(
        &self,
        torrent_id: u64,
        name: Option<&str>,
        previous: &[u64],
        opts: &EnsureOpts<'_>,
    ) -> Result<CacheOutcome> {
        let deadline = Instant::now() + opts.policy.max_wait;
        let mut poll_secs = opts.policy.initial_poll_secs;
        let mut missing_polls = 0_u32;
        let mut last_progress = 0.0_f64;

        loop {
            if Instant::now() >= deadline {
                return Ok(CacheOutcome::Timeout {
                    torrent_id,
                    progress: last_progress,
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(Duration::from_secs(poll_secs).min(remaining)).await;

            let list = self.list_root().await?;
            if let Some(torrent) = list.torrents.iter().find(|t| t.id == torrent_id) {
                missing_polls = 0;
                last_progress = torrent.progress.unwrap_or(0.0);
                poll_secs = Self::calculate_adaptive_poll_secs(torrent.download_rate.unwrap_or(0));
                if let Some(cb) = opts.on_progress {
                    cb(torrent);
                }
                continue;
            }

            if let Some(folder) = resolve_new_folder(&list.folders, name, previous) {
                return Ok(CacheOutcome::Ready(folder));
            }

            missing_polls += 1;
            if missing_polls >= MAX_MISSING_POLLS {
                return Ok(CacheOutcome::Timeout {
                    torrent_id,
                    progress: last_progress,
                });
            }
            poll_secs = FALLBACK_POLL_SECS;
        }
    }
}

fn folder_ids(folders: &[SeedrFolder]) -> Vec<u64> {
    folders.iter().map(|f| f.id).collect()
}

fn names_match(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(b) || b.contains(a))
}

fn find_by_name(folders: &[SeedrFolder], name: &str) -> Option<SeedrFolder> {
    folders.iter().find(|f| names_match(&f.name, name)).cloned()
}

fn find_torrent<'a>(torrents: &'a [SeedrTorrent], name: &str) -> Option<&'a SeedrTorrent> {
    torrents.iter().find(|t| names_match(&t.name, name))
}

/// Prefers a name match, then the largest folder created after the snapshot.
fn resolve_new_folder(
    folders: &[SeedrFolder],
    name: Option<&str>,
    previous: &[u64],
) -> Option<SeedrFolder> {
    if let Some(n) = name {
        if let Some(folder) = find_by_name(folders, n) {
            return Some(folder);
        }
    }
    folders
        .iter()
        .filter(|f| !previous.contains(&f.id))
        .max_by_key(|f| f.size)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: u64, name: &str, size: u64) -> SeedrFolder {
        SeedrFolder {
            id,
            name: name.to_string(),
            size: Some(size),
        }
    }

    #[test]
    fn names_match_handles_containment() {
        assert!(names_match("[SubsPlease] Show - 01.mkv", "Show - 01"));
        assert!(!names_match("", "Show"));
        assert!(!names_match("Show A", "Show B"));
    }

    #[test]
    fn space_free_saturates() {
        let space = Space {
            max: 100,
            used: 250,
        };
        assert_eq!(space.free(), 0);
    }

    #[test]
    fn resolve_new_folder_ignores_known_folders() {
        let folders = vec![folder(1, "old", 900), folder(2, "new", 10)];
        let picked = resolve_new_folder(&folders, None, &[1]);
        assert_eq!(picked.map(|f| f.id), Some(2));
    }

    #[test]
    fn resolve_new_folder_prefers_name_over_size() {
        let folders = vec![folder(1, "huge", 9_000), folder(2, "target - 01", 10)];
        let picked = resolve_new_folder(&folders, Some("target - 01"), &[]);
        assert_eq!(picked.map(|f| f.id), Some(2));
    }

    #[test]
    fn find_by_name_requires_non_empty_match() {
        let folders = vec![folder(1, "some show", 10)];
        assert!(find_by_name(&folders, "some show").is_some());
        assert!(find_by_name(&folders, "").is_none());
    }
}
