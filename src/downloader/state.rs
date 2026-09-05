use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

/// A slice of the file being downloaded by a single worker thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkRange {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

impl ChunkRange {
    /// Total length of this chunk in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start) + 1
    }

    /// Whether this chunk has finished downloading all assigned bytes.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.downloaded >= self.len()
    }

    /// Absolute byte offset from which to resume download.
    #[must_use]
    pub fn resume_offset(&self) -> u64 {
        self.start + self.downloaded
    }
}

/// Download state persisted in `<file_name>.state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    pub total_size: u64,
    pub chunks: Vec<ChunkRange>,
}

impl DownloadState {
    /// Creates a new state with partitioned chunks.
    #[must_use]
    pub fn new(total_size: u64, num_threads: usize) -> Self {
        let threads_u64 = u64::try_from(num_threads).unwrap_or(1).max(1);
        let chunk_size = total_size.div_ceil(threads_u64);
        let mut chunks = Vec::with_capacity(num_threads);

        for i in 0..num_threads {
            let i_u64 = u64::try_from(i).unwrap_or(0);
            let start = i_u64 * chunk_size;
            if start >= total_size {
                break;
            }
            let end = ((i_u64 + 1) * chunk_size).min(total_size) - 1;
            chunks.push(ChunkRange {
                index: i,
                start,
                end,
                downloaded: 0,
            });
        }

        Self { total_size, chunks }
    }

    /// Returns path to download state JSON file.
    #[must_use]
    pub fn state_path(target_dir: &Path, file_name: &str) -> PathBuf {
        target_dir.join(format!("{file_name}.state.json"))
    }

    /// Returns path to pre-allocated .part file.
    #[must_use]
    pub fn part_path(target_dir: &Path, file_name: &str) -> PathBuf {
        target_dir.join(format!("{file_name}.part"))
    }

    /// Loads existing state or creates and persists a new one.
    pub async fn load_or_init(
        target_dir: &Path,
        file_name: &str,
        total_size: u64,
        num_threads: usize,
    ) -> Result<Self> {
        let path = Self::state_path(target_dir, file_name);
        let part = Self::part_path(target_dir, file_name);
        if let Ok(content) = fs::read_to_string(&path).await {
            if let Ok(mut state) = serde_json::from_str::<Self>(&content) {
                if state.total_size == total_size {
                    if !part.exists() {
                        for c in &mut state.chunks {
                            c.downloaded = 0;
                        }
                    }
                    return Ok(state);
                }
            }
        }
        let state = Self::new(total_size, num_threads);
        state.save(target_dir, file_name).await?;
        Ok(state)
    }

    /// Serializes and writes state to disk.
    pub async fn save(&self, target_dir: &Path, file_name: &str) -> Result<()> {
        let path = Self::state_path(target_dir, file_name);
        let content = serde_json::to_string(self).context("Failed to serialize download state")?;
        fs::write(path, content)
            .await
            .context("Failed to write download state file")?;
        Ok(())
    }

    /// Cleans up state file upon download completion.
    pub async fn cleanup(target_dir: &Path, file_name: &str) {
        let _ = fs::remove_file(Self::state_path(target_dir, file_name)).await;
    }

    /// Sum of all downloaded bytes across chunks.
    #[must_use]
    pub fn total_downloaded(&self) -> u64 {
        self.chunks.iter().map(|c| c.downloaded).sum()
    }
}

/// Removes legacy .part.0..31 files from older downloader versions.
pub async fn cleanup_legacy_parts(target_dir: &Path, file_name: &str) {
    for i in 0..32 {
        let p = target_dir.join(format!("{file_name}.part.{i}"));
        let _ = fs::remove_file(p).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_partitioning() {
        let state = DownloadState::new(1000, 4);
        assert_eq!(state.chunks.len(), 4);
        assert_eq!(state.chunks[0].start, 0);
        assert_eq!(state.chunks[0].end, 249);
        assert_eq!(state.chunks[0].len(), 250);

        assert_eq!(state.chunks[3].start, 750);
        assert_eq!(state.chunks[3].end, 999);
        assert_eq!(state.chunks[3].len(), 250);
    }

    #[test]
    fn test_total_downloaded() {
        let mut state = DownloadState::new(1000, 2);
        state.chunks[0].downloaded = 200;
        state.chunks[1].downloaded = 350;
        assert_eq!(state.total_downloaded(), 550);
    }
}
