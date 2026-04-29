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
use river_protocol::conversation::{
    Conversation, Line as BackchannelLine, Message as BackchannelMessage, MessageDirection,
};
use river_protocol::Author as ProtocolAuthor;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

/// Maximum length for tool call arguments display.
const TOOL_ARGS_MAX_LEN: usize = 30;

/// Maximum length for tool result content display.
const TOOL_RESULT_MAX_LEN: usize = 40;

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
    workspace: Option<PathBuf>,
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

                        if input.is_empty() {
                            continue;
                        }

                        // Handle backchannel messages
                        if input.starts_with("/bc ") {
                            if let Some(ref ws) = workspace {
                                let content = input.strip_prefix("/bc ").unwrap().to_string();
                                let msg = BackchannelMessage {
                                    direction: MessageDirection::Outgoing,
                                    timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                                    id: format!("tui-{}", Utc::now().timestamp_millis()),
                                    author: ProtocolAuthor {
                                        name: "tui".to_string(),
                                        id: "debug".to_string(),
                                        bot: false,
                                    },
                                    content,
                                    reactions: vec![],
                                };

                                let path = ws.join("conversations").join("backchannel.txt");
                                if let Err(e) = Conversation::append_line(
                                    &path,
                                    &BackchannelLine::Message(msg),
                                ) {
                                    let mut s = state.write().await;
                                    s.add_system_message(&format!("Backchannel write failed: {}", e));
                                }
                            } else {
                                let mut s = state.write().await;
                                s.add_system_message("No workspace configured for backchannel");
                            }
                            continue;
                        }

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

    // Determine which side is actor, which is spectator (D-09)
    use river_protocol::Baton;
    let (actor_side, spectator_side) = match (state.baton_left, state.baton_right) {
        (Baton::Actor, Baton::Spectator) => ("left", "right"),
        (Baton::Spectator, Baton::Actor) => ("right", "left"),
        _ => ("left", "right"),  // Fallback if both same (shouldn't happen)
    };

    // Line 1: Title bar with status
    let title_line = Line::from(vec![
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
    ]);

    // Line 2: Baton state (D-09)
    let baton_line = Line::from(vec![
        Span::styled("Actor: ", Style::default().fg(Color::White)),
        Span::styled(
            actor_side,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Spectator: "),
        Span::styled(
            spectator_side,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
    ]);

    let header = Paragraph::new(vec![title_line, baton_line])
        .block(Block::default().borders(Borders::ALL));

    f.render_widget(header, area);
}

/// Draw message list.
fn draw_messages(f: &mut Frame, area: Rect, state: &crate::adapter::AdapterState) {
    let width = area.width.saturating_sub(2) as usize; // Account for borders

    // Build items from messages
    let mut all_items: Vec<ListItem> = Vec::new();

    // Add regular messages
    for msg in &state.messages {
        all_items.extend(format_message(msg, width));
    }

    // Add backchannel messages
    for line in &state.backchannel_lines {
        all_items.extend(format_backchannel_line(line));
    }

    // Apply scrolling and height limit
    let visible_count = (area.height as usize).saturating_sub(2);
    let skip = state.conversation_scroll;
    let items: Vec<ListItem> = all_items
        .into_iter()
        .rev()
        .skip(skip)
        .take(visible_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

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
                truncate_str(&tc.function.arguments, TOOL_ARGS_MAX_LEN));
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
                    truncate_str(&tc.function.arguments, TOOL_ARGS_MAX_LEN),
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
            truncate_str(content, TOOL_RESULT_MAX_LEN));
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
            Span::raw(truncate_str(content, TOOL_RESULT_MAX_LEN)),
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

/// Format a backchannel line for display.
fn format_backchannel_line(line: &BackchannelLine) -> Vec<ListItem<'static>> {
    match line {
        BackchannelLine::Message(msg) => {
            let style = Style::default().fg(Color::Cyan);
            let prefix = format!("[BC {}] ", msg.author.id);
            vec![ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(msg.content.clone(), style),
            ]))]
        }
        BackchannelLine::ReadReceipt { message_id, .. } => {
            vec![ListItem::new(Line::from(Span::styled(
                format!("[BC read] {}", message_id),
                Style::default().fg(Color::DarkGray),
            )))]
        }
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
                .title(" Type message (Enter=send, /bc=backchannel, Up/Down/PgUp/PgDn=scroll, Ctrl+C=quit) "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(input, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::DisplayMessage;
    use chrono::Utc;
    use river_context::{FunctionCall, OpenAIMessage, ToolCall};

    // ========== truncate_str tests ==========

    #[test]
    fn test_truncate_str_short_string() {
        // String shorter than max_len should remain unchanged
        let result = truncate_str("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_str_exact_length() {
        // String exactly at max_len should remain unchanged
        let result = truncate_str("hello", 5);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_str_adds_ellipsis() {
        // String longer than max_len should be truncated with ellipsis
        let result = truncate_str("hello world", 8);
        // max_len=8, so we take 8-3=5 chars plus "..."
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_str_very_short_max() {
        // Very short max_len (edge case)
        let result = truncate_str("hello", 3);
        // max_len=3, so we take 3-3=0 chars plus "..."
        assert_eq!(result, "...");
    }

    #[test]
    fn test_truncate_str_empty_string() {
        let result = truncate_str("", 10);
        assert_eq!(result, "");
    }

    // ========== format_message tests for User variant ==========

    #[test]
    fn test_format_message_user() {
        let msg = DisplayMessage::User {
            id: "msg-123".to_string(),
            content: "Hello, world!".to_string(),
            timestamp: Utc::now(),
        };

        let items = format_message(&msg, 80);

        assert_eq!(items.len(), 1);
        // The formatted output should contain the user content
        // We can't easily inspect ListItem internals, but we can verify it returns one item
    }

    #[test]
    fn test_format_message_user_empty_content() {
        let msg = DisplayMessage::User {
            id: "msg-456".to_string(),
            content: "".to_string(),
            timestamp: Utc::now(),
        };

        let items = format_message(&msg, 80);

        assert_eq!(items.len(), 1);
    }

    // ========== format_message tests for System variant ==========

    #[test]
    fn test_format_message_system() {
        let msg = DisplayMessage::System {
            content: "System notification".to_string(),
            timestamp: Utc::now(),
        };

        let items = format_message(&msg, 80);

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_message_system_long_content() {
        let msg = DisplayMessage::System {
            content: "This is a very long system message that exceeds normal display width".to_string(),
            timestamp: Utc::now(),
        };

        let items = format_message(&msg, 40);

        assert_eq!(items.len(), 1);
    }

    // ========== format_context_entry tests ==========

    #[test]
    fn test_format_context_entry_user_role() {
        let entry = OpenAIMessage {
            role: "user".to_string(),
            content: Some("User message content".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_assistant_role() {
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: Some("Assistant response".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("right", &entry, &timestamp, 80);

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_tool_call() {
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call-abc123".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: r#"{"location": "San Francisco"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("right", &entry, &timestamp, 80);

        // Should have one item per tool call
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_multiple_tool_calls() {
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![
                ToolCall {
                    id: "call-1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "tool_a".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                ToolCall {
                    id: "call-2".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "tool_b".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
            ]),
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        // Should have one item per tool call
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_format_context_entry_tool_result() {
        let entry = OpenAIMessage {
            role: "tool".to_string(),
            content: Some("Tool execution result".to_string()),
            tool_calls: None,
            tool_call_id: Some("call-xyz789".to_string()),
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_empty_content() {
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: Some("".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("right", &entry, &timestamp, 80);

        // Empty content results in "(empty)" display
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_none_content() {
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        // None content results in "(empty)" display
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_multiline_content() {
        let entry = OpenAIMessage {
            role: "user".to_string(),
            content: Some("Line 1\nLine 2\nLine 3".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        // Should have one item per line
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_format_context_entry_left_vs_right_side() {
        let entry = OpenAIMessage {
            role: "user".to_string(),
            content: Some("Test content".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let left_items = format_context_entry("left", &entry, &timestamp, 80);
        let right_items = format_context_entry("right", &entry, &timestamp, 80);

        // Both should produce same number of items (formatting differs internally)
        assert_eq!(left_items.len(), right_items.len());
    }

    #[test]
    fn test_format_context_entry_system_role() {
        let entry = OpenAIMessage {
            role: "system".to_string(),
            content: Some("System prompt".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_unknown_role() {
        let entry = OpenAIMessage {
            role: "custom_role".to_string(),
            content: Some("Custom role content".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        // Should still format, using the role name as prefix
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_narrow_width() {
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: Some("This is a long message that should be truncated".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("right", &entry, &timestamp, 30);

        // Should still produce one item even with narrow width
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_tool_result_empty_content() {
        let entry = OpenAIMessage {
            role: "tool".to_string(),
            content: Some("".to_string()),
            tool_calls: None,
            tool_call_id: Some("call-empty".to_string()),
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);

        assert_eq!(items.len(), 1);
    }
}
