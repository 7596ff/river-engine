//! Tool implementations.

use crate::config::WorkerConfig;
use crate::llm::ToolCall;
use crate::state::SharedState;
use river_adapter::{Channel, OutboundRequest, Side};
use river_context::Flash;
use river_snowflake::{SnowflakeGenerator, SnowflakeType};
use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Result from read_history tool.
#[derive(Debug, Serialize, Default)]
pub struct ReadHistoryResult {
    pub success: bool,
    pub messages_fetched: usize,
    pub oldest_id: Option<String>,
    pub newest_id: Option<String>,
    pub error: Option<String>,
    pub retry_after_ms: Option<u64>,
}

/// Tool error codes.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", content = "details")]
pub enum ToolError {
    FileNotFound { path: String },
    IsDirectory { path: String },
    PermissionDenied { path: String },
    MissingParameter { name: String },
    AdapterNotFound { adapter: String },
    AdapterUnreachable { adapter: String },
    SendFailed { reason: String },
    UnsupportedOperation { operation: String },
    TargetNotFound { target: String },
    TargetUnreachable { target: String },
    EmbedServerUnreachable,
    InvalidCursor { cursor: String },
    UnknownModel { model: String },
    PartnerUnreachable,
    SwitchInProgress,
    PartnerRejected { reason: String },
    CommandTimeout { seconds: u64 },
    InvalidDirectory { path: String },
    ParseError { message: String },
}

/// Tool result.
#[derive(Debug)]
pub enum ToolResult {
    Success(serde_json::Value),
    Error(ToolError),
    Summary(String),
    Sleep { minutes: Option<u64> },
    SwitchRoles { new_baton: String },
    ChannelSwitch { previous_adapter: String, previous_channel: String },
}

/// Execute a tool call.
pub async fn execute_tool(
    call: &ToolCall,
    state: &SharedState,
    config: &WorkerConfig,
    generator: &mut SnowflakeGenerator,
    client: &reqwest::Client,
) -> ToolResult {
    let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult::Error(ToolError::ParseError {
                message: e.to_string(),
            });
        }
    };

    match call.name.as_str() {
        "read" => execute_read(&args, state).await,
        "write" => execute_write(&args, state, client).await,
        "delete" => execute_delete(&args, state, client).await,
        "bash" => execute_bash(&args, state).await,
        "speak" => execute_speak(&args, state, client).await,
        "switch_channel" => execute_switch_channel(&args, state).await,
        "sleep" => execute_sleep(&args),
        "watch" => execute_watch(&args, state).await,
        "summary" => execute_summary(&args),
        "create_flash" => execute_create_flash(&args, state, generator, client).await,
        "request_model" => execute_request_model(&args, state, config, client).await,
        "switch_roles" => execute_switch_roles(state, config, client).await,
        "search_embeddings" => execute_search_embeddings(&args, state, client).await,
        "next_embedding" => execute_next_embedding(&args, state, client).await,
        "create_move" => execute_create_move(&args, state, generator).await,
        "create_moment" => execute_create_moment(&args, state, generator).await,
        "adapter" => execute_adapter(&args, state, client).await,
        "read_history" => execute_read_history(&args, state).await,
        _ => ToolResult::Error(ToolError::UnsupportedOperation {
            operation: call.name.clone(),
        }),
    }
}

async fn execute_read(args: &serde_json::Value, state: &SharedState) -> ToolResult {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "path".into(),
            })
        }
    };

    let workspace = {
        let s = state.read().await;
        s.workspace.clone()
    };

    let full_path = workspace.join(path);

    if !full_path.exists() {
        return ToolResult::Error(ToolError::FileNotFound { path: path.into() });
    }

    if full_path.is_dir() {
        return ToolResult::Error(ToolError::IsDirectory { path: path.into() });
    }

    match tokio::fs::read_to_string(&full_path).await {
        Ok(content) => {
            let start_line = args.get("start_line").and_then(|v| v.as_u64()).map(|n| n as usize);
            let end_line = args.get("end_line").and_then(|v| v.as_u64()).map(|n| n as usize);

            let lines: Vec<&str> = content.lines().collect();
            let total_lines = lines.len();

            let (start, end) = match (start_line, end_line) {
                (Some(s), Some(e)) => (s.saturating_sub(1), e.min(total_lines)),
                (Some(s), None) => (s.saturating_sub(1), total_lines),
                (None, Some(e)) => (0, e.min(total_lines)),
                (None, None) => (0, total_lines),
            };

            let selected: Vec<&str> = lines[start..end].to_vec();
            let output = selected.join("\n");

            ToolResult::Success(serde_json::json!({
                "content": output,
                "lines": total_lines
            }))
        }
        Err(e) => ToolResult::Error(ToolError::PermissionDenied {
            path: format!("{}: {}", path, e),
        }),
    }
}

