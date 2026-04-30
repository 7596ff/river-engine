//! Persistent context assembly with compaction
//!
//! The context is built once at session start, messages accumulate in place,
//! and compaction fires at 80% capacity — dropping only messages the spectator
//! has already compressed into moves.

use crate::model::ChatMessage;
use river_db::Database;

/// Configuration for context management
#[derive(Debug, Clone)]
pub struct ContextConfig {
    /// Total context window size in tokens
    pub limit: usize,
    /// Compaction trigger (fraction of limit, e.g. 0.80)
    pub compaction_threshold: f64,
    /// Post-compaction fill target (fraction of limit, e.g. 0.40)
    pub fill_target: f64,
    /// Minimum messages always kept in context
    pub min_messages: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            limit: 128_000,
            compaction_threshold: 0.80,
            fill_target: 0.40,
            min_messages: 20,
        }
    }
}

impl ContextConfig {
    /// Compaction trigger in tokens
    pub fn compaction_tokens(&self) -> usize {
        (self.limit as f64 * self.compaction_threshold) as usize
    }

    /// Fill target in tokens
    pub fn fill_tokens(&self) -> usize {
        (self.limit as f64 * self.fill_target) as usize
    }

    /// Spectator lag warning threshold (midpoint of fill and compaction)
    pub fn lag_warning_tokens(&self) -> usize {
        let midpoint = (self.fill_target + self.compaction_threshold) / 2.0;
        (self.limit as f64 * midpoint) as usize
    }
}

/// A message in the persistent context, carrying turn metadata
#[derive(Debug, Clone)]
pub struct ContextMessage {
    pub chat_message: ChatMessage,
    pub turn_number: u64,
}

impl ContextMessage {
    pub fn new(chat_message: ChatMessage, turn_number: u64) -> Self {
        Self { chat_message, turn_number }
    }

    /// Estimate tokens for this message
    pub fn estimate_tokens(&self, calibration: &TokenCalibration) -> usize {
        calibration.estimate(&self.chat_message)
    }
}

/// Token estimation with calibration from model feedback
#[derive(Debug, Clone)]
pub struct TokenCalibration {
    ratio: f64,
}

impl TokenCalibration {
    pub fn new() -> Self {
        Self { ratio: 1.0 }
    }

    /// Update ratio with weighted moving average from model response.
    /// Uses prompt_tokens specifically (not completion tokens).
    pub fn update(&mut self, actual_prompt_tokens: u64, estimated_prompt_tokens: usize) {
        if estimated_prompt_tokens == 0 || actual_prompt_tokens == 0 {
            return;
        }
        let new_sample = actual_prompt_tokens as f64 / estimated_prompt_tokens as f64;
        self.ratio = 0.7 * self.ratio + 0.3 * new_sample;
    }

    /// Estimate tokens for a string
    pub fn estimate_str(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        let base = (text.len() + 3) / 4;
        (base as f64 * self.ratio) as usize
    }

    /// Estimate tokens for a ChatMessage
    pub fn estimate(&self, msg: &ChatMessage) -> usize {
        let content_tokens = msg.content.as_deref().map_or(0, |s| self.estimate_str(s));
        let tool_tokens = msg.tool_calls.as_ref().map_or(0, |calls| {
            calls.iter().map(|tc| {
                self.estimate_str(&tc.function.name) + self.estimate_str(&tc.function.arguments)
            }).sum()
        });
        content_tokens + tool_tokens + 4 // 4 tokens overhead per message
    }

    /// Current ratio (for diagnostics)
    pub fn ratio(&self) -> f64 {
        self.ratio
    }
}

impl Default for TokenCalibration {
    fn default() -> Self {
        Self::new()
    }
}

/// The persistent context object
pub struct PersistentContext {
    config: ContextConfig,
    /// System prompt (re-read from disk at compaction)
    system_message: ContextMessage,
    /// Moves message (reloaded at compaction)
    moves_message: Option<ContextMessage>,
    /// Conversation messages (accumulate in place)
    messages: Vec<ContextMessage>,
    /// Token calibration
    calibration: TokenCalibration,
}

impl PersistentContext {
    /// Build initial context from DB (session start)
    pub fn build(
        config: ContextConfig,
        system_prompt: String,
        db: &Database,
        session_id: &str,
        channel: &str,
    ) -> Self {
        let calibration = TokenCalibration::new();

        let system_message = ContextMessage::new(
            ChatMessage::system(system_prompt),
            0,
        );

        let mut ctx = Self {
            config,
            system_message,
            moves_message: None,
            messages: Vec::new(),
            calibration,
        };

        ctx.load_from_db(db, session_id, channel);
        ctx
    }

