# Context Persistence Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ephemeral in-memory context with file-backed persistence that survives restarts and archives rotated contexts to SQLite.

**Architecture:** Active context stored as JSONL file in workspace, loaded on wake and appended after each model response. On rotation, file contents archived to DB as blob, new context started with optional summary. Token tracking from API response triggers 80% warning and 90% auto-rotation.

**Tech Stack:** Rust, SQLite (rusqlite), JSONL (serde_json), tokio fs

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/river-core/src/snowflake/types.rs` | Add `Context = 0x06` variant |
| `crates/river-gateway/src/db/migrations/003_contexts.sql` | Schema for contexts table |
| `crates/river-gateway/src/db/contexts.rs` | Context struct and DB operations |
| `crates/river-gateway/src/db/schema.rs` | Run new migration |
| `crates/river-gateway/src/db/mod.rs` | Export contexts module |
| `crates/river-gateway/src/loop/persistence.rs` | ContextFile struct for JSONL operations |
| `crates/river-gateway/src/loop/mod.rs` | Integrate persistence, startup, append |
| `crates/river-gateway/src/loop/context.rs` | Load from file, inject warnings |
| `crates/river-gateway/src/tools/scheduling.rs` | Update rotate_context with summary param |
| `docs/snowflake-generation.md` | Document Context type |

---

### Task 1: Add Context Snowflake Type

**Files:**
- Modify: `crates/river-core/src/snowflake/types.rs`

- [ ] **Step 1: Add Context variant to SnowflakeType enum**

In `crates/river-core/src/snowflake/types.rs`, add `Context = 0x06` to the enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SnowflakeType {
    /// A message in a conversation.
    Message = 0x01,
    /// An embedding vector.
    Embedding = 0x02,
    /// A conversation session.
    Session = 0x03,
    /// A subagent spawned by the main agent.
    Subagent = 0x04,
    /// A tool call invocation.
    ToolCall = 0x05,
    /// A context window for conversation persistence.
    Context = 0x06,
}
```

- [ ] **Step 2: Update from_u8 to handle Context**

```rust
pub fn from_u8(value: u8) -> Option<Self> {
    match value {
        0x01 => Some(SnowflakeType::Message),
        0x02 => Some(SnowflakeType::Embedding),
        0x03 => Some(SnowflakeType::Session),
        0x04 => Some(SnowflakeType::Subagent),
        0x05 => Some(SnowflakeType::ToolCall),
        0x06 => Some(SnowflakeType::Context),
        _ => None,
    }
}
```

- [ ] **Step 3: Update all() to include Context**

```rust
pub fn all() -> &'static [SnowflakeType] {
    &[
        SnowflakeType::Message,
        SnowflakeType::Embedding,
        SnowflakeType::Session,
        SnowflakeType::Subagent,
        SnowflakeType::ToolCall,
        SnowflakeType::Context,
    ]
}
```

- [ ] **Step 4: Update Display impl**

```rust
impl fmt::Display for SnowflakeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnowflakeType::Message => write!(f, "Message"),
            SnowflakeType::Embedding => write!(f, "Embedding"),
            SnowflakeType::Session => write!(f, "Session"),
            SnowflakeType::Subagent => write!(f, "Subagent"),
            SnowflakeType::ToolCall => write!(f, "ToolCall"),
            SnowflakeType::Context => write!(f, "Context"),
        }
    }
}
```

- [ ] **Step 5: Add test for Context type**

```rust
#[test]
fn test_snowflake_type_context() {
    assert_eq!(SnowflakeType::Context.as_u8(), 0x06);
    assert_eq!(SnowflakeType::from_u8(0x06), Some(SnowflakeType::Context));
    assert_eq!(format!("{}", SnowflakeType::Context), "Context");
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-core snowflake`
Expected: All tests pass including new Context test

- [ ] **Step 7: Commit**

```bash
git add crates/river-core/src/snowflake/types.rs
git commit -m "feat(core): add Context snowflake type (0x06)"
```

---

### Task 2: Create Database Migration and Context Module

**Files:**
- Create: `crates/river-gateway/src/db/migrations/003_contexts.sql`
- Create: `crates/river-gateway/src/db/contexts.rs`
- Modify: `crates/river-gateway/src/db/schema.rs`
- Modify: `crates/river-gateway/src/db/mod.rs`

