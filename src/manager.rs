use crate::config::Config;
use crate::modal::Modal;
use crate::organizer::download_and_ingest;
use crate::seedr::SeedrClient;
use crate::ui::{self, AppState};
use anyhow::Result;
use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout, Write};

struct TuiGuard;
impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

pub async fn run_dashboard(
    client: &SeedrClient,
    cfg: &Config,
    non_interactive: bool,
) -> Result<()> {
    let list = client.list_root().await?;
    if non_interactive {
        let state = AppState {
            list,
            selected: 0,
            modal: None,
            status: None,
        };
        let mut term = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        term.draw(|f| ui::draw_ui(f, &state))?;
        return Ok(());
    }

    let _guard = TuiGuard;
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let mut state = AppState {
        list,
        selected: 0,
        modal: None,
        status: None,
    };

    run_event_loop(&mut term, &mut state, client, cfg).await?;
    Ok(())
}

async fn run_event_loop(
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
) -> Result<()> {
    loop {
        if state.selected >= state.list.folders.len() && !state.list.folders.is_empty() {
            state.selected = state.list.folders.len() - 1;
        }
        term.draw(|f| ui::draw_ui(f, state))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_input(term, state, client, cfg, key).await? {
                break;
            }
        }
    }
    Ok(())
}

async fn handle_input(
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    key: KeyEvent,
) -> Result<bool> {
    if let Some(modal) = state.modal.clone() {
        handle_modal_key(state, client, cfg, term, modal, key.code).await
    } else {
        handle_main_key(state, client, cfg, term, key.code).await
    }
}

async fn handle_main_key(
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    code: KeyCode,
) -> Result<bool> {
    state.status = None;
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.list.folders.is_empty() && state.selected + 1 < state.list.folders.len() {
                state.selected += 1;
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            trigger_download(state, client, cfg, term).await?;
        }
        KeyCode::Char('d' | 'x') | KeyCode::Delete => {
            if let Some(f) = state.list.folders.get(state.selected) {
                state.modal = Some(Modal::ConfirmDelete(f.clone()));
            }
        }
        KeyCode::Char('t') => {
            if let Some(t) = state.list.torrents.first() {
                state.modal = Some(Modal::ConfirmCancelTorrent(t.clone()));
            }
        }
        KeyCode::Char('a') => {
            state.modal = Some(Modal::InputMagnet(String::new()));
        }
        KeyCode::Char('c') => {
            if !state.list.folders.is_empty() {
                state.modal = Some(Modal::ConfirmCleanAll(state.list.folders.len()));
            }
        }
        KeyCode::Char('r') => {
            state.list = client.list_root().await?;
            state.status = Some(("Refreshed Seedr cloud data.".to_string(), false));
        }
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        _ => {}
    }
    Ok(false)
}

async fn handle_modal_key(
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    modal: Modal,
    code: KeyCode,
) -> Result<bool> {
    match modal {
        Modal::ConfirmDelete(folder) => {
            if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
                client.delete_folder(folder.id).await?;
                state.list = client.list_root().await?;
                state.status = Some((format!("Deleted '{}'", folder.name), false));
                state.modal = None;
            } else if matches!(code, KeyCode::Esc | KeyCode::Char('n' | 'N')) {
                state.modal = None;
            }
        }
        Modal::ConfirmCancelTorrent(torrent) => {
            if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
                client.delete_torrent(torrent.id).await?;
                state.list = client.list_root().await?;
                state.status = Some((format!("Cancelled '{}'", torrent.name), false));
                state.modal = None;
            } else if matches!(code, KeyCode::Esc | KeyCode::Char('n' | 'N')) {
                state.modal = None;
            }
        }
        Modal::ConfirmCleanAll(_) => {
            if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
                client.delete_all_folders(&state.list.folders).await?;
                state.list = client.list_root().await?;
                state.status = Some(("All cloud folders deleted.".to_string(), false));
                state.modal = None;
            } else if matches!(code, KeyCode::Esc | KeyCode::Char('n' | 'N')) {
                state.modal = None;
            }
        }
        Modal::InputMagnet(mut text) => match code {
            KeyCode::Enter => {
                state.modal = None;
                if !text.is_empty() {
                    execute_add_magnet(state, client, cfg, term, &text).await?;
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
        },
    }
    Ok(false)
}

async fn trigger_download(
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    if let Some(folder) = state.list.folders.get(state.selected).cloned() {
        pause_tui()?;
        download_and_ingest(client, cfg, &folder, false).await?;
        resume_tui(term)?;
        state.list = client.list_root().await?;
        state.status = Some((format!("Ingested '{}'", folder.name), false));
    }
    Ok(())
}

async fn execute_add_magnet(
    state: &mut AppState,
    client: &SeedrClient,
    cfg: &Config,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    magnet: &str,
) -> Result<()> {
    pause_tui()?;
    println!("  {} Sending magnet to Seedr cloud...", "•".cyan());
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
    resume_tui(term)?;
    state.list = client.list_root().await?;
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
