//! Main worker loop.

use crate::config::WorkerConfig;
use crate::llm::{get_tool_definitions, LlmClient, LlmContent};
use crate::persistence::{append_to_context, clear_context, load_context, should_persist};
use crate::state::SharedState;
use crate::tools::{execute_tool, ToolResult};
use crate::workspace_loader::load_channels;
use river_adapter::Side;
use river_context::{build_context, ContextRequest, OpenAIMessage};
use river_protocol::Channel;
use river_snowflake::{AgentBirth, SnowflakeGenerator};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

/// Worker output sent to orchestrator.
#[derive(Debug, Serialize)]
pub struct WorkerOutput {
    pub dyad: String,
    pub side: Side,
    pub status: ExitStatus,
    pub summary: String,
}

/// Exit status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExitStatus {
    Done { wake_after_minutes: Option<u64> },
    ContextExhausted,
    Error { message: String },
}

/// Run the main worker loop.
pub async fn run_loop(
    state: SharedState,
    config: &WorkerConfig,
    client: &reqwest::Client,
) -> WorkerOutput {
    // Initialize LLM client
    let model_config = {
        let s = state.read().await;
        s.model_config.clone()
    };
    let mut llm = LlmClient::new(&model_config);

    // Initialize snowflake generator
    let birth = AgentBirth::now();
    let mut generator = SnowflakeGenerator::new(birth);

    // Path to LLM history (stream of consciousness)
    let context_path = {
        let s = state.read().await;
        let side_str = match s.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        s.workspace.join(side_str).join("context.jsonl")
    };

    // Load LLM history (the stream of consciousness - grows until reset)
    let mut llm_history = load_context(&context_path);

    // If starting fresh with an initial message, add it to history
    // Note: initial messages are user messages and not persisted (they're re-assembled)
    if llm_history.is_empty() {
        let s = state.read().await;
        if let Some(ref initial) = s.initial_message {
            let msg = OpenAIMessage::user(initial.clone());
            llm_history.push(msg.clone());
            // Not persisted - user messages are re-assembled from workspace
        }
    }

    // Get context limit
    let context_limit = {
        let s = state.read().await;
        s.model_config.context_limit
    };

    // Tool definitions
    let tools = get_tool_definitions();

    // Wait for first notification/flash if needed
    wait_for_activation(&state).await;

    // Main loop
    loop {
        // Get current token estimate
        let token_count = {
            let s = state.read().await;
            s.token_count
        };

        // Check context pressure
        if token_count > context_limit * 95 / 100 {
            tracing::warn!("Context at 95%, forcing summary");
            return force_summary(config, &mut llm_history, &mut llm).await;
        }

        // Add context pressure warning to history if needed
        if token_count > context_limit * 80 / 100 {
            let msg = OpenAIMessage::system(
                "Context at 80%. Consider wrapping up or using the summary tool.",
            );
            llm_history.push(msg.clone());
            // Context warnings are persisted
            if should_persist(&msg) {
                append_to_context(&context_path, &msg).ok();
            }
        }

        // Check for pending notifications and add to history
        let notifications = {
            let mut s = state.write().await;
            std::mem::take(&mut s.pending_notifications)
        };

        if !notifications.is_empty() {
            let notif_summary: Vec<String> = notifications
                .iter()
                .map(|n| format!("{}:{} ({} new)", n.channel.adapter, n.channel.id, n.count))
                .collect();
            let msg = OpenAIMessage::system(format!(
                "[New messages: {}]",
                notif_summary.join(", ")
            ));
            llm_history.push(msg.clone());
            // Notifications are not persisted - they are ephemeral
        }

        // Assemble full context: role + identity + workspace + history
        let full_context = match assemble_full_context(&state, &llm_history, context_limit).await {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::error!("Failed to assemble context: {:?}", e);
                // Fall back to just history if assembly fails
                llm_history.clone()
            }
        };

        // Call LLM with full assembled context
        let response = match llm.chat(&full_context, Some(&tools)).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("LLM error: {}", e);
                return WorkerOutput {
                    dyad: config.dyad.clone(),
                    side: config.side.clone(),
                    status: ExitStatus::Error {
                        message: e.to_string(),
                    },
                    summary: "LLM unreachable".into(),
                };
            }
        };

        // Update token count
        {
            let mut s = state.write().await;
            s.token_count = response.usage.total_tokens;
        }

        match response.content {
            LlmContent::Text(text) => {
                // Model responded with text - persist assistant output
                let msg = OpenAIMessage::assistant(&text);
                llm_history.push(msg.clone());
                if should_persist(&msg) {
                    append_to_context(&context_path, &msg).ok();
                }

                // Add a prompt to use tools (not persisted, just for this turn)
                let prompt = OpenAIMessage::system(
                    "Use tools to take action. Use 'speak' to send messages, 'summary' to end session.",
                );
                llm_history.push(prompt.clone());
                // Not persisted - ephemeral prompt
            }
            LlmContent::ToolCalls(calls) => {
                // Add assistant message with tool calls - persist assistant output
                let tool_calls_json: Vec<river_context::ToolCall> = calls
                    .iter()
                    .map(|c| river_context::ToolCall {
                        id: c.id.clone(),
                        call_type: "function".into(),
                        function: river_context::FunctionCall {
                            name: c.name.clone(),
                            arguments: c.arguments.clone(),
                        },
                    })
                    .collect();

                let msg = OpenAIMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(tool_calls_json),
                    tool_call_id: None,
                };
                llm_history.push(msg.clone());
                if should_persist(&msg) {
                    append_to_context(&context_path, &msg).ok();
                }

                // Execute tools
                let mut should_sleep = None;
                let mut summary_text = None;
                let mut new_baton = None;
                let mut channel_switched = false;

                for call in &calls {
                    let result = execute_tool(call, &state, config, &mut generator, client).await;

                    let result_content = match &result {
                        ToolResult::Success(v) => serde_json::to_string(v).unwrap_or_default(),
                        ToolResult::Error(e) => serde_json::to_string(e).unwrap_or_default(),
                        ToolResult::Summary(s) => {
                            summary_text = Some(s.clone());
                            serde_json::json!({"status": "exiting", "summary": s}).to_string()
                        }
                        ToolResult::Sleep { minutes } => {
                            should_sleep = Some(*minutes);
                            serde_json::json!({"sleeping": true, "minutes": minutes}).to_string()
                        }
                        ToolResult::SwitchRoles { new_baton: b } => {
                            new_baton = Some(b.clone());
                            serde_json::json!({"switched": true, "new_baton": b}).to_string()
                        }
                        ToolResult::ChannelSwitch { previous_adapter, previous_channel } => {
                            channel_switched = true;
                            serde_json::json!({
                                "switched": true,
                                "previous": {
                                    "adapter": previous_adapter,
                                    "channel": previous_channel
                                }
                            }).to_string()
                        }
                    };

                    // Add tool result to history (not persisted - tool results go to inbox)
                    let msg = OpenAIMessage::tool(&call.id, result_content);
                    llm_history.push(msg.clone());
                    // Not persisted - tool results are ephemeral in live context
                }

                // Handle summary exit
                if let Some(summary) = summary_text {
                    // Clear context file - worker is done with this conversation
                    if let Err(e) = clear_context(&context_path) {
                        tracing::warn!("Failed to clear context: {}", e);
                    }
                    return WorkerOutput {
                        dyad: config.dyad.clone(),
                        side: config.side.clone(),
                        status: ExitStatus::Done {
                            wake_after_minutes: None,
                        },
                        summary,
                    };
                }

                // Handle role switch - update state so next assemble uses new role
                if let Some(baton_str) = new_baton {
                    // Parse baton string to enum
                    let new_baton_enum = match baton_str.as_str() {
                        "actor" => river_adapter::Baton::Actor,
                        "spectator" => river_adapter::Baton::Spectator,
                        _ => {
                            tracing::warn!("Unknown baton: {}", baton_str);
                            continue;
                        }
                    };

                    // Reload role file into state
                    let role_path = {
                        let s = state.read().await;
                        s.workspace.join("roles").join(format!("{}.md", baton_str))
                    };
                    if let Ok(role_content) = tokio::fs::read_to_string(&role_path).await {
                        let mut s = state.write().await;
                        s.role_content = Some(role_content.clone());
                        s.baton = new_baton_enum;
                        drop(s);

                        // Add a note to history about the switch (not persisted)
                        let msg = OpenAIMessage::system(format!(
                            "[Role switched to {}]",
                            baton_str
                        ));
                        llm_history.push(msg.clone());
                        // Not persisted - role switch is ephemeral notification
                    }
                }

                // Handle sleep
                if let Some(minutes) = should_sleep {
                    let mut s = state.write().await;
                    s.sleeping = true;
                    s.sleep_until = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));
                    drop(s);

                    // Wait for wake
                    sleep_until_wake(&state, minutes).await;
                }

                // Channel switch - no special handling needed
                // The next assemble_full_context() will re-render workspace data
                // with the new channel ordering automatically
                if channel_switched {
                    tracing::debug!("Channel switched, workspace context will be re-rendered next turn");
                }
            }
        }
    }
}

