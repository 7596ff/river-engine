# Spectator Compression Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded spectator pipeline with a prompt-driven runtime that writes LLM-generated moves to the database and LLM-compressed moments to embeddings/.

**Architecture:** The spectator becomes a thin event dispatcher. On each event, it loads a prompt file from `workspace/spectator/`, assembles context from the database, calls the LLM, and handles the structured output (insert move, write moment file, emit warning). The DB gains a `moves` table and `turn_number` on messages.

**Tech Stack:** Rust, rusqlite, tokio, reqwest (ModelClient), chrono, regex

**Spec:** `docs/specs/2026-04-29-spectator-compression-design.md`

---

## File Structure

### New files
- `crates/river-db/src/migrations/004_moves.sql` — moves table DDL
- `crates/river-db/src/moves.rs` — `Move` struct and CRUD on `Database`
- `crates/river-gateway/src/spectator/prompt.rs` — prompt loading and template substitution
- `crates/river-gateway/src/spectator/handlers.rs` — turn complete, compress, and pressure handlers
- `crates/river-gateway/src/spectator/format.rs` — message transcript and move list formatting

### Modified files
- `crates/river-db/src/migrations/001_messages.sql` — add `turn_number` column
- `crates/river-db/src/messages.rs` — add `turn_number` field, `get_turn_messages()` method
- `crates/river-db/src/schema.rs` — register `004_moves` migration
- `crates/river-db/src/lib.rs` — add `pub mod moves` and re-exports
- `crates/river-gateway/src/spectator/mod.rs` — rewrite as prompt dispatch runtime
- `crates/river-gateway/src/agent/task.rs` — add DB handle, turn numbering, persist-before-emit
- `crates/river-gateway/src/server.rs` — wire DB to AgentTask and SpectatorTask

### Deleted files
- `crates/river-gateway/src/spectator/compress.rs`
- `crates/river-gateway/src/spectator/curate.rs`
- `crates/river-gateway/src/spectator/room.rs`
- `crates/river-gateway/tests/iyou_test.rs`

---

### Task 1: Add `turn_number` to messages table and struct

**Files:**
- Modify: `crates/river-db/src/migrations/001_messages.sql`
- Modify: `crates/river-db/src/messages.rs`

- [ ] **Step 1: Write the failing test**

In `crates/river-db/src/messages.rs`, add to the `tests` module:

```rust
#[test]
fn test_insert_message_with_turn_number() {
    let db = Database::open_in_memory().unwrap();
    let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
    let gen = SnowflakeGenerator::new(birth);

    let msg = Message {
        id: gen.next_id(SnowflakeType::Message),
        session_id: "test-session".to_string(),
        role: MessageRole::User,
        content: Some("Hello".to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        turn_number: 1,
        created_at: 1000,
        metadata: None,
    };

    db.insert_message(&msg).unwrap();
    let messages = db.get_session_messages("test-session", 10).unwrap();
    assert_eq!(messages[0].turn_number, 1);
}

#[test]
fn test_get_turn_messages() {
    let db = Database::open_in_memory().unwrap();
    let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
    let gen = SnowflakeGenerator::new(birth);

    // Turn 1: user + assistant
    for (role, content) in [
        (MessageRole::User, "What is X?"),
        (MessageRole::Assistant, "X is Y."),
    ] {
        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".to_string(),
            role,
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            turn_number: 1,
            created_at: 1000,
            metadata: None,
        };
        db.insert_message(&msg).unwrap();
    }

    // Turn 2: different turn
    let msg = Message {
        id: gen.next_id(SnowflakeType::Message),
        session_id: "sess".to_string(),
        role: MessageRole::User,
        content: Some("Next question".to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        turn_number: 2,
        created_at: 2000,
        metadata: None,
    };
    db.insert_message(&msg).unwrap();

    let turn_1 = db.get_turn_messages("sess", 1).unwrap();
    assert_eq!(turn_1.len(), 2);
    assert_eq!(turn_1[0].content, Some("What is X?".to_string()));
    assert_eq!(turn_1[1].content, Some("X is Y.".to_string()));

    let turn_2 = db.get_turn_messages("sess", 2).unwrap();
    assert_eq!(turn_2.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/cassie/river-engine && cargo test -p river-db test_insert_message_with_turn_number test_get_turn_messages 2>&1 | tail -20`

Expected: compilation errors — `turn_number` field doesn't exist on `Message`.

- [ ] **Step 3: Add turn_number to migration and Message struct**

In `crates/river-db/src/migrations/001_messages.sql`, add `turn_number` column after `metadata`. The full column list becomes:

```sql
CREATE TABLE IF NOT EXISTS messages (
    id BLOB PRIMARY KEY,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT,
    tool_calls TEXT,
    tool_call_id TEXT,
    name TEXT,
    created_at INTEGER NOT NULL,
    metadata TEXT,
    turn_number INTEGER NOT NULL DEFAULT 0
);
```

Add the index after the existing indexes:

```sql
CREATE INDEX IF NOT EXISTS idx_messages_turn ON messages(session_id, turn_number);
```

Column order in the table: id(0), session_id(1), role(2), content(3), tool_calls(4), tool_call_id(5), name(6), created_at(7), metadata(8), turn_number(9). Adding at the end avoids disrupting existing column indices.

