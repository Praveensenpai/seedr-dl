mod attach;
mod config;
mod downloader;
mod gemini;
mod manager;
mod modal;
mod modal_handler;
mod organizer;
mod seedr;
mod task;
mod ui;
mod worker;

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
#[command(version)]
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
    /// View active background downloads
    Tasks,
    /// Attach to a running background download
    Attach {
        /// Optional folder ID to attach to
        folder_id: Option<u64>,
    },
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
    #[command(hide = true)]
    Worker { folder_id: u64 },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = load_config()?;

    match cli.command {
        Some(Commands::Worker { folder_id }) => {
            worker::run_worker(folder_id).await?;
        }
        Some(Commands::Tasks) => {
            handle_list_tasks();
        }
        Some(Commands::Attach { folder_id }) => {
            handle_cli_attach(folder_id).await?;
        }
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

fn handle_list_tasks() {
    let tasks = task::list_active_tasks();
    if tasks.is_empty() {
        println!("  No active background ingestion tasks.");
        return;
    }
    println!("  Active Background Ingestion Tasks:");
    for t in &tasks {
        let dl_mb = t.downloaded_bytes / 1_048_576;
        let tot_mb = t.total_bytes / 1_048_576;
        // reason: byte values fit safely in f64
        #[allow(clippy::cast_precision_loss)]
        let pct = if t.total_bytes > 0 {
            (t.downloaded_bytes as f64 / t.total_bytes as f64) * 100.0
        } else {
            0.0
        };
        // reason: speed_bps fits safely in f64
        #[allow(clippy::cast_precision_loss)]
        let spd = t.speed_bps as f64 / 1_048_576.0;
        println!(
            "  • [{}] {} — {}/{} MB ({:.1}%, {:.2} MB/s, ETA {}s)",
            t.folder_id, t.folder_name, dl_mb, tot_mb, pct, spd, t.eta_seconds
        );
    }
}

async fn handle_cli_attach(folder_id_opt: Option<u64>) -> Result<()> {
    let tasks = task::list_active_tasks();
    let folder_id = match folder_id_opt {
        Some(id) => id,
        None => {
            if let Some(first) = tasks.first() {
                first.folder_id
            } else {
                bail!("No active background downloads to attach to.");
            }
        }
    };
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    attach::run_attach_loop(&mut term, folder_id).await?;
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
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
            if cfg.gemini_api_key.is_some() {
                "Configured".green()
            } else {
                "Not set (using offline regex fallback)".yellow()
            }
        );
        return Ok(());
    }

    if let Some(key) = gemini_key {
        cfg.gemini_api_key = Some(key);
        println!("  {} Updated Gemini API key.", "✔".green());
    }

    if let Some(dir) = media_dir {
        cfg.jellyfin_media_dir = dir;
        println!("  {} Updated Jellyfin media path.", "✔".green());
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