    /// Load messages and moves from DB (used by session start and compaction)
    fn load_from_db(&mut self, db: &Database, session_id: &str, channel: &str) {
        // Get spectator cursor
        let cursor = db.get_max_turn(channel).unwrap_or(None).unwrap_or(0);

        // Load all messages above cursor
        let above_cursor = db.get_messages_above_turn(session_id, cursor)
            .unwrap_or_default();

        let mut messages: Vec<ContextMessage> = above_cursor.iter().map(|m| {
            ContextMessage::new(db_message_to_chat(m), m.turn_number)
        }).collect();

        // Backfill if fewer than min_messages
        if messages.len() < self.config.min_messages {
            self.backfill_messages(db, session_id, cursor + 1, &mut messages);
        }

        self.messages = messages;

        // Load moves with budget
        self.load_moves(db, channel);
    }

    /// Backfill messages from below a threshold turn to reach min_messages.
    /// Stops if adding the next turn would exceed compaction threshold.
    fn backfill_messages(
        &self,
        db: &Database,
        session_id: &str,
        below_turn: u64,
        messages: &mut Vec<ContextMessage>,
    ) {
        let needed = self.config.min_messages - messages.len();
        let backfill_turns = db.get_distinct_turns_below(session_id, below_turn, needed)
            .unwrap_or_default();

        let system_tokens = self.calibration.estimate(&self.system_message.chat_message);
        let current_tokens: usize = system_tokens + messages.iter()
            .map(|m| m.estimate_tokens(&self.calibration))
            .sum::<usize>();

        let mut running_total = current_tokens;

        for turn in backfill_turns {
            let turn_msgs = db.get_messages_for_turns(session_id, &[turn])
                .unwrap_or_default();

            let turn_tokens: usize = turn_msgs.iter()
                .map(|m| {
                    self.calibration.estimate_str(m.content.as_deref().unwrap_or("")) + 4
                })
                .sum();

            // Stop backfill if adding this turn would exceed compaction threshold
            if running_total + turn_tokens > self.config.compaction_tokens() {
                break;
            }

            let turn_ctx_msgs: Vec<ContextMessage> = turn_msgs.iter().map(|m| {
                ContextMessage::new(db_message_to_chat(m), m.turn_number)
            }).collect();

            // Insert at beginning (these are older)
            messages.splice(0..0, turn_ctx_msgs);
            running_total += turn_tokens;

            if messages.len() >= self.config.min_messages {
                break;
            }
        }
    }

    /// Load moves newest-first, fitting within the fill target budget
    fn load_moves(&mut self, db: &Database, channel: &str) {
        let system_tokens = self.calibration.estimate(&self.system_message.chat_message);
        let message_tokens: usize = self.messages.iter()
            .map(|m| m.estimate_tokens(&self.calibration))
            .sum();
        let used = system_tokens + message_tokens;
        let budget = self.config.fill_tokens().saturating_sub(used);

        if budget == 0 {
            self.moves_message = None;
            return;
        }

        let mut moves_text = String::new();
        let mut moves_tokens = 0;
        let batch_size = 50;

        let batch = db.get_moves_newest_first(channel, batch_size)
            .unwrap_or_default();

        if batch.is_empty() {
            self.moves_message = None;
            return;
        }

        for m in &batch {
            let entry_tokens = self.calibration.estimate_str(&m.summary) + 4;
            if moves_tokens + entry_tokens > budget {
                break;
            }
            // Prepend (loading newest first but want chronological order)
            moves_text.insert_str(0, &format!("{}\n", m.summary));
            moves_tokens += entry_tokens;
        }

        if moves_text.is_empty() {
            self.moves_message = None;
        } else {
            self.moves_message = Some(ContextMessage::new(
                ChatMessage::system(format!("[Conversation arc]\n{}", moves_text.trim())),
                0,
            ));
        }
    }

    /// Append a message to the context
    pub fn append(&mut self, msg: ContextMessage) {
        self.messages.push(msg);
    }

    /// Estimate total tokens in the current context
    pub fn estimate_total_tokens(&self) -> usize {
        let system = self.calibration.estimate(&self.system_message.chat_message);
        let moves = self.moves_message.as_ref()
            .map_or(0, |m| self.calibration.estimate(&m.chat_message));
        let messages: usize = self.messages.iter()
            .map(|m| m.estimate_tokens(&self.calibration))
            .sum();
        system + moves + messages
    }

    /// Check if compaction should fire
    pub fn needs_compaction(&self) -> bool {
        self.estimate_total_tokens() >= self.config.compaction_tokens()
    }