In `crates/river-db/src/messages.rs`, add `turn_number` to the `Message` struct:

```rust
pub struct Message {
    pub id: Snowflake,
    pub session_id: String,
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls: Option<String>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    pub created_at: i64,
    pub metadata: Option<String>,
    pub turn_number: u64,
}
```

Update `from_row` — add `turn_number` at the end (index 9), keeping all existing indices unchanged:

```rust
Ok(Self {
    id,
    session_id: row.get(1)?,
    role,
    content: row.get(3)?,
    tool_calls: row.get(4)?,
    tool_call_id: row.get(5)?,
    name: row.get(6)?,
    created_at: row.get(7)?,
    metadata: row.get(8)?,
    turn_number: row.get::<_, i64>(9)? as u64,
})
```

Update `insert_message` to include `turn_number`:

```rust
pub fn insert_message(&self, msg: &Message) -> RiverResult<()> {
    self.conn().execute(
        "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            msg.id.to_bytes().to_vec(),
            msg.session_id,
            msg.role.as_str(),
            msg.content,
            msg.tool_calls,
            msg.tool_call_id,
            msg.name,
            msg.created_at,
            msg.metadata,
            msg.turn_number as i64,
        ],
    ).map_err(|e| RiverError::database(e.to_string()))?;
    Ok(())
}
```

Update all SELECT queries to include `turn_number` at the end:

```sql
SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
```

Add `get_turn_messages`:

```rust
/// Get messages for a specific turn in a session
pub fn get_turn_messages(&self, session_id: &str, turn_number: u64) -> RiverResult<Vec<Message>> {
    let mut stmt = self.conn().prepare(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata, turn_number
         FROM messages
         WHERE session_id = ? AND turn_number = ?
         ORDER BY created_at"
    ).map_err(|e| RiverError::database(e.to_string()))?;

    let messages = stmt.query_map(params![session_id, turn_number as i64], Message::from_row)
        .map_err(|e| RiverError::database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RiverError::database(e.to_string()))?;

    Ok(messages)
}
```

- [ ] **Step 4: Fix existing tests**

Update both existing tests (`test_insert_and_get_message`, `test_message_ordering`) to include `turn_number: 0` in their `Message` constructors.

- [ ] **Step 5: Run all river-db tests**

Run: `cd /home/cassie/river-engine && cargo test -p river-db 2>&1 | tail -20`

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cd /home/cassie/river-engine
git add crates/river-db/src/migrations/001_messages.sql crates/river-db/src/messages.rs
git commit -m "feat(db): add turn_number to messages table and struct"
```

---

### Task 2: Create moves table and CRUD

**Files:**
- Create: `crates/river-db/src/migrations/004_moves.sql`
- Create: `crates/river-db/src/moves.rs`
- Modify: `crates/river-db/src/schema.rs`
- Modify: `crates/river-db/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/river-db/src/moves.rs` with tests only (no implementation):

```rust
//! Move CRUD operations

use river_core::{Snowflake, RiverError, RiverResult};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use crate::schema::Database;

