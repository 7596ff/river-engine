//! Adapter state management.

use chrono::{DateTime, Utc};
use river_adapter::FeatureId;
use river_context::OpenAIMessage;
use river_protocol::conversation::Line as BackchannelLine;
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

    // Backchannel
    pub backchannel_lines: Vec<BackchannelLine>,

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
            backchannel_lines: Vec::new(),
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
        let id = self.generator.next(SnowflakeType::Message).unwrap().to_string();
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

    pub fn generate_message_id(&mut self) -> String {
        self.generator.next(SnowflakeType::Message).unwrap().to_string()
    }

    pub fn add_backchannel_line(&mut self, line: BackchannelLine) {
        self.backchannel_lines.push(line);
        self.conversation_scroll = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state() -> AdapterState {
        AdapterState::new(
            "test-dyad".to_string(),
            "tui".to_string(),
            "test-channel".to_string(),
        )
    }

    #[test]
    fn test_adapter_state_new_initialization() {
        let state = create_test_state();

        assert_eq!(state.dyad, "test-dyad");
        assert_eq!(state.adapter_type, "tui");
        assert_eq!(state.channel, "test-channel");
        assert!(state.worker_endpoint.is_none());
        assert!(state.messages.is_empty());
        assert_eq!(state.conversation_scroll, 0);
        assert_eq!(state.left_lines_read, 0);
        assert_eq!(state.right_lines_read, 0);
        assert!(state.backchannel_lines.is_empty());
        assert!(state.input.is_empty());
    }

    #[test]
    fn test_add_user_message_returns_unique_ids() {
        let mut state = create_test_state();

        let id1 = state.add_user_message("Hello");
        let id2 = state.add_user_message("World");

        assert_ne!(id1, id2);
        assert_eq!(state.messages.len(), 2);
    }

    #[test]
    fn test_add_user_message_resets_scroll() {
        let mut state = create_test_state();
        state.conversation_scroll = 10;

        state.add_user_message("Test message");

        assert_eq!(state.conversation_scroll, 0);
    }

    #[test]
    fn test_add_user_message_stores_content() {
        let mut state = create_test_state();

        state.add_user_message("Hello, world!");

        match &state.messages[0] {
            DisplayMessage::User { content, .. } => {
                assert_eq!(content, "Hello, world!");
            }
            _ => panic!("Expected User message"),
        }
    }

    #[test]
    fn test_add_system_message() {
        let mut state = create_test_state();
        state.conversation_scroll = 5;

        state.add_system_message("System notification");

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.conversation_scroll, 0);

        match &state.messages[0] {
            DisplayMessage::System { content, .. } => {
                assert_eq!(content, "System notification");
            }
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn test_add_context_entry_left_side() {
        let mut state = create_test_state();
        let entry = OpenAIMessage {
            role: "user".to_string(),
            content: Some("Left context".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };

        state.add_context_entry("left", entry);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.left_lines_read, 1);
        assert_eq!(state.right_lines_read, 0);

        match &state.messages[0] {
            DisplayMessage::Context { side, entry, .. } => {
                assert_eq!(side, "left");
                assert_eq!(entry.content, Some("Left context".to_string()));
            }
            _ => panic!("Expected Context message"),
        }
    }

    #[test]
    fn test_add_context_entry_right_side() {
        let mut state = create_test_state();
        let entry = OpenAIMessage {
            role: "assistant".to_string(),
            content: Some("Right context".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };

        state.add_context_entry("right", entry);

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.left_lines_read, 0);
        assert_eq!(state.right_lines_read, 1);

        match &state.messages[0] {
            DisplayMessage::Context { side, .. } => {
                assert_eq!(side, "right");
            }
            _ => panic!("Expected Context message"),
        }
    }

    #[test]
    fn test_add_context_entry_unknown_side() {
        let mut state = create_test_state();
        let entry = OpenAIMessage {
            role: "user".to_string(),
            content: Some("Unknown side".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };

        state.add_context_entry("unknown", entry);

        // Should still add message but not increment counters
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.left_lines_read, 0);
        assert_eq!(state.right_lines_read, 0);
    }

    #[test]
    fn test_context_lines_read() {
        let mut state = create_test_state();
        state.left_lines_read = 5;
        state.right_lines_read = 3;

        assert_eq!(state.context_lines_read("left"), 5);
        assert_eq!(state.context_lines_read("right"), 3);
        assert_eq!(state.context_lines_read("unknown"), 0);
    }

    #[test]
    fn test_supported_features() {
        let features = supported_features();

        assert_eq!(features.len(), 7);
        assert!(features.contains(&FeatureId::SendMessage));
        assert!(features.contains(&FeatureId::ReceiveMessage));
        assert!(features.contains(&FeatureId::EditMessage));
        assert!(features.contains(&FeatureId::DeleteMessage));
        assert!(features.contains(&FeatureId::ReadHistory));
        assert!(features.contains(&FeatureId::AddReaction));
        assert!(features.contains(&FeatureId::TypingIndicator));
    }

    #[test]
    fn test_generate_message_id_uniqueness() {
        let mut state = create_test_state();

        let ids: Vec<String> = (0..100).map(|_| state.generate_message_id()).collect();

        // Check all IDs are unique
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique_ids.len(), ids.len());
    }

    #[test]
    fn test_generate_message_id_not_empty() {
        let mut state = create_test_state();

        let id = state.generate_message_id();

        assert!(!id.is_empty());
    }
}
