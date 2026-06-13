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
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

use crate::channels::Channels;
use crate::context::ContextSnapshot;
use crate::memory::Memory;
use crate::turn::{DEFAULT_CHANNEL, Health, LOCAL_ADAPTER, OutboundMessage};

/// The instrument panel pages (board cards): single self-contained
/// HTML files, vendored d3-force, no build step. Windows, never hands.
const GRAPH_VIEW_HTML: &str = include_str!("../assets/graph-view.html");
const CONTEXT_VIEW_HTML: &str = include_str!("../assets/context-view.html");

#[derive(Clone)]
struct SurfaceState {
    channels: Channels,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Receiver<Health>,
    memory: Option<Memory>,
    context: watch::Receiver<ContextSnapshot>,
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
    memory: Option<Memory>,
    context: watch::Receiver<ContextSnapshot>,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_on(listener, channels, outbound, health, memory, context, shutdown).await
}

async fn serve_on(
    listener: tokio::net::TcpListener,
    channels: Channels,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Receiver<Health>,
    memory: Option<Memory>,
    context: watch::Receiver<ContextSnapshot>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let state = SurfaceState {
        channels,
        outbound,
        health,
        memory,
        context,
    };
    let app = Router::new()
        .route("/chat", get(chat_handler))
        .route("/message", post(message_handler))
        .route("/health", get(health_handler))
        .route("/graph", get(graph_handler))
        .route("/graph/view", get(|| async { Html(GRAPH_VIEW_HTML) }))
        .route("/context", get(context_handler))
        .route("/context/view", get(|| async { Html(CONTEXT_VIEW_HTML) }))
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

/// The activation graph, read-only (board card). Graph assembly walks
/// the workspace and runs pairwise cosines, so it runs off the async
/// thread.
async fn graph_handler(
    State(state): State<SurfaceState>,
) -> Result<Json<crate::memory::GraphPayload>, (StatusCode, String)> {
    let Some(memory) = state.memory.clone() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "no embedding model configured; the memory body is not running".into(),
        ));
    };
    tokio::task::spawn_blocking(move || memory.graph())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// The live context window, read-only (board card): the snapshot the
/// turn loop published at its last settle.
async fn context_handler(State(state): State<SurfaceState>) -> Json<ContextSnapshot> {
    Json(state.context.borrow().clone())
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
        snapshot_tx: watch::Sender<ContextSnapshot>,
        _shutdown_tx: watch::Sender<bool>,
        _dir: tempfile::TempDir,
    }

    async fn start() -> Surface {
        start_with_memory(None).await
    }

    async fn start_with_memory(memory: Option<Memory>) -> Surface {
        let dir = tempfile::tempdir().unwrap();
        let (notify_tx, notify_rx) = mpsc::channel(256);
        let channels = Channels::open(dir.path(), notify_tx).unwrap();
        let (outbound, _) = broadcast::channel(16);
        let (health_tx, health_rx) = watch::channel(Health::default());
        let (snapshot_tx, snapshot_rx) = watch::channel(ContextSnapshot::default());
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
            memory,
            snapshot_rx,
            shutdown_rx,
        ));
        Surface {
            addr,
            channels,
            notify_rx,
            outbound,
            health_tx,
            snapshot_tx,
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
    async fn graph_endpoint_serves_nodes_links_and_views() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(ws.join("knowledge")).unwrap();
        std::fs::write(
            ws.join("knowledge/a.md"),
            "---\nid: NA\nlinks:\n  - extends: NB\n---\n\nclaim a",
        )
        .unwrap();
        std::fs::write(ws.join("knowledge/b.md"), "---\nid: NB\n---\n\nclaim b").unwrap();
        let memory = Memory::open(
            &dir.path().join("data"),
            &ws,
            &[],
            std::sync::Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();
        memory.bump("NA", 0.5, crate::memory::Carrier::Cognitive).unwrap();

        let surface = start_with_memory(Some(memory)).await;
        let graph: serde_json::Value = reqwest::get(format!("http://{}/graph", surface.addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(graph["flash_threshold"], 1.0);
        let nodes = graph["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        let na = nodes.iter().find(|n| n["id"] == "NA").unwrap();
        assert_eq!(na["score"], 0.5);
        assert_eq!(na["path"], "knowledge/a.md", "workspace-relative");
        let links = graph["links"].as_array().unwrap();
        assert!(links.iter().any(|l| l["source"] == "NA"
            && l["target"] == "NB"
            && l["type"] == "extends"));

        // The view pages are self-contained HTML.
        let view = reqwest::get(format!("http://{}/graph/view", surface.addr))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(view.contains("forceSimulation"), "d3-force vendored inline");
        let cview = reqwest::get(format!("http://{}/context/view", surface.addr))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(cview.contains("compaction"));
    }

    #[tokio::test]
    async fn graph_without_memory_is_unavailable_not_a_crash() {
        let surface = start().await;
        let status = reqwest::get(format!("http://{}/graph", surface.addr))
            .await
            .unwrap()
            .status();
        assert_eq!(status, reqwest::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn context_endpoint_serves_the_latest_snapshot() {
        let surface = start().await;
        surface
            .snapshot_tx
            .send(ContextSnapshot {
                turn_number: 9,
                channel: "local_main".into(),
                limit: 128_000,
                estimate_total: 12_000.0,
                hot_messages: 14,
                hot_first_turn: Some(3),
                hot_last_turn: Some(9),
                memory_slot: "[flash] the heron".into(),
                ..ContextSnapshot::default()
            })
            .unwrap();

        let ctx: serde_json::Value = reqwest::get(format!("http://{}/context", surface.addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(ctx["turn_number"], 9);
        assert_eq!(ctx["hot_messages"], 14);
        assert_eq!(ctx["hot_first_turn"], 3);
        assert_eq!(ctx["memory_slot"], "[flash] the heron");
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