/// Wait for first activation (notification or flash).
async fn wait_for_activation(state: &SharedState) {
    loop {
        {
            let s = state.read().await;
            if !s.pending_notifications.is_empty() || !s.pending_flashes.is_empty() {
                return;
            }
            // If start_sleeping was true, we're already activated
            if !s.sleeping && s.pending_notifications.is_empty() && s.pending_flashes.is_empty() {
                // Wait a bit and check again
            } else if s.sleeping {
                // Already in sleep mode, will wait for wake
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Sleep until woken by flash, notification, or timeout.
async fn sleep_until_wake(state: &SharedState, minutes: Option<u64>) {
    let deadline = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));

    loop {
        {
            let s = state.read().await;
            if !s.sleeping {
                return;
            }
        }

        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                let mut s = state.write().await;
                s.sleeping = false;
                return;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Force a summary when context is exhausted.
async fn force_summary(
    config: &WorkerConfig,
    messages: &mut Vec<OpenAIMessage>,
    llm: &mut LlmClient,
) -> WorkerOutput {
    // Add summary request
    messages.push(OpenAIMessage::system(
        "Context limit reached. Summarize what you've accomplished and what remains. This will be passed to your next session.",
    ));

    // Call LLM without tools for final summary
    let response = llm.chat(messages, None).await;

    let summary = match response {
        Ok(r) => match r.content {
            LlmContent::Text(t) => t,
            _ => "Context exhausted, no summary available".into(),
        },
        Err(e) => format!("Context exhausted, summary failed: {}", e),
    };

    WorkerOutput {
        dyad: config.dyad.clone(),
        side: config.side.clone(),
        status: ExitStatus::ContextExhausted,
        summary,
    }
}

/// Assemble context from workspace data using river-context.
///
/// This loads moves, moments, and messages from workspace files,
/// then assembles them into a properly ordered context for the LLM.
///
/// # Arguments
/// * `workspace` - Path to workspace directory
/// * `channels` - Channels to include, ordered by recency (current first)
/// * `flashes` - Pending flashes to inject
/// * `history` - LLM conversation history (from context.jsonl)
/// * `max_tokens` - Token budget
#[allow(dead_code)]
async fn assemble_context_from_workspace(
    workspace: &Path,
    channels: &[Channel],
    flashes: Vec<river_context::Flash>,
    history: Vec<OpenAIMessage>,
    max_tokens: usize,
) -> Result<Vec<OpenAIMessage>, river_context::ContextError> {
    // Load channel data from workspace files
    let channel_contexts = load_channels(workspace, channels).await;

    // Build the context request
    let request = ContextRequest {
        channels: channel_contexts,
        flashes,
        history,
        max_tokens,
        now: chrono::Utc::now().to_rfc3339(),
    };

    // Assemble using river-context
    let response = build_context(request)?;

    Ok(response.messages)
}

/// Assemble full context for LLM call.
///
/// Structure:
/// 1. Role content (defines behavior)
/// 2. Identity content (defines self-perception)
/// 3. Workspace context via build_context():
///    - moments, moves from channels (reordered by current channel)
///    - flashes (ephemeral)
/// 4. LLM history from context.jsonl (stream of consciousness)
/// 5. Current channel messages
async fn assemble_full_context(
    state: &SharedState,
    llm_history: &[OpenAIMessage],
    max_tokens: usize,
) -> Result<Vec<OpenAIMessage>, river_context::ContextError> {
    let (role_content, identity_content, name, workspace, channels, flashes) = {
        let s = state.read().await;

        // Build channel list with current first
        let mut channels = vec![s.current_channel.clone()];
        for key in &s.watch_list {
            if let Some((adapter, id)) = key.split_once(':') {
                if adapter != s.current_channel.adapter || id != s.current_channel.id {
                    channels.push(Channel {
                        adapter: adapter.to_string(),
                        id: id.to_string(),
                        name: None,
                    });
                }
            }
        }

        (
            s.role_content.clone(),
            s.identity_content.clone(),
            s.name.clone(),
            s.workspace.clone(),
            channels,
            s.pending_flashes.clone(),
        )
    };

    // Load channel data from workspace
    let channel_contexts = load_channels(&workspace, &channels).await;

    // Build context request
    let request = ContextRequest {
        channels: channel_contexts,
        flashes,
        history: llm_history.to_vec(),
        max_tokens,
        now: chrono::Utc::now().to_rfc3339(),
    };

    // Assemble workspace context + history
    let mut response = build_context(request)?;

    // Prepend role and identity
    let mut full_context = Vec::new();

    if let Some(role) = role_content {
        full_context.push(OpenAIMessage::system(role));
    }

    if let Some(identity) = identity_content {
        let identity_with_name = if let Some(ref n) = name {
            format!("Your name is {}.\n\n{}", n, identity)
        } else {
            identity
        };
        full_context.push(OpenAIMessage::system(identity_with_name));
    } else if let Some(n) = name {
        full_context.push(OpenAIMessage::system(format!("Your name is {}.", n)));
    }

    // Add workspace context + history + messages
    full_context.append(&mut response.messages);

    Ok(full_context)
}
