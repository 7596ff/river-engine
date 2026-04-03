//! Main worker loop.

use crate::config::WorkerConfig;
use crate::llm::{get_tool_definitions, LlmClient, LlmContent};
use crate::persistence::{append_to_context, load_context};
use crate::state::SharedState;
use crate::tools::{execute_tool, ToolResult};
use river_adapter::Side;
use river_context::OpenAIMessage;
use river_snowflake::{AgentBirth, SnowflakeGenerator};
use serde::{Deserialize, Serialize};
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

    // Load existing context
    let context_path = {
        let s = state.read().await;
        let side_str = match s.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        s.workspace.join(side_str).join("context.jsonl")
    };
    let mut messages = load_context(&context_path);

    // If starting fresh, inject role and identity content
    if messages.is_empty() {
        let s = state.read().await;

        // Inject role content first (defines behavior)
        if let Some(ref role) = s.role_content {
            messages.push(OpenAIMessage::system(role.clone()));
        }

        // Inject identity content (defines self-perception)
        if let Some(ref identity) = s.identity_content {
            messages.push(OpenAIMessage::system(identity.clone()));
        }

        // Inject initial message if provided
        if let Some(ref initial) = s.initial_message {
            messages.push(OpenAIMessage::user(initial.clone()));
        }

        drop(s);

        // Persist the initial context
        for msg in &messages {
            append_to_context(&context_path, msg).ok();
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
        // Check for pending flashes
        let pending_flashes = {
            let mut s = state.write().await;
            std::mem::take(&mut s.pending_flashes)
        };

        // Inject flashes as system messages
        for flash in pending_flashes {
            messages.push(OpenAIMessage::system(format!(
                "[Flash from {}] {}",
                flash.from, flash.content
            )));
        }

        // Get current token estimate
        let token_count = {
            let s = state.read().await;
            s.token_count
        };

        // Check context pressure
        if token_count > context_limit * 95 / 100 {
            tracing::warn!("Context at 95%, forcing summary");
            return force_summary(&state, config, &mut messages, &mut llm).await;
        }

        if token_count > context_limit * 80 / 100 {
            messages.push(OpenAIMessage::system(
                "Context at 80%. Consider wrapping up or summarizing.",
            ));
        }

        // Check for pending notifications
        let notifications = {
            let mut s = state.write().await;
            std::mem::take(&mut s.pending_notifications)
        };

        // Add notification status if any
        if !notifications.is_empty() {
            let notif_summary: Vec<String> = notifications
                .iter()
                .map(|n| format!("{}:{} ({} new)", n.channel.adapter, n.channel.id, n.count))
                .collect();
            messages.push(OpenAIMessage::system(format!(
                "[New messages: {}]",
                notif_summary.join(", ")
            )));
        }

        // Call LLM
        let response = match llm.chat(&messages, Some(&tools)).await {
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
                // Model responded with text
                messages.push(OpenAIMessage::assistant(&text));
                append_to_context(&context_path, messages.last().unwrap()).ok();

                // Add a prompt to use tools
                messages.push(OpenAIMessage::system(
                    "Use tools to take action. Use 'speak' to send messages, 'summary' to end session.",
                ));
            }
            LlmContent::ToolCalls(calls) => {
                // Add assistant message with tool calls
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

                messages.push(OpenAIMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(tool_calls_json),
                    tool_call_id: None,
                });
                append_to_context(&context_path, messages.last().unwrap()).ok();

                // Execute tools
                let mut should_sleep = None;
                let mut summary_text = None;
                let mut new_baton = None;

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
                    };

                    // Add tool result
                    messages.push(OpenAIMessage::tool(&call.id, result_content));
                    append_to_context(&context_path, messages.last().unwrap()).ok();
                }

                // Handle summary exit
                if let Some(summary) = summary_text {
                    return WorkerOutput {
                        dyad: config.dyad.clone(),
                        side: config.side.clone(),
                        status: ExitStatus::Done {
                            wake_after_minutes: None,
                        },
                        summary,
                    };
                }

                // Handle role switch
                if let Some(baton) = new_baton {
                    // Reload role file
                    let role_path = {
                        let s = state.read().await;
                        s.workspace.join("roles").join(format!("{}.md", baton))
                    };
                    if let Ok(role_content) = tokio::fs::read_to_string(&role_path).await {
                        messages.push(OpenAIMessage::system(format!(
                            "[Role switched to {}]\n\n{}",
                            baton, role_content
                        )));
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
    state: &SharedState,
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
