# Context Assembly Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-turn context rebuild with a persistent context object that accumulates messages and compacts via spectator cursor coordination.

**Architecture:** A `PersistentContext` struct holds `Vec<ContextMessage>` (ChatMessage + turn_number). Messages append in place. At 80% token capacity, compaction drops turns already covered by spectator moves, reloads moves, and rebuilds to ~40%. Token estimation uses a calibrated ratio updated from model API responses.

**Tech Stack:** Rust, SQLite (river-db), tokio

**Spec:** `docs/superpowers/specs/2026-04-30-context-assembly-rework-design.md`

---

### Task 1: Add DB queries for context assembly

**Files:**
- Modify: `crates/river-db/src/messages.rs`
- Modify: `crates/river-db/src/moves.rs`

Two new queries needed by the context assembler. These are additive — no existing code changes.

- [ ] **Step 1: Write test for get_messages_above_turn**

In `crates/river-db/src/messages.rs`, add to the test module:

```rust
#[test]
fn test_get_messages_above_turn() {
    let db = Database::open_in_memory().unwrap();
    let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
    let gen = SnowflakeGenerator::new(birth);

    // Insert messages at turns 1, 2, 3
    for turn in 1..=3 {
        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".into(),
            role: MessageRole::User,
            content: Some(format!("Turn {} message", turn)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            created_at: turn as i64 * 100,
            metadata: None,
            turn_number: turn,
        };
        db.insert_message(&msg).unwrap();
    }

    // Get messages above turn 1
    let msgs = db.get_messages_above_turn("sess", 1).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].turn_number, 2);
    assert_eq!(msgs[1].turn_number, 3);

    // Get messages above turn 0 (all)
    let msgs = db.get_messages_above_turn("sess", 0).unwrap();
    assert_eq!(msgs.len(), 3);

    // Get messages above turn 3 (none)
    let msgs = db.get_messages_above_turn("sess", 3).unwrap();
    assert_eq!(msgs.len(), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd ~/river-engine && cargo test -p river-db test_get_messages_above_turn 2>&1 | tail -5
```

Expected: FAIL — method doesn't exist.

- [ ] **Step 3: Implement get_messages_above_turn**

In `crates/river-db/src/messages.rs`, add to the `impl Database` block:

```rust
/// Get messages with turn_number > the given turn, ordered chronologically
pub fn get_messages_above_turn(&self, session_id: &str, turn: u64) -> RiverResult<Vec<Message>> {
    let mut stmt = self.conn().prepare(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
         FROM messages
         WHERE session_id = ? AND turn_number > ?
         ORDER BY created_at ASC"
    ).map_err(|e| RiverError::database(e.to_string()))?;

    let messages = stmt.query_map(params![session_id, turn as i64], Message::from_row)
        .map_err(|e| RiverError::database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RiverError::database(e.to_string()))?;

    Ok(messages)
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd ~/river-engine && cargo test -p river-db test_get_messages_above_turn 2>&1 | tail -5
```

Expected: PASS

- [ ] **Step 5: Write test for get_messages_for_turns**

A query to backfill specific turns (newest turns below cursor):

```rust
#[test]
fn test_get_messages_for_turns() {
    let db = Database::open_in_memory().unwrap();
    let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
    let gen = SnowflakeGenerator::new(birth);

    // Insert 2 messages at turn 5, 3 at turn 6
    for i in 0..2 {
        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".into(),
            role: MessageRole::User,
            content: Some(format!("Turn 5 msg {}", i)),
            tool_calls: None, tool_call_id: None, name: None,
            created_at: 500 + i,
            metadata: None,
            turn_number: 5,
        };
        db.insert_message(&msg).unwrap();
    }
    for i in 0..3 {
        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".into(),
            role: MessageRole::User,
            content: Some(format!("Turn 6 msg {}", i)),
            tool_calls: None, tool_call_id: None, name: None,
            created_at: 600 + i,
            metadata: None,
            turn_number: 6,
        };
        db.insert_message(&msg).unwrap();
    }

    // Get messages for turns [5, 6]
    let msgs = db.get_messages_for_turns("sess", &[5, 6]).unwrap();
    assert_eq!(msgs.len(), 5);

    // Get messages for turns [6] only
    let msgs = db.get_messages_for_turns("sess", &[6]).unwrap();
    assert_eq!(msgs.len(), 3);

    // Empty turns list
    let msgs = db.get_messages_for_turns("sess", &[]).unwrap();
    assert_eq!(msgs.len(), 0);
}
```

