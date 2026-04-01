# Snowflake ID Generation

This document describes all locations in the River Engine codebase where Snowflake IDs are generated.

## Overview

Snowflake IDs are 128-bit unique identifiers with the following structure:

| Bits | Field | Description |
|------|-------|-------------|
| 64 | Timestamp | Microseconds since agent birth |
| 36 | Agent Birth | yyyymmddhhmmss packed |
| 8 | Type | Entity type identifier |
| 20 | Sequence | Counter for same-microsecond IDs |

## Snowflake Types

Defined in `crates/river-core/src/snowflake/types.rs`:

| Type | Value | Description |
|------|-------|-------------|
| Message | 0x01 | Conversation messages |
| Embedding | 0x02 | Memory embeddings |
| Session | 0x03 | Conversation sessions |
| Subagent | 0x04 | Spawned subagents |
| ToolCall | 0x05 | Tool invocations |
| Context | 0x06 | Context windows for persistence |

## Generation Locations

### 1. Agent Birth Command

**File:** `crates/river-gateway/src/main.rs:110`

**Purpose:** Creates the agent's birth memory - its first memory that encodes identity.

```rust
let birth_memory = Memory {
    id: gen.next_id(SnowflakeType::Embedding),
    content: format!("i am {}", name),
    source: "system:birth".to_string(),
    ...
};
```

**When:** Called via `river-gateway birth --data-dir <path> --name <name>`

---

### 2. Message Persistence

**File:** `crates/river-gateway/src/loop/mod.rs:576`

**Purpose:** Assigns IDs to messages as they're persisted to the database during the settle phase.

```rust
let msg = Message {
    id: self.snowflake_gen.next_id(SnowflakeType::Message),
    session_id: PRIMARY_SESSION_ID.to_string(),
    role,
    content: chat_msg.content.clone(),
    ...
};
```

**When:** After each agent loop cycle, during `persist_messages()`

---

### 3. Memory Creation (Embed Tool)

**File:** `crates/river-gateway/src/tools/memory.rs:71`

**Purpose:** Creates new memories when the agent uses the `embed` tool.

```rust
let id = self.snowflake_gen.next_id(SnowflakeType::Embedding);
let memory = Memory {
    id,
    content: content.to_string(),
    embedding,
    source: source.to_string(),
    ...
};
```

**When:** Agent calls `embed` tool to store a new memory

---

### 4. Sub-Session Creation

**File:** `crates/river-gateway/src/session/mod.rs:56`

**Purpose:** Creates IDs for sub-sessions (non-primary conversation threads).

```rust
pub fn sub_session(snowflake_gen: &SnowflakeGenerator) -> Self {
    let id = snowflake_gen.next_id(SnowflakeType::Session);
    Self {
        id: id.to_string(),
        ...
    }
}
```

**When:** Creating auxiliary sessions (not commonly used in current implementation)

---

### 5. Subagent Registration

**File:** `crates/river-gateway/src/subagent/mod.rs:56`

**Purpose:** Assigns unique IDs to spawned subagents.

```rust
pub fn register(
    &mut self,
    subagent_type: SubagentType,
    task: String,
    model: String,
) -> (Snowflake, Arc<InternalQueue>) {
    let id = self.snowflake_gen.next_id(SnowflakeType::Subagent);
    ...
}
```

**When:** Agent uses `spawn_subagent` tool

---

### 6. Context Initialization

**File:** `crates/river-gateway/src/loop/mod.rs`

**Purpose:** Creates snowflake IDs for context windows used in conversation persistence.

```rust
fn create_fresh_context(&mut self) -> RiverResult<()> {
    let id = self.snowflake_gen.next_id(SnowflakeType::Context);
    // ...
}
```

**When:** At agent startup if no active context exists, or after context rotation

---

### 7. Context Archival

**File:** `crates/river-gateway/src/loop/mod.rs`

**Purpose:** Generates archive timestamp snowflake when rotating context.

