use crate::task::{cancel_task, load_task, TaskState, TaskStatus};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::Stdout;
use std::time::Duration;

pub async fn run_attach_loop(
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    folder_id: u64,
) -> Result<()> {
    let mut retries = 0;
    loop {
        let Some(task) = load_task(folder_id) else {
            if retries < 15 {
                retries += 1;
                tokio::time::sleep(Duration::from_millis(150)).await;
                continue;
            }
            break;
        };

        let is_done = task.status == TaskStatus::Completed || task.status == TaskStatus::Failed;
        term.draw(|f| draw_attach_screen(f, &task))?;

        if is_done {
            wait_for_dismiss(term, folder_id)?;
            break;
        }

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('d' | 'b') | KeyCode::Esc => break,
                        KeyCode::Char('x') => {
                            cancel_task(folder_id);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn wait_for_dismiss(term: &mut Terminal<CrosstermBackend<Stdout>>, folder_id: u64) -> Result<()> {
    loop {
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && matches!(
                        key.code,
                        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q' | 'b' | 'd')
                    )
                {
                    break;
                }
            }
        }
    }
    crate::task::remove_task(folder_id);
    term.clear()?;
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
pub fn draw_attach_screen(f: &mut Frame, task: &TaskState) {
    let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(4),
            Constraint::Length(3),
        ])
        .split(size);

    let title = Paragraph::new(vec![Line::from(vec![
        Span::styled(" Downloading: ", Style::default().bold()),
        Span::styled(&task.file_name, Style::default().fg(Color::Cyan).bold()),
    ])])
    .block(
        Block::default()
            .title(" ⚡ Live Download Monitor ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(title, chunks[0]);

    let ratio = if task.total_bytes > 0 {
        (task.downloaded_bytes as f64 / task.total_bytes as f64).min(1.0)
    } else {
        0.0
    };
    let dl_mb = task.downloaded_bytes / 1_048_576;
    let tot_mb = task.total_bytes / 1_048_576;
    let label = format!("{dl_mb} MB / {tot_mb} MB ({:.1}%)", ratio * 100.0);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Progress ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, chunks[1]);

    let spd_mb = task.speed_bps as f64 / 1_048_576.0;
    let eta_m = task.eta_seconds / 60;
    let eta_s = task.eta_seconds % 60;
    let err_msg = task.error.as_deref().unwrap_or("Ingestion failed");
    let status_str = match task.status {
        TaskStatus::Downloading => "Downloading from Seedr CDN...".cyan(),
        TaskStatus::Organizing => "Analyzing with Gemini AI & moving to Jellyfin..."
            .yellow()
            .bold(),
        TaskStatus::Completed => "✔ Ingestion complete! File placed in Jellyfin."
            .green()
            .bold(),
        TaskStatus::Failed => format!("✖ Failed: {err_msg}").red().bold(),
    };

    let stats = Paragraph::new(vec![
        Line::from(format!(" Speed:    {spd_mb:.2} MB/s")),
        Line::from(format!(" ETA:      {eta_m}m {eta_s}s remaining")),
        Line::from(vec![
            Span::raw(" Status:   "),
            Span::styled(status_str.to_string(), Style::default()),
        ]),
    ])
    .block(
        Block::default()
            .title(" Telemetry ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    f.render_widget(stats, chunks[2]);

    let footer_text = if task.status == TaskStatus::Completed || task.status == TaskStatus::Failed {
        Line::from(vec![
            Span::styled(" [Enter] / [Esc] / [q] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Return to Manager"),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " [d] / [Esc] ",
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::raw("Detach (download continues in background)       "),
            Span::styled("[x] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Cancel download"),
        ])
    };

    let footer = Paragraph::new(footer_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .alignment(Alignment::Center);
    f.render_widget(footer, chunks[3]);
}
