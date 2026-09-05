use crate::attach::run_attach_loop;
use crate::config::Config;
use crate::modal::Modal;
use crate::modal_handler::handle_modal_key;
use crate::seedr::SeedrClient;
use crate::task::list_active_tasks;
use crate::ui::{self, AppState};
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
    if non_interactive {
        let state = AppState {
            list,
            local_tasks,
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
        local_tasks,
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
        state.local_tasks = list_active_tasks();
        if state.selected >= state.list.folders.len() && !state.list.folders.is_empty() {
            state.selected = state.list.folders.len() - 1;
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
        handle_modal_key(state, client, cfg, term, modal, key.code).await
    } else {
        handle_main_key(state, client, cfg, term, key.code).await
    }
}

async fn handle_main_key(
    state: &mut AppState,
    client: &SeedrClient,
    _cfg: &Config,
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
                state.modal = Some(Modal::SelectDownloadMode(folder.clone()));
            }
        }
        KeyCode::Char('b') => {
            if let Some(folder) = state.list.folders.get(state.selected) {
                spawn_worker(folder.id)?;
                state.local_tasks = list_active_tasks();
                state.status = Some((
                    format!("Started background task for '{}'", folder.name),
                    false,
                ));
            }
        }
        KeyCode::Char('A') => {
            if let Some(first_task) = state.local_tasks.first() {
                run_attach_loop(term, first_task.folder_id).await?;
                state.local_tasks = list_active_tasks();
            } else {
                state.status = Some((
                    "No active background downloads to attach to.".to_string(),
                    true,
                ));
            }
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
            state.local_tasks = list_active_tasks();
            state.status = Some(("Refreshed Seedr cloud and local tasks.".to_string(), false));
        }
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        _ => {}
    }
    Ok(false)
}
