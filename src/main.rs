mod config;
mod downloader;
mod gemini;
mod manager;
mod modal;
mod organizer;
mod seedr;
mod ui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use config::{load_auth, load_config, save_auth, save_config, Auth, Config};
use manager::run_dashboard;
use seedr::SeedrClient;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "seedr-dl")]
#[command(author = "Praveensenpai <pvnt20@gmail.com>")]
#[command(version = "0.2.0")]
#[command(about = "Seedr Cloud Manager with Gemini AI Ingestion for Jellyfin")]
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
    /// Clean all completed items from your Seedr cloud
    Clean,
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
        Some(Commands::List) => {
            let client = get_authenticated_client().await?;
            run_dashboard(&client, &cfg, true).await?;
        }
        Some(Commands::Clean) => {
            let client = get_authenticated_client().await?;
            let list = client.list_root().await?;
            client.delete_all_folders(&list.folders).await?;
            println!("  {} All completed cloud folders deleted.", "✔".green());
        }
        Some(Commands::Config {
            gemini_key,
            media_dir,
            show,
        }) => {
            handle_config(&mut cfg, gemini_key, media_dir, show)?;
        }
        None => {
            let client = get_authenticated_client().await?;
            if let Some(magnet) = cli.magnet {
                handle_direct_magnet(&client, &cfg, &magnet, cli.yes).await?;
            } else {
                run_dashboard(&client, &cfg, cli.yes).await?;
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

async fn handle_direct_magnet(
    client: &SeedrClient,
    cfg: &Config,
    magnet: &str,
    non_interactive: bool,
) -> Result<()> {
    println!("  {} Sending magnet link to Seedr cloud...", "•".cyan());
    let torrent_id = client.add_magnet(magnet).await?;

    let folder = client
        .wait_for_caching(torrent_id, "Torrent")
        .await?
        .context("Could not find completed folder in Seedr cloud")?;

    organizer::download_and_ingest(client, cfg, &folder, non_interactive).await
}
