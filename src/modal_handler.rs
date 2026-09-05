use crate::config::Config;
use crate::gemini::{self, MediaInfo};
use crate::history::{self, HistoryEntry, RenameStatus};
use crate::modal::Modal;
use crate::organizer::{delete_media, reorganize_media};
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
        Modal::ConfirmAiRename {
            entry,
            new_info,
            target_path: _,
        } => {
            handle_ai_rename_confirm(state, cfg, entry, &new_info, code)?;
        }
        Modal::InputManualRename {
            entry,
            title,
            year,
            active_field,
        } => {
            handle_manual_rename_input(state, cfg, entry, title, year, active_field, code)?;
        }
        Modal::ConfirmDeleteMedia { entry, typed } => {
            handle_delete_media_confirm(state, cfg, entry, typed, code)?;
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

fn handle_ai_rename_confirm(
    state: &mut AppState,
    cfg: &Config,
    mut entry: HistoryEntry,
    new_info: &MediaInfo,
    code: KeyCode,
) -> Result<()> {
    if matches!(code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
        match reorganize_media(
            &mut entry,
            new_info,
            &cfg.jellyfin_media_dir,
            RenameStatus::Gemini,
        ) {
            Ok(_) => {
                state.history_entries = history::load_history()?;
                state.status = Some((format!("Renamed to '{}'", new_info.title), false));
            }
            Err(e) => {
                state.status = Some((format!("Rename failed: {e}"), true));
            }
        }
    }
    state.modal = None;
    Ok(())
}

fn handle_manual_rename_input(
    state: &mut AppState,
    cfg: &Config,
    mut entry: HistoryEntry,
    mut title: String,
    mut year: String,
    mut active_field: usize,
    code: KeyCode,
) -> Result<()> {
    match code {
        KeyCode::Tab => {
            active_field = (active_field + 1) % 2;
            state.modal = Some(Modal::InputManualRename {
                entry,
                title,
                year,
                active_field,
            });
        }
        KeyCode::Backspace => {
            if active_field == 0 {
                title.pop();
            } else {
                year.pop();
            }
            state.modal = Some(Modal::InputManualRename {
                entry,
                title,
                year,
                active_field,
            });
        }
        KeyCode::Char(c) => {
            if active_field == 0 {
                title.push(c);
            } else if c.is_ascii_digit() && year.len() < 4 {
                year.push(c);
            }
            state.modal = Some(Modal::InputManualRename {
                entry,
                title,
                year,
                active_field,
            });
        }
        KeyCode::Enter => {
            state.modal = None;
            let y = year.trim().parse::<u32>().ok();
            let s_ep = match (entry.season, entry.episode) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => None,
            };
            let new_info = gemini::media_info_from_manual(&entry.original_name, &title, y, s_ep);
            match reorganize_media(
                &mut entry,
                &new_info,
                &cfg.jellyfin_media_dir,
                RenameStatus::Manual,
            ) {
                Ok(_) => {
                    state.history_entries = history::load_history()?;
                    state.status = Some((format!("Renamed to '{title}'"), false));
                }
                Err(e) => {
                    state.status = Some((format!("Rename failed: {e}"), true));
                }
            }
        }
        KeyCode::Esc => state.modal = None,
        _ => {}
    }
    Ok(())
}

fn handle_delete_media_confirm(
    state: &mut AppState,
    cfg: &Config,
    entry: HistoryEntry,
    mut typed: String,
    code: KeyCode,
) -> Result<()> {
    match code {
        KeyCode::Backspace => {
            typed.pop();
            state.modal = Some(Modal::ConfirmDeleteMedia { entry, typed });
        }
        KeyCode::Char(c) => {
            if typed.len() < 10 {
                typed.push(c);
            }
            state.modal = Some(Modal::ConfirmDeleteMedia { entry, typed });
        }
        KeyCode::Enter => {
            if typed == "DELETE" {
                state.modal = None;
                match delete_media(&entry, &cfg.jellyfin_media_dir) {
                    Ok(()) => {
                        state.history_entries = history::load_history()?;
                        state.status = Some((
                            format!("Permanently deleted '{}'", entry.clean_title),
                            false,
                        ));
                    }
                    Err(e) => {
                        state.status = Some((format!("Delete failed: {e}"), true));
                    }
                }
            } else {
                state.status = Some(("Type 'DELETE' to confirm deletion.".to_string(), true));
            }
        }
        KeyCode::Esc => state.modal = None,
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
