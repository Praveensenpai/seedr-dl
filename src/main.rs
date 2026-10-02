mod history_cli;
mod task;
mod worker;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use seedr_dl::config::{load_auth, load_config, save_config, Config};
use seedr_dl::notifier::{NotificationEvent, Notifier, NotifierConfig};
use seedr_dl::seedr::{CachingQuery, ListContentsResponse, SeedrClient, SeedrFolder, SeedrTorrent};
use seedr_dl::transfer::TransferOptions;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "seedr-dl")]
#[command(author = "Praveensenpai <pvnt20@gmail.com>")]
#[command(version)]
#[command(about = "Pure Rust CLI Seedr.cc downloader with webhooks and multi-chunk streaming")]
struct Cli {
    /// Magnet link, torrent URL, or cloud folder ID to download
    #[arg(value_name = "TARGET")]
    target: Option<String>,

    /// Destination directory for downloaded files
    #[arg(short = 'o', long = "output", global = true)]
    output: Option<PathBuf>,

    /// Webhook URL to receive HTTP POST progress and completion notifications
    #[arg(long, global = true)]
    callback_url: Option<String>,

    /// Emit events as single-line JSON (`ndjson`) to stdout
    #[arg(long, global = true)]
    json: bool,

    /// Skip confirmation prompts
    #[arg(short = 'y', long = "yes", global = true)]
    yes: bool,

    /// Keep item in Seedr cloud after download
    #[arg(long, global = true)]
    keep: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with Seedr.cc
    Auth,
    /// Download a magnet link, torrent URL, or existing cloud folder
    Download {
        /// Magnet link, torrent URL, or folder ID
        target: String,
        /// Run in background as an asynchronous task
        #[arg(long)]
        background: bool,
    },
    /// List files and folders currently in your Seedr cloud
    List,
    /// Delete a folder or cancel a torrent in your Seedr cloud
    Delete {
        /// Folder ID or torrent ID to delete
        id: u64,
    },
    /// Clean all completed items from your Seedr cloud
    Clean,
    /// View active background downloads
    Tasks,
    /// Cancel a running background download
    Cancel {
        /// Folder ID of the background task
        folder_id: u64,
    },
    /// Configure default download directory
    Config {
        /// Set default download directory
        #[arg(long)]
        download_dir: Option<PathBuf>,
        /// Show current configuration
        #[arg(long)]
        show: bool,
    },
    /// Manage downloaded media history
    History {
        #[command(subcommand)]
        subcmd: Option<history_cli::HistoryCommands>,
    },
    #[command(hide = true)]
    Worker {
        folder_id: u64,
        #[arg(long)]
        callback_url: Option<String>,
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = load_config()?;

    match cli.command {
        Some(Commands::Worker {
            folder_id,
            callback_url,
            output_dir,
        }) => worker::run_worker(folder_id, callback_url, output_dir).await,
        Some(Commands::Tasks) => {
            task::print_active_tasks(cli.json);
            Ok(())
        }
        Some(Commands::Cancel { folder_id }) => {
            task::cancel_task(folder_id);
            println!("  {} Cancelled task for folder {folder_id}", "✔".green());
            Ok(())
        }
        Some(Commands::Auth) => {
            seedr_dl::interactive_auth().await?;
            Ok(())
        }
        Some(Commands::List) => handle_list(cli.json).await,
        Some(Commands::Delete { id }) => handle_delete(id, cli.yes).await,
        Some(Commands::Clean) => handle_clean(cli.yes).await,
        Some(Commands::Config { download_dir, show }) => {
            handle_config(&mut cfg, download_dir, show)
        }
        Some(Commands::History { subcmd }) => history_cli::handle_history_cli(subcmd),
        Some(Commands::Download {
            ref target,
            background,
        }) => handle_download(&cfg, target, &cli, background).await,
        None => {
            if let Some(ref target) = cli.target {
                handle_download(&cfg, target, &cli, false).await
            } else {
                handle_list(cli.json).await
            }
        }
    }
}

async fn handle_download(cfg: &Config, target: &str, cli: &Cli, bg: bool) -> Result<()> {
    let client = get_authenticated_client().await?;
    let notifier = Notifier::new(NotifierConfig {
        callback_url: cli.callback_url.clone(),
        json_output: cli.json,
        callback_fn: None,
        progress_interval_ms: 500,
    });

    let ctx = DownloadCtx {
        client: &client,
        cfg,
        cli,
        notifier: &notifier,
    };

    if let Err(err) = ctx.execute(target, bg).await {
        notifier.notify(NotificationEvent::Failed {
            folder_id: 0,
            file_name: None,
            error: format!("{err:#}"),
        });
        notifier.flush().await;
        return Err(err);
    }
    Ok(())
}

struct DownloadCtx<'a> {
    client: &'a SeedrClient,
    cfg: &'a Config,
    cli: &'a Cli,
    notifier: &'a Notifier,
}

