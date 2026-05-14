//! Ratatui terminal rendering

use crate::format::{format_entry, FormattedLine};
use crate::post::BystanderClient;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use river_core::channels::entry::HomeChannelEntry;
use std::io::stdout;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Run the TUI. Ensures terminal cleanup on all exit paths.
pub async fn run(
    agent: String,
    mut entry_rx: mpsc::UnboundedReceiver<HomeChannelEntry>,
    client: Arc<BystanderClient>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    crossterm::execute!(stdout(), EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;

    let result = run_inner(agent, &mut entry_rx, client).await;

    let _ = disable_raw_mode();
    let _ = crossterm::execute!(stdout(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture);

    result
}

async fn run_inner(
    agent: String,
    entry_rx: &mut mpsc::UnboundedReceiver<HomeChannelEntry>,
    client: Arc<BystanderClient>,
) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut lines: Vec<FormattedLine> = Vec::new();
    let mut input = String::new();
    let mut scroll_offset: u16 = 0;
    let mut follow_tail = true;
    let mut _status_error: Option<String> = None;

    loop {
        // Calculate input height (expands with content)
        let input_line_count = {
            let width = terminal.size()?.width.saturating_sub(4) as usize;
            if width == 0 {
                1
            } else {
                let display_len = input.len() + 2; // "> " prefix
                (display_len / width.max(1) + 1).max(1) as u16
            }
        };
        let input_height = input_line_count + 2; // borders

        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),
                    Constraint::Length(1),
                    Constraint::Length(input_height),
                ])
                .split(frame.area());

            // --- Log ---
            let prefix_width = 20; // "YYYY-MM-DD HH:MM:SS " = 20 chars
            let inner_width = chunks[0].width.saturating_sub(2).max(1) as usize;
            let inner_height = chunks[0].height.saturating_sub(2);

            // Do our own wrapping so line count is exact
            let log_lines: Vec<Line> = lines
                .iter()
                .flat_map(|fl| {
                    let content_lines: Vec<&str> = fl.text.lines().collect();
                    if content_lines.is_empty() {
                        vec![Line::from(Span::raw(""))]
                    } else {
                        content_lines
                            .iter()
                            .enumerate()
                            .flat_map(|(i, line)| {
                                let full = if i == 0 {
                                    line.to_string()
                                } else {
                                    format!("{}{}", " ".repeat(prefix_width), line)
                                };
                                // Wrap this line manually
                                if full.is_empty() {
                                    vec![Line::from(Span::raw(""))]
                                } else {
                                    let chars: Vec<char> = full.chars().collect();
                                    chars
                                        .chunks(inner_width)
                                        .map(|chunk| Line::from(Span::raw(chunk.iter().collect::<String>())))
                                        .collect::<Vec<_>>()
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                })
                .collect();

            let total_lines = log_lines.len() as u16;

            let log_widget = Paragraph::new(log_lines)
                .block(Block::default().borders(Borders::ALL));

            if follow_tail {
                scroll_offset = total_lines.saturating_sub(inner_height);
            }

            let log_widget = log_widget.scroll((scroll_offset, 0));
            frame.render_widget(log_widget, chunks[0]);

            // --- Status bar ---
            let mut status_spans = vec![
                Span::raw(" [river] "),
                Span::styled(&agent, Style::default().fg(Color::Cyan)),
            ];
            if let Some(ref err) = _status_error {
                status_spans.push(Span::styled(
                    format!(" | {}", err),
                    Style::default().fg(Color::Red),
                ));
            }
            let status_widget = Paragraph::new(Line::from(status_spans))
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(status_widget, chunks[1]);

            // --- Input ---
            let input_widget = Paragraph::new(format!("> {}", input))
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });
            frame.render_widget(input_widget, chunks[2]);
        })?;

        // Event loop
        tokio::select! {
            poll_result = tokio::task::spawn_blocking(|| {
                event::poll(std::time::Duration::from_millis(50)).unwrap_or(false)
            }) => {
                if !poll_result.unwrap_or(false) {
                    continue;
                }
                let evt = tokio::task::block_in_place(|| event::read())?;
                match evt {
                    Event::Key(key) => {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Enter, _) if !input.is_empty() => {
                                let content = std::mem::take(&mut input);
                                let c = client.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = c.post(&content).await {
                                        tracing::error!("bystander post failed: {}", e);
                                    }
                                });
                                _status_error = None;
                                follow_tail = true;
                            }
                            (KeyCode::Char(c), _) => { input.push(c); }
                            (KeyCode::Backspace, _) => { input.pop(); }
                            (KeyCode::Up, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(1);
                            }
                            (KeyCode::Down, _) => {
                                scroll_offset = scroll_offset.saturating_add(1);
                                let total = lines.len() as u16;
                                if scroll_offset >= total { follow_tail = true; }
                            }
                            (KeyCode::PageUp, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(10);
                            }
                            (KeyCode::PageDown, _) => {
                                scroll_offset = scroll_offset.saturating_add(10);
                                let total = lines.len() as u16;
                                if scroll_offset >= total { follow_tail = true; }
                            }
                            _ => {}
                        }
                    }
                    Event::Mouse(mouse) => {
                        use crossterm::event::MouseEventKind;
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(3);
                            }
                            MouseEventKind::ScrollDown => {
                                scroll_offset = scroll_offset.saturating_add(3);
                                let total = lines.len() as u16;
                                if scroll_offset >= total { follow_tail = true; }
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
            entry = entry_rx.recv() => {
                match entry {
                    Some(e) => {
                        lines.push(format_entry(e));
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}
