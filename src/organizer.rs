use crate::config::{cache_dir, get_gemini_key, Config};
use crate::downloader::Downloader;
use crate::gemini::{self, MediaInfo};
use crate::seedr::{SeedrClient, SeedrFolder};
use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
    let downloader = Downloader::new();
    let temp_dir = cache_dir();

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
        let info = gemini::parse_media(&file.name, gemini_key.as_deref()).await;

        organize_file(
            &downloaded_path,
            &info,
            &cfg.jellyfin_media_dir,
            non_interactive,
        )?;
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
    non_interactive: bool,
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

    if !non_interactive && !confirm_move()? {
        println!("  {} Move skipped by user.", "•".yellow());
        return Ok(source_file.to_path_buf());
    }

    fs::create_dir_all(&dest_dir).with_context(|| {
        format!(
            "Failed to create Jellyfin directory: {}",
            dest_dir.display()
        )
    })?;

    // Attempt atomic rename, fall back to copy+remove if cross-device
    if fs::rename(source_file, &dest_file).is_err() {
        fs::copy(source_file, &dest_file)
            .with_context(|| format!("Failed to copy to Jellyfin: {}", dest_file.display()))?;
        let _ = fs::remove_file(source_file);
    }

    println!("  {} Placed in Jellyfin media library!", "✔".green().bold());
    Ok(dest_file)
}

fn confirm_move() -> Result<bool> {
    print!("  Move to Jellyfin library? [Y/n]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}
