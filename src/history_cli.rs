use crate::config::{get_gemini_key, Config};
use crate::gemini::{self, MediaType};
use crate::history::{
    self, find_history_entry, load_history, media_file_exists, HistoryEntry, RenameStatus,
};
use crate::organizer::{delete_media, reorganize_media};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use colored::Colorize;
use std::io::{self, Write};
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum HistoryCommands {
    /// List all downloaded media items in history
    List,
    /// Re-analyze and rename an entry with Gemini AI
    Rename {
        /// Download ID or prefix to rename
        id: String,
    },
    /// Manually rename an entry's title, year, or season/episode
    Manual {
        /// Download ID or prefix to rename
        id: String,
    },
    /// Delete a downloaded item from disk and history (requires typing DELETE)
    Delete {
        /// Download ID or prefix to delete
        id: String,
    },
    /// Scan Jellyfin library to add untracked media files to history
    Scan,
}

pub async fn handle_history_cli(cmd: Option<HistoryCommands>, cfg: &Config) -> Result<()> {
    match cmd.unwrap_or(HistoryCommands::List) {
        HistoryCommands::List => list_history()?,
        HistoryCommands::Rename { id } => handle_ai_rename(&id, cfg).await?,
        HistoryCommands::Manual { id } => handle_manual_rename(&id, cfg)?,
        HistoryCommands::Delete { id } => handle_delete(&id, cfg)?,
        HistoryCommands::Scan => handle_scan(&cfg.jellyfin_media_dir)?,
    }
    Ok(())
}

fn list_history() -> Result<()> {
    let entries = load_history()?;
    if entries.is_empty() {
        println!("  {}", "No download history found.".yellow());
        println!("  Tip: Run 'seedr-dl history scan' to index existing Jellyfin media.");
        return Ok(());
    }

    println!("  {}", "Downloaded Media History:".cyan().bold());
    println!();
    for e in &entries {
        let exists = media_file_exists(&e.file_path);
        let status_badge = match e.rename_status {
            RenameStatus::Gemini => "AI".green(),
            RenameStatus::RegexFallback => "Regex".yellow(),
            RenameStatus::Manual => "Manual".blue(),
        };
        let file_badge = if exists {
            "✔ on disk".green()
        } else {
            "✖ missing".red()
        };
        let sz_mb = e.file_size / 1_048_576;
        let yr_str = e.year.map_or_else(String::new, |y| format!(" ({y})"));
        let ep_str = match (e.season, e.episode) {
            (Some(s), Some(ep)) => format!(" S{s:02}E{ep:02}"),
            _ => String::new(),
        };

        println!(
            "  • [{}] {}{}{} [{} MB | {} | {} | {}]",
            e.id.cyan().bold(),
            e.clean_title.bold(),
            yr_str,
            ep_str,
            sz_mb,
            status_badge,
            file_badge,
            e.downloaded_at.dimmed()
        );
        println!("    Path: {}", e.file_path.display().to_string().dimmed());
    }
    println!();
    Ok(())
}

async fn handle_ai_rename(id_query: &str, cfg: &Config) -> Result<()> {
    let mut entry = find_history_entry(id_query)?
        .with_context(|| format!("No history item found matching '{id_query}'"))?;

    println!("  Target: {}", entry.clean_title.bold());
    println!("  Original: {}", entry.original_name.dimmed());
    println!("  {} Querying Gemini AI...", "•".cyan());

    let gemini_key = get_gemini_key(cfg).with_context(|| {
        "Gemini API key not configured. Set GEMINI_API_KEY or run 'seedr-dl config --gemini-key <KEY>'"
    })?;

    let new_info = gemini::query_gemini(&entry.original_name, &gemini_key).await?;

    let new_target = cfg
        .jellyfin_media_dir
        .join(&new_info.relative_folder)
        .join(&new_info.clean_filename);

    println!();
    println!("  {}", "Proposed Renaming:".cyan().bold());
    println!("  Old: {}", entry.file_path.display().to_string().red());
    println!("  New: {}", new_target.display().to_string().green());
    println!();

    print!("  Apply this renaming? [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("y") {
        reorganize_media(
            &mut entry,
            &new_info,
            &cfg.jellyfin_media_dir,
            RenameStatus::Gemini,
        )?;
        println!("  {} Successfully renamed media!", "✔".green().bold());
    } else {
        println!("  {} Renaming cancelled.", "•".yellow());
    }
    Ok(())
}

