//! Terminal UI using ratatui.

use crate::adapter::DisplayMessage;
use crate::SharedState;
use chrono::Utc;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use river_adapter::{Author, EventMetadata, InboundEvent};
use std::io;
use std::time::Duration;
use tokio::sync::mpsc;

/// Events that trigger UI updates.
#[derive(Debug)]
pub enum UiEvent {
    Refresh,
}

/// Run the TUI.
pub async fn run(
    state: SharedState,
    mut ui_rx: mpsc::Receiver<UiEvent>,
    worker_endpoint: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let http_client = reqwest::Client::new();

    loop {
        // Draw UI
        {
            let s = state.read().await;
            terminal.draw(|f| draw_ui(f, &s))?;
        }

        // Check for UI events (non-blocking)
        while let Ok(_) = ui_rx.try_recv() {
            // Just triggers a redraw
        }

        // Poll for keyboard events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        break;
                    }
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                        // Toggle debug mode
                        let mut s = state.write().await;
                        s.show_debug = !s.show_debug;
                    }
                    (KeyCode::Enter, _) => {
                        // Send message
                        let input = {
                            let mut s = state.write().await;
                            let input = s.input.clone();
                            s.input.clear();
                            input
                        };

                        if !input.is_empty() {
                            // Add to display
                            let msg_id = {
                                let mut s = state.write().await;
                                s.add_user_message(&input)
                            };

                            // Send to worker
                            let channel = {
                                let s = state.read().await;
                                s.channel.clone()
                            };

                            let event = InboundEvent {
                                adapter: "mock".into(),
                                metadata: EventMetadata::MessageCreate {
                                    channel: channel.clone(),
                                    author: Author {
                                        id: "user-1".into(),
                                        name: "Human".into(),
                                        bot: false,
                                    },
                                    content: input,
                                    message_id: msg_id,
                                    timestamp: Utc::now().to_rfc3339(),
                                    reply_to: None,
                                    attachments: vec![],
                                },
                            };

                            // Send to worker's /notify endpoint
                            let _ = http_client
                                .post(format!("{}/notify", worker_endpoint))
                                .json(&event)
                                .timeout(Duration::from_secs(5))
                                .send()
                                .await;
                        }
                    }
                    (KeyCode::Backspace, _) => {
                        let mut s = state.write().await;
                        s.input.pop();
                    }
                    (KeyCode::Char(c), _) => {
                        let mut s = state.write().await;
                        s.input.push(c);
                    }
                    (KeyCode::Up, _) => {
                        let mut s = state.write().await;
                        if s.scroll_offset < s.messages.len().saturating_sub(1) {
                            s.scroll_offset += 1;
                        }
                    }
                    (KeyCode::Down, _) => {
                        let mut s = state.write().await;
                        if s.scroll_offset > 0 {
                            s.scroll_offset -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// Draw the UI.
fn draw_ui(f: &mut Frame, state: &crate::adapter::AdapterState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Messages
            Constraint::Length(3), // Input
        ])
        .split(f.area());

    draw_header(f, chunks[0], state);
    draw_messages(f, chunks[1], state);
    draw_input(f, chunks[2], state);
}

/// Draw header bar.
fn draw_header(f: &mut Frame, area: Rect, state: &crate::adapter::AdapterState) {
    let status = if state.worker_endpoint.is_some() {
        "connected"
    } else {
        "waiting..."
    };

    let debug_status = if state.show_debug { "debug:on" } else { "debug:off" };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" River Mock Adapter "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("dyad:{}", state.dyad),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" "),
        Span::styled(
            format!("channel:{}", state.channel),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(format!("[{}]", status), Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(
            format!("[{}]", debug_status),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));

    f.render_widget(header, area);
}

/// Draw message list.
fn draw_messages(f: &mut Frame, area: Rect, state: &crate::adapter::AdapterState) {
    let items: Vec<ListItem> = state
        .messages
        .iter()
        .rev()
        .skip(state.scroll_offset)
        .take(area.height as usize - 2)
        .map(|msg| format_message(msg, state.show_debug))
        .flatten()
        .collect();

    // Reverse to show oldest at top
    let items: Vec<ListItem> = items.into_iter().rev().collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Messages "));

    f.render_widget(list, area);
}

/// Format a message for display.
fn format_message(msg: &DisplayMessage, show_debug: bool) -> Vec<ListItem<'static>> {
    match msg {
        DisplayMessage::User {
            content, timestamp, ..
        } => {
            let time = timestamp.format("%H:%M:%S").to_string();
            vec![ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("user> ", Style::default().fg(Color::Cyan)),
                Span::raw(content.clone()),
            ]))]
        }
        DisplayMessage::Worker {
            content, timestamp, ..
        } => {
            let time = timestamp.format("%H:%M:%S").to_string();
            // Wrap long content
            let lines: Vec<ListItem> = content
                .lines()
                .enumerate()
                .map(|(i, line)| {
                    if i == 0 {
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("[{}] ", time),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled("worker> ", Style::default().fg(Color::Green)),
                            Span::raw(line.to_string()),
                        ]))
                    } else {
                        ListItem::new(Line::from(vec![
                            Span::raw("          "), // Indent continuation
                            Span::raw(line.to_string()),
                        ]))
                    }
                })
                .collect();
            lines
        }
        DisplayMessage::System {
            content, timestamp, ..
        } => {
            let time = timestamp.format("%H:%M:%S").to_string();
            vec![ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("[System] {}", content),
                    Style::default().fg(Color::Yellow),
                ),
            ]))]
        }
        DisplayMessage::ToolCall {
            tool,
            args,
            result,
            timestamp,
        } => {
            if !show_debug {
                return vec![];
            }

            let time = timestamp.format("%H:%M:%S").to_string();
            let mut lines = vec![ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("──── TOOL: {} ────", tool),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::DIM),
                ),
            ]))];

            // Show args (truncated)
            let args_display = if args.len() > 80 {
                format!("{}...", &args[..80])
            } else {
                args.clone()
            };
            lines.push(ListItem::new(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(args_display, Style::default().fg(Color::DarkGray)),
            ])));

            // Show result (truncated)
            if let Some(result) = result {
                let result_display = if result.len() > 80 {
                    format!("{}...", &result[..80])
                } else {
                    result.clone()
                };
                lines.push(ListItem::new(Line::from(vec![
                    Span::styled("└─> ", Style::default().fg(Color::DarkGray)),
                    Span::styled(result_display, Style::default().fg(Color::DarkGray)),
                ])));
            }

            lines
        }
    }
}

/// Draw input line.
fn draw_input(f: &mut Frame, area: Rect, state: &crate::adapter::AdapterState) {
    let input = Paragraph::new(format!("> {}_", state.input))
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Type message (Enter to send, Ctrl+C to quit, Ctrl+D toggle debug) "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(input, area);
}