- [ ] **Step 1: Create migration file**

Create `crates/river-gateway/src/db/migrations/003_contexts.sql`:

```sql
-- Contexts table for conversation persistence
CREATE TABLE IF NOT EXISTS contexts (
    id BLOB PRIMARY KEY,              -- 128-bit snowflake (type 0x06)
    archived_at BLOB,                 -- Snowflake generated at rotation, NULL while active
    token_count INTEGER,              -- Last known prompt_tokens from API
    summary TEXT,                     -- Summary provided at rotation
    blob BLOB                         -- JSONL content, NULL while active
);
```

- [ ] **Step 2: Create contexts.rs with Context struct**

Create `crates/river-gateway/src/db/contexts.rs`:

```rust
//! Context CRUD operations for conversation persistence

use river_core::{RiverError, RiverResult, Snowflake};
use rusqlite::{params, Row};

use super::Database;

/// Context entry for conversation persistence
#[derive(Debug, Clone)]
pub struct Context {
    pub id: Snowflake,
    pub archived_at: Option<Snowflake>,
    pub token_count: Option<i64>,
    pub summary: Option<String>,
    pub blob: Option<Vec<u8>>,
}

impl Context {
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

        let archived_at = row.get::<_, Option<Vec<u8>>>(1)?.and_then(|bytes| {
            let arr: [u8; 16] = bytes.try_into().ok()?;
            Some(Snowflake::from_bytes(arr))
        });

        Ok(Self {
            id,
            archived_at,
            token_count: row.get(2)?,
            summary: row.get(3)?,
            blob: row.get(4)?,
        })
    }

    /// Check if this context is active (not archived)
    pub fn is_active(&self) -> bool {
        self.blob.is_none()
    }
}

impl Database {
    /// Insert a new context (active, no blob)
    pub fn insert_context(&self, id: Snowflake) -> RiverResult<()> {
        self.conn()
            .execute(
                "INSERT INTO contexts (id) VALUES (?)",
                params![id.to_bytes().to_vec()],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }

    /// Get the latest context (by snowflake order)
    pub fn get_latest_context(&self) -> RiverResult<Option<Context>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, archived_at, token_count, summary, blob
                 FROM contexts ORDER BY id DESC LIMIT 1",
            )
            .map_err(|e| RiverError::database(e.to_string()))?;

        let mut rows = stmt
            .query([])
            .map_err(|e| RiverError::database(e.to_string()))?;

        match rows.next().map_err(|e| RiverError::database(e.to_string()))? {
            Some(row) => Ok(Some(Context::from_row(row).map_err(|e| RiverError::database(e.to_string()))?)),
            None => Ok(None),
        }
    }

    /// Archive a context (set blob, archived_at, token_count, summary)
    pub fn archive_context(
        &self,
        id: Snowflake,
        archived_at: Snowflake,
        token_count: i64,
        summary: Option<&str>,
        blob: &[u8],
    ) -> RiverResult<()> {
        self.conn()
            .execute(
                "UPDATE contexts SET archived_at = ?, token_count = ?, summary = ?, blob = ?
                 WHERE id = ?",
                params![
                    archived_at.to_bytes().to_vec(),
                    token_count,
                    summary,
                    blob,
                    id.to_bytes().to_vec(),
                ],
            )
            .map_err(|e| RiverError::database(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    fn test_generator() -> SnowflakeGenerator {
        let birth = AgentBirth::new(2026, 3, 18, 12, 0, 0).unwrap();
        SnowflakeGenerator::new(birth)
    }

    #[test]
    fn test_insert_and_get_context() {
        let db = Database::open_in_memory().unwrap();
        let gen = test_generator();
        let id = gen.next_id(SnowflakeType::Context);

        db.insert_context(id).unwrap();

        let ctx = db.get_latest_context().unwrap().unwrap();
        assert_eq!(ctx.id, id);
        assert!(ctx.is_active());
        assert!(ctx.archived_at.is_none());
        assert!(ctx.blob.is_none());
    }

    #[test]
    fn test_archive_context() {
        let db = Database::open_in_memory().unwrap();
        let gen = test_generator();
        let id = gen.next_id(SnowflakeType::Context);

        db.insert_context(id).unwrap();

        let archived_at = gen.next_id(SnowflakeType::Context);
        let blob = b"test blob content";
        db.archive_context(id, archived_at, 1000, Some("test summary"), blob).unwrap();

        let ctx = db.get_latest_context().unwrap().unwrap();
        assert!(!ctx.is_active());
        assert_eq!(ctx.archived_at, Some(archived_at));
        assert_eq!(ctx.token_count, Some(1000));
        assert_eq!(ctx.summary, Some("test summary".to_string()));
        assert_eq!(ctx.blob, Some(blob.to_vec()));
    }

    #[test]
    fn test_get_latest_returns_newest() {
        let db = Database::open_in_memory().unwrap();
        let gen = test_generator();

        let id1 = gen.next_id(SnowflakeType::Context);
        let id2 = gen.next_id(SnowflakeType::Context);

        db.insert_context(id1).unwrap();
        db.insert_context(id2).unwrap();

        let ctx = db.get_latest_context().unwrap().unwrap();
        assert_eq!(ctx.id, id2);
    }

    #[test]
    fn test_no_context_returns_none() {
        let db = Database::open_in_memory().unwrap();
        let ctx = db.get_latest_context().unwrap();
        assert!(ctx.is_none());
    }
}
```

