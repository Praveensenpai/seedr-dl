use crate::seedr::{SeedrFolder, SeedrTorrent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

#[derive(Clone, Debug)]
pub enum Modal {
    ConfirmDelete(SeedrFolder),
    ConfirmCancelTorrent(SeedrTorrent),
    ConfirmCleanAll(usize),
    InputMagnet(String),
}

pub fn render_modal(f: &mut Frame, area: Rect, modal: &Modal) {
    let popup_area = centered_rect(65, 25, area);
    f.render_widget(Clear, popup_area);

    match modal {
        Modal::ConfirmDelete(folder) => render_delete_modal(f, popup_area, folder),
        Modal::ConfirmCancelTorrent(torrent) => render_cancel_modal(f, popup_area, torrent),
        Modal::ConfirmCleanAll(count) => render_clean_modal(f, popup_area, *count),
        Modal::InputMagnet(input) => render_magnet_modal(f, popup_area, input),
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