/// A move: structural summary of one agent turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Move {
    pub id: Snowflake,
    pub channel: String,
    pub turn_number: u64,
    pub summary: String,
    pub tool_calls: Option<String>, // JSON
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn test_gen() -> SnowflakeGenerator {
        SnowflakeGenerator::new(AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap())
    }

    #[test]
    fn test_move_insert_and_query() {
        let db = test_db();
        let gen = test_gen();

        let m = Move {
            id: gen.next_id(SnowflakeType::Embedding),
            channel: "general".to_string(),
            turn_number: 1,
            summary: "User asked about X, agent explored files".to_string(),
            tool_calls: Some(r#"["read","glob"]"#.to_string()),
            created_at: 1000,
        };

        db.insert_move(&m).unwrap();

        let moves = db.get_moves("general", 100).unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].summary, "User asked about X, agent explored files");
        assert_eq!(moves[0].turn_number, 1);
    }

    #[test]
    fn test_get_moves_ordered_by_turn() {
        let db = test_db();
        let gen = test_gen();

        for turn in [3, 1, 2] {
            let m = Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: turn,
                summary: format!("Turn {}", turn),
                tool_calls: None,
                created_at: 1000 + turn as i64,
            };
            db.insert_move(&m).unwrap();
        }

        let moves = db.get_moves("general", 100).unwrap();
        assert_eq!(moves.len(), 3);
        assert_eq!(moves[0].turn_number, 1);
        assert_eq!(moves[1].turn_number, 2);
        assert_eq!(moves[2].turn_number, 3);
    }

    #[test]
    fn test_get_max_turn_empty() {
        let db = test_db();
        assert_eq!(db.get_max_turn("general").unwrap(), None);
    }

    #[test]
    fn test_get_max_turn() {
        let db = test_db();
        let gen = test_gen();

        for turn in [1, 5, 3] {
            let m = Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: turn,
                summary: format!("Turn {}", turn),
                tool_calls: None,
                created_at: 1000,
            };
            db.insert_move(&m).unwrap();
        }

        assert_eq!(db.get_max_turn("general").unwrap(), Some(5));
        assert_eq!(db.get_max_turn("other").unwrap(), None);
    }

    #[test]
    fn test_count_moves() {
        let db = test_db();
        let gen = test_gen();

        assert_eq!(db.count_moves("general").unwrap(), 0);

        for turn in 1..=3 {
            let m = Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: turn,
                summary: format!("Turn {}", turn),
                tool_calls: None,
                created_at: 1000,
            };
            db.insert_move(&m).unwrap();
        }

        assert_eq!(db.count_moves("general").unwrap(), 3);
        assert_eq!(db.count_moves("other").unwrap(), 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/cassie/river-engine && cargo test -p river-db moves 2>&1 | tail -20`

Expected: compilation errors — no `insert_move`, `get_moves`, etc. on `Database`.

- [ ] **Step 3: Create migration file**

Create `crates/river-db/src/migrations/004_moves.sql`:

```sql
CREATE TABLE IF NOT EXISTS moves (
    id BLOB PRIMARY KEY,
    channel TEXT NOT NULL,
    turn_number INTEGER NOT NULL,
    summary TEXT NOT NULL,
    tool_calls TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_moves_channel_turn
    ON moves (channel, turn_number);
```

- [ ] **Step 4: Register migration in schema.rs**

In `crates/river-db/src/schema.rs`, add after the `003_contexts` migration line:

```rust
self.run_migration("004_moves", include_str!("migrations/004_moves.sql"))?;
```

- [ ] **Step 5: Implement Move CRUD**

In `crates/river-db/src/moves.rs`, add after the `Move` struct definition (before `#[cfg(test)]`):

```rust
impl Move {
    fn from_row(row: &Row) -> Result<Self, rusqlite::Error> {
        let id_bytes: Vec<u8> = row.get(0)?;
        let id_array: [u8; 16] = id_bytes.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Blob,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid snowflake ID length",
                )),
            )
        })?;
        let id = Snowflake::from_bytes(id_array);

        Ok(Self {
            id,
            channel: row.get(1)?,
            turn_number: row.get::<_, i64>(2)? as u64,
            summary: row.get(3)?,
            tool_calls: row.get(4)?,
            created_at: row.get(5)?,
        })
    }
}

impl Database {
    /// Insert a move
    pub fn insert_move(&self, m: &Move) -> RiverResult<()> {
        self.conn()
            .execute(
                "INSERT INTO moves (id, channel, turn_number, summary, tool_calls, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    m.id.to_bytes().to_vec(),
                    m.channel,
                    m.turn_number as i64,
                    m.summary,
                    m.tool_calls,
                    m.created_at,
                ],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get moves for a channel, ordered by turn_number ascending
    pub fn get_moves(&self, channel: &str, limit: usize) -> RiverResult<Vec<Move>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, channel, turn_number, summary, tool_calls, created_at
                 FROM moves
                 WHERE channel = ?
                 ORDER BY turn_number ASC
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

    /// Get highest turn number with a move for a channel
    pub fn get_max_turn(&self, channel: &str) -> RiverResult<Option<u64>> {
        let result: Option<i64> = self
            .conn()
            .query_row(
                "SELECT MAX(turn_number) FROM moves WHERE channel = ?",
                params![channel],
                |row| row.get(0),
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(result.map(|n| n as u64))
    }

    /// Count moves for a channel
    pub fn count_moves(&self, channel: &str) -> RiverResult<usize> {
        let count: i64 = self
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM moves WHERE channel = ?",
                params![channel],
                |row| row.get(0),
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        Ok(count as usize)
    }
}
```

- [ ] **Step 6: Add module to lib.rs**

In `crates/river-db/src/lib.rs`, add:

```rust
pub mod moves;
```

And update re-exports:

```rust
pub use moves::Move;
```

- [ ] **Step 7: Run all river-db tests**

Run: `cd /home/cassie/river-engine && cargo test -p river-db 2>&1 | tail -30`

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
cd /home/cassie/river-engine
git add crates/river-db/
git commit -m "feat(db): add moves table with CRUD operations"
```

---

### Task 3: Prompt loading and template substitution

**Files:**
- Create: `crates/river-gateway/src/spectator/prompt.rs`

- [ ] **Step 1: Write the tests**

Create `crates/river-gateway/src/spectator/prompt.rs`:

```rust
//! Prompt loading and template substitution

use std::path::Path;

/// Load a prompt file. Returns None if the file does not exist.
pub fn load_prompt(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Substitute `{key}` placeholders in a template with values.
pub fn substitute(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{}}}", key), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_prompt_exists() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test.md");
        std::fs::write(&path, "You are the spectator.").unwrap();

        let result = load_prompt(&path);
        assert_eq!(result, Some("You are the spectator.".to_string()));
    }

    #[test]
    fn test_load_prompt_missing() {
        let result = load_prompt(Path::new("/nonexistent/prompt.md"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_substitute_single_var() {
        let template = "Turn {turn_number} completed.";
        let result = substitute(template, &[("turn_number", "5")]);
        assert_eq!(result, "Turn 5 completed.");
    }

    #[test]
    fn test_substitute_multiple_vars() {
        let template = "Channel: {channel}, Moves: {moves}";
        let result = substitute(template, &[("channel", "general"), ("moves", "1,2,3")]);
        assert_eq!(result, "Channel: general, Moves: 1,2,3");
    }

    #[test]
    fn test_substitute_no_vars() {
        let template = "No variables here.";
        let result = substitute(template, &[]);
        assert_eq!(result, "No variables here.");
    }

    #[test]
    fn test_substitute_missing_var_left_as_is() {
        let template = "Hello {name}, your id is {id}.";
        let result = substitute(template, &[("name", "Iris")]);
        assert_eq!(result, "Hello Iris, your id is {id}.");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/cassie/river-engine && cargo test -p river-gateway prompt 2>&1 | tail -20`

Expected: all pass (this is implementation + tests in one step since the code is trivial).

- [ ] **Step 3: Commit**

```bash
cd /home/cassie/river-engine
git add crates/river-gateway/src/spectator/prompt.rs
git commit -m "feat(spectator): prompt loading and template substitution"
```

---

### Task 4: Message transcript and move list formatting

**Files:**
- Create: `crates/river-gateway/src/spectator/format.rs`

- [ ] **Step 1: Write the implementation with tests**

Create `crates/river-gateway/src/spectator/format.rs`:

```rust
//! Formatting utilities for spectator handlers

use river_db::{Message, Move};

/// Format a list of messages into a readable transcript for the LLM.
///
/// Output format:
/// ```text
/// [user] What is X?
/// [assistant] X is Y.
/// [assistant/tool_call] read("file.rs")
/// [tool] Contents of file...
/// ```
pub fn format_transcript(messages: &[Message]) -> String {
    let mut lines = Vec::new();
    for msg in messages {
        let role = msg.role.as_str();
        if let Some(ref content) = msg.content {
            lines.push(format!("[{}] {}", role, content));
        }
        if let Some(ref tool_calls) = msg.tool_calls {
            lines.push(format!("[{}/tool_call] {}", role, tool_calls));
        }
    }
    lines.join("\n")
}

/// Format a list of moves for the compression prompt.
///
/// Output format:
/// ```text
/// Turn 1: User asked about X, agent explored files
/// Turn 2: Agent wrote implementation based on findings
/// ```
pub fn format_moves(moves: &[Move]) -> String {
    moves
        .iter()
        .map(|m| format!("Turn {}: {}", m.turn_number, m.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build a fallback move summary from messages when the LLM fails.
///
/// Format: "User message -> assistant response with tools: read, write"
pub fn fallback_summary(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    let mut tool_names = Vec::new();

    for msg in messages {
        match msg.role {
            river_db::MessageRole::User => parts.push("User message"),
            river_db::MessageRole::Assistant => parts.push("assistant response"),
            river_db::MessageRole::Tool => {
                if let Some(ref name) = msg.name {
                    if !tool_names.contains(name) {
                        tool_names.push(name.clone());
                    }
                }
            }
            river_db::MessageRole::System => {}
        }
    }

    let mut result = parts.join(" -> ");
    if !tool_names.is_empty() {
        result.push_str(&format!(" with tools: {}", tool_names.join(", ")));
    }
    if result.is_empty() {
        result = "Empty turn".to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};
    use river_db::MessageRole;

    fn test_gen() -> SnowflakeGenerator {
        SnowflakeGenerator::new(AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap())
    }

    fn make_msg(gen: &SnowflakeGenerator, role: MessageRole, content: &str) -> Message {
        Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "sess".to_string(),
            role,
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            turn_number: 1,
            created_at: 1000,
            metadata: None,
        }
    }

    #[test]
    fn test_format_transcript() {
        let gen = test_gen();
        let messages = vec![
            make_msg(&gen, MessageRole::User, "What is X?"),
            make_msg(&gen, MessageRole::Assistant, "X is Y."),
        ];
        let result = format_transcript(&messages);
        assert!(result.contains("[user] What is X?"));
        assert!(result.contains("[assistant] X is Y."));
    }

    #[test]
    fn test_format_moves() {
        let gen = test_gen();
        let moves = vec![
            Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: 1,
                summary: "User asked about X".to_string(),
                tool_calls: None,
                created_at: 1000,
            },
            Move {
                id: gen.next_id(SnowflakeType::Embedding),
                channel: "general".to_string(),
                turn_number: 2,
                summary: "Agent wrote response".to_string(),
                tool_calls: None,
                created_at: 2000,
            },
        ];
        let result = format_moves(&moves);
        assert_eq!(result, "Turn 1: User asked about X\nTurn 2: Agent wrote response");
    }

    #[test]
    fn test_fallback_summary() {
        let gen = test_gen();
        let mut tool_msg = make_msg(&gen, MessageRole::Tool, "file contents");
        tool_msg.name = Some("read".to_string());

        let messages = vec![
            make_msg(&gen, MessageRole::User, "Read the file"),
            make_msg(&gen, MessageRole::Assistant, "Let me read it"),
            tool_msg,
        ];
        let result = fallback_summary(&messages);
        assert_eq!(result, "User message -> assistant response with tools: read");
    }

    #[test]
    fn test_fallback_summary_empty() {
        let result = fallback_summary(&[]);
        assert_eq!(result, "Empty turn");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/cassie/river-engine && cargo test -p river-gateway format 2>&1 | tail -20`

Expected: all pass.

- [ ] **Step 3: Commit**

```bash
cd /home/cassie/river-engine
git add crates/river-gateway/src/spectator/format.rs
git commit -m "feat(spectator): transcript and move formatting utilities"
```

---

### Task 5: Moment response parser

**Files:**
- Modify: `crates/river-gateway/src/spectator/handlers.rs` (create with parser only first)

- [ ] **Step 1: Write the parser with tests**

Create `crates/river-gateway/src/spectator/handlers.rs`:

```rust
//! Spectator event handlers

use regex::Regex;

/// Parsed moment response from LLM
#[derive(Debug, Clone, PartialEq)]
pub struct MomentResponse {
    pub start_turn: u64,
    pub end_turn: u64,
    pub narrative: String,
}

/// Parse a moment LLM response.
///
/// Expected format:
/// ```text
/// turns: 5-20
/// ---
/// The narrative paragraph here...
/// ```
///
/// Returns Err if the format is not followed.
pub fn parse_moment_response(response: &str) -> Result<MomentResponse, String> {
    // Split on first "---"
    let parts: Vec<&str> = response.splitn(2, "---").collect();
    if parts.len() != 2 {
        return Err("No '---' separator found in response".to_string());
    }

    let header = parts[0].trim();
    let narrative = parts[1].trim().to_string();

    if narrative.is_empty() {
        return Err("Empty narrative after '---' separator".to_string());
    }

    // Parse turns: N-M
    let re = Regex::new(r"turns:\s*(\d+)\s*-\s*(\d+)").unwrap();
    let caps = re
        .captures(header)
        .ok_or_else(|| format!("No 'turns: N-M' found in header: '{}'", header))?;

    let start_turn: u64 = caps[1]
        .parse()
        .map_err(|e| format!("Invalid start turn: {}", e))?;
    let end_turn: u64 = caps[2]
        .parse()
        .map_err(|e| format!("Invalid end turn: {}", e))?;

    Ok(MomentResponse {
        start_turn,
        end_turn,
        narrative,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_moment_parses_turn_range() {
        let response = "turns: 5-20\n---\nThe agent worked through configuration issues.";
        let result = parse_moment_response(response).unwrap();
        assert_eq!(result.start_turn, 5);
        assert_eq!(result.end_turn, 20);
        assert_eq!(result.narrative, "The agent worked through configuration issues.");
    }

    #[test]
    fn test_moment_parses_with_whitespace() {
        let response = "turns:  12 - 34 \n---\n\nA multi-paragraph\nnarrative here.";
        let result = parse_moment_response(response).unwrap();
        assert_eq!(result.start_turn, 12);
        assert_eq!(result.end_turn, 34);
        assert!(result.narrative.contains("multi-paragraph"));
    }

    #[test]
    fn test_moment_rejects_missing_separator() {
        let response = "turns: 5-20\nThe agent worked through issues.";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No '---' separator"));
    }

    #[test]
    fn test_moment_rejects_missing_turn_range() {
        let response = "some header\n---\nThe narrative.";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No 'turns: N-M'"));
    }

    #[test]
    fn test_moment_rejects_empty_narrative() {
        let response = "turns: 1-10\n---\n";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty narrative"));
    }

    #[test]
    fn test_moment_rejects_empty_narrative_whitespace() {
        let response = "turns: 1-10\n---\n   \n  ";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty narrative"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/cassie/river-engine && cargo test -p river-gateway parse_moment 2>&1 | tail -20`

Expected: all pass.

- [ ] **Step 3: Commit**

```bash
cd /home/cassie/river-engine
git add crates/river-gateway/src/spectator/handlers.rs
git commit -m "feat(spectator): strict moment response parser"
```

---

### Task 6: Rewrite spectator mod.rs as prompt dispatch runtime

**Files:**
- Modify: `crates/river-gateway/src/spectator/mod.rs`
- Delete: `crates/river-gateway/src/spectator/compress.rs`
- Delete: `crates/river-gateway/src/spectator/curate.rs`
- Delete: `crates/river-gateway/src/spectator/room.rs`

This is the largest task. It replaces the entire spectator module.

- [ ] **Step 1: Delete old modules**

```bash
rm crates/river-gateway/src/spectator/compress.rs
rm crates/river-gateway/src/spectator/curate.rs
rm crates/river-gateway/src/spectator/room.rs
```

- [ ] **Step 2: Rewrite mod.rs**

Replace the entire contents of `crates/river-gateway/src/spectator/mod.rs` with:

```rust
//! Spectator — prompt-driven observing self
//!
//! The spectator is a thin event dispatcher. On each event it loads
//! a prompt file, assembles context, calls the LLM, and handles
//! the structured output.

pub mod format;
pub mod handlers;
pub mod prompt;

use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::r#loop::ModelClient;
use crate::session::PRIMARY_SESSION_ID;
use chrono::Utc;
use river_core::SnowflakeGenerator;
use river_db::Database;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Compression threshold: consider creating a moment when moves exceed this
const COMPRESSION_MOVES_THRESHOLD: usize = 50;

/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    /// Directory containing spectator prompt files
    pub spectator_dir: PathBuf,
    /// Directory for writing moment files
    pub moments_dir: PathBuf,
    /// Model timeout
    pub model_timeout: std::time::Duration,
}

/// The spectator task
pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    model_client: ModelClient,
    db: Arc<Mutex<Database>>,
    snowflake_gen: Arc<SnowflakeGenerator>,
    /// Cached identity (system prompt)
    identity: String,
    /// Cached prompt templates (None = handler disabled)
    on_turn_complete: Option<String>,
    on_compress: Option<String>,
    on_pressure: Option<String>,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            config,
            bus,
            model_client,
            db,
            snowflake_gen,
            identity: String::new(),
            on_turn_complete: None,
            on_compress: None,
            on_pressure: None,
        }
    }

    /// Main run loop
    pub async fn run(mut self) {
        // Load identity — required, fail if missing
        let identity_path = self.config.spectator_dir.join("identity.md");
        self.identity = match prompt::load_prompt(&identity_path) {
            Some(id) => {
                tracing::info!("Spectator identity loaded from {:?}", identity_path);
                id
            }
            None => {
                tracing::error!("Spectator identity.md not found at {:?} — cannot start", identity_path);
                return;
            }
        };

        // Load optional prompts
        self.on_turn_complete = prompt::load_prompt(
            &self.config.spectator_dir.join("on-turn-complete.md"),
        );
        self.on_compress = prompt::load_prompt(
            &self.config.spectator_dir.join("on-compress.md"),
        );
        self.on_pressure = prompt::load_prompt(
            &self.config.spectator_dir.join("on-pressure.md"),
        );

        tracing::info!(
            turn_complete = self.on_turn_complete.is_some(),
            compress = self.on_compress.is_some(),
            pressure = self.on_pressure.is_some(),
            "Spectator handlers loaded"
        );

        let mut event_rx = self.bus.subscribe();

        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
                    channel,
                    turn_number,
                    tool_calls,
                    ..
                })) => {
                    self.handle_turn_complete(&channel, turn_number, &tool_calls).await;
                }
                Ok(CoordinatorEvent::Agent(AgentEvent::ContextPressure {
                    usage_percent,
                    ..
                })) => {
                    self.handle_pressure(usage_percent).await;
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Spectator: shutdown received");
                    break;
                }
                Ok(_) => {
                    // Ignore other events
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Event receive error");
                }
            }
        }

        tracing::info!("Spectator task stopped");
    }

    async fn handle_turn_complete(&self, channel: &str, turn_number: u64, tool_calls: &[String]) {
        let template = match &self.on_turn_complete {
            Some(t) => t,
            None => return,
        };

        // Lock-query-drop: get messages for this turn
        let messages = {
            let db = match self.db.lock() {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!(error = %e, "DB lock poisoned");
                    return;
                }
            };
            match db.get_turn_messages(PRIMARY_SESSION_ID, turn_number) {
                Ok(msgs) => msgs,
                Err(e) => {
                    tracing::error!(turn = turn_number, error = %e, "Failed to query turn messages");
                    return;
                }
            }
        }; // MutexGuard dropped here

        if messages.is_empty() {
            tracing::error!(turn = turn_number, "No messages found for turn — skipping");
            return;
        }

        // Format transcript and substitute into prompt
        let transcript = format::format_transcript(&messages);
        let user_prompt = prompt::substitute(template, &[
            ("transcript", &transcript),
            ("turn_number", &turn_number.to_string()),
        ]);

        // Call LLM
        let summary = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(turn = turn_number, error = %e, "Model call failed, using fallback");
                format::fallback_summary(&messages)
            }
        };

        // Lock-query-drop: insert move
        {
            let db = match self.db.lock() {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!(error = %e, "DB lock poisoned");
                    return;
                }
            };
            let m = river_db::Move {
                id: self.snowflake_gen.next_id(river_core::SnowflakeType::Embedding),
                channel: channel.to_string(),
                turn_number,
                summary: summary.clone(),
                tool_calls: Some(serde_json::to_string(tool_calls).unwrap_or_default()),
                created_at: Utc::now().timestamp(),
            };
            if let Err(e) = db.insert_move(&m) {
                tracing::error!(error = %e, "Failed to insert move");
                return;
            }
        }; // MutexGuard dropped here

        // Emit event
        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
            channel: channel.to_string(),
            timestamp: Utc::now(),
        }));

        tracing::debug!(turn = turn_number, channel = %channel, "Move recorded");

        // Check compression threshold
        if self.on_compress.is_some() {
            let count = {
                let db = self.db.lock().unwrap();
                db.count_moves(channel).unwrap_or(0)
            };
            if count > COMPRESSION_MOVES_THRESHOLD {
                self.handle_compress(channel).await;
            }
        }
    }

    async fn handle_compress(&self, channel: &str) {
        let template = match &self.on_compress {
            Some(t) => t,
            None => return,
        };

        // Lock-query-drop: get all moves
        let moves = {
            let db = self.db.lock().unwrap();
            match db.get_moves(channel, 10_000) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to query moves for compression");
                    return;
                }
            }
        };

        let moves_text = format::format_moves(&moves);
        let user_prompt = prompt::substitute(template, &[
            ("moves", &moves_text),
            ("channel", channel),
        ]);

        // Call LLM
        let response_text = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::error!(error = %e, "Model call failed for compression");
                return;
            }
        };

        // Parse — strict, no fallback
        let moment = match handlers::parse_moment_response(&response_text) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "Failed to parse moment response");
                return;
            }
        };

        // Write moment file
        let timestamp = Utc::now();
        let sanitized_channel = channel.replace(['/', '\\', ' '], "-");
        let filename = format!("{}-{}.md", sanitized_channel, timestamp.format("%Y%m%d%H%M%S"));
        let moment_path = self.config.moments_dir.join(&filename);

        let content = format!(
            "---\nchannel: {}\nturns: {}-{}\ncreated: {}\nauthor: spectator\ntype: moment\n---\n\n{}",
            channel,
            moment.start_turn,
            moment.end_turn,
            timestamp.to_rfc3339(),
            moment.narrative,
        );

        if let Err(e) = tokio::fs::create_dir_all(&self.config.moments_dir).await {
            tracing::error!(error = %e, "Failed to create moments directory");
            return;
        }

        if let Err(e) = tokio::fs::write(&moment_path, &content).await {
            tracing::error!(error = %e, "Failed to write moment file");
            return;
        }

        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MomentCreated {
            summary: moment.narrative,
            timestamp,
        }));

        tracing::info!(
            channel = %channel,
            turns = format!("{}-{}", moment.start_turn, moment.end_turn),
            path = %moment_path.display(),
            "Moment created"
        );
    }

    async fn handle_pressure(&self, usage_percent: f64) {
        let template = match &self.on_pressure {
            Some(t) => t,
            None => return,
        };

        let user_prompt = prompt::substitute(template, &[
            ("usage_percent", &format!("{:.1}", usage_percent)),
        ]);

        if let Ok(warning) = self.call_model(&user_prompt).await {
            self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Warning {
                content: warning,
                timestamp: Utc::now(),
            }));
        }
    }

    /// Call the model with the spectator's identity as system prompt.
    async fn call_model(&self, user_prompt: &str) -> Result<String, String> {
        use crate::r#loop::context::ChatMessage;

        let messages = vec![
            ChatMessage::system(self.identity.clone()),
            ChatMessage::user(user_prompt.to_string()),
        ];

        let response = self
            .model_client
            .complete(&messages, &[])
            .await
            .map_err(|e| format!("Model error: {}", e))?;

        response
            .content
            .ok_or_else(|| "Model returned no content".to_string())
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cd /home/cassie/river-engine && cargo check -p river-gateway 2>&1 | tail -30`

Expected: will fail due to `server.rs` still constructing the old `SpectatorTask`. That's Task 7.

- [ ] **Step 4: Commit the spectator rewrite**

```bash
cd /home/cassie/river-engine
git add -A crates/river-gateway/src/spectator/
git commit -m "feat(spectator): prompt-driven dispatch runtime

Replaces Compressor, Curator, RoomWriter with prompt dispatch.
Behavior defined by workspace/spectator/*.md files.
identity.md required, event prompts optional."
```

---

### Task 7: Wire DB and SnowflakeGenerator to AgentTask and SpectatorTask in server.rs

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`
- Modify: `crates/river-gateway/src/server.rs`
- Delete: `crates/river-gateway/tests/iyou_test.rs`

- [ ] **Step 1: Add DB handle and SnowflakeGenerator to AgentTask**

In `crates/river-gateway/src/agent/task.rs`, add to imports:

```rust
use river_db::{Database, Message, MessageRole};
use crate::session::PRIMARY_SESSION_ID;
use river_core::{SnowflakeGenerator, SnowflakeType};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
```

Add two fields to the `AgentTask` struct (after `last_prompt_tokens`):

```rust
    db: Arc<Mutex<Database>>,
    snowflake_gen: Arc<SnowflakeGenerator>,
```

Update `AgentTask::new()` — add parameters and initialize:

```rust
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
        let context_assembler = ContextAssembler::new(
            config.context_budget.clone(),
            config.embeddings_dir.clone(),
        );

        Self {
            config,
            bus,
            message_queue,
            model_client,
            tool_executor,
            context_assembler,
            flash_queue,
            turn_count: 0,
            channel_context: None,
            conversation: Vec::new(),
            last_prompt_tokens: 0,
            db,
            snowflake_gen,
        }
    }
```

- [ ] **Step 2: Add persist_turn_messages to AgentTask**

Add this method to the `impl AgentTask` block:

```rust
    /// Persist all conversation messages from this turn to the database.
    /// Must be called before emitting TurnComplete (ordering guarantee).
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

        let mut persisted = 0;
        for chat_msg in &self.conversation {
            // Skip system messages
            if chat_msg.role == "system" {
                continue;
            }

            let role = match chat_msg.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                _ => continue,
            };

            let tool_calls_json = chat_msg.tool_calls.as_ref().map(|tc| {
                serde_json::to_string(tc).unwrap_or_default()
            });

            let msg = Message {
                id: self.snowflake_gen.next_id(SnowflakeType::Message),
                session_id: PRIMARY_SESSION_ID.to_string(),
                role,
                content: chat_msg.content.clone(),
                tool_calls: tool_calls_json,
                tool_call_id: chat_msg.tool_call_id.clone(),
                name: chat_msg.name.clone(),
                created_at: now,
                metadata: None,
                turn_number: self.turn_count,
            };

            if let Err(e) = db.insert_message(&msg) {
                tracing::warn!(error = %e, "Failed to persist message");
            } else {
                persisted += 1;
            }
        }

        if persisted > 0 {
            tracing::debug!(persisted = persisted, turn = self.turn_count, "Messages persisted");
        }
    }
```

- [ ] **Step 3: Update turn_cycle settle section**

In the `turn_cycle` method (around line 336), change the settle section to persist before emitting:

```rust
        // ========== SETTLE ==========
        // Persist messages BEFORE emitting TurnComplete (ordering guarantee)
        self.persist_turn_messages();

        let transcript_summary = format!(
            "Turn {} completed: {} messages, {} tool calls ({} failed)",
            self.turn_count,
            incoming.len(),
            stats.total_tool_calls,
            stats.failed_tool_calls
        );

        self.bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
```

- [ ] **Step 4: Update server.rs AgentTask construction**

In `crates/river-gateway/src/server.rs`, update the `AgentTask::new()` call (around line 396):

```rust
    let agent_task = AgentTask::new(
        agent_config,
        coordinator.bus().clone(),
        message_queue,
        agent_model_client,
        state.tool_executor.clone(),
        flash_queue.clone(),
        db_arc.clone(),
        snowflake_gen.clone(),
    );
```

- [ ] **Step 5: Update server.rs SpectatorTask construction**

Replace the spectator setup block (around lines 407-428) with:

```rust
    // Create and spawn spectator task
    let spectator_model = ModelClient::new(
        spectator_model_url.clone(),
        spectator_model_name.clone(),
        Duration::from_secs(60),
    )?;

    let spectator_config = SpectatorConfig {
        spectator_dir: config.workspace.join("spectator"),
        moments_dir: config.workspace.join("embeddings").join("moments"),
        model_timeout: Duration::from_secs(60),
    };

    let spectator_task = SpectatorTask::new(
        spectator_config,
        coordinator.bus().clone(),
        spectator_model,
        db_arc.clone(),
        snowflake_gen.clone(),
    );

    coordinator.spawn_task("spectator", |_| spectator_task.run());
```

Remove unused imports for `VectorStore` and old `SpectatorConfig::from_workspace` if they cause warnings.

- [ ] **Step 6: Delete iyou_test.rs**

```bash
rm crates/river-gateway/tests/iyou_test.rs
```

This test file references `Compressor`, `RoomWriter`, `Curator`, and `SpectatorConfig::from_workspace`, all of which are deleted.

- [ ] **Step 7: Verify compilation**

Run: `cd /home/cassie/river-engine && cargo check -p river-gateway 2>&1 | tail -30`

Iterate until clean. Common fixes needed:
- Remove unused imports in `server.rs` (`VectorStore`, old spectator types)
- Update any remaining references to old `SpectatorConfig::from_workspace`

- [ ] **Step 8: Commit**

```bash
cd /home/cassie/river-engine
git add -A crates/river-gateway/
git commit -m "feat: wire DB to AgentTask/SpectatorTask, delete iyou_test

AgentTask gains Arc<Mutex<Database>> and SnowflakeGenerator.
persist_turn_messages() called before TurnComplete emission.
SpectatorTask constructed with new prompt-driven config.
Old integration test deleted (referenced removed types)."
```

---

### Task 8: Full compilation and integration test

**Files:**
- All modified crates

- [ ] **Step 1: Full workspace build**

Run: `cd /home/cassie/river-engine && cargo build 2>&1 | tail -30`

Fix any remaining compilation errors across the workspace (other crates may reference old spectator types).

- [ ] **Step 2: Run all tests**

Run: `cd /home/cassie/river-engine && cargo test 2>&1 | tail -50`

Fix any failing tests. Old spectator tests that reference `Compressor`, `Curator`, `RoomWriter` should have been deleted in Task 6. If any remain, delete them.

- [ ] **Step 3: Commit any fixes**

```bash
cd /home/cassie/river-engine
git add -A
git commit -m "fix: resolve compilation and test issues across workspace"
```

---

### Task 9: Create default spectator prompt files

**Files:**
- Create: example spectator directory with default prompts

- [ ] **Step 1: Create workspace/spectator/ with defaults**

These are example files. In a real deployment they'd live in the agent's workspace, but we need them for testing.

Create `crates/river-gateway/tests/fixtures/spectator/identity.md`:

```markdown
You are the spectator — the observing self. You watch the agent's turns and write structural summaries. You focus on what shifted, what was decided, what was attempted — the shape of the exchange, not the content itself. Write in second person.
```

Create `crates/river-gateway/tests/fixtures/spectator/on-turn-complete.md`:

```markdown
Given this transcript of agent turn {turn_number}, write a one-to-two sentence structural note capturing what happened. Focus on the shape of the exchange — what shifted, what was decided, what was attempted.

Transcript:
{transcript}
```

Create `crates/river-gateway/tests/fixtures/spectator/on-compress.md`:

```markdown
Below are structural move notes from channel "{channel}". Identify a coherent arc within these moves and compress it into a narrative paragraph.

You must respond in exactly this format:

turns: START-END
---
Your narrative paragraph here.

Choose a range that forms a natural arc. You do not have to use all the moves.

Moves:
{moves}
```

Create `crates/river-gateway/tests/fixtures/spectator/on-pressure.md`:

```markdown
Context usage is at {usage_percent}%. Write a brief warning (one sentence) about what the agent should consider.
```

- [ ] **Step 2: Commit**

```bash
cd /home/cassie/river-engine
git add crates/river-gateway/tests/fixtures/spectator/
git commit -m "feat: default spectator prompt files for testing"
```