- [ ] **Step 3: Add migration to schema.rs**

In `crates/river-gateway/src/db/schema.rs`, add the new migration call in `migrate()`:

```rust
fn migrate(&self) -> RiverResult<()> {
    self.conn
        .execute_batch(
            "
        CREATE TABLE IF NOT EXISTS migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            applied_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        ",
        )
        .map_err(|e| RiverError::database(e.to_string()))?;

    self.run_migration("001_messages", include_str!("migrations/001_messages.sql"))?;
    self.run_migration("002_memories", include_str!("migrations/002_memories.sql"))?;
    self.run_migration("003_contexts", include_str!("migrations/003_contexts.sql"))?;
    Ok(())
}
```

- [ ] **Step 4: Export contexts module in mod.rs**

In `crates/river-gateway/src/db/mod.rs`:

```rust
//! Database layer

mod schema;
mod messages;
mod memories;
mod contexts;

pub use schema::{Database, init_db};
pub use messages::{Message, MessageRole};
pub use memories::{Memory, f32_vec_to_bytes, bytes_to_f32_vec};
pub use contexts::Context;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway db::contexts`
Expected: All 4 context tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/db/migrations/003_contexts.sql \
        crates/river-gateway/src/db/contexts.rs \
        crates/river-gateway/src/db/schema.rs \
        crates/river-gateway/src/db/mod.rs
git commit -m "feat(gateway): add contexts table and Context struct"
```

---

### Task 3: Create ContextFile for JSONL Persistence

**Files:**
- Create: `crates/river-gateway/src/loop/persistence.rs`
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Create persistence.rs**

Create `crates/river-gateway/src/loop/persistence.rs`:

```rust
//! Context file persistence (JSONL format)

use crate::loop::ChatMessage;
use river_core::{RiverError, RiverResult};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// JSONL file for context persistence
pub struct ContextFile {
    path: PathBuf,
}

impl ContextFile {
    /// Create a new context file (overwrites if exists)
    pub fn create(workspace: &Path) -> RiverResult<Self> {
        let path = workspace.join("context.jsonl");

        // Create empty file
        File::create(&path).map_err(|e| RiverError::io(e.to_string()))?;

        Ok(Self { path })
    }

    /// Open an existing context file
    pub fn open(workspace: &Path) -> RiverResult<Self> {
        let path = workspace.join("context.jsonl");

        if !path.exists() {
            return Err(RiverError::io(format!("Context file not found: {:?}", path)));
        }

        Ok(Self { path })
    }

    /// Create with initial summary message
    pub fn create_with_summary(workspace: &Path, summary: &str) -> RiverResult<Self> {
        let file = Self::create(workspace)?;
        let msg = ChatMessage::system(format!("Previous context summary: {}", summary));
        file.append(&msg)?;
        Ok(file)
    }

    /// Check if context file exists in workspace
    pub fn exists(workspace: &Path) -> bool {
        workspace.join("context.jsonl").exists()
    }

