use crate::config::Config;
use crate::modal::Modal;
use crate::seedr::{SeedrClient, SeedrFolder, SeedrTorrent};
use crate::ui::AppState;
use crate::worker::spawn_worker;
use anyhow::Result;
use colored::Colorize;
use crossterm::event::KeyCode;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};

pub async fn handle_modal_key(
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    modal: Modal,
    code: KeyCode,
) -> Result<bool> {
    match modal {
        Modal::ConfirmDelete(folder) => {
            handle_delete_confirm(state, client, folder, code).await?;
        }
        Modal::ConfirmCancelTorrent(torrent) => {
            handle_cancel_torrent_confirm(state, client, torrent, code).await?;
        }
        Modal::ConfirmCleanAll(_) => {
            handle_clean_all_confirm(state, client, code).await?;
        }
        Modal::InputMagnet(text) => {
            handle_magnet_input(state, client, cfg, term, text, code).await?;
        }
    }
    Ok(false)
}

async fn handle_delete_confirm(
    state: &mut AppState,
    client: &SeedrClient,
    folder: SeedrFolder,
    code: KeyCode,
) -> Result<()> {
    if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
        client.delete_folder(folder.id).await?;
        state.list = client.list_root().await?;
        state.status = Some((format!("Deleted '{}'", folder.name), false));
    }
    state.modal = None;
    Ok(())
}

async fn handle_cancel_torrent_confirm(
    state: &mut AppState,
    client: &SeedrClient,
    torrent: SeedrTorrent,
    code: KeyCode,
) -> Result<()> {
    if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
        client.delete_torrent(torrent.id).await?;
        state.list = client.list_root().await?;
        state.status = Some((format!("Cancelled '{}'", torrent.name), false));
    }
    state.modal = None;
    Ok(())
}

async fn handle_clean_all_confirm(
    state: &mut AppState,
    client: &SeedrClient,
    code: KeyCode,
) -> Result<()> {
    if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
        client.delete_all_folders(&state.list.folders).await?;
        state.list = client.list_root().await?;
        state.status = Some(("All cloud folders deleted.".to_string(), false));
    }
    state.modal = None;
    Ok(())
}

async fn handle_magnet_input(
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    mut text: String,
    code: KeyCode,
) -> Result<()> {
    match code {
        KeyCode::Enter => {
            state.modal = None;
            if !text.is_empty() {
                execute_add_magnet(client, cfg, term, &text).await?;
                state.list = client.list_root().await?;
            }
        }
        KeyCode::Esc => state.modal = None,
        KeyCode::Backspace => {
            text.pop();
            state.modal = Some(Modal::InputMagnet(text));
        }
        KeyCode::Char(c) => {
            text.push(c);
            state.modal = Some(Modal::InputMagnet(text));
        }
        _ => {}
    }
    Ok(())
}

async fn execute_add_magnet(
    client: &SeedrClient,
    _cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    magnet: &str,
) -> Result<()> {
    pause_tui()?;
    println!("  {} Sending magnet to Seedr cloud...", "•".cyan());
    let id = client.add_magnet(magnet).await?;
    let folder = client.wait_for_caching(id, "Torrent").await?;
    resume_tui(term)?;
    if let Some(f) = folder {
        spawn_worker(f.id, &f.name, f.size.unwrap_or(0))?;
        crate::attach::run_attach_loop(term, f.id).await?;
    }
    Ok(())
}

fn pause_tui() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show)?;
    Ok(())
}

fn resume_tui(term: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    term.clear()?;
    Ok(())
}