async fn execute_write(
    args: &serde_json::Value,
    state: &SharedState,
    client: &reqwest::Client,
) -> ToolResult {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "path".into(),
            })
        }
    };

    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "content".into(),
            })
        }
    };

    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("overwrite");
    let at_line = args.get("at_line").and_then(|v| v.as_u64()).map(|n| n as usize);

    if mode == "insert" && at_line.is_none() {
        return ToolResult::Error(ToolError::MissingParameter {
            name: "at_line".into(),
        });
    }

    let (workspace, embed_endpoint) = {
        let s = state.read().await;
        (s.workspace.clone(), s.registry.embed_endpoint().map(String::from))
    };

    let full_path = workspace.join(path);

    // Create parent directories
    if let Some(parent) = full_path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return ToolResult::Error(ToolError::PermissionDenied {
                path: format!("Failed to create directory: {}", e),
            });
        }
    }

    let result = match mode {
        "append" => {
            let mut file = match tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&full_path)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    return ToolResult::Error(ToolError::PermissionDenied {
                        path: e.to_string(),
                    })
                }
            };
            tokio::io::AsyncWriteExt::write_all(&mut file, content.as_bytes()).await
        }
        "insert" => {
            let existing = tokio::fs::read_to_string(&full_path).await.unwrap_or_default();
            let mut lines: Vec<&str> = existing.lines().collect();
            let line_num = at_line.unwrap().saturating_sub(1);
            lines.insert(line_num.min(lines.len()), content);
            tokio::fs::write(&full_path, lines.join("\n")).await
        }
        _ => tokio::fs::write(&full_path, content).await,
    };

    match result {
        Ok(()) => {
            let bytes = content.len();

            // Notify embed server if in embeddings directory
            if path.starts_with("embeddings/") {
                if let Some(endpoint) = embed_endpoint {
                    let _ = client
                        .post(format!("{}/index", endpoint))
                        .json(&serde_json::json!({
                            "source": path,
                            "content": content
                        }))
                        .send()
                        .await;
                }
            }

            ToolResult::Success(serde_json::json!({
                "written": true,
                "bytes": bytes
            }))
        }
        Err(e) => ToolResult::Error(ToolError::PermissionDenied {
            path: e.to_string(),
        }),
    }
}

async fn execute_delete(
    args: &serde_json::Value,
    state: &SharedState,
    client: &reqwest::Client,
) -> ToolResult {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "path".into(),
            })
        }
    };

    let (workspace, embed_endpoint) = {
        let s = state.read().await;
        (s.workspace.clone(), s.registry.embed_endpoint().map(String::from))
    };

    let full_path = workspace.join(path);

    if !full_path.exists() {
        return ToolResult::Error(ToolError::FileNotFound { path: path.into() });
    }

    if full_path.is_dir() {
        return ToolResult::Error(ToolError::IsDirectory { path: path.into() });
    }

    match tokio::fs::remove_file(&full_path).await {
        Ok(()) => {
            // Notify embed server if in embeddings directory
            if path.starts_with("embeddings/") {
                if let Some(endpoint) = embed_endpoint {
                    let _ = client
                        .delete(format!("{}/source/{}", endpoint, urlencoding::encode(path)))
                        .send()
                        .await;
                }
            }

            ToolResult::Success(serde_json::json!({ "deleted": true }))
        }
        Err(e) => ToolResult::Error(ToolError::PermissionDenied {
            path: e.to_string(),
        }),
    }
}