    /// Delete context file if it exists
    pub fn delete(workspace: &Path) -> RiverResult<()> {
        let path = workspace.join("context.jsonl");
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| RiverError::io(e.to_string()))?;
        }
        Ok(())
    }

    /// Append a message to the file
    pub fn append(&self, message: &ChatMessage) -> RiverResult<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| RiverError::io(e.to_string()))?;

        let json = serde_json::to_string(message)
            .map_err(|e| RiverError::io(e.to_string()))?;

        writeln!(file, "{}", json).map_err(|e| RiverError::io(e.to_string()))?;

        Ok(())
    }

    /// Load all messages from the file
    pub fn load(&self) -> RiverResult<Vec<ChatMessage>> {
        let file = File::open(&self.path)
            .map_err(|e| RiverError::io(e.to_string()))?;

        let reader = BufReader::new(file);
        let mut messages = Vec::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| RiverError::io(e.to_string()))?;

            // Skip empty lines
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<ChatMessage>(&line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    // Log and skip corrupted lines (truncate incomplete trailing lines)
                    tracing::warn!(
                        line_num = line_num + 1,
                        error = %e,
                        "Skipping corrupted line in context file"
                    );
                }
            }
        }

        Ok(messages)
    }

    /// Read raw file contents as bytes (for archiving to DB)
    pub fn read_raw(&self) -> RiverResult<Vec<u8>> {
        std::fs::read(&self.path).map_err(|e| RiverError::io(e.to_string()))
    }

    /// Get the file path
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_and_append() {
        let tmp = TempDir::new().unwrap();
        let file = ContextFile::create(tmp.path()).unwrap();

        let msg = ChatMessage::user("Hello");
        file.append(&msg).unwrap();

        let messages = file.load().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, Some("Hello".to_string()));
    }

    #[test]
    fn test_multiple_appends() {
        let tmp = TempDir::new().unwrap();
        let file = ContextFile::create(tmp.path()).unwrap();

        file.append(&ChatMessage::user("First")).unwrap();
        file.append(&ChatMessage::assistant(Some("Second".to_string()), None)).unwrap();
        file.append(&ChatMessage::tool("call_1", "Third")).unwrap();

        let messages = file.load().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[2].role, "tool");
    }

    #[test]
    fn test_create_with_summary() {
        let tmp = TempDir::new().unwrap();
        let file = ContextFile::create_with_summary(tmp.path(), "Test summary").unwrap();

        let messages = file.load().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].content.as_ref().unwrap().contains("Test summary"));
    }

    #[test]
    fn test_exists_and_delete() {
        let tmp = TempDir::new().unwrap();

        assert!(!ContextFile::exists(tmp.path()));

        ContextFile::create(tmp.path()).unwrap();
        assert!(ContextFile::exists(tmp.path()));

        ContextFile::delete(tmp.path()).unwrap();
        assert!(!ContextFile::exists(tmp.path()));
    }

    #[test]
    fn test_open_existing() {
        let tmp = TempDir::new().unwrap();

        // Create and write
        let file = ContextFile::create(tmp.path()).unwrap();
        file.append(&ChatMessage::user("Test")).unwrap();

        // Open and read
        let file2 = ContextFile::open(tmp.path()).unwrap();
        let messages = file2.load().unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_open_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let result = ContextFile::open(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_corrupted_line_skipped() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("context.jsonl");

        // Write valid line, corrupted line, valid line
        std::fs::write(&path, r#"{"role":"user","content":"First"}
not valid json
{"role":"user","content":"Third"}
"#).unwrap();

        let file = ContextFile::open(tmp.path()).unwrap();
        let messages = file.load().unwrap();

        // Should have 2 messages (corrupted one skipped)
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, Some("First".to_string()));
        assert_eq!(messages[1].content, Some("Third".to_string()));
    }

    #[test]
    fn test_read_raw() {
        let tmp = TempDir::new().unwrap();
        let file = ContextFile::create(tmp.path()).unwrap();
        file.append(&ChatMessage::user("Test")).unwrap();

        let raw = file.read_raw().unwrap();
        assert!(!raw.is_empty());
        assert!(String::from_utf8_lossy(&raw).contains("Test"));
    }
}
```

- [ ] **Step 2: Export persistence module in loop/mod.rs**

Add to the top of `crates/river-gateway/src/loop/mod.rs`:

```rust
//! Agent loop module - the heart of the agent