- [ ] **Step 6: Implement get_messages_for_turns**

```rust
/// Get messages for specific turn numbers, ordered chronologically
pub fn get_messages_for_turns(&self, session_id: &str, turns: &[u64]) -> RiverResult<Vec<Message>> {
    if turns.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: Vec<String> = turns.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
         FROM messages
         WHERE session_id = ? AND turn_number IN ({})
         ORDER BY created_at ASC",
        placeholders.join(", ")
    );

    let mut stmt = self.conn().prepare(&sql)
        .map_err(|e| RiverError::database(e.to_string()))?;

    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    params_vec.push(Box::new(session_id.to_string()));
    for t in turns {
        params_vec.push(Box::new(*t as i64));
    }
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let messages = stmt.query_map(params_refs.as_slice(), Message::from_row)
        .map_err(|e| RiverError::database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RiverError::database(e.to_string()))?;

    Ok(messages)
}
```

- [ ] **Step 7: Run test**

```bash
cd ~/river-engine && cargo test -p river-db test_get_messages_for_turns 2>&1 | tail -5
```

Expected: PASS

- [ ] **Step 8: Write test for get_moves_newest_first**

In `crates/river-db/src/moves.rs`, add to the test module:

```rust
#[test]
fn test_get_moves_newest_first() {
    let db = Database::open_in_memory().unwrap();
    let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
    let gen = SnowflakeGenerator::new(birth);

    for turn in 1..=10 {
        let m = Move {
            id: gen.next_id(SnowflakeType::Embedding),
            channel: "general".into(),
            turn_number: turn,
            summary: format!("Move for turn {}", turn),
            tool_calls: None,
            created_at: turn as i64 * 100,
        };
        db.insert_move(&m).unwrap();
    }

    // Get 3 newest
    let moves = db.get_moves_newest_first("general", 3).unwrap();
    assert_eq!(moves.len(), 3);
    assert_eq!(moves[0].turn_number, 10);
    assert_eq!(moves[1].turn_number, 9);
    assert_eq!(moves[2].turn_number, 8);

    // Get more than exist
    let moves = db.get_moves_newest_first("general", 100).unwrap();
    assert_eq!(moves.len(), 10);

    // Wrong channel
    let moves = db.get_moves_newest_first("other", 10).unwrap();
    assert_eq!(moves.len(), 0);
}
```

- [ ] **Step 9: Implement get_moves_newest_first**

In `crates/river-db/src/moves.rs`:

```rust
/// Get moves for a channel, ordered by turn_number descending (newest first)
pub fn get_moves_newest_first(&self, channel: &str, limit: usize) -> RiverResult<Vec<Move>> {
    let mut stmt = self
        .conn()
        .prepare(
            "SELECT id, channel, turn_number, summary, tool_calls, created_at
             FROM moves
             WHERE channel = ?
             ORDER BY turn_number DESC
             LIMIT ?",
        )
        .map_err(|e| RiverError::database(e.to_string()))?;

    let moves = stmt
        .query_map(params![channel, limit as i64], Move::from_row)
        .map_err(|e| RiverError::database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RiverError::database(e.to_string()))?;

    Ok(moves)
}
```

- [ ] **Step 10: Run test**

```bash
cd ~/river-engine && cargo test -p river-db test_get_moves_newest_first 2>&1 | tail -5
```

Expected: PASS

- [ ] **Step 11: Write test for get_distinct_turns_below**

A query to find distinct turn numbers below the cursor for backfill:

In `crates/river-db/src/messages.rs`:

```rust
#[test]
fn test_get_distinct_turns_below() {
    let db = Database::open_in_memory().unwrap();
    let birth = AgentBirth::new(2026, 4, 30, 12, 0, 0).unwrap();
    let gen = SnowflakeGenerator::new(birth);

    for turn in 1..=5 {
        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".into(),
            role: MessageRole::User,
            content: Some(format!("Turn {}", turn)),
            tool_calls: None, tool_call_id: None, name: None,
            created_at: turn as i64 * 100,
            metadata: None,
            turn_number: turn,
        };
        db.insert_message(&msg).unwrap();
    }

    // Get 3 newest turns below turn 5
    let turns = db.get_distinct_turns_below("sess", 5, 3).unwrap();
    assert_eq!(turns, vec![4, 3, 2]); // newest first

    // Get turns below turn 2
    let turns = db.get_distinct_turns_below("sess", 2, 10).unwrap();
    assert_eq!(turns, vec![1]);
}
```

