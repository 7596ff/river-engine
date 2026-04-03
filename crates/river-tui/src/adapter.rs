//! Adapter state management.

use chrono::{DateTime, Utc};
use river_adapter::FeatureId;
use river_context::OpenAIMessage;
use river_snowflake::{AgentBirth, SnowflakeGenerator, SnowflakeType};

/// Display message types.
#[derive(Debug, Clone)]
pub enum DisplayMessage {
    /// User input from TUI (not from context file).
    User {
        id: String,
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// System status messages.
    System {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Context entry from the worker's context.jsonl.
    Context {
        side: String, // "left" or "right"
        entry: OpenAIMessage,
        timestamp: DateTime<Utc>,
    },
}

/// Adapter state.
pub struct AdapterState {
    // Identity
    pub dyad: String,
    pub adapter_type: String,
    pub channel: String,

    // Worker binding
    pub worker_endpoint: Option<String>,

    // Messages display
    pub messages: Vec<DisplayMessage>,
    pub conversation_scroll: usize,

    // Context tailing (per side)
    pub left_lines_read: usize,
    pub right_lines_read: usize,

    // Input
    pub input: String,

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
            left_lines_read: 0,
            right_lines_read: 0,
            input: String::new(),
            generator: SnowflakeGenerator::new(birth),
        }
    }

    pub fn context_lines_read(&self, side: &str) -> usize {
        match side {
            "left" => self.left_lines_read,
            "right" => self.right_lines_read,
            _ => 0,
        }
    }

    pub fn add_user_message(&mut self, content: &str) -> String {
        let id = self.generator.next(SnowflakeType::Message).to_string();
        self.messages.push(DisplayMessage::User {
            id: id.clone(),
            content: content.to_string(),
            timestamp: Utc::now(),
        });
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

    pub fn add_context_entry(&mut self, side: &str, entry: OpenAIMessage) {
        self.messages.push(DisplayMessage::Context {
            side: side.to_string(),
            entry,
            timestamp: Utc::now(),
        });
        // Increment the appropriate counter
        match side {
            "left" => self.left_lines_read += 1,
            "right" => self.right_lines_read += 1,
            _ => {}
        }
        self.conversation_scroll = 0;
    }

    pub fn total_context_lines(&self) -> usize {
        self.left_lines_read + self.right_lines_read
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