pub mod state;
pub mod queue;
pub mod context;
pub mod model;
pub mod persistence;

pub use state::{LoopEvent, LoopState, WakeTrigger, ToolCallRequest, FunctionCall};
pub use queue::MessageQueue;
pub use context::{ChatMessage, ContextBuilder};
pub use model::{ModelClient, ModelResponse, Usage};
pub use persistence::ContextFile;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway loop::persistence`
Expected: All 8 persistence tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/loop/persistence.rs \
        crates/river-gateway/src/loop/mod.rs
git commit -m "feat(gateway): add ContextFile for JSONL persistence"
```

---

### Task 4: Update rotate_context Tool

**Files:**
- Modify: `crates/river-gateway/src/tools/scheduling.rs`

- [ ] **Step 1: Update ContextRotation to store summary**

Replace the `ContextRotation` struct and impl:

```rust
/// Shared state for context rotation requests
///
/// When rotation is requested, the loop will transition to settling/sleeping
/// after completing the current tool calls.
#[derive(Debug)]
pub struct ContextRotation {
    /// Whether rotation has been requested
    requested: AtomicBool,
    /// Summary for the rotation
    summary: RwLock<Option<String>>,
}

impl ContextRotation {
    pub fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            summary: RwLock::new(None),
        }
    }

    /// Request a context rotation with summary
    pub fn request(&self, summary: String) {
        self.requested.store(true, Ordering::SeqCst);
        if let Ok(mut s) = self.summary.try_write() {
            *s = Some(summary);
        }
    }

    /// Request auto-rotation (no summary)
    pub fn request_auto(&self) {
        self.requested.store(true, Ordering::SeqCst);
        if let Ok(mut s) = self.summary.try_write() {
            *s = None;
        }
    }

    /// Check if rotation is requested and take the summary
    /// Returns Some((has_summary, summary)) if rotation was requested
    pub fn take_request(&self) -> Option<Option<String>> {
        if self.requested.swap(false, Ordering::SeqCst) {
            self.summary
                .try_write()
                .ok()
                .map(|mut s| s.take())
        } else {
            None
        }
    }

    /// Check if rotation is pending (without clearing)
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}
```

- [ ] **Step 2: Update RotateContextTool parameters**

```rust
impl Tool for RotateContextTool {
    fn name(&self) -> &str {
        "rotate_context"
    }

    fn description(&self) -> &str {
        "Rotate context with a summary. The summary becomes a system message in the new context, preserving continuity."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Summary of current context to carry forward. This becomes a system message in the new context."
                }
            },
            "required": ["summary"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let summary = args["summary"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing required 'summary' parameter"))?
            .to_string();

        if summary.trim().is_empty() {
            return Err(RiverError::tool("Summary cannot be empty"));
        }

        self.rotation.request(summary);

        Ok(ToolResult::success(
            "Context rotation requested. Your summary will be preserved in the new context."
        ))
    }
}
```

- [ ] **Step 3: Update tests**

```rust
#[test]
fn test_context_rotation_with_summary() {
    let rotation = ContextRotation::new();
    rotation.request("Test summary".to_string());

    assert!(rotation.is_requested());

    let result = rotation.take_request();
    assert!(result.is_some());
    assert_eq!(result.unwrap(), Some("Test summary".to_string()));
    assert!(!rotation.is_requested());
}

#[test]
fn test_context_rotation_auto() {
    let rotation = ContextRotation::new();
    rotation.request_auto();

    assert!(rotation.is_requested());

    let result = rotation.take_request();
    assert!(result.is_some());
    assert_eq!(result.unwrap(), None);
}

#[test]
fn test_rotate_context_tool_requires_summary() {
    let rotation = Arc::new(ContextRotation::new());
    let tool = RotateContextTool::new(rotation.clone());

    // Missing summary should fail
    let result = tool.execute(serde_json::json!({}));
    assert!(result.is_err());

    // Empty summary should fail
    let result = tool.execute(serde_json::json!({"summary": ""}));
    assert!(result.is_err());

    // Valid summary should succeed
    let result = tool.execute(serde_json::json!({"summary": "Test summary"}));
    assert!(result.is_ok());
    assert!(rotation.is_requested());
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway tools::scheduling`
Expected: All scheduling tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/tools/scheduling.rs
git commit -m "feat(gateway): update rotate_context to require summary parameter"
```

---

### Task 5: Integrate Context Persistence into AgentLoop

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Add context persistence fields to AgentLoop**

Add new fields to `AgentLoop` struct:

```rust
pub struct AgentLoop {
    // ... existing fields ...