- [ ] **Step 12: Implement get_distinct_turns_below**

```rust
/// Get distinct turn numbers below a threshold, ordered newest first
pub fn get_distinct_turns_below(&self, session_id: &str, below_turn: u64, limit: usize) -> RiverResult<Vec<u64>> {
    let mut stmt = self.conn().prepare(
        "SELECT DISTINCT turn_number FROM messages
         WHERE session_id = ? AND turn_number < ?
         ORDER BY turn_number DESC
         LIMIT ?"
    ).map_err(|e| RiverError::database(e.to_string()))?;

    let turns = stmt.query_map(params![session_id, below_turn as i64, limit as i64], |row| {
        row.get::<_, i64>(0).map(|n| n as u64)
    })
    .map_err(|e| RiverError::database(e.to_string()))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| RiverError::database(e.to_string()))?;

    Ok(turns)
}
```

- [ ] **Step 13: Run all new tests**

```bash
cd ~/river-engine && cargo test -p river-db 2>&1 | tail -10
```

Expected: all pass

- [ ] **Step 14: Commit**

```bash
cd ~/river-engine && git add crates/river-db/src/messages.rs crates/river-db/src/moves.rs
git commit -m "feat(db): add queries for context assembly — messages by turn, moves newest-first, distinct turns"
```

---

### Task 2: Rewrite context.rs — ContextMessage, ContextConfig, PersistentContext, token calibration

**Files:**
- Rewrite: `crates/river-gateway/src/agent/context.rs`

This is the core of the rework. The file is completely rewritten. Read the spec at `docs/superpowers/specs/2026-04-30-context-assembly-rework-design.md` for full details.

- [ ] **Step 1: Write the new context.rs with types and token estimation**

Replace the entire contents of `crates/river-gateway/src/agent/context.rs` with:

```rust
//! Persistent context assembly with compaction
//!
//! The context is built once at session start, messages accumulate in place,
//! and compaction fires at 80% capacity — dropping only messages the spectator
//! has already compressed into moves.

use crate::model::ChatMessage;
use river_db::{Database, Move};
use std::sync::{Arc, Mutex};

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

    /// Update ratio with weighted moving average from model response
    /// Uses prompt_tokens specifically (not completion tokens)
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
            let needed = self.config.min_messages - messages.len();
            // Get distinct turns below cursor, newest first
            let backfill_turns = db.get_distinct_turns_below(session_id, cursor + 1, needed)
                .unwrap_or_default();

            let system_tokens = self.calibration.estimate(&self.system_message.chat_message);
            let current_tokens: usize = system_tokens + messages.iter()
                .map(|m| m.estimate_tokens(&self.calibration))
                .sum::<usize>();

            for turn in backfill_turns {
                let turn_msgs = db.get_messages_for_turns(session_id, &[turn])
                    .unwrap_or_default();

                let turn_tokens: usize = turn_msgs.iter()
                    .map(|m| self.calibration.estimate_str(
                        m.content.as_deref().unwrap_or("")) + 4)
                    .sum();

                // Stop backfill if adding this turn would exceed compaction threshold
                if current_tokens + turn_tokens > self.config.compaction_tokens() {
                    break;
                }

                let turn_ctx_msgs: Vec<ContextMessage> = turn_msgs.iter().map(|m| {
                    ContextMessage::new(db_message_to_chat(m), m.turn_number)
                }).collect();

                // Insert at beginning (these are older)
                messages.splice(0..0, turn_ctx_msgs);

                if messages.len() >= self.config.min_messages {
                    break;
                }
            }
        }

        self.messages = messages;

        // Load moves with budget
        self.load_moves(db, channel);
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
        let mut offset = 0;
        let batch_size = 50;

        loop {
            let batch = db.get_moves_newest_first(channel, batch_size)
                .unwrap_or_default();

            if batch.is_empty() || offset >= batch.len() {
                break;
            }

            for m in batch.iter().skip(offset) {
                let entry_tokens = self.calibration.estimate_str(&m.summary) + 4;
                if moves_tokens + entry_tokens > budget {
                    // Reached budget — stop loading
                    break;
                }
                // Prepend (we're loading newest first but want chronological order)
                moves_text.insert_str(0, &format!("{}\n", m.summary));
                moves_tokens += entry_tokens;
            }

            offset += batch_size;

            // If we filled the budget or exhausted moves, stop
            if moves_tokens >= budget || batch.len() < batch_size {
                break;
            }
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
            let system_tokens = self.calibration.estimate(&self.system_message.chat_message);
            let current_msg_tokens: usize = self.messages.iter()
                .map(|m| m.estimate_tokens(&self.calibration))
                .sum();
            let current_total = system_tokens + current_msg_tokens;

            // Find the lowest turn_number we still have
            let lowest_kept = self.messages.first().map(|m| m.turn_number).unwrap_or(cursor + 1);
            let backfill_turns = db.get_distinct_turns_below(session_id, lowest_kept, 20)
                .unwrap_or_default();

            let mut running_total = current_total;
            for turn in backfill_turns {
                let turn_msgs = db.get_messages_for_turns(session_id, &[turn])
                    .unwrap_or_default();

                let turn_tokens: usize = turn_msgs.iter()
                    .map(|m| self.calibration.estimate_str(
                        m.content.as_deref().unwrap_or("")) + 4)
                    .sum();

                if running_total + turn_tokens > self.config.compaction_tokens() {
                    break;
                }

                let turn_ctx_msgs: Vec<ContextMessage> = turn_msgs.iter().map(|m| {
                    ContextMessage::new(db_message_to_chat(m), m.turn_number)
                }).collect();

                self.messages.splice(0..0, turn_ctx_msgs);
                running_total += turn_tokens;

                if self.messages.len() >= self.config.min_messages {
                    break;
                }
            }
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
        // Simulate oscillating content
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
}
```