    /// Run compaction
    pub fn compact(
        &mut self,
        system_prompt: String,
        db: &Database,
        session_id: &str,
        channel: &str,
        current_turn: u64,
    ) {
        // Update system prompt
        self.system_message = ContextMessage::new(
            ChatMessage::system(system_prompt),
            0,
        );

        // Get cursor
        let cursor = db.get_max_turn(channel).unwrap_or(None).unwrap_or(0);

        // Drop turns at or below cursor
        self.messages.retain(|m| m.turn_number > cursor);

        // Backfill if needed
        if self.messages.len() < self.config.min_messages {
            let lowest_kept = self.messages.first()
                .map(|m| m.turn_number)
                .unwrap_or(cursor + 1);
            self.backfill_messages(db, session_id, lowest_kept, &mut self.messages.clone());

            // Re-do backfill properly (can't borrow self and messages simultaneously)
            let mut messages = std::mem::take(&mut self.messages);
            if messages.len() < self.config.min_messages {
                self.backfill_messages(db, session_id, lowest_kept, &mut messages);
            }
            self.messages = messages;
        }

        // Reload moves
        self.load_moves(db, channel);

        // Spectator lag detection
        let lag_turns = current_turn.saturating_sub(cursor);
        let total_tokens = self.estimate_total_tokens();
        if total_tokens > self.config.lag_warning_tokens() && lag_turns >= 10 {
            let warning = format!(
                "[System: Context compression is behind by {} turns. Context is at {:.0}%. Long-term memory is degraded. Consider shorter responses or informing the user.]",
                lag_turns,
                (total_tokens as f64 / self.config.limit as f64) * 100.0
            );
            self.messages.push(ContextMessage::new(
                ChatMessage::system(warning),
                current_turn,
            ));
        }
    }

    /// Update token calibration from model response
    pub fn update_calibration(&mut self, actual_prompt_tokens: u64, estimated_prompt_tokens: usize) {
        self.calibration.update(actual_prompt_tokens, estimated_prompt_tokens);
    }

    /// Get calibration reference (for external token estimation)
    pub fn calibration(&self) -> &TokenCalibration {
        &self.calibration
    }

    /// Get all messages as ChatMessage vec for sending to model
    pub fn to_messages(&self) -> Vec<ChatMessage> {
        let mut result = vec![self.system_message.chat_message.clone()];
        if let Some(ref moves) = self.moves_message {
            result.push(moves.chat_message.clone());
        }
        result.extend(self.messages.iter().map(|m| m.chat_message.clone()));
        result
    }

    /// Get the config
    pub fn config(&self) -> &ContextConfig {
        &self.config
    }

