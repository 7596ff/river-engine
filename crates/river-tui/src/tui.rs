//! Ratatui terminal interface

use crate::gateway::{Author, GatewayClient, IncomingMessage};
use crate::state::{ChatLine, SharedState};
use chrono::Local;
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
use std::io::stdout;
use std::sync::Arc;

/// Run the TUI event loop. Ensures terminal cleanup on all exit paths.
pub async fn run(
    state: SharedState,
    gateway: Arc<GatewayClient>,
    user_name: String,
    channel: String,
) -> anyhow::Result<()> {
    // Set up terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    // Run the inner loop, capturing the result
    let result = run_inner(state, gateway, user_name, channel).await;

    // Always restore terminal, even on error/panic
    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);

    result
}

async fn run_inner(
    state: SharedState,
    gateway: Arc<GatewayClient>,
    user_name: String,
    channel: String,
) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut input = String::new();
    let mut scroll_offset: u16 = 0;
    let mut follow_tail = true;

    loop {
        // Draw
        let messages = state.get_messages();
        let connected = state.is_gateway_connected();
        let server_ok = state.is_server_healthy();

        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),    // message log
                    Constraint::Length(1), // status bar
                    Constraint::Length(3), // input
                ])
                .split(frame.area());

            // --- Message log ---
            let msg_lines: Vec<Line> = messages
                .iter()
                .map(|m| {
                    let time = m.timestamp.format("%H:%M").to_string();
                    let name_color = if m.is_agent {
                        Color::Cyan
                    } else {
                        Color::Green
                    };
                    Line::from(vec![
                        Span::styled(format!("{} ", time), Style::default().fg(Color::DarkGray)),
                        Span::styled(format!("{}: ", m.sender), Style::default().fg(name_color)),
                        Span::raw(&m.content),
                    ])
                })
                .collect();

            let msg_widget = Paragraph::new(msg_lines.clone())
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });

            // Auto-scroll (counts messages, not wrapped lines — known v1 limitation)
            let inner_height = chunks[0].height.saturating_sub(2);
            let total_lines = msg_lines.len() as u16;
            if follow_tail && total_lines > inner_height {
                scroll_offset = total_lines.saturating_sub(inner_height);
            }

            let msg_widget = msg_widget.scroll((scroll_offset, 0));
            frame.render_widget(msg_widget, chunks[0]);

            // --- Status bar ---
            let gw_indicator = if connected { "●" } else { "○" };
            let gw_color = if connected { Color::Green } else { Color::Red };
            let gw_text = if connected {
                "connected"
            } else {
                "disconnected"
            };

            let mut status_spans = vec![
                Span::raw(" [tui "),
                Span::styled(gw_indicator, Style::default().fg(gw_color)),
                Span::raw(format!(" gateway: {}", gw_text)),
            ];

            if !server_ok {
                status_spans.push(Span::styled(
                    " | server: down",
                    Style::default().fg(Color::Red),
                ));
            }

            status_spans.push(Span::raw("]"));

            let status_widget = Paragraph::new(Line::from(status_spans))
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(status_widget, chunks[1]);

            // --- Input ---
            let input_widget = Paragraph::new(format!("> {}_", input))
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(input_widget, chunks[2]);
        })?;

        // Wait for either a crossterm event or a notify signal
        tokio::select! {
            // Check for terminal input events
            poll_result = tokio::task::spawn_blocking(|| {
                event::poll(std::time::Duration::from_millis(100)).unwrap_or(false)
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
                                follow_tail = true;

                                // Add to local display
                                state.push_message(ChatLine {
                                    timestamp: Local::now(),
                                    sender: user_name.clone(),
                                    content: content.clone(),
                                    is_agent: false,
                                });

                                // Send to gateway
                                let gw = gateway.clone();
                                let ch = channel.clone();
                                let name = user_name.clone();
                                tokio::spawn(async move {
                                    let msg = IncomingMessage {
                                        adapter: "tui".into(),
                                        event_type: "MessageCreate".into(),
                                        channel: ch,
                                        author: Author {
                                            id: "local-user".into(),
                                            name,
                                        },
                                        content,
                                        message_id: Some(format!("tui-{}", chrono::Utc::now().timestamp_millis())),
                                    };
                                    if let Err(e) = gw.send_incoming(msg).await {
                                        tracing::error!(error = %e, "Failed to send message to gateway");
                                    }
                                });
                            }
                            (KeyCode::Char(c), _) => {
                                input.push(c);
                            }
                            (KeyCode::Backspace, _) => { input.pop(); }
                            (KeyCode::Up, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(1);
                            }
                            (KeyCode::Down, _) => {
                                scroll_offset = scroll_offset.saturating_add(1);
                                // Re-enable follow if scrolled past the end
                                let total = state.get_messages().len() as u16;
                                if scroll_offset >= total {
                                    follow_tail = true;
                                }
                            }
                            (KeyCode::PageUp, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(10);
                            }
                            (KeyCode::PageDown, _) => {
                                scroll_offset = scroll_offset.saturating_add(10);
                                let total = state.get_messages().len() as u16;
                                if scroll_offset >= total {
                                    follow_tail = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {} // re-render on next loop
                    _ => {}
                }
            }
            // Wake up when a new message arrives from the gateway
            _ = state.notify.notified() => {
                // New message arrived — the next loop iteration will re-render
                follow_tail = true;
            }
        }
    }

    Ok(())
}
