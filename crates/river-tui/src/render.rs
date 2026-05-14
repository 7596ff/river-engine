//! Ratatui terminal rendering

use crate::format::{FormattedLine, HomeChannelFormatter};
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
    stdout().execute(EnterAlternateScreen)?;

    let result = run_inner(agent, &mut entry_rx, client).await;

    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);

    result
}

async fn run_inner(
    agent: String,
    entry_rx: &mut mpsc::UnboundedReceiver<HomeChannelEntry>,
    client: Arc<BystanderClient>,
) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut formatter = HomeChannelFormatter::new();
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
                            .map(|(i, line)| {
                                if i == 0 {
                                    Line::from(Span::raw(line.to_string()))
                                } else {
                                    let indent = " ".repeat(prefix_width);
                                    Line::from(Span::raw(format!("{}{}", indent, line)))
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                })
                .collect();

            let log_widget = Paragraph::new(log_lines.clone())
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });

            if follow_tail {
                // Estimate wrapped line count for scroll
                let inner_width = chunks[0].width.saturating_sub(2).max(1) as usize;
                let total: u16 = log_lines
                    .iter()
                    .map(|line| {
                        let len: usize = line.spans.iter().map(|s| s.content.len()).sum();
                        ((len / inner_width) + 1).min(u16::MAX as usize) as u16
                    })
                    .fold(0u16, |a, b| a.saturating_add(b));
                let inner_height = chunks[0].height.saturating_sub(2);
                scroll_offset = total.saturating_sub(inner_height);
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
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
            entry = entry_rx.recv() => {
                match entry {
                    Some(e) => {
                        let new_lines = formatter.push(e);
                        lines.extend(new_lines);
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}
