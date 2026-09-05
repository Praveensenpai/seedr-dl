use crate::config::{downloads_dir, get_gemini_key, Config};
use crate::downloader::Downloader;
use crate::gemini::{self, MediaInfo};
use crate::history::{self, HistoryEntry, RenameStatus};
use crate::seedr::{SeedrClient, SeedrFolder};
use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Metadata and settings for ingesting a downloaded file.
pub struct IngestTarget<'a> {
    /// Original release / file name from Seedr.
    pub original_name: &'a str,
    /// Renaming method applied.
    pub rename_status: RenameStatus,
    /// Whether to skip interactive prompts.
    pub non_interactive: bool,
}

pub async fn download_and_ingest(
    client: &SeedrClient,
    cfg: &Config,
    folder: &SeedrFolder,
    non_interactive: bool,
) -> Result<()> {
    let contents = client.list_folder(folder.id).await?;
    if contents.files.is_empty() {
        println!(
            "\n  {} No files found inside cloud folder '{}'.",
            "✖".red(),
            folder.name
        );
        print!("\n  Press [Enter] to return to manager...");
        io::stdout().flush()?;
        let mut pause = String::new();
        io::stdin().read_line(&mut pause)?;
        return Ok(());
    }

    let gemini_key = get_gemini_key(cfg);
    let downloader = Downloader::new(cfg.download_threads);
    let temp_dir = downloads_dir();

    for file in &contents.files {
        let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
        let sz_mb = file.size / 1_048_576;
        println!(
            "\n  {} Fetching download URL for: {} ({} MB)",
            "•".cyan(),
            file.name,
            sz_mb
        );
        let download_url = client.get_download_url(file_id).await?;

        let downloaded_path = downloader
            .download(&download_url, &temp_dir, &file.name)
            .await?;

        println!("  {} Analyzing title with Gemini AI...", "•".cyan());
        let (info, status) = gemini::parse_media(&file.name, gemini_key.as_deref()).await;
        let target = IngestTarget {
            original_name: &file.name,
            rename_status: status,
            non_interactive,
        };

        organize_file(&downloaded_path, &info, &cfg.jellyfin_media_dir, &target)?;
    }

    println!("\n  {} Processing complete!", "✔".green().bold());
    print!("  Delete item from Seedr cloud to free space? [y/N]: ");
    io::stdout().flush()?;
    let mut del_input = String::new();
    io::stdin().read_line(&mut del_input)?;
    if del_input.trim().eq_ignore_ascii_case("y") {
        client.delete_folder(folder.id).await?;
        println!("  {} Cloud item deleted.", "✔".green());
    }

    print!("\n  Press [Enter] to return to manager...");
    io::stdout().flush()?;
    let mut pause_line = String::new();
    io::stdin().read_line(&mut pause_line)?;

    Ok(())
}

pub fn organize_file(
    source_file: &Path,
    info: &MediaInfo,
    media_root: &Path,
    target: &IngestTarget,
) -> Result<PathBuf> {
    let dest_dir = media_root.join(&info.relative_folder);
    let dest_file = dest_dir.join(&info.clean_filename);

    println!();
    println!("  {}", "✨ Identified for Jellyfin:".cyan().bold());
    println!("     {} {:?}", "Type:    ".dimmed(), info.media_type);
    println!("     {} {}", "Title:   ".dimmed(), info.title);
    if let Some(y) = info.year {
        println!("     {} {}", "Year:    ".dimmed(), y);
    }
    if let (Some(s), Some(e)) = (info.season, info.episode) {
        println!("     {} S{:02}E{:02}", "Episode: ".dimmed(), s, e);
    }
    println!(
        "     {} {}",
        "Target:  ".dimmed(),
        dest_file.display().to_string().green()
    );
    println!();

    if !target.non_interactive && !confirm_move()? {
        println!("  {} Move skipped by user.", "•".yellow());
        return Ok(source_file.to_path_buf());
    }

    fs::create_dir_all(&dest_dir).with_context(|| {
        format!(
            "Failed to create Jellyfin directory: {}",
            dest_dir.display()
        )
    })?;

    if fs::rename(source_file, &dest_file).is_err() {
        fs::copy(source_file, &dest_file)
            .with_context(|| format!("Failed to copy to Jellyfin: {}", dest_file.display()))?;
        let _ = fs::remove_file(source_file);
    }

    let file_size = fs::metadata(&dest_file).map_or(0, |m| m.len());
    let entry = HistoryEntry {
        id: history::generate_id(),
        original_name: target.original_name.to_string(),
        clean_title: info.title.clone(),
        file_path: dest_file.clone(),
        file_size,
        media_type: info.media_type.clone(),
        year: info.year,
        season: info.season,
        episode: info.episode,
        downloaded_at: history::current_timestamp(),
        rename_status: target.rename_status,
    };
    let _ = history::add_history_entry(entry);

    println!("  {} Placed in Jellyfin media library!", "✔".green().bold());
    Ok(dest_file)
}

/// Moves and re-indexes an existing media file to match newly analyzed metadata.
pub fn reorganize_media(
    entry: &mut HistoryEntry,
    new_info: &MediaInfo,
    media_root: &Path,
    new_status: RenameStatus,
) -> Result<PathBuf> {
    if !entry.file_path.exists() {
        bail!("File no longer exists at: {}", entry.file_path.display());
    }

    let new_dest_dir = media_root.join(&new_info.relative_folder);
    let new_dest_file = new_dest_dir.join(&new_info.clean_filename);

    if new_dest_file != entry.file_path {
        fs::create_dir_all(&new_dest_dir)?;
        if fs::rename(&entry.file_path, &new_dest_file).is_err() {
            fs::copy(&entry.file_path, &new_dest_file)?;
            let _ = fs::remove_file(&entry.file_path);
        }
        clean_empty_parent_dirs(&entry.file_path, media_root);
    }

    entry.file_path.clone_from(&new_dest_file);
    entry.clean_title.clone_from(&new_info.title);
    entry.year = new_info.year;
    entry.season = new_info.season;
    entry.episode = new_info.episode;
    entry.media_type = new_info.media_type.clone();
    entry.rename_status = new_status;

    history::update_history_entry(entry)?;
    Ok(new_dest_file)
}

/// Permanently deletes a downloaded media file from disk and removes it from history.
pub fn delete_media(entry: &HistoryEntry, media_root: &Path) -> Result<()> {
    if entry.file_path.exists() {
        fs::remove_file(&entry.file_path)
            .with_context(|| format!("Failed to delete {}", entry.file_path.display()))?;
        clean_empty_parent_dirs(&entry.file_path, media_root);
    }
    let _ = history::remove_history_entry(&entry.id)?;
    Ok(())
}

/// Recursively removes empty parent directories up to, but not including, the media root.
pub fn clean_empty_parent_dirs(file_path: &Path, root: &Path) {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        let is_empty = fs::read_dir(dir).is_ok_and(|mut it| it.next().is_none());
        if !is_empty {
            break;
        }
        let _ = fs::remove_dir(dir);
        current = dir.parent();
    }
}

fn confirm_move() -> Result<bool> {
    print!("  Move to Jellyfin library? [Y/n]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}