async fn execute_bash(args: &serde_json::Value, state: &SharedState) -> ToolResult {
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "command".into(),
            })
        }
    };

    let timeout_secs = args
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .min(600);

    let workspace = {
        let s = state.read().await;
        s.workspace.clone()
    };

    let working_dir = match args.get("working_directory").and_then(|v| v.as_str()) {
        Some(dir) => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                path
            } else {
                workspace.join(dir)
            }
        }
        None => workspace.clone(),
    };

    if !working_dir.exists() {
        return ToolResult::Error(ToolError::InvalidDirectory {
            path: working_dir.to_string_lossy().into(),
        });
    }

    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => ToolResult::Success(serde_json::json!({
            "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
            "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
            "exit_code": output.status.code().unwrap_or(-1)
        })),
        Ok(Err(e)) => ToolResult::Error(ToolError::PermissionDenied {
            path: e.to_string(),
        }),
        Err(_) => ToolResult::Error(ToolError::CommandTimeout {
            seconds: timeout_secs,
        }),
    }
}

async fn execute_speak(
    args: &serde_json::Value,
    state: &SharedState,
    client: &reqwest::Client,
) -> ToolResult {
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "content".into(),
            })
        }
    };

    let (current_channel, registry, workspace, dyad, baton) = {
        let s = state.read().await;
        (
            s.current_channel.clone(),
            s.registry.clone(),
            s.workspace.clone(),
            s.dyad.clone(),
            s.baton,
        )
    };

    let adapter = args
        .get("adapter")
        .and_then(|v| v.as_str())
        .unwrap_or(&current_channel.adapter);
    let channel_id = args
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or(&current_channel.id);
    let channel_name = current_channel.name.as_deref();
    let reply_to = args.get("reply_to").and_then(|v| v.as_str()).map(String::from);

    // Check for backchannel special routing
    if channel_id == "backchannel" || channel_name == Some("backchannel") {
        use crate::conversation::backchannel_path;
        use river_protocol::conversation::{Conversation, Line, Message, MessageDirection};

        let id = format!("bc-{}", chrono::Utc::now().timestamp_millis());
        let msg = Message {
            direction: MessageDirection::Outgoing,
            timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            id: id.clone(),
            author: river_protocol::Author {
                name: dyad.clone(),
                id: format!("{:?}", baton),
                bot: true,
            },
            content: content.to_string(),
            reactions: vec![],
        };

        let path = backchannel_path(&workspace);
        if let Err(e) = Conversation::append_line(&path, &Line::Message(msg)) {
            return ToolResult::Error(ToolError::SendFailed {
                reason: format!("Failed to write to backchannel: {}", e),
            });
        }

        return ToolResult::Success(serde_json::json!({
            "message_id": id,
            "sent": true,
            "channel": "backchannel"
        }));
    }

    let endpoint = match registry.adapter_endpoint(adapter) {
        Some(ep) => ep,
        None => {
            return ToolResult::Error(ToolError::AdapterNotFound {
                adapter: adapter.into(),
            })
        }
    };

    let request = OutboundRequest::SendMessage {
        channel: channel_id.into(),
        content: content.into(),
        reply_to,
    };

    match client
        .post(format!("{}/execute", endpoint))
        .json(&request)
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            ToolResult::Success(serde_json::json!({
                "message_id": body.get("message_id"),
                "sent": true
            }))
        }
        Ok(resp) => {
            let msg = resp.text().await.unwrap_or_default();
            ToolResult::Error(ToolError::SendFailed { reason: msg })
        }
        Err(_) => ToolResult::Error(ToolError::AdapterUnreachable {
            adapter: adapter.into(),
        }),
    }
}

async fn execute_switch_channel(args: &serde_json::Value, state: &SharedState) -> ToolResult {
    let adapter = match args.get("adapter").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "adapter".into(),
            })
        }
    };

    let channel_id = match args.get("channel").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "channel".into(),
            })
        }
    };

    let previous = {
        let mut s = state.write().await;
        let prev = s.current_channel.clone();
        s.current_channel = Channel {
            adapter: adapter.into(),
            id: channel_id.into(),
            name: None,
        };
        prev
    };

    ToolResult::ChannelSwitch {
        previous_adapter: previous.adapter,
        previous_channel: previous.id,
    }
}

fn execute_sleep(args: &serde_json::Value) -> ToolResult {
    let minutes = args.get("minutes").and_then(|v| v.as_u64());
    ToolResult::Sleep { minutes }
}