- [ ] **Step 2: Verify it compiles and tests pass**

```bash
cd ~/river-engine && cargo test -p river-gateway agent::context 2>&1 | tail -10
```

Expected: all new tests pass. Some existing tests that imported old types (`ContextBudget`, `AssembledContext`, `LayerStats`) will fail — that's expected and will be fixed in Task 3.

- [ ] **Step 3: Commit**

```bash
cd ~/river-engine && git add crates/river-gateway/src/agent/context.rs
git commit -m "feat(context): rewrite context assembly — persistent context, compaction, calibrated tokens"
```

---

### Task 3: Update agent/mod.rs exports

**Files:**
- Modify: `crates/river-gateway/src/agent/mod.rs`

- [ ] **Step 1: Update exports**

Replace the current exports in `crates/river-gateway/src/agent/mod.rs`:

```rust
//! Agent (I) — the acting self
//!
//! The agent runs as a peer task in the coordinator, managing the wake/think/act/settle
//! turn cycle. It uses a persistent context that accumulates messages and compacts
//! via spectator cursor coordination.

pub mod channel;
pub mod context;
pub mod task;
pub mod tools;

pub use channel::ChannelContext;
pub use context::{PersistentContext, ContextConfig, ContextMessage, TokenCalibration};
pub use task::{AgentTask, AgentTaskConfig};
```

- [ ] **Step 2: Fix any compilation errors from removed types**

Search for uses of the old types (`ContextBudget`, `AssembledContext`, `LayerStats`, `ContextAssembler`) outside of `agent/`:

```bash
cd ~/river-engine && grep -rn "ContextBudget\|AssembledContext\|LayerStats\|ContextAssembler" --include="*.rs" crates/river-gateway/src/ | grep -v "agent/context.rs" | grep -v "agent/mod.rs"
```

Fix any references found (likely in `server.rs` and `task.rs`). These will be fully addressed in Task 4.

- [ ] **Step 3: Commit**

```bash
cd ~/river-engine && git add crates/river-gateway/src/agent/mod.rs
git commit -m "refactor(agent): update exports for new context types"
```

---