fn handle_manual_rename(id_query: &str, cfg: &Config) -> Result<()> {
    let mut entry = find_history_entry(id_query)?
        .with_context(|| format!("No history item found matching '{id_query}'"))?;

    println!("  Current Title: {}", entry.clean_title);
    print!("  Enter new Title (or Enter to keep): ");
    io::stdout().flush()?;
    let mut title_in = String::new();
    io::stdin().read_line(&mut title_in)?;
    let title = if title_in.trim().is_empty() {
        entry.clean_title.clone()
    } else {
        title_in.trim().to_string()
    };

    print!("  Enter release Year (or Enter to keep/skip): ");
    io::stdout().flush()?;
    let mut year_in = String::new();
    io::stdin().read_line(&mut year_in)?;
    let year: Option<u32> = if year_in.trim().is_empty() {
        entry.year
    } else {
        year_in.trim().parse().ok()
    };

    let season_ep = match (entry.season, entry.episode) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    };

    let new_info = gemini::media_info_from_manual(&entry.original_name, &title, year, season_ep);

    let new_target = cfg
        .jellyfin_media_dir
        .join(&new_info.relative_folder)
        .join(&new_info.clean_filename);

    println!();
    println!("  {}", "Proposed Renaming:".cyan().bold());
    println!("  Old: {}", entry.file_path.display().to_string().red());
    println!("  New: {}", new_target.display().to_string().green());
    println!();

    print!("  Apply this renaming? [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("y") {
        reorganize_media(
            &mut entry,
            &new_info,
            &cfg.jellyfin_media_dir,
            RenameStatus::Manual,
        )?;
        println!("  {} Successfully renamed media!", "✔".green().bold());
    } else {
        println!("  {} Renaming cancelled.", "•".yellow());
    }
    Ok(())
}

fn handle_delete(id_query: &str, cfg: &Config) -> Result<()> {
    let entry = find_history_entry(id_query)?
        .with_context(|| format!("No history item found matching '{id_query}'"))?;

    let sz_mb = entry.file_size / 1_048_576;
    println!();
    println!("  {}", "⚠️  PERMANENT DELETION WARNING ⚠️".red().bold());
    println!("  Title: {}", entry.clean_title.bold());
    println!("  Size:  {sz_mb} MB");
    println!(
        "  Path:  {}",
        entry.file_path.display().to_string().yellow()
    );
    println!();
    println!("  This will permanently delete the file from your disk.");
    print!("  Type \"DELETE\" to confirm: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim() == "DELETE" {
        delete_media(&entry, &cfg.jellyfin_media_dir)?;
        println!(
            "  {} Permanently deleted from disk and history.",
            "✔".green().bold()
        );
    } else {
        println!("  {} Deletion cancelled.", "•".yellow());
    }
    Ok(())
}

fn handle_scan(media_root: &Path) -> Result<()> {
    if !media_root.exists() {
        bail!("Media root does not exist: {}", media_root.display());
    }
    let mut added = 0;
    scan_dir_for_media(media_root, media_root, &mut added)?;
    println!(
        "  {} Scan complete. Added {added} media item(s) to history.",
        "✔".green().bold()
    );
    Ok(())
}

fn scan_dir_for_media(current: &Path, root: &Path, added: &mut usize) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(current) else {
        return Ok(());
    };
    for entry_res in entries {
        let entry = entry_res?;
        let path = entry.path();
        if path.is_dir() {
            scan_dir_for_media(&path, root, added)?;
        } else if is_video_file(&path) && add_discovered_file(&path, root)? {
            *added += 1;
        }
    }
    Ok(())
}

fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_lowercase().as_str(),
                "mkv" | "mp4" | "avi" | "mov" | "webm"
            )
        })
}

fn add_discovered_file(path: &Path, root: &Path) -> Result<bool> {
    let existing = load_history()?;
    if existing.iter().any(|e| e.file_path == path) {
        return Ok(false);
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let info = gemini::parse_with_regex(file_name);
    let size = std::fs::metadata(path).map_or(0, |m| m.len());

    let entry = HistoryEntry {
        id: history::generate_id(),
        original_name: file_name.to_string(),
        clean_title: info.title,
        file_path: path.to_path_buf(),
        file_size: size,
        media_type: if path.starts_with(root.join("shows")) {
            MediaType::Show
        } else {
            MediaType::Movie
        },
        year: info.year,
        season: info.season,
        episode: info.episode,
        downloaded_at: history::current_timestamp(),
        rename_status: RenameStatus::RegexFallback,
    };
    history::add_history_entry(entry)?;
    Ok(true)
}
