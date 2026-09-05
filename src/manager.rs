use crate::config::{cache_dir, get_gemini_key, Config};
use crate::downloader::Downloader;
use crate::gemini;
use crate::organizer;
use crate::seedr::{ListContentsResponse, SeedrClient, SeedrFolder};
use anyhow::{Context, Result};
use colored::Colorize;
use std::io::{self, Write};

pub async fn run_dashboard(
    client: &SeedrClient,
    cfg: &Config,
    non_interactive: bool,
) -> Result<()> {
    loop {
        let list = client.list_root().await?;
        print_dashboard(&list);

        if non_interactive {
            break;
        }

        print!("  Action: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let cmd = input.trim().to_lowercase();

        match cmd.as_str() {
            "q" | "exit" | "0" => break,
            "r" | "refresh" => {}
            "a" | "add" => handle_add_prompt(client, cfg).await?,
            "c" | "clean" => handle_clean_all(client, &list.folders).await?,
            _ => {
                if let Some(rest) = cmd.strip_prefix("dt ") {
                    handle_delete_torrent(client, &list.torrents, rest).await?;
                } else if let Some(rest) =
                    cmd.strip_prefix("d ").or_else(|| cmd.strip_prefix("rm "))
                {
                    handle_delete_by_index(client, &list.folders, rest).await?;
                } else if let Ok(idx) = cmd.parse::<usize>() {
                    if idx > 0 && idx <= list.folders.len() {
                        let folder = &list.folders[idx - 1];
                        download_and_ingest(client, cfg, folder, false).await?;
                    } else {
                        println!("  {} Invalid item number.", "✖".red());
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_dashboard(list: &ListContentsResponse) {
    println!();
    println!(
        "  {}",
        "┌────────────────────────────────────────────────────────┐".cyan()
    );
    println!(
        "  {}  ⚡ {}  {}",
        "│".cyan(),
        "Seedr Cloud Manager & Jellyfin Pipeline".bold(),
        "│".cyan()
    );
    println!(
        "  {}",
        "└────────────────────────────────────────────────────────┘".cyan()
    );

    if let (Some(used), Some(max)) = (list.space_used, list.space_max) {
        let used_mb = used / 1_048_576;
        let max_mb = max / 1_048_576;
        let free_mb = max_mb.saturating_sub(used_mb);
        let bar = storage_progress_bar(used_mb, max_mb);
        println!("  • Storage: [{bar}] {used_mb} MB / {max_mb} MB ({free_mb} MB free)");
    }

    if !list.torrents.is_empty() {
        println!(
            "\n  {}",
            "── Active Cloud Downloads ─────────────────────────────".yellow()
        );
        for (i, t) in list.torrents.iter().enumerate() {
            let pct = t.progress.unwrap_or(0.0);
            let sz = t.size.unwrap_or(0) / 1_048_576;
            println!("  [t{}] ⏳ {} ({} MB, {:.1}%)", i + 1, t.name, sz, pct);
        }
    }

    println!(
        "\n  {}",
        "── Completed Cloud Items ──────────────────────────────".cyan()
    );
    if list.folders.is_empty() {
        println!("  {} (no completed files in cloud)", "•".dimmed());
    } else {
        for (idx, f) in list.folders.iter().enumerate() {
            let sz = f.size.unwrap_or(0) / 1_048_576;
            println!("  [{}] 📁 {} ({} MB)", idx + 1, f.name.bold(), sz);
        }
    }

    println!(
        "\n  {}",
        "── Commands ───────────────────────────────────────────".dimmed()
    );
    println!("  [1-N] Download & move to Jellyfin   [d N]  Delete completed item N");
    println!("  [dt N] Cancel active torrent N      [c]    Delete ALL cloud folders");
    println!("  [a]   Add magnet link               [r]    Refresh");
    println!("  [q]   Quit manager");
    println!();
}

fn storage_progress_bar(used: u64, max: u64) -> String {
    let width = 20;
    if max == 0 {
        return "░".repeat(width);
    }
    let filled = usize::try_from((used * width as u64) / max)
        .unwrap_or(0)
        .min(width);
    let empty = width - filled;
    format!(
        "{}{}",
        "█".repeat(filled).yellow(),
        "░".repeat(empty).dimmed()
    )
}

async fn handle_add_prompt(client: &SeedrClient, cfg: &Config) -> Result<()> {
    print!("  Paste magnet link or torrent URL: ");
    io::stdout().flush()?;
    let mut magnet = String::new();
    io::stdin().read_line(&mut magnet)?;
    let magnet = magnet.trim();
    if magnet.is_empty() {
        return Ok(());
    }

    println!("  {} Sending to Seedr cloud...", "•".cyan());
    let id = client.add_magnet(magnet).await?;
    let folder = client.wait_for_caching(id, "Torrent").await?;

    if let Some(f) = folder {
        print!("  Download and ingest into Jellyfin now? [Y/n]: ");
        io::stdout().flush()?;
        let mut ans = String::new();
        io::stdin().read_line(&mut ans)?;
        let t = ans.trim().to_lowercase();
        if t.is_empty() || t == "y" || t == "yes" {
            download_and_ingest(client, cfg, &f, false).await?;
        }
    }
    Ok(())
}

async fn handle_delete_by_index(
    client: &SeedrClient,
    folders: &[SeedrFolder],
    rest: &str,
) -> Result<()> {
    let idx: usize = rest.trim().parse().unwrap_or(0);
    if idx == 0 || idx > folders.len() {
        println!("  {} Invalid folder number: {rest}", "✖".red());
        return Ok(());
    }
    let folder = &folders[idx - 1];
    print!("  Delete '{}' from cloud? [y/N]: ", folder.name);
    io::stdout().flush()?;
    let mut ans = String::new();
    io::stdin().read_line(&mut ans)?;
    if ans.trim().eq_ignore_ascii_case("y") {
        client.delete_folder(folder.id).await?;
        println!(
            "  {} Deleted '{}' from Seedr cloud.",
            "✔".green(),
            folder.name
        );
    }
    Ok(())
}

async fn handle_delete_torrent(
    client: &SeedrClient,
    torrents: &[crate::seedr::SeedrTorrent],
    rest: &str,
) -> Result<()> {
    let idx: usize = rest.trim().parse().unwrap_or(0);
    if idx == 0 || idx > torrents.len() {
        println!("  {} Invalid torrent number: {rest}", "✖".red());
        return Ok(());
    }
    let t = &torrents[idx - 1];
    print!("  Cancel and delete torrent '{}'? [y/N]: ", t.name);
    io::stdout().flush()?;
    let mut ans = String::new();
    io::stdin().read_line(&mut ans)?;
    if ans.trim().eq_ignore_ascii_case("y") {
        client.delete_torrent(t.id).await?;
        println!("  {} Deleted torrent '{}' from cloud.", "✔".green(), t.name);
    }
    Ok(())
}

async fn handle_clean_all(client: &SeedrClient, folders: &[SeedrFolder]) -> Result<()> {
    if folders.is_empty() {
        println!("  {} No completed folders to delete.", "•".dimmed());
        return Ok(());
    }
    print!(
        "  Delete all {} cloud folders to free space? [y/N]: ",
        folders.len()
    );
    io::stdout().flush()?;
    let mut ans = String::new();
    io::stdin().read_line(&mut ans)?;
    if ans.trim().eq_ignore_ascii_case("y") {
        client.delete_all_folders(folders).await?;
        println!("  {} All cloud folders deleted!", "✔".green());
    }
    Ok(())
}

pub async fn download_and_ingest(
    client: &SeedrClient,
    cfg: &Config,
    folder: &SeedrFolder,
    non_interactive: bool,
) -> Result<()> {
    let contents = client.list_folder(folder.id).await?;
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

        organizer::organize_file(
            &downloaded_path,
            &info,
            &cfg.jellyfin_media_dir,
            non_interactive,
        )?;
    }

    print!("  Delete item from Seedr cloud to free space? [y/N]: ");
    io::stdout().flush()?;
    let mut del_input = String::new();
    io::stdin().read_line(&mut del_input)?;
    if del_input.trim().eq_ignore_ascii_case("y") {
        client.delete_folder(folder.id).await?;
        println!("  {} Cloud item deleted.", "✔".green());
    }

    Ok(())
}