impl DownloadCtx<'_> {
    async fn dispatch(&self, folder: &SeedrFolder, bg: bool) -> Result<()> {
        let out_dir = self
            .cli
            .output
            .clone()
            .unwrap_or_else(|| self.cfg.download_dir.clone());

        if bg {
            let size = folder.size.unwrap_or(0);
            worker::spawn_worker(&worker::WorkerSpawnOpts {
                folder_id: folder.id,
                folder_name: &folder.name,
                file_size: size,
                callback_url: self.cli.callback_url.as_deref(),
                output_dir: Some(&out_dir),
            })?;
            if !self.cli.json {
                println!(
                    "  {} Background download started for: {}",
                    "✔".green(),
                    folder.name
                );
            }
            return Ok(());
        }

        let opts = TransferOptions {
            output_dir: out_dir,
            delete_cloud: !self.cli.keep,
            notifier: Some(self.notifier),
        };
        seedr_dl::transfer_download_folder(self.client, self.cfg, folder, &opts).await?;
        Ok(())
    }

    async fn execute(&self, target: &str, bg: bool) -> Result<()> {
        if let Ok(folder_id) = target.parse::<u64>() {
            let root = self.client.list_root().await?;
            let existing_name = root
                .folders
                .iter()
                .find(|f| f.id == folder_id)
                .map(|f| f.name.clone());
            let items = seedr_dl::collect_folder_files(self.client, folder_id).await?;
            let total_size = items.iter().map(|f| f.file.size).sum::<u64>();
            let folder = SeedrFolder {
                id: folder_id,
                name: existing_name.unwrap_or_else(|| format!("folder-{folder_id}")),
                size: Some(total_size),
            };
            return self.dispatch(&folder, bg).await;
        }

        if !self.cli.json {
            println!("  {} Adding torrent to Seedr cloud...", "•".cyan());
        }
        self.notifier.notify(NotificationEvent::Status {
            folder_id: 0,
            file_name: None,
            status: "caching".into(),
            message: "Adding torrent to Seedr cloud".into(),
        });

        let root_before = self.client.list_root().await?;
        let prev_ids: Vec<u64> = root_before.folders.iter().map(|f| f.id).collect();
        let name_hint = seedr_dl::extract_magnet_name(target);
        let existing = name_hint.as_ref().and_then(|name| {
            root_before.folders.iter().find(|f| f.name == *name).cloned()
        });

        let folder = if let Some(f) = existing {
            f
        } else {
            if !root_before.folders.is_empty() {
                let _ = self.client.delete_all_folders(&root_before.folders).await;
            }
            for t in &root_before.torrents {
                let _ = self.client.delete_torrent(t.id).await;
            }
            let torrent_id = self.client.add_magnet(target).await?;
            let display_name = name_hint.clone().unwrap_or_else(|| format!("torrent-{torrent_id}"));
            task::register_caching_task(torrent_id, &display_name);

            let mut on_progress = |t: &SeedrTorrent| {
                task::update_caching_progress(torrent_id, t);
            };

            let query = CachingQuery {
                torrent_id,
                name: name_hint.as_deref(),
                previous_folders: &prev_ids,
            };
            let folder_res = self
                .client
                .wait_for_caching(&query, Some(&mut on_progress))
                .await;
            task::remove_task(torrent_id);
            folder_res?.context("Could not find completed folder in Seedr cloud")?
        };

        self.dispatch(&folder, bg).await
    }
}

async fn handle_list(json: bool) -> Result<()> {
    let client = get_authenticated_client().await?;
    let list = client.list_root().await?;

    if json {
        let out = serde_json::to_string_pretty(&list)?;
        println!("{out}");
        return Ok(());
    }

    print_cloud_contents(&list);
    Ok(())
}

fn print_cloud_contents(list: &ListContentsResponse) {
    println!("  {}", "Seedr Cloud Contents:".cyan().bold());
    if list.torrents.is_empty() && list.folders.is_empty() {
        println!("  (Cloud is empty)");
        return;
    }
    for t in &list.torrents {
        let pct = t.progress.unwrap_or(0.0);
        println!("  • [Torrent {}] {} ({:.1}%)", t.id, t.name, pct);
    }
    for f in &list.folders {
        let sz_mb = f.size.unwrap_or(0) / 1_048_576;
        println!("  • [Folder {}] {} ({} MB)", f.id, f.name, sz_mb);
    }
}

async fn handle_delete(id: u64, non_interactive: bool) -> Result<()> {
    let client = get_authenticated_client().await?;
    if !non_interactive && !confirm_action("Delete cloud item?")? {
        println!("  Cancelled.");
        return Ok(());
    }
    if client.delete_folder(id).await.is_err() {
        client.delete_torrent(id).await?;
    }
    println!("  {} Cloud item {id} deleted.", "✔".green());
    Ok(())
}

async fn handle_clean(non_interactive: bool) -> Result<()> {
    let client = get_authenticated_client().await?;
    let list = client.list_root().await?;
    if list.folders.is_empty() {
        println!("  No completed folders to clean.");
        return Ok(());
    }
    if !non_interactive && !confirm_action("Delete all completed cloud folders?")? {
        println!("  Cancelled.");
        return Ok(());
    }
    client.delete_all_folders(&list.folders).await?;
    println!("  {} All completed cloud folders deleted.", "✔".green());
    Ok(())
}

async fn get_authenticated_client() -> Result<SeedrClient> {
    if let Some(auth) = load_auth()? {
        return Ok(SeedrClient::new(auth.access_token));
    }
    println!("  {} No active Seedr credentials found. Let's log in!", "•".yellow());
    let auth = seedr_dl::interactive_auth().await?;
    Ok(SeedrClient::new(auth.access_token))
}

fn handle_config(cfg: &mut Config, download_dir: Option<PathBuf>, show: bool) -> Result<()> {
    if show || download_dir.is_none() {
        println!(
            "  {}\n  • Download directory: {}",
            "Current Configuration:".cyan().bold(),
            cfg.download_dir.display()
        );
        return Ok(());
    }
    if let Some(dir) = download_dir {
        cfg.download_dir = dir;
        println!("  {} Updated default download directory.", "✔".green());
    }
    save_config(cfg)
}

fn confirm_action(prompt: &str) -> Result<bool> {
    print!("  {prompt} [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("y"))
}
