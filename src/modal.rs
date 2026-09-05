use crate::gemini::MediaInfo;
use crate::history::HistoryEntry;
use crate::seedr::{SeedrFolder, SeedrTorrent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum Modal {
    ConfirmDelete(SeedrFolder),
    ConfirmCancelTorrent(SeedrTorrent),
    ConfirmCleanAll(usize),
    InputMagnet(String),
    ConfirmAiRename {
        entry: HistoryEntry,
        new_info: MediaInfo,
        target_path: PathBuf,
    },
    InputManualRename {
        entry: HistoryEntry,
        title: String,
        year: String,
        active_field: usize,
    },
    ConfirmDeleteMedia {
        entry: HistoryEntry,
        typed: String,
    },
}

pub fn render_modal(f: &mut Frame, area: Rect, modal: &Modal) {
    let (w, h) = match modal {
        Modal::ConfirmAiRename { .. } => (80, 40),
        Modal::InputManualRename { .. } | Modal::ConfirmDeleteMedia { .. } => (75, 40),
        _ => (65, 25),
    };
    let popup_area = centered_rect(w, h, area);
    f.render_widget(Clear, popup_area);

    match modal {
        Modal::ConfirmDelete(folder) => render_delete_modal(f, popup_area, folder),
        Modal::ConfirmCancelTorrent(torrent) => render_cancel_modal(f, popup_area, torrent),
        Modal::ConfirmCleanAll(count) => render_clean_modal(f, popup_area, *count),
        Modal::InputMagnet(input) => render_magnet_modal(f, popup_area, input),
        Modal::ConfirmAiRename {
            entry,
            new_info: _,
            target_path,
        } => render_ai_rename_modal(f, popup_area, entry, target_path),
        Modal::InputManualRename {
            entry,
            title,
            year,
            active_field,
        } => render_manual_rename_modal(f, popup_area, entry, title, year, *active_field),
        Modal::ConfirmDeleteMedia { entry, typed } => {
            render_delete_media_modal(f, popup_area, entry, typed);
        }
    }
}

fn render_delete_modal(f: &mut Frame, area: Rect, folder: &SeedrFolder) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(" Delete '"),
            Span::styled(&folder.name, Style::default().fg(Color::Yellow)),
            Span::raw("' from Seedr?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [Enter] / [y] ", Style::default().fg(Color::Red)),
            Span::raw("Confirm Delete      "),
            Span::styled("[Esc] / [n] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Cancel"),
        ]),
    ])
    .block(
        Block::default()
            .title(" Confirm Deletion ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Red)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_cancel_modal(f: &mut Frame, area: Rect, torrent: &SeedrTorrent) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(" Cancel torrent '"),
            Span::styled(&torrent.name, Style::default().fg(Color::Yellow)),
            Span::raw("' ?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [Enter] / [y] ", Style::default().fg(Color::Red)),
            Span::raw("Confirm Cancel      "),
            Span::styled("[Esc] / [n] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Keep"),
        ]),
    ])
    .block(
        Block::default()
            .title(" Cancel Download ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_clean_modal(f: &mut Frame, area: Rect, count: usize) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(format!(" Delete ALL {count} cloud items to free storage?")),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [Enter] / [y] ", Style::default().fg(Color::Red)),
            Span::raw("Delete All      "),
            Span::styled("[Esc] / [n] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Cancel"),
        ]),
    ])
    .block(
        Block::default()
            .title(" Clean Cloud Storage ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Red)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_magnet_modal(f: &mut Frame, area: Rect, input: &str) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(" Paste magnet link or torrent URL:"),
        Line::from(vec![
            Span::styled(" > ", Style::default().fg(Color::Cyan)),
            Span::styled(input, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [Enter] ", Style::default().fg(Color::Green)),
            Span::raw("Submit      "),
            Span::styled("[Esc] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Cancel"),
        ]),
    ])
    .block(
        Block::default()
            .title(" Add Magnet Link ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_ai_rename_modal(
    f: &mut Frame,
    area: Rect,
    entry: &HistoryEntry,
    target_path: &std::path::Path,
) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Original: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&entry.original_name, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Current:  ", Style::default().fg(Color::Red)),
            Span::raw(entry.file_path.display().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Target:   ", Style::default().fg(Color::Green)),
            Span::styled(
                target_path.display().to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [Enter] / [y] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Apply Reorganization    "),
            Span::styled("[Esc] / [n] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Cancel"),
        ]),
    ])
    .block(
        Block::default()
            .title(" ✨ Gemini AI Re-rename Preview ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_manual_rename_modal(
    f: &mut Frame,
    area: Rect,
    _entry: &HistoryEntry,
    title: &str,
    year: &str,
    active_field: usize,
) {
    let title_style = if active_field == 0 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let year_style = if active_field == 1 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(" Edit canonical title and release year:"),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Title: > ", title_style),
            Span::styled(title, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled(" Year:  > ", year_style),
            Span::styled(year, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [Tab] ", Style::default().fg(Color::Cyan)),
            Span::raw("Switch Field   "),
            Span::styled("[Enter] ", Style::default().fg(Color::Green)),
            Span::raw("Apply Rename   "),
            Span::styled("[Esc] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Cancel"),
        ]),
    ])
    .block(
        Block::default()
            .title(" ✏️ Manual Media Rename ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_delete_media_modal(f: &mut Frame, area: Rect, entry: &HistoryEntry, typed: &str) {
    let sz_mb = entry.file_size / 1_048_576;
    let is_delete_typed = typed == "DELETE";
    let status_color = if is_delete_typed {
        Color::Green
    } else {
        Color::Yellow
    };

    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "⚠️  PERMANENT DELETION FROM DISK ⚠️",
            Style::default().fg(Color::Red).bold(),
        )),
        Line::from(format!("{} ({} MB)", entry.clean_title, sz_mb)),
        Line::from(Span::styled(
            entry.file_path.display().to_string(),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::raw("Type \"DELETE\" to confirm deletion:")),
        Line::from(vec![
            Span::styled(" > ", Style::default().fg(status_color)),
            Span::styled(typed, Style::default().fg(status_color).bold()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [Enter] ",
                if is_delete_typed {
                    Style::default().fg(Color::Red).bold()
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ),
            Span::raw("Confirm Delete      "),
            Span::styled("[Esc] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Cancel"),
        ]),
    ])
    .block(
        Block::default()
            .title(" Confirm Disk Deletion ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Red)),
    )
    .alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