async fn execute_watch(args: &serde_json::Value, state: &SharedState) -> ToolResult {
    let mut s = state.write().await;

    if let Some(add) = args.get("add").and_then(|v| v.as_array()) {
        for ch in add {
            if let (Some(adapter), Some(id)) = (
                ch.get("adapter").and_then(|v| v.as_str()),
                ch.get("id").and_then(|v| v.as_str()),
            ) {
                let channel = Channel {
                    adapter: adapter.into(),
                    id: id.into(),
                    name: ch.get("name").and_then(|v| v.as_str()).map(String::from),
                };
                s.watch(&channel);
            }
        }
    }

    if let Some(remove) = args.get("remove").and_then(|v| v.as_array()) {
        for ch in remove {
            if let (Some(adapter), Some(id)) = (
                ch.get("adapter").and_then(|v| v.as_str()),
                ch.get("id").and_then(|v| v.as_str()),
            ) {
                let channel = Channel {
                    adapter: adapter.into(),
                    id: id.into(),
                    name: None,
                };
                s.unwatch(&channel);
            }
        }
    }

    let watching: Vec<serde_json::Value> = s
        .watch_list
        .iter()
        .map(|key| {
            let parts: Vec<&str> = key.splitn(2, ':').collect();
            serde_json::json!({
                "adapter": parts.get(0).unwrap_or(&""),
                "id": parts.get(1).unwrap_or(&"")
            })
        })
        .collect();

    ToolResult::Success(serde_json::json!({ "watching": watching }))
}

fn execute_summary(args: &serde_json::Value) -> ToolResult {
    let summary = args
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    ToolResult::Summary(summary)
}

async fn execute_create_flash(
    args: &serde_json::Value,
    state: &SharedState,
    generator: &mut SnowflakeGenerator,
    client: &reqwest::Client,
) -> ToolResult {
    let target_dyad = match args.get("target_dyad").and_then(|v| v.as_str()) {
        Some(d) => d,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "target_dyad".into(),
            })
        }
    };

    let target_side = match args.get("target_side").and_then(|v| v.as_str()) {
        Some("left") => Side::Left,
        Some("right") => Side::Right,
        _ => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "target_side".into(),
            })
        }
    };

    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "content".into(),
            })
        }
    };

    let ttl_minutes = args.get("ttl_minutes").and_then(|v| v.as_u64()).unwrap_or(60) as u32;

    let (registry, dyad, side, partner_endpoint, ground) = {
        let s = state.read().await;
        (
            s.registry.clone(),
            s.dyad.clone(),
            s.side.clone(),
            s.partner_endpoint.clone(),
            s.ground.clone(),
        )
    };

    // Shortcut: if target is our partner, use cached endpoint
    let endpoint = if target_dyad == dyad && target_side == state.read().await.partner_side() {
        match partner_endpoint {
            Some(ep) => ep,
            None => {
                return ToolResult::Error(ToolError::TargetNotFound {
                    target: format!("{}:{:?}", target_dyad, target_side),
                })
            }
        }
    } else {
        match registry.worker_endpoint(target_dyad, &target_side) {
            Some(ep) => ep.to_string(),
            None => {
                return ToolResult::Error(ToolError::TargetNotFound {
                    target: format!("{}:{:?}", target_dyad, target_side),
                })
            }
        }
    };

    let id = match generator.next(SnowflakeType::Flash) {
        Ok(id) => id,
        Err(e) => {
            return ToolResult::Error(ToolError::ParseError {
                message: format!("Failed to generate snowflake: {:?}", e),
            })
        }
    };
    let id_str = river_snowflake::format(&id);

    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::minutes(ttl_minutes as i64);

    // Format sender as "dyad:side (ground_name)"
    let from = format!("{}:{:?} ({})", dyad, side, ground.name);

    let flash = Flash {
        id: id_str.clone(),
        from,
        content: content.into(),
        expires_at: expires.to_rfc3339(),
    };

    match client
        .post(format!("{}/flash", endpoint))
        .json(&flash)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => ToolResult::Success(serde_json::json!({
            "id": id_str,
            "sent": true
        })),
        Ok(_) | Err(_) => ToolResult::Error(ToolError::TargetUnreachable {
            target: format!("{}:{:?}", target_dyad, target_side),
        }),
    }
}