### Task 4: Update task.rs — new turn cycle with persistent context

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`
- Modify: `crates/river-gateway/src/server.rs`

This is the integration task. The turn cycle changes from "rebuild context each turn" to "append to persistent context, compact when needed."

- [ ] **Step 1: Update AgentTaskConfig**

In `crates/river-gateway/src/agent/task.rs`, replace `AgentTaskConfig`:

```rust
/// Configuration for the agent task
#[derive(Debug, Clone)]
pub struct AgentTaskConfig {
    /// Workspace path for loading identity and context files
    pub workspace: PathBuf,
    /// Context configuration
    pub context_config: ContextConfig,
    /// Timeout for model calls
    pub model_timeout: Duration,
    /// Maximum tool calls per turn (safety limit)
    pub max_tool_calls: usize,
    /// Heartbeat interval (how often to wake if no messages)
    pub heartbeat_interval: Duration,
}

impl Default for AgentTaskConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            context_config: ContextConfig::default(),
            model_timeout: Duration::from_secs(120),
            max_tool_calls: 50,
            heartbeat_interval: Duration::from_secs(45 * 60),
        }
    }
}
```

Remove `embeddings_dir`, `context_budget`, `history_limit`, `context_limit` — these are replaced by `context_config`.

- [ ] **Step 2: Update AgentTask struct**

Replace the struct fields:

```rust
pub struct AgentTask {
    config: AgentTaskConfig,
    bus: EventBus,
    message_queue: Arc<MessageQueue>,
    model_client: ModelClient,
    tool_executor: Arc<RwLock<ToolExecutor>>,
    flash_queue: Arc<FlashQueue>,
    db: Arc<Mutex<Database>>,
    snowflake_gen: Arc<SnowflakeGenerator>,
    turn_count: u64,
    channel_context: Option<ChannelContext>,
    /// Pending channel switch (applied at start of next turn)
    pending_channel_switch: Option<ChannelContext>,
    /// The persistent context object
    context: PersistentContext,
    /// Last estimated prompt tokens (for calibration)
    last_estimated_prompt_tokens: usize,
}
```

Remove `conversation: Vec<ChatMessage>`, `last_prompt_tokens: u64`, `context_assembler: ContextAssembler`.

- [ ] **Step 3: Update AgentTask::new**

Build the initial `PersistentContext` from DB:

```rust
impl AgentTask {
    pub fn new(
        config: AgentTaskConfig,
        bus: EventBus,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        flash_queue: Arc<FlashQueue>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        // Build system prompt synchronously (will be re-read at compaction)
        let system_prompt = Self::build_system_prompt_sync(&config.workspace);

        let channel = "default"; // Will be updated on first channel switch

        let context = {
            let db_guard = db.lock().expect("DB lock poisoned");
            PersistentContext::build(
                config.context_config.clone(),
                system_prompt,
                &db_guard,
                crate::session::PRIMARY_SESSION_ID,
                channel,
            )
        };

        Self {
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            flash_queue,
            db,
            snowflake_gen,
            turn_count: 0,
            channel_context: None,
            pending_channel_switch: None,
            context,
            last_estimated_prompt_tokens: 0,
        }
    }
}
```

- [ ] **Step 4: Rewrite the turn_cycle method**

```rust
async fn turn_cycle(&mut self, is_heartbeat: bool) {
    self.turn_count += 1;
    let mut stats = TurnStats::default();

    // ========== CHECK PENDING CHANNEL SWITCH ==========
    if let Some(new_channel) = self.pending_channel_switch.take() {
        let channel_name = new_channel.display_name().to_string();
        self.channel_context = Some(new_channel);

        let system_prompt = self.build_system_prompt().await;
        let db_guard = self.db.lock().expect("DB lock poisoned");
        self.context = PersistentContext::build(
            self.config.context_config.clone(),
            system_prompt,
            &db_guard,
            crate::session::PRIMARY_SESSION_ID,
            &channel_name,
        );
        drop(db_guard);

        tracing::info!(channel = %channel_name, "Channel switch applied");
    }

    // ========== WAKE ==========
    self.flash_queue.tick_turn().await;
    let channel_name = self.channel_context
        .as_ref()
        .map(|c| c.display_name().to_string())
        .unwrap_or_else(|| "default".to_string());

    self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
        channel: channel_name.clone(),
        turn_number: self.turn_count,
        timestamp: Utc::now(),
    }));

    // Drain incoming messages
    let incoming = self.message_queue.drain();
    for msg in &incoming {
        let chat_msg = ChatMessage::user(format!(
            "[{}] {}: {}",
            msg.channel, msg.author.name, msg.content
        ));
        self.context.append(ContextMessage::new(chat_msg, self.turn_count));
    }

    if is_heartbeat && incoming.is_empty() {
        self.context.append(ContextMessage::new(
            ChatMessage::user(":heartbeat:".into()),
            self.turn_count,
        ));
    }

    // ========== CHECK COMPACTION ==========
    if self.context.needs_compaction() {
        let system_prompt = self.build_system_prompt().await;
        let db_guard = self.db.lock().expect("DB lock poisoned");
        self.context.compact(
            system_prompt,
            &db_guard,
            crate::session::PRIMARY_SESSION_ID,
            &channel_name,
            self.turn_count,
        );
        drop(db_guard);
        tracing::info!(
            turn = self.turn_count,
            tokens = self.context.estimate_total_tokens(),
            "Context compacted"
        );
    }

    // Check context pressure
    let total_tokens = self.context.estimate_total_tokens();
    let context_percent = (total_tokens as f64 / self.config.context_config.limit as f64) * 100.0;
    if context_percent >= 80.0 {
        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::ContextPressure {
            usage_percent: context_percent,
            timestamp: Utc::now(),
        }));
    }

    // Get tool schemas
    let tools: Vec<ToolSchema> = {
        let executor = self.tool_executor.read().await;
        executor.schemas()
    };

    // ========== THINK + ACT LOOP ==========
    let mut messages = self.context.to_messages();
    let mut iteration = 0;

    loop {
        iteration += 1;
        if iteration > self.config.max_tool_calls {
            tracing::warn!(iterations = iteration, "Max tool call iterations reached");
            break;
        }

        // Track estimated tokens before model call (for calibration)
        self.last_estimated_prompt_tokens = self.context.estimate_total_tokens();

        let response = match self.model_client.complete(&messages, &tools).await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!(error = %e, "Model call failed");
                break;
            }
        };

        // Calibrate token estimation
        self.context.update_calibration(
            response.usage.prompt_tokens as u64,
            self.last_estimated_prompt_tokens,
        );

        stats.prompt_tokens = response.usage.prompt_tokens as u64;

        // Add assistant response
        let assistant_msg = ChatMessage::assistant(
            response.content.clone(),
            if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) },
        );
        messages.push(assistant_msg.clone());
        self.context.append(ContextMessage::new(assistant_msg, self.turn_count));

        if response.tool_calls.is_empty() {
            break;
        }

        // Execute tool calls
        let tool_results = self.execute_tool_calls(&response.tool_calls, &mut stats).await;

        for result in &tool_results {
            let tool_msg = ChatMessage::tool(&result.0, &result.1);
            messages.push(tool_msg.clone());
            self.context.append(ContextMessage::new(tool_msg, self.turn_count));
        }

        // Check for mid-turn messages
        let mid_turn_messages = self.message_queue.drain();
        if !mid_turn_messages.is_empty() {
            let mut content = String::from("Messages received during tool execution:\n");
            for msg in mid_turn_messages {
                content.push_str(&format!(
                    "- [{}] {}: {}\n",
                    msg.channel, msg.author.name, msg.content
                ));
            }
            let system_msg = ChatMessage::system(content);
            messages.push(system_msg.clone());
            self.context.append(ContextMessage::new(system_msg, self.turn_count));
        }
    }

    // ========== SETTLE ==========
    self.persist_turn_messages();

    let transcript_summary = format!(
        "Turn {} completed: {} messages, {} tool calls ({} failed)",
        self.turn_count, incoming.len(), stats.total_tool_calls, stats.failed_tool_calls
    );

    self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
        channel: channel_name,
        turn_number: self.turn_count,
        transcript_summary,
        tool_calls: stats.tool_calls,
        timestamp: Utc::now(),
    }));
}
```

- [ ] **Step 5: Update set_channel_context to defer**

```rust
/// Switch to a different channel (takes effect at next turn start)
pub fn set_channel_context(&mut self, context: ChannelContext) {
    let old = self.channel_context
        .as_ref()
        .map(|c| c.display_name().to_string())
        .unwrap_or_else(|| "unset".to_string());
    let new = context.display_name().to_string();

    self.bus.publish(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched {
        from: old.clone(),
        to: new.clone(),
        timestamp: Utc::now(),
    }));

    tracing::info!(from = %old, to = %new, "Channel switch pending (applied at next turn)");
    self.pending_channel_switch = Some(context);
}
```

- [ ] **Step 6: Update persist_turn_messages to use context messages**

The persist method now needs to iterate over the context's messages for the current turn. Update it to persist from the context rather than a separate `conversation` vec:

```rust
fn persist_turn_messages(&self) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let db = match self.db.lock() {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(error = %e, "DB lock poisoned in persist_turn_messages");
            return;
        }
    };

    // Note: messages are persisted from the context object, which tracks turn_number
    // We only persist messages from the current turn (not previously persisted ones)
    // This is handled by the fact that we only persist at settle, and only new messages
    // were appended during this turn cycle.
    // For now, we use get_recent to avoid double-persisting.
    // TODO: Track a persistence cursor to avoid re-persisting
}
```

Actually — the current code persists ALL conversation messages every settle, which would double-persist on the second turn. This is a pre-existing bug. For this task, keep the existing behavior but mark it for fixing. The important change is using `self.turn_count` for `turn_number`.

- [ ] **Step 7: Remove trim_conversation**

Delete the `trim_conversation` method entirely — compaction replaces it.

- [ ] **Step 8: Add build_system_prompt_sync**

A synchronous version for use in `new()`:

```rust
fn build_system_prompt_sync(workspace: &Path) -> String {
    let mut parts = Vec::new();

    for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
        let path = workspace.join("actor").join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            parts.push(content);
        }
    }

    let prefs = Preferences::load(workspace);
    let time_str = format_current_time(prefs.timezone());
    parts.push(format!("Current time: {}", time_str));

    if parts.is_empty() {
        "You are an AI assistant.".to_string()
    } else {
        parts.join("\n\n---\n\n")
    }
}
```

- [ ] **Step 9: Update server.rs to use new config**

In `crates/river-gateway/src/server.rs`, update the `AgentTaskConfig` construction to use `ContextConfig` instead of the old fields. Find where `AgentTaskConfig` is built and replace:

```rust
let agent_config = AgentTaskConfig {
    workspace: config.workspace.clone(),
    context_config: ContextConfig {
        limit: config.context_limit as usize,
        ..ContextConfig::default()
    },
    model_timeout: Duration::from_secs(120),
    max_tool_calls: 50,
    heartbeat_interval: Duration::from_secs(45 * 60),
};
```

- [ ] **Step 10: Build and fix compilation errors**

```bash
cd ~/river-engine && cargo build 2>&1 | head -30
```

Fix any remaining compilation errors. Common issues:
- References to removed fields (`embeddings_dir`, `history_limit`, `context_limit`, `context_budget`)
- References to removed types (`ContextBudget`, `AssembledContext`, `LayerStats`, `ContextAssembler`)
- Test code using old types

- [ ] **Step 11: Run all tests**

```bash
cd ~/river-engine && cargo test 2>&1 | tail -20
```

Fix any failures. Update existing tests in `task.rs` to use the new config types.

- [ ] **Step 12: Commit**

```bash
cd ~/river-engine && git add -A
git commit -m "feat(agent): persistent context with compaction — replaces per-turn rebuild"
```

---

### Task 5: Final verification and cleanup

**Files:**
- All modified files from previous tasks

- [ ] **Step 1: Full test suite**

```bash
cd ~/river-engine && cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 2: Verify no old types remain**

```bash
cd ~/river-engine && grep -rn "ContextBudget\|warm_flashes\|warm_retrieved\|warm_moves\|hot_min_messages\|output_reserved" --include="*.rs" crates/ | grep -v "test" | grep -v "TODO"
```

Expected: no matches (except possibly in test helpers that may reference defaults).

- [ ] **Step 3: Verify key behaviors compile**

Check that these compile (they exercise the main code paths):

```bash
cd ~/river-engine && cargo test -p river-gateway agent::context 2>&1 | tail -10
cd ~/river-engine && cargo test -p river-db 2>&1 | tail -10
```

- [ ] **Step 4: Commit any final fixes**

```bash
cd ~/river-engine && git add -A
git commit -m "cleanup: remove old context types, fix remaining references"
```
