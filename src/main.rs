mod config;
mod downloader;
mod gemini;
mod organizer;
mod seedr;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use config::{
    cache_dir, get_gemini_key, load_auth, load_config, save_auth, save_config, Auth, Config,
};
use downloader::Downloader;
use seedr::SeedrClient;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "seedr-dl")]
#[command(author = "Praveensenpai <pvnt20@gmail.com>")]
#[command(version = "0.1.0")]
#[command(about = "Pure Rust Seedr.cc downloader with Gemini AI renaming for Jellyfin")]
struct Cli {
    /// Magnet link or torrent URL to download
    #[arg(value_name = "MAGNET_OR_URL")]
    magnet: Option<String>,

    /// Skip confirmation prompts
    #[arg(short = 'y', long = "yes", global = true)]
    yes: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with Seedr.cc
    Auth,
    /// List files and folders currently in your Seedr cloud
    List,
    /// Configure Jellyfin path or Gemini API key
    Config {
        /// Set Gemini API Key
        #[arg(long)]
        gemini_key: Option<String>,
        /// Set Jellyfin media root directory
        #[arg(long)]
        media_dir: Option<PathBuf>,
        /// Show current configuration
        #[arg(long)]
        show: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = load_config()?;

    match cli.command {
        Some(Commands::Auth) => {
            handle_auth().await?;
        }
        Some(Commands::List) => handle_list(&cfg, cli.yes).await?,
        Some(Commands::Config {
            gemini_key,
            media_dir,
            show,
        }) => {
            handle_config(&mut cfg, gemini_key, media_dir, show)?;
        }
        None => {
            if let Some(magnet) = cli.magnet {
                handle_download_magnet(&cfg, &magnet, cli.yes).await?;
            } else {
                handle_list(&cfg, cli.yes).await?;
            }
        }
    }
    Ok(())
}

async fn get_authenticated_client() -> Result<SeedrClient> {
    if let Some(auth) = load_auth()? {
        return Ok(SeedrClient::new(auth.access_token));
    }
    println!(
        "  {} No active Seedr credentials found. Let's log in!",
        "•".yellow()
    );
    let auth = handle_auth().await?;
    Ok(SeedrClient::new(auth.access_token))
}

async fn handle_auth() -> Result<Auth> {
    print!("  Enter Seedr email: ");
    io::stdout().flush()?;
    let mut email = String::new();
    io::stdin().read_line(&mut email)?;

    print!("  Enter Seedr password: ");
    io::stdout().flush()?;
    let mut pass = String::new();
    io::stdin().read_line(&mut pass)?;

    let email = email.trim();
    let pass = pass.trim();

    if email.is_empty() || pass.is_empty() {
        bail!("Email and password cannot be empty");
    }

    println!("  {} Authenticating with Seedr.cc...", "•".dimmed());
    let auth = SeedrClient::login(email, pass).await?;
    save_auth(&auth)?;
    println!(
        "  {} Successfully authenticated and saved token!",
        "✔".green().bold()
    );
    Ok(auth)
}

fn handle_config(
    cfg: &mut Config,
    gemini_key: Option<String>,
    media_dir: Option<PathBuf>,
    show: bool,
) -> Result<()> {
    if show || (gemini_key.is_none() && media_dir.is_none()) {
        println!("  {}", "Current Configuration:".cyan().bold());
        println!(
            "  • Jellyfin media path: {}",
            cfg.jellyfin_media_dir.display()
        );
        println!(
            "  • Gemini API key:      {}",
            cfg.gemini_api_key
                .as_deref()
                .unwrap_or("[Not Set - Using Regex]")
        );
        return Ok(());
    }

    if let Some(k) = gemini_key {
        cfg.gemini_api_key = Some(k.trim().to_string());
        println!("  {} Gemini API key updated.", "✔".green());
    }
    if let Some(m) = media_dir {
        cfg.jellyfin_media_dir = m;
        println!("  {} Jellyfin media path updated.", "✔".green());
    }
    save_config(cfg)?;
    Ok(())
}

async fn handle_download_magnet(cfg: &Config, magnet: &str, non_interactive: bool) -> Result<()> {
    let client = get_authenticated_client().await?;
    println!("  {} Sending magnet link to Seedr cloud...", "•".cyan());
    let torrent_id = client.add_magnet(magnet).await?;

    let folder = client
        .wait_for_caching(torrent_id, "Torrent")
        .await?
        .context("Could not find completed folder in Seedr cloud")?;

    download_and_organize_folder(&client, cfg, &folder, non_interactive).await
}

async fn handle_list(cfg: &Config, non_interactive: bool) -> Result<()> {
    let client = get_authenticated_client().await?;
    let list = client.list_root().await?;

    println!();
    println!("  {}", "📁 Seedr Cloud Contents:".cyan().bold());
    if list.folders.is_empty() && list.files.is_empty() {
        println!("  {} No completed files in Seedr cloud.", "•".dimmed());
        return Ok(());
    }

    if !list.torrents.is_empty() {
        println!("  {}", "Active Seedr Caching:".yellow());
        for t in &list.torrents {
            let pct = t.progress.unwrap_or(0.0);
            let sz = t.size.unwrap_or(0) / 1024 / 1024;
            println!("  • ⏳ {} ({} MB, {:.1}%)", t.name, sz, pct);
        }
    }

    for (idx, f) in list.folders.iter().enumerate() {
        let sz = f.size.unwrap_or(0) / 1024 / 1024;
        println!("  [{}] 📁 {} ({} MB)", idx + 1, f.name.bold(), sz);
    }

    print!("\nEnter item number to download and ingest into Jellyfin (or 0 to cancel): ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    let num: usize = choice.trim().parse().unwrap_or(0);

    if num > 0 && num <= list.folders.len() {
        let folder = &list.folders[num - 1];
        download_and_organize_folder(&client, cfg, folder, non_interactive).await?;
    }
    Ok(())
}

async fn download_and_organize_folder(
    client: &SeedrClient,
    cfg: &Config,
    folder: &seedr::SeedrFolder,
    non_interactive: bool,
) -> Result<()> {
    let contents = client.list_folder(folder.id).await?;
    let gemini_key = get_gemini_key(cfg);
    let downloader = Downloader::new();
    let temp_dir = cache_dir();

    for file in &contents.files {
        let file_id = file.folder_file_id.or(file.id).context("File ID missing")?;
        let sz_mb = file.size / 1024 / 1024;
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
