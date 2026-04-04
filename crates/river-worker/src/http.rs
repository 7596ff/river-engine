//! HTTP server for worker endpoints.

use crate::conversation::conversation_path_for_channel;
use crate::state::SharedState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use river_adapter::{Baton, Channel, EventMetadata, InboundEvent};
use river_context::Flash;
use river_protocol::conversation::{Conversation, Line, Message, MessageDirection};
use river_protocol::{Author as ProtocolAuthor, Registry};
use serde::Serialize;

/// Build the router.
pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/notify", post(handle_notify))
        .route("/flash", post(handle_flash))
        .route("/registry", post(handle_registry))
        .route("/prepare_switch", post(handle_prepare_switch))
        .route("/commit_switch", post(handle_commit_switch))
        .route("/health", get(handle_health))
        .with_state(state)
}

/// Extract channel and message_id from event metadata.
fn extract_channel_info(adapter: &str, metadata: &EventMetadata) -> Option<(Channel, Option<String>)> {
    match metadata {
        EventMetadata::MessageCreate { channel, message_id, .. } => Some((
            Channel { adapter: adapter.into(), id: channel.clone(), name: None },
            Some(message_id.clone()),
        )),
        EventMetadata::MessageUpdate { channel, message_id, .. } => Some((
            Channel { adapter: adapter.into(), id: channel.clone(), name: None },
            Some(message_id.clone()),
        )),
        EventMetadata::MessageDelete { channel, message_id } => Some((
            Channel { adapter: adapter.into(), id: channel.clone(), name: None },
            Some(message_id.clone()),
        )),
        EventMetadata::ReactionAdd { channel, message_id, .. } => Some((
            Channel { adapter: adapter.into(), id: channel.clone(), name: None },
            Some(message_id.clone()),
        )),
        EventMetadata::ReactionRemove { channel, message_id, .. } => Some((
            Channel { adapter: adapter.into(), id: channel.clone(), name: None },
            Some(message_id.clone()),
        )),
        EventMetadata::TypingStart { channel, .. } => Some((
            Channel { adapter: adapter.into(), id: channel.clone(), name: None },
            None,
        )),
        _ => None,
    }
}

/// POST /notify - Receive events from adapters.
async fn handle_notify(
    State(state): State<SharedState>,
    Json(event): Json<InboundEvent>,
) -> StatusCode {
    tracing::debug!("Received notify: {:?}", event.metadata.event_type());

    let mut s = state.write().await;

    // Write MessageCreate events to conversation files (skip backchannel)
    if let EventMetadata::MessageCreate {
        channel,
        author,
        content,
        message_id,
        timestamp,
        ..
    } = &event.metadata
    {
        // Skip backchannel - it's a shared file handled by speak tool
        if channel != "backchannel" {
            let conv_channel = Channel {
                adapter: event.adapter.clone(),
                id: channel.clone(),
                name: None,
            };
            let path = conversation_path_for_channel(&s.workspace, &conv_channel);

            let msg = Message {
                direction: MessageDirection::Unread,
                timestamp: timestamp.clone(),
                id: message_id.clone(),
                author: ProtocolAuthor {
                    name: author.name.clone(),
                    id: author.id.clone(),
                    bot: author.bot,
                },
                content: content.clone(),
                reactions: vec![],
            };

            if let Err(e) = Conversation::append_line(&path, &Line::Message(msg)) {
                tracing::warn!(error = %e, "Failed to write message to conversation file");
            }
        }
    }

    // Extract channel info from event
    if let Some((channel, message_id)) = extract_channel_info(&event.adapter, &event.metadata) {
        // If sleeping and channel is watched, wake up
        if s.sleeping && s.is_watched(&channel) {
            tracing::info!(
                channel = %channel.id,
                message_id = ?message_id,
                "Waking from sleep due to watched channel notification"
            );
            s.sleeping = false;
            s.sleep_until = None;
        }

        // Add to pending notifications
        if let Some(notif) = s.pending_notifications.iter_mut().find(|n| {
            n.channel.adapter == channel.adapter && n.channel.id == channel.id
        }) {
            notif.count += 1;
        } else {
            s.pending_notifications.push(crate::state::Notification {
                channel,
                count: 1,
            });
        }
    }

    StatusCode::OK
}

/// POST /flash - Receive flash from another worker.
async fn handle_flash(
    State(state): State<SharedState>,
    Json(flash): Json<Flash>,
) -> StatusCode {
    tracing::debug!("Received flash from {}", flash.from);

    let mut s = state.write().await;

    // Wake if sleeping
    if s.sleeping {
        tracing::info!("Waking from sleep due to flash");
        s.sleeping = false;
        s.sleep_until = None;
    }

    // Queue flash for injection
    s.pending_flashes.push(flash);

    StatusCode::OK
}

/// POST /registry - Receive registry updates from orchestrator.
async fn handle_registry(
    State(state): State<SharedState>,
    Json(registry): Json<Registry>,
) -> StatusCode {
    tracing::debug!("Received registry update with {} processes", registry.processes.len());

    let mut s = state.write().await;
    s.registry = registry;

    StatusCode::OK
}

/// Prepare switch response.
#[derive(Debug, Serialize)]
pub struct PrepareSwitchResponse {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// POST /prepare_switch - Prepare for role switch.
async fn handle_prepare_switch(
    State(state): State<SharedState>,
) -> Result<Json<PrepareSwitchResponse>, StatusCode> {
    let mut s = state.write().await;

    // Check if already in a switch
    if s.switch_pending {
        return Ok(Json(PrepareSwitchResponse {
            ready: false,
            reason: Some("switch_already_pending".into()),
        }));
    }

    // Mark as pending
    s.switch_pending = true;

    Ok(Json(PrepareSwitchResponse {
        ready: true,
        reason: None,
    }))
}

/// Commit switch response.
#[derive(Debug, Serialize)]
pub struct CommitSwitchResponse {
    pub committed: bool,
    pub new_baton: String,
}

/// POST /commit_switch - Execute the role switch.
async fn handle_commit_switch(
    State(state): State<SharedState>,
) -> Result<Json<CommitSwitchResponse>, StatusCode> {
    let mut s = state.write().await;

    if !s.switch_pending {
        return Err(StatusCode::CONFLICT);
    }

    // Swap baton
    let new_baton = match s.baton {
        Baton::Actor => Baton::Spectator,
        Baton::Spectator => Baton::Actor,
    };
    s.baton = new_baton.clone();

    // Clear pending flag
    s.switch_pending = false;

    let baton_str = match new_baton {
        Baton::Actor => "actor",
        Baton::Spectator => "spectator",
    };

    Ok(Json(CommitSwitchResponse {
        committed: true,
        new_baton: baton_str.into(),
    }))
}

/// Health response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
}

/// GET /health - Health check.
async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
    })
}
