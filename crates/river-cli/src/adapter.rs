//! Adapter state management.

use chrono::{DateTime, Utc};
use river_adapter::FeatureId;
use river_snowflake::{AgentBirth, SnowflakeGenerator, SnowflakeType};
use serde::{Deserialize, Serialize};

/// Display message types.
#[derive(Debug, Clone)]
pub enum DisplayMessage {
    User {
        id: String,
        content: String,
        timestamp: DateTime<Utc>,
    },
    Worker {
        id: String,
        content: String,
        timestamp: DateTime<Utc>,
    },
    System {
        content: String,
        timestamp: DateTime<Utc>,
    },
}

impl DisplayMessage {
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::User { timestamp, .. } => *timestamp,
            Self::Worker { timestamp, .. } => *timestamp,
            Self::System { timestamp, .. } => *timestamp,
        }
    }
}

/// Tool trace for debug display.
#[derive(Debug, Clone)]
pub struct ToolTrace {
    pub tool: String,
    pub args: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Active flash with TTL.
#[derive(Debug, Clone)]
pub struct ActiveFlash {
    pub from: String,
    pub content: String,
    pub expires_at: DateTime<Utc>,
}

/// Debug events from worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DebugEvent {
    ToolCall {
        tool: String,
        args: serde_json::Value,
        result: Option<serde_json::Value>,
        error: Option<String>,
        timestamp: String,
    },
    Thinking {
        started: bool,
        timestamp: String,
    },
    LlmRequest {
        model: String,
        token_count: usize,
        timestamp: String,
    },
    LlmResponse {
        token_count: usize,
        has_tool_calls: bool,
        timestamp: String,
    },
}

/// Flash message from worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashMessage {
    pub from: String,
    pub content: String,
    pub expires_at: String,
}

/// Adapter state.
pub struct AdapterState {
    // Identity
    pub dyad: String,
    pub adapter_type: String,
    pub channel: String,

    // Worker binding
    pub worker_endpoint: Option<String>,

    // Conversation (top pane)
    pub messages: Vec<DisplayMessage>,
    pub conversation_scroll: usize,

    // Debug info (bottom pane)
    pub tool_traces: Vec<ToolTrace>,
    pub flashes: Vec<ActiveFlash>,
    pub thinking: bool,
    pub debug_scroll: usize,
    pub show_debug: bool,

    // Input (multi-line)
    pub input: String,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub last_char_was_enter: bool,

    // Quit safety
    pub quit_pressed: bool,

    // Snowflake generation
    pub generator: SnowflakeGenerator,
}

impl AdapterState {
    pub fn new(dyad: String, adapter_type: String, channel: String) -> Self {
        let birth = AgentBirth::now();
        Self {
            dyad,
            adapter_type,
            channel,
            worker_endpoint: None,
            messages: Vec::new(),
            conversation_scroll: 0,
            tool_traces: Vec::new(),
            flashes: Vec::new(),
            thinking: false,
            debug_scroll: 0,
            show_debug: true,
            input: String::new(),
            input_history: Vec::new(),
            history_index: None,
            last_char_was_enter: false,
            quit_pressed: false,
            generator: SnowflakeGenerator::new(birth),
        }
    }

    pub fn add_user_message(&mut self, content: &str) -> String {
        let id = self.generator.next(SnowflakeType::Message).to_string();
        self.messages.push(DisplayMessage::User {
            id: id.clone(),
            content: content.to_string(),
            timestamp: Utc::now(),
        });
        // Auto-scroll to bottom
        self.conversation_scroll = 0;
        id
    }

    pub fn add_worker_message(&mut self, content: &str) -> String {
        let id = self.generator.next(SnowflakeType::Message).to_string();
        self.messages.push(DisplayMessage::Worker {
            id: id.clone(),
            content: content.to_string(),
            timestamp: Utc::now(),
        });
        self.thinking = false;
        // Auto-scroll to bottom
        self.conversation_scroll = 0;
        id
    }

    pub fn add_system_message(&mut self, content: &str) {
        self.messages.push(DisplayMessage::System {
            content: content.to_string(),
            timestamp: Utc::now(),
        });
        self.conversation_scroll = 0;
    }

    pub fn add_tool_trace(&mut self, tool: &str, args: &str, result: Option<&str>, error: Option<&str>) {
        self.tool_traces.push(ToolTrace {
            tool: tool.to_string(),
            args: args.to_string(),
            result: result.map(|s| s.to_string()),
            error: error.map(|s| s.to_string()),
            timestamp: Utc::now(),
        });
        // Keep last 50 traces
        if self.tool_traces.len() > 50 {
            self.tool_traces.remove(0);
        }
    }

    pub fn add_flash(&mut self, from: &str, content: &str, expires_at: DateTime<Utc>) {
        self.flashes.push(ActiveFlash {
            from: from.to_string(),
            content: content.to_string(),
            expires_at,
        });
    }

    pub fn cleanup_expired_flashes(&mut self) {
        let now = Utc::now();
        self.flashes.retain(|f| f.expires_at > now);
    }

    pub fn generate_message_id(&mut self) -> String {
        self.generator.next(SnowflakeType::Message).to_string()
    }
}

/// Features supported by mock adapter.
pub fn supported_features() -> Vec<FeatureId> {
    vec![
        FeatureId::SendMessage,
        FeatureId::ReceiveMessage,
        FeatureId::EditMessage,
        FeatureId::DeleteMessage,
        FeatureId::ReadHistory,
        FeatureId::AddReaction,
        FeatureId::TypingIndicator,
    ]
}