```rust
fn archive_current_context(&mut self, summary: Option<&str>) -> RiverResult<()> {
    let archived_at = self.snowflake_gen.next_id(SnowflakeType::Context);
    // ...
}
```

**When:** Manual rotation via `rotate_context` tool or auto-rotation at 90% capacity

---

### 8. Migration Tool - Database Initialization

**File:** `crates/river-migrate/src/main.rs:323`

**Purpose:** Creates birth memory when initializing a new database.

```rust
let birth_memory_id = snowflake_gen.next_id(SnowflakeType::Embedding);
conn.execute(
    "INSERT INTO memories (id, content, embedding, source, timestamp, expires_at, metadata)
     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
    rusqlite::params![
        birth_memory_id.to_bytes(),
        format!("i am {}", agent_name),
        ...
    ],
)?;
```

**When:** Running `river-migrate init`

---

### 9. Migration Tool - Message Import

**File:** `crates/river-migrate/src/main.rs:409`

**Purpose:** Assigns IDs to imported messages from JSON files.

```rust
for msg in messages_input.messages {
    let id = snowflake_gen.next_id(SnowflakeType::Message);
    conn.execute(
        "INSERT INTO messages ...",
        rusqlite::params![id.to_bytes(), ...],
    )?;
}
```

**When:** Running `river-migrate import-messages`

---

### 10. Migration Tool - Memory Import

**File:** `crates/river-migrate/src/main.rs:483`

**Purpose:** Assigns IDs to imported memories from JSON files.

```rust
for mem in memories_input.memories {
    let id = snowflake_gen.next_id(SnowflakeType::Embedding);
    conn.execute(
        "INSERT INTO memories ...",
        rusqlite::params![id.to_bytes(), ...],
    )?;
}
```

**When:** Running `river-migrate import-memories`

---

## Test-Only Locations

These locations generate snowflakes only in test code:

| File | Lines | Purpose |
|------|-------|---------|
| `crates/river-gateway/src/memory/search.rs` | 142, 172 | Memory search tests |
| `crates/river-gateway/src/db/messages.rs` | 157, 183 | Message DB tests |
| `crates/river-gateway/src/db/memories.rs` | 267, 289, 312, 333 | Memory DB tests |

---

## Generator Initialization

The `SnowflakeGenerator` is created at these locations:

### Gateway Server

**File:** `crates/river-gateway/src/server.rs:110`

```rust
let snowflake_gen = Arc::new(river_core::SnowflakeGenerator::new(gateway_config.agent_birth));
```

The generator is then passed to:
- `SubagentManager::new()`
- `EmbedTool::new()`
- `AgentLoop::new()`

### Migration Tool

**File:** `crates/river-migrate/src/main.rs:319, 403, 478`

Each command creates its own generator from the stored agent birth.

---

## ID Uniqueness Guarantees

1. **Timestamp precision:** Microsecond resolution
2. **Sequence counter:** 20-bit (1,048,575 IDs per microsecond)
3. **Thread safety:** Atomic operations for concurrent generation
4. **Monotonic ordering:** IDs always increase within a generator instance

---

## Diagram: Snowflake Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            SnowflakeGenerator                                │
│                           (Arc<SnowflakeGenerator>)                          │
└────────────────────────────────────┬────────────────────────────────────────┘
                                     │
        ┌────────────────────────────┼────────────────────────────┐
        │                            │                            │
        ▼                            ▼                            ▼
┌───────────────────┐    ┌───────────────────┐    ┌───────────────────────┐
│     AgentLoop     │    │     EmbedTool     │    │   SubagentManager     │
│                   │    │                   │    │                       │
│ Message IDs (0x01)│    │ Embedding IDs     │    │ Subagent IDs (0x04)   │
│ Context IDs (0x06)│    │ (0x02)            │    │                       │
└───────────────────┘    └───────────────────┘    └───────────────────────┘
```
