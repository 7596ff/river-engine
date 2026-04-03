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
use river_context::OpenAIMessage;
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
                            let notify_result = http_client
                                .post(format!("{}/notify", worker_endpoint))
                                .json(&event)
                                .timeout(Duration::from_secs(5))
                                .send()
                                .await;

                            match notify_result {
                                Ok(response) => {
                                    if !response.status().is_success() {
                                        let mut s = state.write().await;
                                        s.add_system_message(&format!(
                                            "Send failed: HTTP {}",
                                            response.status()
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let mut s = state.write().await;
                                    s.add_system_message(&format!("Send failed: {}", e));
                                }
                            }
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
                        if s.conversation_scroll < s.messages.len().saturating_sub(1) {
                            s.conversation_scroll += 1;
                        }
                    }
                    (KeyCode::Down, _) => {
                        let mut s = state.write().await;
                        if s.conversation_scroll > 0 {
                            s.conversation_scroll -= 1;
                        }
                    }
                    (KeyCode::PageUp, _) => {
                        let mut s = state.write().await;
                        let page_size = 10; // Scroll 10 lines at a time
                        s.conversation_scroll = s
                            .conversation_scroll
                            .saturating_add(page_size)
                            .min(s.messages.len().saturating_sub(1));
                    }
                    (KeyCode::PageDown, _) => {
                        let mut s = state.write().await;
                        let page_size = 10;
                        s.conversation_scroll = s.conversation_scroll.saturating_sub(page_size);
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

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " River Mock Adapter ",
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
            format!("L:{} R:{}", state.left_lines_read, state.right_lines_read),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));

    f.render_widget(header, area);
}

/// Draw message list.
fn draw_messages(f: &mut Frame, area: Rect, state: &crate::adapter::AdapterState) {
    let width = area.width.saturating_sub(2) as usize; // Account for borders

    let items: Vec<ListItem> = state
        .messages
        .iter()
        .rev()
        .skip(state.conversation_scroll)
        .take(area.height as usize - 2)
        .map(|msg| format_message(msg, width))
        .flatten()
        .collect();

    // Reverse to show oldest at top
    let items: Vec<ListItem> = items.into_iter().rev().collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Context "));

    f.render_widget(list, area);
}

/// Format a message for display.
fn format_message(msg: &DisplayMessage, width: usize) -> Vec<ListItem<'static>> {
    match msg {
        DisplayMessage::User {
            content, timestamp, ..
        } => {
            let time = timestamp.format("%H:%M:%S").to_string();
            let content_part = format!("you> {}", content);
            let available = width.saturating_sub(11); // "[HH:MM:SS] "
            let padding = available.saturating_sub(content_part.len()) / 2;
            vec![ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" ".repeat(padding)),
                Span::styled("you> ", Style::default().fg(Color::Cyan)),
                Span::raw(content.clone()),
            ]))]
        }
        DisplayMessage::System {
            content, timestamp, ..
        } => {
            let time = timestamp.format("%H:%M:%S").to_string();
            let content_part = format!("[sys] {}", content);
            let available = width.saturating_sub(11);
            let padding = available.saturating_sub(content_part.len()) / 2;
            vec![ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" ".repeat(padding)),
                Span::styled(
                    format!("[sys] {}", content),
                    Style::default().fg(Color::Yellow),
                ),
            ]))]
        }
        DisplayMessage::Context { side, entry, timestamp } => {
            format_context_entry(side, entry, timestamp, width)
        }
    }
}

/// Format a context entry (OpenAI message) for display.
fn format_context_entry(side: &str, entry: &OpenAIMessage, timestamp: &chrono::DateTime<Utc>, width: usize) -> Vec<ListItem<'static>> {
    let time = timestamp.format("%H:%M:%S").to_string();
    let role = entry.role.as_str();

    // Side indicator (L/R)
    let side_char = if side == "left" { "L" } else { "R" };
    let is_right = side == "right";
    let available = width.saturating_sub(11); // "[HH:MM:SS] "

    // Determine color and prefix based on role
    let (color, prefix) = match role {
        "user" => (Color::Cyan, "user"),
        "assistant" => (Color::Green, "asst"),
        "system" => (Color::Magenta, "sys"),
        "tool" => (Color::Blue, "tool"),
        _ => (Color::White, role),
    };

    // Handle tool calls
    if let Some(ref tool_calls) = entry.tool_calls {
        let mut items = vec![];
        for tc in tool_calls {
            let content_part = format!("{} {}> call {} {}",
                side_char, prefix, tc.function.name,
                truncate_str(&tc.function.arguments, 30));
            let padding = if is_right { available.saturating_sub(content_part.len()) } else { 0 };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" ".repeat(padding)),
                Span::styled(
                    format!("{} ", side_char),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{}> ", prefix),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("call {} ", tc.function.name),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    truncate_str(&tc.function.arguments, 30),
                    Style::default().fg(Color::DarkGray),
                ),
            ])));
        }
        return items;
    }

    // Handle tool result
    if let Some(ref tool_call_id) = entry.tool_call_id {
        let content = entry.content.as_deref().unwrap_or("");
        let id_preview_len = 8.min(tool_call_id.len());
        let content_part = format!("{} {}> [{}] {}",
            side_char, prefix, &tool_call_id[..id_preview_len],
            truncate_str(content, 40));
        let padding = if is_right { available.saturating_sub(content_part.len()) } else { 0 };

        return vec![ListItem::new(Line::from(vec![
            Span::styled(
                format!("[{}] ", time),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" ".repeat(padding)),
            Span::styled(
                format!("{} ", side_char),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{}> ", prefix),
                Style::default().fg(color),
            ),
            Span::styled(
                format!("[{}] ", &tool_call_id[..id_preview_len]),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(truncate_str(content, 40)),
        ]))];
    }

    // Regular content message
    let content = entry.content.as_deref().unwrap_or("");
    let lines: Vec<ListItem> = content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let truncated = truncate_str(line, available.saturating_sub(10));
            if i == 0 {
                let content_part = format!("{} {}> {}", side_char, prefix, truncated);
                let padding = if is_right { available.saturating_sub(content_part.len()) } else { 0 };

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("[{}] ", time),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(
                        format!("{} ", side_char),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("{}> ", prefix),
                        Style::default().fg(color),
                    ),
                    Span::raw(truncated),
                ]))
            } else {
                // Continuation lines
                let padding = if is_right { available.saturating_sub(truncated.len()) } else { 11 };
                ListItem::new(Line::from(vec![
                    Span::raw(" ".repeat(padding)),
                    Span::raw(truncated),
                ]))
            }
        })
        .collect();

    if lines.is_empty() {
        let content_part = format!("{} {}> (empty)", side_char, prefix);
        let padding = if is_right { available.saturating_sub(content_part.len()) } else { 0 };

        vec![ListItem::new(Line::from(vec![
            Span::styled(
                format!("[{}] ", time),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" ".repeat(padding)),
            Span::styled(
                format!("{} ", side_char),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{}> ", prefix),
                Style::default().fg(color),
            ),
            Span::styled("(empty)", Style::default().fg(Color::DarkGray)),
        ]))]
    } else {
        lines
    }
}

/// Truncate a string to max length, adding ellipsis if needed.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Draw input line.
fn draw_input(f: &mut Frame, area: Rect, state: &crate::adapter::AdapterState) {
    let input = Paragraph::new(format!("> {}_", state.input))
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Type message (Enter=send, Up/Down/PgUp/PgDn=scroll, Ctrl+C=quit) "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(input, area);
}
