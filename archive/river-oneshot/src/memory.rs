//! Memory management for river-oneshot.

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{ConversationTurn, MemoryEntry, PlannedAction, TurnOutput};

/// A flash: a curated memory surfaced into context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flash {
    /// Unique identifier.
    pub id: String,
    /// The content to surface.
    pub content: String,
    /// Source identifier (for deduplication).
    pub source: String,
    /// When this flash was created.
    pub created: DateTime<Utc>,
    /// How many turns until expiry (None = duration-based).
    pub turns_remaining: Option<u8>,
    /// Absolute expiry time (None = turn-based).
    pub expires_at: Option<DateTime<Utc>>,
}

impl Flash {
    /// Create a flash that expires after N turns.
    pub fn turns(content: impl Into<String>, source: impl Into<String>, turns: u8) -> Self {
        Self {
            id: uuid_simple(),
            content: content.into(),
            source: source.into(),
            created: Utc::now(),
            turns_remaining: Some(turns),
            expires_at: None,
        }
    }

    /// Create a flash that expires after a duration.
    pub fn duration(content: impl Into<String>, source: impl Into<String>, duration: Duration) -> Self {
        Self {
            id: uuid_simple(),
            content: content.into(),
            source: source.into(),
            created: Utc::now(),
            turns_remaining: None,
            expires_at: Some(Utc::now() + duration),
        }
    }

    /// Check if this flash has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(turns) = self.turns_remaining {
            if turns == 0 {
                return true;
            }
        }
        if let Some(expires) = self.expires_at {
            if Utc::now() > expires {
                return true;
            }
        }
        false
    }

    /// Tick down turn-based TTL.
    pub fn tick_turn(&mut self) {
        if let Some(ref mut turns) = self.turns_remaining {
            *turns = turns.saturating_sub(1);
        }
    }
}

/// Simple UUID generation without external crate.
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:x}-{:x}", now.as_secs(), now.subsec_nanos())
}

/// Memory handle for conversation and action state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memory {
    /// Recent turns for LLM context.
    pub conversation: Vec<ConversationTurn>,
    /// Queued but not yet executed.
    #[serde(default)]
    pub pending_actions: Vec<PlannedAction>,
    /// Active flashes (surfaced memories).
    #[serde(default)]
    pub flashes: Vec<Flash>,
    /// Cached output from previous cycle.
    #[serde(skip)]
    pub deferred_output: Option<TurnOutput>,
}

impl Memory {
    /// Create a new memory instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a user message.
    pub fn record_user(&mut self, message: &str) {
        self.conversation.push(ConversationTurn::User(message.to_string()));
    }

    /// Record an assistant response.
    pub fn record_assistant(&mut self, text: &str) {
        self.conversation.push(ConversationTurn::Assistant(text.to_string()));
    }

    /// Record a tool use request from the assistant.
    pub fn record_tool_use(&mut self, id: &str, name: &str, input: serde_json::Value) {
        self.conversation.push(ConversationTurn::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input,
        });
    }

    /// Record a tool result.
    pub fn record_tool_result(&mut self, id: &str, content: &str, success: bool) {
        self.conversation.push(ConversationTurn::ToolResult {
            id: id.to_string(),
            content: content.to_string(),
            success,
        });
    }

    /// Record a turn output in memory.
    pub fn record(&mut self, output: &TurnOutput) {
        match output {
            TurnOutput::Thought(plan) => {
                // Record any text response
                if let Some(response) = &plan.response {
                    self.record_assistant(response);
                }
                // Record tool use requests
                for action in &plan.actions {
                    self.record_tool_use(
                        &action.tool_use_id,
                        &action.skill_name,
                        action.parameters.clone(),
                    );
                }
            }
            TurnOutput::Action(result) => {
                // Record tool result
                let content = if result.success {
                    serde_json::to_string(&result.payload).unwrap_or_default()
                } else {
                    result.error.clone().unwrap_or_else(|| "Unknown error".to_string())
                };
                self.record_tool_result(&result.tool_use_id, &content, result.success);
            }
        }
    }

    /// Add a flash (surfaced memory).
    pub fn add_flash(&mut self, flash: Flash) {
        // Deduplicate by source
        self.flashes.retain(|f| f.source != flash.source);
        self.flashes.push(flash);
    }

    /// Get active (non-expired) flashes.
    pub fn active_flashes(&self) -> Vec<&Flash> {
        self.flashes.iter().filter(|f| !f.is_expired()).collect()
    }

    /// Tick all turn-based flashes and remove expired ones.
    pub fn tick_flashes(&mut self) {
        for flash in &mut self.flashes {
            flash.tick_turn();
        }
        self.flashes.retain(|f| !f.is_expired());
    }

    /// Save memory to disk.
    pub async fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        tokio::fs::write(path, json).await?;
        Ok(())
    }

    /// Load memory from disk.
    pub async fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let json = tokio::fs::read_to_string(path).await?;
            let memory: Memory = serde_json::from_str(&json)?;
            Ok(memory)
        } else {
            Ok(Self::new())
        }
    }

    /// Truncate conversation to fit within a limit.
    /// Keeps the most recent turns.
    pub fn truncate(&mut self, max_turns: usize) {
        if self.conversation.len() > max_turns {
            let remove = self.conversation.len() - max_turns;
            self.conversation.drain(..remove);
        }
    }

    /// Find memories relevant to a query.
    /// TODO: Implement vector search in Phase 4.
    pub fn relevant_to(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
}
