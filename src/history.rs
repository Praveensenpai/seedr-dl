use crate::config::config_dir;
use crate::gemini::MediaType;
use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Status of how media was renamed and structured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenameStatus {
    /// Renamed using Gemini AI.
    Gemini,
    /// Renamed using offline regex parser.
    RegexFallback,
    /// Manually renamed by user.
    Manual,
}

/// A record of a completed ingestion stored in ~/.config/seedr-dl/history.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unique identifier for this download record.
    pub id: String,
    /// Original torrent / file name from Seedr.
    pub original_name: String,
    /// Cleaned canonical title.
    pub clean_title: String,
    /// Current path on disk in Jellyfin media library.
    pub file_path: PathBuf,
    /// Total file size in bytes.
    pub file_size: u64,
    /// Type of media (Movie or Show).
    pub media_type: MediaType,
    /// Release year if identified.
    pub year: Option<u32>,
    /// TV season number if identified.
    pub season: Option<u32>,
    /// TV episode number if identified.
    pub episode: Option<u32>,
    /// Human-readable download timestamp.
    pub downloaded_at: String,
    /// Whether AI, regex, or manual renaming was used.
    pub rename_status: RenameStatus,
}

/// Returns the file path for the history database.
pub fn history_path() -> PathBuf {
    config_dir().join("history.json")
}

/// Generates a unique, timestamp-based ID for history entries.
pub fn generate_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("dl_{now}")
}

/// Returns formatted current local date and time string.
pub fn current_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Loads all history entries from disk.
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

/// Adds a new entry to history and saves to disk.
pub fn add_history_entry(entry: HistoryEntry) -> Result<()> {
    let mut entries = load_history()?;
    entries.retain(|e| e.id != entry.id);
    entries.insert(0, entry);
    save_history(&entries)
}

/// Updates an existing history entry by matching its ID.
pub fn update_history_entry(entry: &HistoryEntry) -> Result<()> {
    let mut entries = load_history()?;
    if let Some(existing) = entries.iter_mut().find(|e| e.id == entry.id) {
        *existing = entry.clone();
        save_history(&entries)?;
    }
    Ok(())
}

/// Removes a history entry by ID and saves to disk.
pub fn remove_history_entry(id: &str) -> Result<Option<HistoryEntry>> {
    let mut entries = load_history()?;
    let removed = entries
        .iter()
        .position(|e| e.id == id)
        .map(|idx| entries.remove(idx));
    if removed.is_some() {
        save_history(&entries)?;
    }
    Ok(removed)
}

/// Finds a history entry by full ID or prefix match.
pub fn find_history_entry(query: &str) -> Result<Option<HistoryEntry>> {
    let entries = load_history()?;
    let trimmed = query.trim();
    let found = entries
        .into_iter()
        .find(|e| e.id == trimmed || e.id.starts_with(trimmed));
    Ok(found)
}

/// Checks if target file actually exists on disk.
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
            clean_title: "Test".to_string(),
            file_path: PathBuf::from("/tmp/test.mkv"),
            file_size: 1024,
            media_type: MediaType::Movie,
            year: Some(2025),
            season: None,
            episode: None,
            downloaded_at: "2026-09-06 00:00".to_string(),
            rename_status: RenameStatus::Gemini,
        };
        let Ok(json) = serde_json::to_string(&entry) else {
            panic!("serialization failed");
        };
        let Ok(deserialized): Result<HistoryEntry, _> = serde_json::from_str(&json) else {
            panic!("deserialization failed");
        };
        assert_eq!(deserialized.clean_title, "Test");
        assert_eq!(deserialized.rename_status, RenameStatus::Gemini);
    }
}
