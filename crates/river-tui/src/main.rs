//! The TUI client (wall ch. 06): a terminal chat window — message
//! log, status bar, input line — speaking the gateway's /chat
//! WebSocket protocol. Holds no state, renders what it receives, dies
//! harmlessly.

use clap::Parser;
use futures_util::{SinkExt as _, StreamExt as _};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;

#[derive(Parser)]
#[command(name = "river-tui", version, about = "Terminal chat window for a river gateway.")]
struct Cli {
    /// The gateway's local surface chat endpoint.
    #[arg(long, default_value = "ws://127.0.0.1:7700/chat")]
    url: String,

    /// The author name to speak as.
    #[arg(long, default_value_t = whoami())]
    author: String,
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "ground".to_string())
}

struct App {
    author: String,
    url: String,
    connected: bool,
    log: Vec<(String, String)>,
    input: String,
    quit: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let (ws, _) = tokio_tungstenite::connect_async(&cli.url)
        .await
        .map_err(|e| anyhow::anyhow!("connecting to {}: {e}", cli.url))?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Terminal events arrive from a blocking reader thread.
    let (key_tx, mut key_rx) = mpsc::channel::<KeyEvent>(64);
    std::thread::spawn(move || {
        loop {
            match ratatui::crossterm::event::read() {
                Ok(Event::Key(key)) => {
                    if key_tx.blocking_send(key).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let mut app = App {
        author: cli.author,
        url: cli.url,
        connected: true,
        log: Vec::new(),
        input: String::new(),
        quit: false,
    };

    let mut terminal = ratatui::init();
    let result = loop {
        if let Err(e) = terminal.draw(|frame| draw(frame, &app)) {
            break Err(e.into());
        }
        if app.quit {
            break Ok(());
        }

        tokio::select! {
            key = key_rx.recv() => match key {
                Some(key) => {
                    if let Some(line) = handle_key(&mut app, key) {
                        let payload = serde_json::json!({
                            "author": app.author,
                            "content": line,
                        });
                        if ws_sink
                            .send(tungstenite::Message::Text(payload.to_string().into()))
                            .await
                            .is_err()
                        {
                            app.connected = false;
                        }
                    }
                }
                None => break Ok(()),
            },
            frame = ws_stream.next() => match frame {
                Some(Ok(tungstenite::Message::Text(text))) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        let content = value["content"].as_str().unwrap_or_default();
                        let channel = value["channel"].as_str().unwrap_or("?");
                        app.log.push((format!("agent [{channel}]"), content.to_string()));
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    app.connected = false;
                    app.log.push(("system".into(), "connection lost".into()));
                }
            },
        }
    };

    ratatui::restore();
    result
}

/// Returns the line to send when the key submits the input.
fn handle_key(app: &mut App, key: KeyEvent) -> Option<String> {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit = true;
            None
        }
        KeyCode::Esc => {
            app.quit = true;
            None
        }
        KeyCode::Enter => {
            let line = app.input.trim().to_string();
            if line.is_empty() {
                return None;
            }
            app.input.clear();
            app.log.push((app.author.clone(), line.clone()));
            Some(line)
        }
        KeyCode::Backspace => {
            app.input.pop();
            None
        }
        KeyCode::Char(c) => {
            app.input.push(c);
            None
        }
        _ => None,
    }
}

fn draw(frame: &mut ratatui::Frame, app: &App) {
    let [log_area, status_area, input_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    let visible = log_area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(visible);
    let lines: Vec<Line> = app.log[start..]
        .iter()
        .map(|(author, content)| {
            Line::from(vec![
                Span::styled(format!("{author}: "), Style::default().fg(Color::Cyan)),
                Span::raw(content.clone()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("river")),
        log_area,
    );

    let status = if app.connected {
        Span::styled(
            format!(" {} — connected as {} ", app.url, app.author),
            Style::default().fg(Color::Green),
        )
    } else {
        Span::styled(
            format!(" {} — disconnected ", app.url),
            Style::default().fg(Color::Red),
        )
    };
    frame.render_widget(Paragraph::new(Line::from(status)), status_area);

    frame.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title("say")),
        input_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App {
            author: "cass".into(),
            url: "ws://test".into(),
            connected: true,
            log: Vec::new(),
            input: String::new(),
            quit: false,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_then_enter_submits() {
        let mut app = app();
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('h'))), None);
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('i'))), None);
        let sent = handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(sent.as_deref(), Some("hi"));
        assert!(app.input.is_empty());
        assert_eq!(app.log[0], ("cass".into(), "hi".into()));
    }

    #[test]
    fn empty_enter_sends_nothing() {
        let mut app = app();
        assert_eq!(handle_key(&mut app, key(KeyCode::Enter)), None);
        assert!(app.log.is_empty());
    }

    #[test]
    fn backspace_edits() {
        let mut app = app();
        handle_key(&mut app, key(KeyCode::Char('h')));
        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Backspace));
        handle_key(&mut app, key(KeyCode::Char('i')));
        let sent = handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(sent.as_deref(), Some("hi"));
    }

    #[test]
    fn esc_and_ctrl_c_quit() {
        let mut app = app();
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(app.quit);

        let mut app = self::tests::app();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.quit);
    }
}