async fn execute_request_model(
    args: &serde_json::Value,
    state: &SharedState,
    config: &WorkerConfig,
    client: &reqwest::Client,
) -> ToolResult {
    let model = match args.get("model").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "model".into(),
            })
        }
    };

    let (dyad, side) = {
        let s = state.read().await;
        (s.dyad.clone(), s.side.clone())
    };

    let result = client
        .post(format!("{}/model/switch", config.orchestrator_endpoint))
        .json(&serde_json::json!({
            "dyad": dyad,
            "side": side,
            "model": model
        }))
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();

            // Update state
            if let Ok(new_config) = serde_json::from_value::<crate::config::ModelConfig>(body.clone()) {
                let mut s = state.write().await;
                s.model_config = new_config;
            }

            ToolResult::Success(serde_json::json!({
                "switched": true,
                "model": body
            }))
        }
        Ok(resp) if resp.status() == 400 => ToolResult::Error(ToolError::UnknownModel {
            model: model.into(),
        }),
        Ok(_) | Err(_) => ToolResult::Error(ToolError::AdapterUnreachable {
            adapter: "orchestrator".into(),
        }),
    }
}

async fn execute_switch_roles(
    state: &SharedState,
    config: &WorkerConfig,
    client: &reqwest::Client,
) -> ToolResult {
    let (dyad, side) = {
        let s = state.read().await;
        (s.dyad.clone(), s.side.clone())
    };

    let result = client
        .post(format!("{}/switch_roles", config.orchestrator_endpoint))
        .json(&serde_json::json!({
            "dyad": dyad,
            "side": side
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let new_baton = body
                .get("your_new_baton")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            ToolResult::SwitchRoles { new_baton }
        }
        Ok(resp) if resp.status().as_u16() == 422 => {
            let reason = resp.text().await.unwrap_or_else(|_| "unknown".into());
            ToolResult::Error(ToolError::PartnerRejected { reason })
        }
        Ok(resp) if resp.status() == 409 => ToolResult::Error(ToolError::SwitchInProgress),
        Ok(resp) if resp.status() == 503 => ToolResult::Error(ToolError::PartnerUnreachable),
        Ok(_) | Err(_) => ToolResult::Error(ToolError::PartnerUnreachable),
    }
}

async fn execute_search_embeddings(
    args: &serde_json::Value,
    state: &SharedState,
    client: &reqwest::Client,
) -> ToolResult {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "query".into(),
            })
        }
    };

    let endpoint = {
        let s = state.read().await;
        s.registry.embed_endpoint().map(String::from)
    };

    let endpoint = match endpoint {
        Some(ep) => ep,
        None => return ToolResult::Error(ToolError::EmbedServerUnreachable),
    };

    match client
        .post(format!("{}/search", endpoint))
        .json(&serde_json::json!({ "query": query }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            ToolResult::Success(body)
        }
        Ok(_) | Err(_) => ToolResult::Error(ToolError::EmbedServerUnreachable),
    }
}

async fn execute_next_embedding(
    args: &serde_json::Value,
    state: &SharedState,
    client: &reqwest::Client,
) -> ToolResult {
    let cursor = match args.get("cursor").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "cursor".into(),
            })
        }
    };

    let endpoint = {
        let s = state.read().await;
        s.registry.embed_endpoint().map(String::from)
    };

    let endpoint = match endpoint {
        Some(ep) => ep,
        None => return ToolResult::Error(ToolError::EmbedServerUnreachable),
    };

    match client
        .post(format!("{}/next", endpoint))
        .json(&serde_json::json!({ "cursor": cursor }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            ToolResult::Success(body)
        }
        Ok(resp) if resp.status() == 404 => ToolResult::Error(ToolError::InvalidCursor {
            cursor: cursor.into(),
        }),
        Ok(_) | Err(_) => ToolResult::Error(ToolError::EmbedServerUnreachable),
    }
}

