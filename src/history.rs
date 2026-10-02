//! Download history persistence and lookup.

use crate::config::config_dir;
use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A record of a completed download stored in ~/.config/seedr-dl/history.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unique identifier for this download record.
    pub id: String,
    /// Original file name from Seedr.
    pub original_name: String,
    /// Final saved path on disk.
    pub file_path: PathBuf,
    /// Total file size in bytes.
    pub file_size: u64,
    /// Human-readable download timestamp.
    pub downloaded_at: String,
}

/// Returns the file path for the history database.
#[must_use]
pub fn history_path() -> PathBuf {
    config_dir().join("history.json")
}

/// Generates a unique, timestamp-based ID for history entries.
#[must_use]
pub fn generate_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("dl_{now}")
}

/// Returns formatted current local date and time string.
#[must_use]
pub fn current_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Loads all history entries from disk.
///
/// # Errors
/// Returns an error if reading or parsing the history database fails.
pub fn load_history() -> Result<Vec<HistoryEntry>> {
    let path = history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read history file: {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let entries: Vec<HistoryEntry> =
        serde_json::from_str(&content).with_context(|| "Failed to parse history.json")?;
    Ok(entries)
}

/// Saves history entries to disk.
///
/// # Errors
/// Returns an error if directory creation or file writing fails.
pub fn save_history(entries: &[HistoryEntry]) -> Result<()> {
    let path = history_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(entries)?;
    fs::write(&path, content)
        .with_context(|| format!("Failed to write history file: {}", path.display()))?;
    Ok(())
}

/// Clears all history entries from disk.
///
/// # Errors
/// Returns an error if saving the empty history fails.
pub fn clear_history() -> Result<()> {
    save_history(&[])
}

/// Adds a new entry to history and saves to disk.
///
/// # Errors
/// Returns an error if saving history fails.
pub fn add_history_entry(entry: HistoryEntry) -> Result<()> {
    let mut entries = load_history()?;
    entries.retain(|e| e.id != entry.id);
    entries.insert(0, entry);
    save_history(&entries)
}

/// Checks if target file actually exists on disk.
#[must_use]
pub fn media_file_exists(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_generation() {
        let id = generate_id();
        assert!(id.starts_with("dl_"));
    }

    #[test]
    fn test_serialization() {
        let entry = HistoryEntry {
            id: "dl_123".to_string(),
            original_name: "test.mkv".to_string(),
            file_path: PathBuf::from("/tmp/test.mkv"),
            file_size: 1024,
            downloaded_at: "2026-09-06 00:00".to_string(),
        };
        let Ok(json) = serde_json::to_string(&entry) else {
            panic!("serialization failed");
        };
        let Ok(deserialized): Result<HistoryEntry, _> = serde_json::from_str(&json) else {
            panic!("deserialization failed");
        };
        assert_eq!(deserialized.original_name, "test.mkv");
        assert_eq!(deserialized.file_size, 1024);
    }
}
