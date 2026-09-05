use crate::modal::{render_modal, Modal};
use crate::seedr::{ListContentsResponse, SeedrFolder, SeedrTorrent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};

pub struct AppState {
    pub list: ListContentsResponse,
    pub selected: usize,
    pub modal: Option<Modal>,
    pub status: Option<(String, bool)>,
}

pub fn draw_ui(f: &mut Frame, state: &AppState) {
    let size = f.area();
    let has_torrents = !state.list.torrents.is_empty();

    let constraints = if has_torrents {
        vec![
            Constraint::Length(4),
            Constraint::Length(
                u16::try_from(state.list.torrents.len() + 2)
                    .unwrap_or(4)
                    .min(8),
            ),
            Constraint::Min(6),
            Constraint::Length(3),
        ]
    } else {
        vec![
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(3),
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(size);

    let mut idx = 0;
    render_storage(f, chunks[idx], &state.list);
    idx += 1;

    if has_torrents {
        render_torrents(f, chunks[idx], &state.list.torrents);
        idx += 1;
    }

    render_folders(f, chunks[idx], &state.list.folders, state.selected);
    idx += 1;

    render_footer(f, chunks[idx], state.status.as_ref());

    if let Some(modal) = &state.modal {
        render_modal(f, size, modal);
    }
}

fn render_storage(f: &mut Frame, area: Rect, list: &ListContentsResponse) {
    let (used_mb, max_mb) = match (list.space_used, list.space_max) {
        (Some(u), Some(m)) => (u / 1_048_576, m / 1_048_576),
        _ => (0, 0),
    };
    let ratio = if max_mb > 0 {
        // reason: used_mb and max_mb represent megabytes (< 10^7), fitting safely in f64
        #[allow(clippy::cast_precision_loss)]
        (used_mb as f64 / max_mb as f64).min(1.0)
    } else {
        0.0
    };
    let free_mb = max_mb.saturating_sub(used_mb);
    let label = format!("{used_mb} MB / {max_mb} MB ({free_mb} MB free)");

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" ⚡ Seedr Cloud Storage ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .gauge_style(
            Style::default()
                .fg(if ratio > 0.85 {
                    Color::Red
                } else {
                    Color::Yellow
                })
                .bg(Color::DarkGray),
        )
        .ratio(ratio)
        .label(label);

    f.render_widget(gauge, area);
}

fn render_torrents(f: &mut Frame, area: Rect, torrents: &[SeedrTorrent]) {
    let items: Vec<ListItem> = torrents
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let pct = t.progress.unwrap_or(0.0);
            let sz = t.size.unwrap_or(0) / 1_048_576;
            let text = format!(" [t{}] ⏳ {} ({} MB, {:.1}%)", i + 1, t.name, sz, pct);
            ListItem::new(text).style(Style::default().fg(Color::Yellow))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Active Cloud Downloads ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(list, area);
}

fn render_folders(f: &mut Frame, area: Rect, folders: &[SeedrFolder], selected: usize) {
    let items: Vec<ListItem> = if folders.is_empty() {
        vec![ListItem::new("  (no completed files in cloud)").style(Style::default().dim())]
    } else {
        folders
            .iter()
            .enumerate()
            .map(|(i, folder)| {
                let sz_mb = folder.size.unwrap_or(0) / 1_048_576;
                let is_sel = i == selected;
                let prefix = if is_sel { " ❯ " } else { "   " };
                let content = format!("{prefix}[{}] 📁 {} ({} MB)", i + 1, folder.name, sz_mb);
                let style = if is_sel {
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(content).style(style)
            })
            .collect()
    };

    let title = format!(" Completed Cloud Items ({}) ", folders.len());
    let list_widget = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(list_widget, area);
}

fn render_footer(f: &mut Frame, area: Rect, status: Option<&(String, bool)>) {
    let text = if let Some((msg, is_err)) = status {
        if *is_err {
            Line::from(vec![
                Span::styled(" ✖ ", Style::default().fg(Color::Red).bold()),
                Span::styled(msg.as_str(), Style::default().fg(Color::Red)),
            ])
        } else {
            Line::from(vec![
                Span::styled(" ✔ ", Style::default().fg(Color::Green).bold()),
                Span::styled(msg.as_str(), Style::default().fg(Color::Green)),
            ])
        }
    } else {
        Line::from(vec![
            Span::styled(" [↑/k, ↓/j] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Select   "),
            Span::styled("[Enter] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Download   "),
            Span::styled("[d] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Delete   "),
            Span::styled("[a] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Add   "),
            Span::styled("[c] ", Style::default().fg(Color::Magenta).bold()),
            Span::raw("Clean   "),
            Span::styled("[r] ", Style::default().fg(Color::Blue).bold()),
            Span::raw("Refresh   "),
            Span::styled("[q] ", Style::default().dim().bold()),
            Span::raw("Quit"),
        ])
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().dim());

    let p = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);
    f.render_widget(p, area);
}