    /// Number of conversation messages (excluding system and moves)
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

/// Convert a river_db::Message to a ChatMessage
fn db_message_to_chat(msg: &river_db::Message) -> ChatMessage {
    let role = msg.role.as_str().to_string();
    match role.as_str() {
        "tool" => ChatMessage {
            role,
            content: msg.content.clone(),
            tool_calls: None,
            tool_call_id: msg.tool_call_id.clone(),
            name: msg.name.clone(),
        },
        "assistant" => {
            let tool_calls = msg.tool_calls.as_ref().and_then(|tc| {
                serde_json::from_str(tc).ok()
            });
            ChatMessage {
                role,
                content: msg.content.clone(),
                tool_calls,
                tool_call_id: None,
                name: None,
            }
        }
        _ => ChatMessage {
            role,
            content: msg.content.clone(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_db::init_db;
    use tempfile::TempDir;

    #[test]
    fn test_context_config_defaults() {
        let config = ContextConfig::default();
        assert_eq!(config.limit, 128_000);
        assert_eq!(config.compaction_tokens(), 102_400);
        assert_eq!(config.fill_tokens(), 51_200);
        assert_eq!(config.lag_warning_tokens(), 76_800); // 60% of 128K
    }

    #[test]
    fn test_token_calibration_default() {
        let cal = TokenCalibration::new();
        assert_eq!(cal.ratio(), 1.0);
        assert_eq!(cal.estimate_str("hello"), 2); // (5+3)/4 = 2
        assert_eq!(cal.estimate_str(""), 0);
    }

    #[test]
    fn test_token_calibration_update() {
        let mut cal = TokenCalibration::new();
        // Actual was 2x the estimate
        cal.update(200, 100);
        // ratio = 0.7 * 1.0 + 0.3 * 2.0 = 1.3
        assert!((cal.ratio() - 1.3).abs() < 0.01);
    }

    #[test]
    fn test_token_calibration_zero_skip() {
        let mut cal = TokenCalibration::new();
        cal.update(0, 100); // zero actual — skip
        assert_eq!(cal.ratio(), 1.0);
        cal.update(100, 0); // zero estimated — skip
        assert_eq!(cal.ratio(), 1.0);
    }

    #[test]
    fn test_token_calibration_smoothing() {
        let mut cal = TokenCalibration::new();
        cal.update(100, 100); // ratio stays ~1.0
        let r1 = cal.ratio();
        cal.update(200, 100); // spike to 2x
        let r2 = cal.ratio();
        cal.update(100, 100); // back to 1x
        let r3 = cal.ratio();
        // WMA should smooth: r2 > r1, r3 < r2 but r3 > r1
        assert!(r2 > r1);
        assert!(r3 < r2);
        assert!(r3 > r1);
    }

    #[test]
    fn test_context_message_wrap_unwrap() {
        let chat = ChatMessage::user("Hello");
        let ctx_msg = ContextMessage::new(chat.clone(), 5);
        assert_eq!(ctx_msg.turn_number, 5);
        assert_eq!(ctx_msg.chat_message.content, chat.content);
    }

    #[test]
    fn test_persistent_context_build_empty() {
        let temp = TempDir::new().unwrap();
        let db = init_db(&temp.path().join("test.db")).unwrap();

        let ctx = PersistentContext::build(
            ContextConfig::default(),
            "You are a helpful assistant.".into(),
            &db,
            "sess",
            "general",
        );

        assert_eq!(ctx.message_count(), 0);
        assert!(ctx.moves_message.is_none());
        let msgs = ctx.to_messages();
        assert_eq!(msgs.len(), 1); // system only
        assert!(msgs[0].content.as_ref().unwrap().contains("helpful assistant"));
    }

    #[test]
    fn test_persistent_context_append_and_estimate() {
        let temp = TempDir::new().unwrap();
        let db = init_db(&temp.path().join("test.db")).unwrap();

        let mut ctx = PersistentContext::build(
            ContextConfig::default(),
            "System".into(),
            &db,
            "sess",
            "general",
        );

        ctx.append(ContextMessage::new(ChatMessage::user("Hello"), 1));
        ctx.append(ContextMessage::new(
            ChatMessage::assistant(Some("Hi there!".into()), None), 1));

        assert_eq!(ctx.message_count(), 2);
        assert!(ctx.estimate_total_tokens() > 0);
        assert!(!ctx.needs_compaction());
    }

    #[test]
    fn test_persistent_context_to_messages_order() {
        let temp = TempDir::new().unwrap();
        let db = init_db(&temp.path().join("test.db")).unwrap();

        let mut ctx = PersistentContext::build(
            ContextConfig::default(),
            "System prompt".into(),
            &db,
            "sess",
            "general",
        );

        ctx.append(ContextMessage::new(ChatMessage::user("msg1"), 1));
        ctx.append(ContextMessage::new(ChatMessage::user("msg2"), 2));

        let msgs = ctx.to_messages();
        assert_eq!(msgs.len(), 3); // system + 2 messages
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.as_ref().unwrap().contains("System prompt"));
        assert!(msgs[1].content.as_ref().unwrap().contains("msg1"));
        assert!(msgs[2].content.as_ref().unwrap().contains("msg2"));
    }

    #[test]
    fn test_compaction_drops_below_cursor() {
        let temp = TempDir::new().unwrap();
        let db = init_db(&temp.path().join("test.db")).unwrap();

        let mut ctx = PersistentContext::build(
            ContextConfig { limit: 1000, min_messages: 0, ..ContextConfig::default() },
            "System".into(),
            &db,
            "sess",
            "general",
        );

        // Add messages at turns 1, 2, 3
        for turn in 1..=3 {
            ctx.append(ContextMessage::new(
                ChatMessage::user(format!("Turn {} msg", turn)),
                turn,
            ));
        }
        assert_eq!(ctx.message_count(), 3);

        // Insert a move at turn 2 (sets cursor to 2)
        use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};
        let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);
        db.insert_move(&river_db::Move {
            id: gen.next_id(SnowflakeType::Embedding),
            channel: "general".into(),
            turn_number: 2,
            summary: "Summary of turns 1-2".into(),
            tool_calls: None,
            created_at: 1000,
        }).unwrap();

        // Compact — should drop turns 1 and 2
        ctx.compact("System".into(), &db, "sess", "general", 3);

        assert_eq!(ctx.message_count(), 1);
        assert!(ctx.messages[0].chat_message.content.as_ref().unwrap().contains("Turn 3"));
    }

    #[test]
    fn test_compaction_keeps_all_when_no_spectator() {
        let temp = TempDir::new().unwrap();
        let db = init_db(&temp.path().join("test.db")).unwrap();

        let mut ctx = PersistentContext::build(
            ContextConfig { limit: 1000, min_messages: 0, ..ContextConfig::default() },
            "System".into(),
            &db,
            "sess",
            "general",
        );

        for turn in 1..=3 {
            ctx.append(ContextMessage::new(
                ChatMessage::user(format!("Turn {} msg", turn)),
                turn,
            ));
        }

        // No moves inserted — cursor is 0, nothing droppable
        ctx.compact("System".into(), &db, "sess", "general", 3);
        assert_eq!(ctx.message_count(), 3);
    }
}
