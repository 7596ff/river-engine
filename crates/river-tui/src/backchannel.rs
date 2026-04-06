//! Bidirectional backchannel watcher.
//!
//! Watches backchannel file for new entries from workers and forwards them
//! to the opposite worker's /notify endpoint.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use std::fs;
use reqwest::Client;
use river_adapter::{InboundEvent, EventMetadata, Author};

/// Watches backchannel file for new entries and forwards to workers.
///
/// Polls backchannel.txt every 100ms, detects new lines, parses "L:" or "R:" prefix
/// to identify author, creates InboundEvent::MessageCreate, and POSTs to recipient
/// worker's /notify endpoint.
///
/// Format assumption (per Issue 6 fix in plan): "L:" prefix for left worker,
/// "R:" prefix for right worker. If workers use different format, test assertions
/// in Plan 04-03 will catch it.
pub async fn watch_backchannel(
    backchannel_path: PathBuf,
    left_endpoint: Arc<RwLock<Option<String>>>,
    right_endpoint: Arc<RwLock<Option<String>>>,
) {
    let client = Client::new();
    let mut last_position = 0u64;  // Track file position to detect new lines

    loop {
        // Read backchannel file from last position
        if let Ok(content) = fs::read_to_string(&backchannel_path) {
            let current_len = content.len() as u64;

            if current_len > last_position {
                // New content available
                let new_content = &content[(last_position as usize)..];

                // Parse new lines as backchannel messages
                for line in new_content.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }

                    // Detect which side wrote the message
                    // Format assumption (per Issue 6 fix): "L:" or "R:" prefix
                    // If workers use different format, test assertions will catch it
                    let (author_side, message) = if line.starts_with("L:") {
                        ("left", &line[2..])
                    } else if line.starts_with("R:") {
                        ("right", &line[2..])
                    } else {
                        continue;  // Unknown format, skip (tests will show if this is an issue)
                    };

                    // Determine recipient (opposite side)
                    let recipient_endpoint = match author_side {
                        "left" => right_endpoint.read().await.clone(),
                        "right" => left_endpoint.read().await.clone(),
                        _ => None,
                    };

                    if let Some(endpoint) = recipient_endpoint {
                        // Create InboundEvent for backchannel message
                        let event = InboundEvent {
                            adapter: "mock".to_string(),
                            metadata: EventMetadata::MessageCreate {
                                channel: "backchannel".to_string(),
                                message_id: format!("bc-{}", std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_micros()),
                                author: Author {
                                    id: author_side.to_string(),
                                    name: format!("{} worker", author_side),
                                    bot: true,
                                },
                                content: message.trim().to_string(),
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                reply_to: None,
                                attachments: vec![],
                            },
                        };

                        // POST to recipient worker's /notify endpoint
                        let _ = client
                            .post(format!("{}/notify", endpoint))
                            .json(&event)
                            .send()
                            .await;
                    }
                }

                last_position = current_len;
            }
        }

        // Poll every 100ms (fast enough for testing, low CPU usage)
        sleep(Duration::from_millis(100)).await;
    }
}