async fn execute_create_move(
    args: &serde_json::Value,
    state: &SharedState,
    generator: &mut SnowflakeGenerator,
) -> ToolResult {
    let channel = match args.get("channel") {
        Some(ch) => ch,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "channel".into(),
            })
        }
    };

    let adapter = channel.get("adapter").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = channel.get("id").and_then(|v| v.as_str()).unwrap_or("");

    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "content".into(),
            })
        }
    };

    let start_id = match args.get("start_message_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "start_message_id".into(),
            })
        }
    };

    let end_id = match args.get("end_message_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "end_message_id".into(),
            })
        }
    };

    let workspace = {
        let s = state.read().await;
        s.workspace.clone()
    };

    let id = match generator.next(SnowflakeType::Move) {
        Ok(id) => id,
        Err(e) => {
            return ToolResult::Error(ToolError::ParseError {
                message: format!("Failed to generate snowflake: {:?}", e),
            })
        }
    };
    let id_str = river_snowflake::format(&id);

    let move_entry = serde_json::json!({
        "id": id_str,
        "channel": { "adapter": adapter, "id": channel_id },
        "content": content,
        "start_message_id": start_id,
        "end_message_id": end_id,
        "created_at": chrono::Utc::now().to_rfc3339()
    });

    let moves_path = workspace.join("moves").join(format!("{}_{}.jsonl", adapter, channel_id));

    // Ensure parent directory exists
    if let Some(parent) = moves_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let line = serde_json::to_string(&move_entry).unwrap_or_default();
    let mut file = match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&moves_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            return ToolResult::Error(ToolError::PermissionDenied {
                path: e.to_string(),
            })
        }
    };

    if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, format!("{}\n", line).as_bytes()).await {
        return ToolResult::Error(ToolError::PermissionDenied {
            path: e.to_string(),
        });
    }

    ToolResult::Success(serde_json::json!({
        "id": id_str,
        "created": true
    }))
}

async fn execute_create_moment(
    args: &serde_json::Value,
    state: &SharedState,
    generator: &mut SnowflakeGenerator,
) -> ToolResult {
    let channel = match args.get("channel") {
        Some(ch) => ch,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "channel".into(),
            })
        }
    };

    let adapter = channel.get("adapter").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = channel.get("id").and_then(|v| v.as_str()).unwrap_or("");

    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "content".into(),
            })
        }
    };

    let start_id = match args.get("start_move_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "start_move_id".into(),
            })
        }
    };

    let end_id = match args.get("end_move_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "end_move_id".into(),
            })
        }
    };

    let workspace = {
        let s = state.read().await;
        s.workspace.clone()
    };

    let id = match generator.next(SnowflakeType::Moment) {
        Ok(id) => id,
        Err(e) => {
            return ToolResult::Error(ToolError::ParseError {
                message: format!("Failed to generate snowflake: {:?}", e),
            })
        }
    };
    let id_str = river_snowflake::format(&id);

    let moment_entry = serde_json::json!({
        "id": id_str,
        "channel": { "adapter": adapter, "id": channel_id },
        "content": content,
        "start_move_id": start_id,
        "end_move_id": end_id,
        "created_at": chrono::Utc::now().to_rfc3339()
    });

    let moments_path = workspace.join("moments").join(format!("{}_{}.jsonl", adapter, channel_id));

    // Ensure parent directory exists
    if let Some(parent) = moments_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let line = serde_json::to_string(&moment_entry).unwrap_or_default();
    let mut file = match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&moments_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            return ToolResult::Error(ToolError::PermissionDenied {
                path: e.to_string(),
            })
        }
    };

    if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, format!("{}\n", line).as_bytes()).await {
        return ToolResult::Error(ToolError::PermissionDenied {
            path: e.to_string(),
        });
    }

    ToolResult::Success(serde_json::json!({
        "id": id_str,
        "created": true
    }))
}

async fn execute_adapter(
    args: &serde_json::Value,
    state: &SharedState,
    client: &reqwest::Client,
) -> ToolResult {
    let adapter = match args.get("adapter").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "adapter".into(),
            })
        }
    };

    let request = match args.get("request") {
        Some(r) => r,
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "request".into(),
            })
        }
    };

    let endpoint = {
        let s = state.read().await;
        s.registry.adapter_endpoint(adapter).map(String::from)
    };

    let endpoint = match endpoint {
        Some(ep) => ep,
        None => {
            return ToolResult::Error(ToolError::AdapterNotFound {
                adapter: adapter.into(),
            })
        }
    };

    match client
        .post(format!("{}/execute", endpoint))
        .json(request)
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            ToolResult::Success(body)
        }
        Ok(resp) => {
            let msg = resp.text().await.unwrap_or_default();
            ToolResult::Error(ToolError::SendFailed { reason: msg })
        }
        Err(_) => ToolResult::Error(ToolError::AdapterUnreachable {
            adapter: adapter.into(),
        }),
    }
}

