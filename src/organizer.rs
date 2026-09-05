use crate::gemini::MediaInfo;
use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
