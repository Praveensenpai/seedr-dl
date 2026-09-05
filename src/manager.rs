use crate::attach::run_attach_loop;
use crate::config::{get_gemini_key, Config};
use crate::gemini;
use crate::history;
use crate::modal::Modal;
use crate::modal_handler::handle_modal_key;
use crate::seedr::SeedrClient;
use crate::task::list_active_tasks;
use crate::ui::{self, AppState, ViewMode};
use crate::worker::spawn_worker;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};

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
    let local_tasks = list_active_tasks();
    let history_entries = history::load_history().unwrap_or_default();
    if non_interactive {
        let state = AppState {
            list,
            local_tasks,
            history_entries,
            selected: 0,
            history_selected: 0,
            view_mode: ViewMode::Cloud,
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
        local_tasks,
        history_entries,
        selected: 0,
        history_selected: 0,
        view_mode: ViewMode::Cloud,
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
        state.local_tasks = list_active_tasks();
        if state.selected >= state.list.folders.len() && !state.list.folders.is_empty() {
            state.selected = state.list.folders.len() - 1;
        }
        if state.history_selected >= state.history_entries.len()
            && !state.history_entries.is_empty()
        {
            state.history_selected = state.history_entries.len() - 1;
        }
        term.draw(|f| ui::draw_ui(f, state))?;

        if event::poll(std::time::Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if handle_input(term, state, client, cfg, key).await? {
                    break;
                }
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
        return handle_modal_key(state, client, cfg, term, modal, key.code).await;
    }

    if key.code == KeyCode::Tab {
        state.view_mode = match state.view_mode {
            ViewMode::Cloud => {
                state.history_entries = history::load_history().unwrap_or_default();
                ViewMode::History
            }
            ViewMode::History => ViewMode::Cloud,
        };
        state.status = None;
        return Ok(false);
    }

    match state.view_mode {
        ViewMode::Cloud => handle_cloud_key(state, client, term, key.code).await,
        ViewMode::History => handle_history_key(state, cfg, key.code).await,
    }
}

async fn handle_cloud_key(
    state: &mut AppState,
    client: &SeedrClient,
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
        KeyCode::Enter => {
            if let Some(folder) = state.list.folders.get(state.selected) {
                let is_running = state.local_tasks.iter().any(|t| t.folder_id == folder.id);
                if !is_running {
                    if let Err(e) = spawn_worker(folder.id, &folder.name, folder.size.unwrap_or(0))
                    {
                        state.status = Some((format!("Failed to start download: {e}"), true));
                        return Ok(false);
                    }
                }
                run_attach_loop(term, folder.id).await?;
                state.local_tasks = list_active_tasks();
                state.list = client.list_root().await?;
            }
        }
        KeyCode::Char('a' | 'A') => {
            handle_attach_shortcut(state, client, term).await?;
        }
        KeyCode::Char('m' | '+') => {
            state.modal = Some(Modal::InputMagnet(String::new()));
        }
        KeyCode::Char('t') => {
            if let Some(t) = state.list.torrents.first() {
                state.modal = Some(Modal::ConfirmCancelTorrent(t.clone()));
            }
        }
        KeyCode::Char('d' | 'x') | KeyCode::Delete => {
            if let Some(f) = state.list.folders.get(state.selected) {
                state.modal = Some(Modal::ConfirmDelete(f.clone()));
            }
        }
        KeyCode::Char('c') => {
            if !state.list.folders.is_empty() {
                state.modal = Some(Modal::ConfirmCleanAll(state.list.folders.len()));
            }
        }
        KeyCode::Char('r') => {
            state.list = client.list_root().await?;
            state.local_tasks = list_active_tasks();
            state.status = Some(("Refreshed Seedr cloud and local tasks.".to_string(), false));
        }
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        _ => {}
    }
    Ok(false)
}

async fn handle_attach_shortcut(
    state: &mut AppState,
    client: &SeedrClient,
    term: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    let target_id = state
        .list
        .folders
        .get(state.selected)
        .map(|f| f.id)
        .filter(|id| state.local_tasks.iter().any(|t| t.folder_id == *id))
        .or_else(|| state.local_tasks.first().map(|t| t.folder_id));

    if let Some(fid) = target_id {
        run_attach_loop(term, fid).await?;
        state.local_tasks = list_active_tasks();
        state.list = client.list_root().await?;
    } else {
        state.status = Some((
            "No active downloads running to attach to.".to_string(),
            false,
        ));
    }
    Ok(())
}

async fn handle_history_key(state: &mut AppState, cfg: &Config, code: KeyCode) -> Result<bool> {
    state.status = None;
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.history_selected = state.history_selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.history_entries.is_empty()
                && state.history_selected + 1 < state.history_entries.len()
            {
                state.history_selected += 1;
            }
        }
        KeyCode::Char('r' | 'R') => {
            trigger_history_ai_rename(state, cfg).await;
        }
        KeyCode::Char('m' | 'M') => {
            trigger_history_manual_rename(state);
        }
        KeyCode::Char('d' | 'D' | 'x' | 'X') | KeyCode::Delete => {
            trigger_history_delete(state);
        }
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        _ => {}
    }
    Ok(false)
}

async fn trigger_history_ai_rename(state: &mut AppState, cfg: &Config) {
    let Some(entry) = state.history_entries.get(state.history_selected).cloned() else {
        return;
    };
    let Some(key) = get_gemini_key(cfg) else {
        state.status = Some((
            "Gemini API key not set. Use [m] for manual rename.".to_string(),
            true,
        ));
        return;
    };

    state.status = Some(("Querying Gemini AI...".to_string(), false));
    match gemini::query_gemini(&entry.original_name, &key).await {
        Ok(new_info) => {
            let target_path = cfg
                .jellyfin_media_dir
                .join(&new_info.relative_folder)
                .join(&new_info.clean_filename);
            state.modal = Some(Modal::ConfirmAiRename {
                entry,
                new_info,
                target_path,
            });
        }
        Err(e) => {
            state.status = Some((format!("Gemini query failed: {e}"), true));
        }
    }
}

fn trigger_history_manual_rename(state: &mut AppState) {
    if let Some(entry) = state.history_entries.get(state.history_selected).cloned() {
        let yr_str = entry.year.map_or_else(String::new, |y| y.to_string());
        state.modal = Some(Modal::InputManualRename {
            entry: entry.clone(),
            title: entry.clean_title,
            year: yr_str,
            active_field: 0,
        });
    }
}

fn trigger_history_delete(state: &mut AppState) {
    if let Some(entry) = state.history_entries.get(state.history_selected).cloned() {
        state.modal = Some(Modal::ConfirmDeleteMedia {
            entry,
            typed: String::new(),
        });
    }
}
