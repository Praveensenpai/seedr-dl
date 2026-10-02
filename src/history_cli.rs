//! CLI interface for download history inspection.

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use seedr_dl::history::{clear_history, load_history, media_file_exists};

#[derive(Subcommand, Debug)]
pub enum HistoryCommands {
    /// List all downloaded items in history
    List,
    /// Clear all download history
    Clear,
}

/// Handles history subcommands from the CLI.
///
/// # Errors
/// Returns an error if history loading or clearing fails.
pub fn handle_history_cli(cmd: Option<HistoryCommands>) -> Result<()> {
    match cmd.unwrap_or(HistoryCommands::List) {
        HistoryCommands::List => list_history(),
        HistoryCommands::Clear => {
            clear_history()?;
            println!("  {} Download history cleared.", "✔".green());
            Ok(())
        }
    }
}

fn list_history() -> Result<()> {
    let entries = load_history()?;
    if entries.is_empty() {
        println!("  {}", "No download history found.".yellow());
        return Ok(());
    }

    println!("  {}", "Download History:".cyan().bold());
    println!();
    for e in &entries {
        let exists = media_file_exists(&e.file_path);
        let exists_badge = if exists {
            "[on disk]".green()
        } else {
            "[missing]".red()
        };
        let size_mb = e.file_size / 1_048_576;
        println!(
            "  • [{}] {} ({} MB) {}",
            e.id.dimmed(),
            e.original_name.bold(),
            size_mb,
            exists_badge
        );
        println!("    Path: {}", e.file_path.display().to_string().dimmed());
        println!("    Date: {}", e.downloaded_at.dimmed());
        println!();
    }
    Ok(())
}
