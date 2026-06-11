//! The local chat surface (wall ch. 06): the engine's own front door.
//! A small HTTP + WebSocket server bound to localhost — Ground's
//! door, not a multi-user system. Every connected client sees the
//! same channel (`local_main`).
//!
//! Wire protocol: client → server `{"author": "...", "content": "..."}`;
//! server → client `{"channel": "...", "content": "..."}`.

use std::net::{Ipv4Addr, SocketAddr};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

use crate::channels::Channels;
use crate::turn::{DEFAULT_CHANNEL, Health, LOCAL_ADAPTER, OutboundMessage};

#[derive(Clone)]
struct SurfaceState {
    channels: Channels,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Receiver<Health>,
}

#[derive(Debug, Deserialize)]
struct ClientMessage {
    author: String,
    content: String,
}

/// Serve until the shutdown signal flips true. Binds localhost only —
/// this is the gateway's sole HTTP exposure.
pub async fn serve(
    port: u16,
    channels: Channels,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Receiver<Health>,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_on(listener, channels, outbound, health, shutdown).await
}

async fn serve_on(
    listener: tokio::net::TcpListener,
    channels: Channels,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Receiver<Health>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let state = SurfaceState {
        channels,
        outbound,
        health,
    };
    let app = Router::new()
        .route("/chat", get(chat_handler))
        .route("/message", post(message_handler))
        .route("/health", get(health_handler))
        .with_state(state);

    tracing::info!(addr = %listener.local_addr()?, "local surface listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|&stop| stop).await;
        })
        .await?;
    Ok(())
}

async fn health_handler(State(state): State<SurfaceState>) -> Json<Health> {
    Json(state.health.borrow().clone())
}

async fn message_handler(
    State(state): State<SurfaceState>,
    Json(msg): Json<ClientMessage>,
) -> Json<serde_json::Value> {
    // Write-then-notify happens inside the channel layer; a failure
    // here means the message is NOT durably logged and the caller is
    // told so (wall ch. 05).
    let ok = state
        .channels
        .inbound(
            DEFAULT_CHANNEL,
            &msg.author,
            None,
            &msg.content,
            LOCAL_ADAPTER,
            None,
        )
        .await
        .is_ok();
    Json(serde_json::json!({ "ok": ok }))
}

async fn chat_handler(
    State(state): State<SurfaceState>,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    upgrade.on_upgrade(move |socket| chat_connection(socket, state))
}

async fn chat_connection(socket: WebSocket, state: SurfaceState) {
    let (mut sink, mut stream) = socket.split();
    let mut outbound = state.outbound.subscribe();

    loop {
        tokio::select! {
            agent_msg = outbound.recv() => match agent_msg {
                Ok(out) if out.channel == DEFAULT_CHANNEL => {
                    let payload = serde_json::json!({
                        "channel": out.channel,
                        "content": out.content,
                    });
                    if sink
                        .send(Message::Text(payload.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(_) => {} // another channel's traffic; not for this surface
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "ws client lagged behind outbound");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            client_msg = stream.next() => match client_msg {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(msg) => {
                            if state
                                .channels
                                .inbound(
                                    DEFAULT_CHANNEL,
                                    &msg.author,
                                    None,
                                    &msg.content,
                                    LOCAL_ADAPTER,
                                    None,
                                )
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "malformed ws client message");
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ping/pong/binary: ignored
                Some(Err(e)) => {
                    tracing::debug!(error = %e, "ws receive error");
                    break;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::Notification;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite;

    struct Surface {
        addr: SocketAddr,
        channels: Channels,
        notify_rx: mpsc::Receiver<Notification>,
        outbound: broadcast::Sender<OutboundMessage>,
        health_tx: watch::Sender<Health>,
        _shutdown_tx: watch::Sender<bool>,
        _dir: tempfile::TempDir,
    }

    async fn start() -> Surface {
        let dir = tempfile::tempdir().unwrap();
        let (notify_tx, notify_rx) = mpsc::channel(256);
        let channels = Channels::open(dir.path(), notify_tx).unwrap();
        let (outbound, _) = broadcast::channel(16);
        let (health_tx, health_rx) = watch::channel(Health::default());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_on(
            listener,
            channels.clone(),
            outbound.clone(),
            health_rx,
            shutdown_rx,
        ));
        Surface {
            addr,
            channels,
            notify_rx,
            outbound,
            health_tx,
            _shutdown_tx: shutdown_tx,
            _dir: dir,
        }
    }

    #[tokio::test]
    async fn post_message_logs_then_notifies() {
        let mut surface = start().await;
        let client = reqwest::Client::new();
        let response: serde_json::Value = client
            .post(format!("http://{}/message", surface.addr))
            .json(&serde_json::json!({ "author": "cass", "content": "hello" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(response["ok"], true);

        let note = surface.notify_rx.recv().await.unwrap();
        assert_eq!(note.channel, DEFAULT_CHANNEL);

        let entries = surface.channels.scan(DEFAULT_CHANNEL).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].author.as_deref(), Some("cass"));
        assert_eq!(entries[0].content.as_deref(), Some("hello"));
        assert_eq!(entries[0].id, note.ulid);
    }

    #[tokio::test]
    async fn health_serves_live_state() {
        let surface = start().await;
        surface
            .health_tx
            .send(Health {
                turn_number: 7,
                last_settle: Some("2026-06-11T04:00:00Z".into()),
                context_messages: 3,
                ..Health::default()
            })
            .unwrap();

        let health: serde_json::Value = reqwest::get(format!("http://{}/health", surface.addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["turn_number"], 7);
        assert_eq!(health["context_messages"], 3);
    }

    #[tokio::test]
    async fn ws_chat_round_trip() {
        let mut surface = start().await;
        let (mut ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{}/chat", surface.addr))
                .await
                .unwrap();

        // Client speaks → channel log + notification.
        ws.send(tungstenite::Message::Text(
            r#"{"author":"cass","content":"hello"}"#.into(),
        ))
        .await
        .unwrap();
        let note = surface.notify_rx.recv().await.unwrap();
        assert_eq!(note.channel, DEFAULT_CHANNEL);

        // Agent speaks → client receives.
        surface
            .outbound
            .send(OutboundMessage {
                channel: DEFAULT_CHANNEL.into(),
                content: "good morning".into(),
            })
            .unwrap();
        let frame = ws.next().await.unwrap().unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(payload["content"], "good morning");
        assert_eq!(payload["channel"], DEFAULT_CHANNEL);
    }
}
