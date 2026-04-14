//! Context building for cycles.

use std::path::Path;

use chrono::{Local, Utc};

use crate::memory::Memory;
use crate::types::{
    ContentBlock, ConversationTurn, CycleInput, Message, MessageContent, ReasoningContext,
    ToolDef, TurnOutput,
};

/// Build the system prompt from workspace files and defaults.
pub async fn build_system_prompt(workspace: &Path, default_prompt: &str) -> String {
    let mut parts = Vec::new();

    // 1. Load identity files from workspace
    for filename in &["IDENTITY.md", "RULES.md", "AGENTS.md"] {
        let path = workspace.join(filename);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            let content = content.trim();
            if !content.is_empty() {
                parts.push(content.to_string());
            }
        }
    }

    // 2. If no identity files, use default
    if parts.is_empty() {
        parts.push(default_prompt.to_string());
    }

    // 3. Add current time
    let now = Local::now();
    let time_str = now.format("%Y-%m-%d %H:%M:%S %Z").to_string();
    parts.push(format!("Current time: {}", time_str));

    parts.join("\n\n---\n\n")
}

/// Build reasoning context from cycle input and memory.
pub fn build_reasoning_context(
    input: &CycleInput,
    memory: &Memory,
    system_prompt: &str,
    tools: Vec<ToolDef>,
) -> ReasoningContext {
    let mut messages = vec![];

    // 1. System prompt (already includes identity + time)
    messages.push(Message::system(system_prompt));

    // 2. Active flashes (surfaced memories)
    let active_flashes = memory.active_flashes();
    if !active_flashes.is_empty() {
        let flash_content: Vec<String> = active_flashes
            .iter()
            .map(|f| format!("[{}] {}", f.source, f.content))
            .collect();
        messages.push(Message::system(format!(
            "[Relevant context]\n{}",
            flash_content.join("\n\n")
        )));
    }

    // 3. Conversation history
    for turn in &memory.conversation {
        match turn {
            ConversationTurn::User(text) => {
                messages.push(Message::user(text));
            }
            ConversationTurn::Assistant(text) => {
                messages.push(Message::assistant(text));
            }
            ConversationTurn::ToolUse { id, name, input } => {
                // Assistant's tool use request
                messages.push(Message {
                    role: crate::types::Role::Assistant,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    }]),
                });
            }
            ConversationTurn::ToolResult { id, content, success } => {
                // Tool result from user role
                messages.push(Message::tool_result(id, content, !success));
            }
        }
    }

    // 4. Previous action result (if execution won last cycle)
    if let Some(TurnOutput::Action(ref result)) = input.previous_output {
        let content = if result.success {
            serde_json::to_string(&result.payload).unwrap_or_default()
        } else {
            result.error.clone().unwrap_or_else(|| "Unknown error".to_string())
        };
        messages.push(Message::tool_result(&result.tool_use_id, &content, !result.success));
    }

    // 5. New user input (if any)
    if let Some(ref msg) = input.user_message {
        messages.push(Message::user(msg));
    }

    ReasoningContext { messages, tools }
}

/// Parse an LLM response into a Plan.
pub fn parse_plan(response: crate::types::LlmResponse) -> crate::types::Plan {
    let mut actions = vec![];
    let mut response_text = None;

    for block in response.content {
        match block {
            ContentBlock::Text { text } => {
                response_text = Some(text);
            }
            ContentBlock::ToolUse { id, name, input } => {
                actions.push(crate::types::PlannedAction {
                    tool_use_id: id,
                    skill_name: name,
                    parameters: input,
                    priority: 0,
                });
            }
            ContentBlock::ToolResult { .. } => {
                // Shouldn't appear in LLM response, ignore
            }
        }
    }

    crate::types::Plan {
        summary: response_text.clone().unwrap_or_default(),
        actions,
        response: response_text,
    }
}
