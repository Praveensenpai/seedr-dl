use crate::history::{media_file_exists, HistoryEntry, RenameStatus};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style, Stylize},
    widgets::{Block, BorderType, Borders, List, ListItem},
    Frame,
};

/// Renders the download history table in the Ratatui interface.
pub fn render_history(f: &mut Frame, area: Rect, entries: &[HistoryEntry], selected: usize) {
    let items: Vec<ListItem> = if entries.is_empty() {
        vec![ListItem::new(
            "  (no download history recorded yet. Run 'seedr-dl history scan' to index)",
        )
        .style(Style::default().dim())]
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let is_sel = i == selected;
                let exists = media_file_exists(&entry.file_path);
                let sz_mb = entry.file_size / 1_048_576;
                let method = match entry.rename_status {
                    RenameStatus::Gemini => "AI",
                    RenameStatus::RegexFallback => "Regex",
                    RenameStatus::Manual => "Manual",
                };
                let status_icon = if exists { "✔" } else { "✖ Missing" };
                let yr_str = entry.year.map_or_else(String::new, |y| format!(" ({y})"));
                let ep_str = match (entry.season, entry.episode) {
                    (Some(s), Some(ep)) => format!(" S{s:02}E{ep:02}"),
                    _ => String::new(),
                };
                let prefix = if is_sel { " ❯ " } else { "   " };
                let content = format!(
                    "{prefix}[{}] 🎬 {}{}{} — {} MB [{}] [{}] ({})",
                    i + 1,
                    entry.clean_title,
                    yr_str,
                    ep_str,
                    sz_mb,
                    method,
                    status_icon,
                    entry.downloaded_at
                );

                let style = if is_sel {
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else if !exists {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(content).style(style)
            })
            .collect()
    };

    let title = format!(
        " Downloaded Media History & Management ({}) ",
        entries.len()
    );
    let list_widget = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(list_widget, area);
}