async fn execute_read_history(
    args: &serde_json::Value,
    state: &SharedState,
) -> ToolResult {
    // Extract required params
    let channel = match args.get("channel").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "channel".into(),
            });
        }
    };

    let adapter = match args.get("adapter").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => {
            return ToolResult::Error(ToolError::MissingParameter {
                name: "adapter".into(),
            });
        }
    };

    // Optional params
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|l| l.min(100) as u32);
    let before = args.get("before").and_then(|v| v.as_str()).map(String::from);
    let after = args.get("after").and_then(|v| v.as_str()).map(String::from);

    // Check mutual exclusivity
    if before.is_some() && after.is_some() {
        return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
            success: false,
            error: Some("Cannot specify both 'before' and 'after'".into()),
            ..Default::default()
        }).unwrap());
    }

    let s = state.read().await;

    // Find adapter in registry
    let adapter_entry = s.registry.processes.iter().find(|p| {
        if let river_protocol::ProcessEntry::Adapter { adapter_type, .. } = p {
            adapter_type == &adapter
        } else {
            false
        }
    });

    let adapter_endpoint = match adapter_entry {
        Some(river_protocol::ProcessEntry::Adapter { endpoint, features, .. }) => {
            // Check ReadHistory feature
            if !features.contains(&(river_adapter::FeatureId::ReadHistory as u16)) {
                return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                    success: false,
                    error: Some("Adapter does not support ReadHistory".into()),
                    ..Default::default()
                }).unwrap());
            }
            endpoint.clone()
        }
        _ => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some(format!("Adapter '{}' not found", adapter)),
                ..Default::default()
            }).unwrap());
        }
    };

    let workspace = s.workspace.clone();
    drop(s);

    // Build request
    let request = river_adapter::OutboundRequest::ReadHistory {
        channel: channel.clone(),
        limit,
        before,
        after,
    };

    // Call adapter
    let client = reqwest::Client::new();
    let response = match client
        .post(format!("{}/execute", adapter_endpoint))
        .json(&request)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some(format!("Request failed: {}", e)),
                ..Default::default()
            }).unwrap());
        }
    };

    // Parse response
    let resp: river_adapter::OutboundResponse = match response.json().await {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some(format!("Invalid response: {}", e)),
                ..Default::default()
            }).unwrap());
        }
    };

    if !resp.ok {
        let err = resp.error.unwrap_or_else(|| river_adapter::ResponseError::new(
            river_adapter::ErrorCode::PlatformError,
            "Unknown error",
        ));
        return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
            success: false,
            error: Some(err.message),
            retry_after_ms: err.retry_after_ms,
            ..Default::default()
        }).unwrap());
    }

    // Extract messages
    let messages = match resp.data {
        Some(river_adapter::ResponseData::History { messages }) => messages,
        _ => {
            return ToolResult::Success(serde_json::to_value(ReadHistoryResult {
                success: false,
                error: Some("Unexpected response format".into()),
                ..Default::default()
            }).unwrap());
        }
    };

    // Write to conversation file
    let conv_channel = river_adapter::Channel {
        adapter: adapter.clone(),
        id: channel.clone(),
        name: None,
    };
    let path = crate::conversation::conversation_path_for_channel(&workspace, &conv_channel);

    let mut oldest_id: Option<String> = None;
    let mut newest_id: Option<String> = None;

    for msg in &messages {
        // Track oldest/newest
        if oldest_id.is_none() || msg.message_id < *oldest_id.as_ref().unwrap() {
            oldest_id = Some(msg.message_id.clone());
        }
        if newest_id.is_none() || msg.message_id > *newest_id.as_ref().unwrap() {
            newest_id = Some(msg.message_id.clone());
        }

        let line = river_protocol::conversation::Message {
            direction: river_protocol::conversation::MessageDirection::Unread,
            timestamp: msg.timestamp.clone(),
            id: msg.message_id.clone(),
            author: river_protocol::Author {
                name: msg.author.name.clone(),
                id: msg.author.id.clone(),
                bot: msg.author.bot,
            },
            content: msg.content.clone(),
            reactions: vec![],
        };

        if let Err(e) = river_protocol::conversation::Conversation::append_line(
            &path,
            &river_protocol::conversation::Line::Message(line),
        ) {
            tracing::warn!(error = %e, "Failed to write history message to conversation file");
        }
    }

    ToolResult::Success(serde_json::to_value(ReadHistoryResult {
        success: true,
        messages_fetched: messages.len(),
        oldest_id,
        newest_id,
        error: None,
        retry_after_ms: None,
    }).unwrap())
}