    /// Current context ID (snowflake)
    context_id: Option<Snowflake>,
    /// Context file for persistence
    context_file: Option<ContextFile>,
    /// Last known prompt token count
    last_prompt_tokens: u64,
}
```

- [ ] **Step 2: Update AgentLoop::new**

Add initialization of new fields and call startup logic:

```rust
impl AgentLoop {
    pub fn new(
        event_rx: mpsc::Receiver<LoopEvent>,
        message_queue: Arc<MessageQueue>,
        model_client: ModelClient,
        tool_executor: Arc<RwLock<ToolExecutor>>,
        db: Arc<Mutex<Database>>,
        snowflake_gen: Arc<SnowflakeGenerator>,
        config: LoopConfig,
    ) -> Self {
        let git = GitOps::new(&config.workspace);
        let heartbeat_scheduler = Arc::new(HeartbeatScheduler::new(config.default_heartbeat_minutes));
        let context_rotation = Arc::new(ContextRotation::new());

        Self {
            state: LoopState::Sleeping,
            event_rx,
            message_queue,
            model_client,
            context: ContextBuilder::new(),
            tool_executor,
            db,
            snowflake_gen,
            heartbeat_scheduler,
            context_rotation,
            shutdown_requested: false,
            git,
            config,
            pending_notifications: Vec::new(),
            needs_context_reset: true,
            context_id: None,
            context_file: None,
            last_prompt_tokens: 0,
        }
    }
```

- [ ] **Step 3: Add initialize_context method**

Add this method to handle startup context initialization:

```rust
/// Initialize context on startup - either resume or create fresh
fn initialize_context(&mut self) -> RiverResult<()> {
    let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
    let latest = db.get_latest_context()?;
    drop(db); // Release lock early

    match latest {
        Some(ctx) if ctx.is_active() => {
            // Resume existing active context
            if ContextFile::exists(&self.config.workspace) {
                tracing::info!(context_id = %ctx.id, "Resuming active context from file");
                self.context_id = Some(ctx.id);
                self.context_file = Some(ContextFile::open(&self.config.workspace)?);
            } else {
                // DB says active but file missing - create empty file, log warning
                tracing::warn!(context_id = %ctx.id, "Active context but file missing - creating empty");
                self.context_id = Some(ctx.id);
                self.context_file = Some(ContextFile::create(&self.config.workspace)?);
            }
        }
        _ => {
            // No context or latest is archived - create fresh
            self.create_fresh_context()?;
        }
    }

    // Clean up orphan files (file exists but no active context in DB)
    if self.context_id.is_none() && ContextFile::exists(&self.config.workspace) {
        tracing::warn!("Deleting orphan context file");
        ContextFile::delete(&self.config.workspace)?;
    }

    Ok(())
}

/// Create a fresh context with new ID and empty file
fn create_fresh_context(&mut self) -> RiverResult<()> {
    let id = self.snowflake_gen.next_id(SnowflakeType::Context);

    let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
    db.insert_context(id)?;
    drop(db);

    self.context_id = Some(id);
    self.context_file = Some(ContextFile::create(&self.config.workspace)?);

    tracing::info!(context_id = %id, "Created fresh context");
    Ok(())
}

/// Create fresh context with summary from rotation
fn create_context_with_summary(&mut self, summary: &str) -> RiverResult<()> {
    let id = self.snowflake_gen.next_id(SnowflakeType::Context);

    let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
    db.insert_context(id)?;
    drop(db);

    self.context_id = Some(id);
    self.context_file = Some(ContextFile::create_with_summary(&self.config.workspace, summary)?);

    tracing::info!(context_id = %id, "Created context with summary");
    Ok(())
}
```

- [ ] **Step 4: Add archive_current_context method**

```rust
/// Archive current context to database
fn archive_current_context(&mut self, summary: Option<&str>) -> RiverResult<()> {
    let context_id = self.context_id.ok_or_else(|| RiverError::internal("No active context"))?;
    let context_file = self.context_file.as_ref().ok_or_else(|| RiverError::internal("No context file"))?;

    let blob = context_file.read_raw()?;
    let archived_at = self.snowflake_gen.next_id(SnowflakeType::Context);

    let db = self.db.lock().map_err(|_| RiverError::database("Lock poisoned"))?;
    db.archive_context(
        context_id,
        archived_at,
        self.last_prompt_tokens as i64,
        summary,
        &blob,
    )?;
    drop(db);

    tracing::info!(
        context_id = %context_id,
        token_count = self.last_prompt_tokens,
        has_summary = summary.is_some(),
        "Archived context"
    );

    Ok(())
}
```

- [ ] **Step 5: Update run() to call initialize_context**

In the `run()` method, add initialization at the start:

```rust
pub async fn run(&mut self) {
    tracing::info!("Agent loop starting");

    // Initialize context persistence
    if let Err(e) = self.initialize_context() {
        tracing::error!(error = %e, "Failed to initialize context - continuing without persistence");
    }

    loop {
        // ... rest of run loop
    }
}
```

- [ ] **Step 6: Update think_phase to track tokens and inject warning**

After receiving model response, add token tracking and warning:

```rust
// In think_phase, after getting response:

// Track token count
self.last_prompt_tokens = response.usage.prompt_tokens;

// Check for 80% warning
let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
if context_percent >= 80.0 && context_percent < 90.0 {
    tracing::warn!(
        percent = format!("{:.1}", context_percent),
        "Context at 80%+ - warning will be injected"
    );
    // Warning injected later when building next context
}

// Check for 90% auto-rotation
if context_percent >= 90.0 {
    tracing::warn!(
        percent = format!("{:.1}", context_percent),
        "Context at 90%+ - triggering auto-rotation"
    );
    self.context_rotation.request_auto();
}
```

- [ ] **Step 7: Update think_phase to append messages to file**

After adding assistant response to context, also append to file:

```rust
// Add assistant message to context
self.context.add_assistant_response(
    response.content.clone(),
    if response.tool_calls.is_empty() {
        None
    } else {
        Some(response.tool_calls.clone())
    },
);

// Persist to file
if let Some(ref file) = self.context_file {
    let msg = ChatMessage::assistant(
        response.content.clone(),
        if response.tool_calls.is_empty() {
            None
        } else {
            Some(response.tool_calls.clone())
        },
    );
    if let Err(e) = file.append(&msg) {
        tracing::error!(error = %e, "Failed to append assistant message to context file");
    }
}
```

- [ ] **Step 8: Update settle_phase to handle rotation**

In `settle_phase`, handle rotation requests:

```rust
async fn settle_phase(&mut self) {
    tracing::debug!("Settling...");

    // Check for rotation request
    if let Some(summary) = self.context_rotation.take_request() {
        tracing::info!(has_summary = summary.is_some(), "Processing context rotation");

        // Archive current context
        if let Err(e) = self.archive_current_context(summary.as_deref()) {
            tracing::error!(error = %e, "Failed to archive context");
        } else {
            // Create new context
            let result = if let Some(ref s) = summary {
                self.create_context_with_summary(s)
            } else {
                tracing::warn!("Auto-rotation with no summary - continuity lost");
                self.create_fresh_context()
            };

            if let Err(e) = result {
                tracing::error!(error = %e, "Failed to create new context");
            }

            self.needs_context_reset = true;
            self.last_prompt_tokens = 0;
        }
    }

    // Persist conversation messages to database
    self.persist_messages();

    // ... rest of settle_phase
}
```

- [ ] **Step 9: Update wake_phase to load from file and inject warning**

In `wake_phase`, when `needs_context_reset` is true, load from file:

```rust
if self.needs_context_reset {
    tracing::info!("Building fresh context (first wake or post-rotation)");
    self.context.clear();

    // Load system prompt
    self.context.assemble(
        &self.config.workspace,
        trigger,
        queued_messages,
    ).await;

    // Load context from file if exists
    if let Some(ref file) = self.context_file {
        match file.load() {
            Ok(messages) => {
                for msg in messages {
                    self.context.add_message(msg);
                }
                tracing::info!(message_count = self.context.messages().len(), "Loaded context from file");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to load context file");
            }
        }
    }

    // Reset the executor's context tracking
    {
        let mut executor = self.tool_executor.write().await;
        executor.reset_context();
    }

    self.needs_context_reset = false;
}
```

Also inject 80% warning if needed (add after loading context):

```rust
// Inject 80% warning if needed
let context_percent = (self.last_prompt_tokens as f64 / self.config.context_limit as f64) * 100.0;
if context_percent >= 80.0 && context_percent < 90.0 {
    self.context.add_message(ChatMessage::system(format!(
        "WARNING: Context at {:.1}%. Consider summarizing and calling rotate_context soon.",
        context_percent
    )));
}
```

- [ ] **Step 10: Add use statements**

Add at top of file:

```rust
use river_core::SnowflakeType;
use crate::loop::persistence::ContextFile;
```

- [ ] **Step 11: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 12: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(gateway): integrate context persistence into agent loop"
```

---

### Task 6: Append User and Tool Messages to Context File

**Files:**
- Modify: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Update wake_phase to append trigger message**

When adding the wake trigger message, also append to file:

```rust
// In the non-reset path of wake_phase, after adding messages to context:
// Add wake trigger
match &trigger {
    WakeTrigger::Message(msg) => {
        let chat_msg = ChatMessage::user(format!(
            "[{}] {}: {}",
            msg.channel, msg.author.name, msg.content
        ));
        self.context.add_message(chat_msg.clone());

        // Persist to file
        if let Some(ref file) = self.context_file {
            if let Err(e) = file.append(&chat_msg) {
                tracing::error!(error = %e, "Failed to append user message to context file");
            }
        }
    }
    WakeTrigger::Heartbeat => {
        // Heartbeat is system message - don't persist
        self.context.add_message(ChatMessage::system(
            "Heartbeat wake. No new messages. Check on your tasks and state.".to_string()
        ));
    }
}
```

- [ ] **Step 2: Update act_phase to append tool results**

When adding tool results to context, also append to file:

```rust
// In act_phase, after adding tool result to context:
for result in &results {
    let content = match &result.result {
        Ok(r) => r.output.clone(),
        Err(e) => format!("Error: {}", e),
    };

    let chat_msg = ChatMessage::tool(result.tool_call_id.clone(), content);
    // Note: add_tool_results already adds to context, so just persist

    if let Some(ref file) = self.context_file {
        if let Err(e) = file.append(&chat_msg) {
            tracing::error!(error = %e, "Failed to append tool result to context file");
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/loop/mod.rs
git commit -m "feat(gateway): append all messages to context file"
```

---

### Task 7: Update Documentation

**Files:**
- Modify: `docs/snowflake-generation.md`

- [ ] **Step 1: Add Context type to snowflake documentation**

Add new section after Subagent Registration:

```markdown
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
```

- [ ] **Step 2: Update Types used section**

```markdown
**Types used:**
- `Message` (0x01) - Conversation messages
- `Embedding` (0x02) - Memory embeddings
- `Session` (0x03) - Sub-sessions
- `Subagent` (0x04) - Spawned subagents
- `ToolCall` (0x05) - Defined but not currently generated
- `Context` (0x06) - Context windows for persistence
```

- [ ] **Step 3: Commit**

```bash
git add docs/snowflake-generation.md
git commit -m "docs: add Context snowflake type to documentation"
```

---

### Task 8: Integration Testing

**Files:**
- Tests in existing test files

- [ ] **Step 1: Run full test suite**

```bash
cargo test --workspace
```

Expected: All tests pass

- [ ] **Step 2: Build in release mode**

```bash
cargo build --release -p river-gateway
```

Expected: Builds successfully

- [ ] **Step 3: Commit any test fixes if needed**

```bash
git add -A
git commit -m "test: fix any issues found during integration testing"
```

---

## Summary

This plan implements context persistence in 8 tasks:

1. **Add Context snowflake type** - New type 0x06
2. **Create DB migration and Context module** - Table and CRUD operations
3. **Create ContextFile** - JSONL read/write/append
4. **Update rotate_context tool** - Require summary parameter
5. **Integrate into AgentLoop** - Startup, tracking, rotation
6. **Append all messages** - User, assistant, tool messages
7. **Update documentation** - Snowflake docs
8. **Integration testing** - Full test suite
