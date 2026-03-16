# River Engine: Plan 2 - Gateway Core

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the gateway binary with tool loop, session management, database layer, and core file tools.

**Architecture:** The gateway is the heart of each agent. It runs the continuous tool loop, manages sessions, executes tools, and persists state to SQLite. This plan builds the foundation - later plans add communication adapters, memory system, and orchestrator integration.

**Tech Stack:** Rust, axum (HTTP), SQLite (rusqlite), tokio (async runtime), reqwest (HTTP client for model calls)

**Spec Reference:** `/home/cassie/river-engine/docs/superpowers/specs/2026-03-16-river-engine-design.md`

---

## File Structure

```
crates/
├── river-core/          # (existing) Shared types
└── river-gateway/       # (new) Gateway binary
    ├── Cargo.toml
    └── src/
        ├── main.rs           # Entry point, CLI args
        ├── lib.rs            # Library root
        ├── config.rs         # Gateway-specific config loading
        ├── server.rs         # Axum HTTP server setup
        ├── state.rs          # Shared application state
        ├── db/
        │   ├── mod.rs
        │   ├── schema.rs     # SQLite schema & migrations
        │   └── messages.rs   # Message CRUD operations
        ├── session/
        │   ├── mod.rs
        │   ├── manager.rs    # Session lifecycle
        │   ├── context.rs    # Context assembly
        │   └── state.rs      # Session state types
        ├── tools/
        │   ├── mod.rs
        │   ├── registry.rs   # Tool registration
        │   ├── executor.rs   # Tool execution
        │   ├── file.rs       # read, write, edit, glob, grep
        │   └── shell.rs      # bash tool
        ├── loop/
        │   ├── mod.rs
        │   └── runner.rs     # Tool loop implementation
        └── api/
            ├── mod.rs
            └── routes.rs     # HTTP endpoints
```

---

## Chunk 1: Gateway Crate Setup

### Task 1: Create Gateway Crate

**Files:**
- Create: `crates/river-gateway/Cargo.toml`
- Create: `crates/river-gateway/src/main.rs`
- Create: `crates/river-gateway/src/lib.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Update workspace Cargo.toml with new dependencies**

Add to `[workspace.dependencies]`:
```toml
tokio = { version = "1.0", features = ["full"] }
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace"] }
reqwest = { version = "0.12", features = ["json"] }
rusqlite = { version = "0.32", features = ["bundled"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4.0", features = ["derive"] }
```

- [ ] **Step 2: Create river-gateway Cargo.toml**

```toml
[package]
name = "river-gateway"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "River Engine gateway - agent runtime"

[[bin]]
name = "river-gateway"
path = "src/main.rs"

[dependencies]
river-core = { path = "../river-core" }
tokio.workspace = true
axum.workspace = true
tower.workspace = true
tower-http.workspace = true
reqwest.workspace = true
rusqlite.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile = "3.10"
```

- [ ] **Step 3: Create main.rs with CLI**

```rust
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-gateway")]
#[command(about = "River Engine Gateway - Agent Runtime")]
struct Args {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Workspace directory
    #[arg(short, long)]
    workspace: PathBuf,

    /// Data directory for database
    #[arg(short, long)]
    data_dir: PathBuf,

    /// Gateway port
    #[arg(short, long, default_value = "3000")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    tracing::info!("Starting River Gateway");
    tracing::info!("Workspace: {:?}", args.workspace);
    tracing::info!("Data dir: {:?}", args.data_dir);
    tracing::info!("Port: {}", args.port);

    // TODO: Initialize and run gateway
    Ok(())
}
```

- [ ] **Step 4: Create lib.rs**

```rust
//! River Gateway - Agent Runtime

pub mod config;
pub mod db;
pub mod session;
pub mod tools;
pub mod state;
pub mod server;
pub mod api;
pub mod r#loop;
```

- [ ] **Step 5: Create stub module files**

Create empty files for each module.

- [ ] **Step 6: Add anyhow to workspace dependencies**

```toml
anyhow = "1.0"
```

And add to river-gateway Cargo.toml dependencies.

- [ ] **Step 7: Verify it compiles**

Run: `nix-shell --run "cargo check -p river-gateway"`

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat(gateway): create gateway crate skeleton"
```

---

## Chunk 2: Database Layer

### Task 2: Database Schema

**Files:**
- Create: `crates/river-gateway/src/db/mod.rs`
- Create: `crates/river-gateway/src/db/schema.rs`

- [ ] **Step 1: Create db/mod.rs**

```rust
//! Database layer

mod schema;
mod messages;

pub use schema::{Database, init_db};
pub use messages::{Message, MessageRole};
```

- [ ] **Step 2: Create db/schema.rs with migrations**

```rust
use rusqlite::{Connection, Result};
use std::path::Path;

/// Database wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create database at path
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open in-memory database (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Run migrations
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                applied_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            "
        )?;

        self.run_migration("001_messages", include_str!("migrations/001_messages.sql"))?;
        Ok(())
    }

    fn run_migration(&self, name: &str, sql: &str) -> Result<()> {
        let applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = ?)",
            [name],
            |row| row.get(0),
        )?;

        if !applied {
            self.conn.execute_batch(sql)?;
            self.conn.execute(
                "INSERT INTO migrations (name) VALUES (?)",
                [name],
            )?;
            tracing::info!("Applied migration: {}", name);
        }
        Ok(())
    }

    /// Get connection reference
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Initialize database at path
pub fn init_db(path: &Path) -> Result<Database> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    Database::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.conn().is_autocommit());
    }

    #[test]
    fn test_migrations_idempotent() {
        let db = Database::open_in_memory().unwrap();
        // Running migrate again should not fail
        db.migrate().unwrap();
    }
}
```

- [ ] **Step 3: Create migrations directory and first migration**

Create `crates/river-gateway/src/db/migrations/001_messages.sql`:

```sql
-- Messages table
CREATE TABLE IF NOT EXISTS messages (
    id BLOB PRIMARY KEY,           -- 128-bit snowflake
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,            -- 'system', 'user', 'assistant', 'tool'
    content TEXT,
    tool_calls TEXT,               -- JSON array of tool calls
    tool_call_id TEXT,             -- For tool response messages
    name TEXT,                     -- Tool name for tool responses
    created_at INTEGER NOT NULL,
    metadata TEXT                  -- JSON
);

CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);

-- Sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_active INTEGER NOT NULL,
    context_tokens INTEGER DEFAULT 0,
    metadata TEXT                  -- JSON
);
```

- [ ] **Step 4: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway db::"`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(gateway): add database schema and migrations"
```

---

### Task 3: Message CRUD

**Files:**
- Create: `crates/river-gateway/src/db/messages.rs`

- [ ] **Step 1: Implement Message types and CRUD**

```rust
use river_core::Snowflake;
use rusqlite::{params, Result, Row};
use serde::{Deserialize, Serialize};

use super::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub session_id: String,
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls: Option<String>,  // JSON
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    pub created_at: i64,
    pub metadata: Option<String>,    // JSON
}

impl Message {
    fn from_row(row: &Row) -> Result<Self> {
        let id_bytes: Vec<u8> = row.get(0)?;
        let id = Snowflake::from_bytes(id_bytes.try_into().unwrap_or([0u8; 16]));

        Ok(Self {
            id,
            session_id: row.get(1)?,
            role: MessageRole::from_str(&row.get::<_, String>(2)?).unwrap_or(MessageRole::User),
            content: row.get(3)?,
            tool_calls: row.get(4)?,
            tool_call_id: row.get(5)?,
            name: row.get(6)?,
            created_at: row.get(7)?,
            metadata: row.get(8)?,
        })
    }
}

impl Database {
    /// Insert a message
    pub fn insert_message(&self, msg: &Message) -> Result<()> {
        self.conn().execute(
            "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            ],
        )?;
        Ok(())
    }

    /// Get messages for a session, ordered by creation time
    pub fn get_session_messages(&self, session_id: &str, limit: usize) -> Result<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata
             FROM messages
             WHERE session_id = ?
             ORDER BY created_at DESC
             LIMIT ?"
        )?;

        let messages = stmt.query_map(params![session_id, limit as i64], Message::from_row)?
            .collect::<Result<Vec<_>>>()?;

        // Reverse to get chronological order
        Ok(messages.into_iter().rev().collect())
    }

    /// Get recent messages across all sessions
    pub fn get_recent_messages(&self, limit: usize) -> Result<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata
             FROM messages
             ORDER BY created_at DESC
             LIMIT ?"
        )?;

        let messages = stmt.query_map(params![limit as i64], Message::from_row)?
            .collect::<Result<Vec<_>>>()?;

        Ok(messages.into_iter().rev().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};

    #[test]
    fn test_insert_and_get_message() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let msg = Message {
            id: gen.next_id(SnowflakeType::Message),
            session_id: "test-session".to_string(),
            role: MessageRole::User,
            content: Some("Hello, world!".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            created_at: 1234567890,
            metadata: None,
        };

        db.insert_message(&msg).unwrap();

        let messages = db.get_session_messages("test-session", 10).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, Some("Hello, world!".to_string()));
    }

    #[test]
    fn test_message_ordering() {
        let db = Database::open_in_memory().unwrap();
        let birth = AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        for i in 0..5 {
            let msg = Message {
                id: gen.next_id(SnowflakeType::Message),
                session_id: "test-session".to_string(),
                role: MessageRole::User,
                content: Some(format!("Message {}", i)),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                created_at: 1000 + i,
                metadata: None,
            };
            db.insert_message(&msg).unwrap();
        }

        let messages = db.get_session_messages("test-session", 10).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, Some("Message 0".to_string()));
        assert_eq!(messages[4].content, Some("Message 4".to_string()));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway db::"`

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(gateway): add message CRUD operations"
```

---

## Chunk 3: Tool System

### Task 4: Tool Registry

**Files:**
- Create: `crates/river-gateway/src/tools/mod.rs`
- Create: `crates/river-gateway/src/tools/registry.rs`

- [ ] **Step 1: Create tools/mod.rs**

```rust
//! Tool system

mod registry;
mod executor;
mod file;
mod shell;

pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::ToolExecutor;
```

- [ ] **Step 2: Create tools/registry.rs**

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub output_file: Option<String>,  // If output was redirected to file
}

/// JSON Schema for tool parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,  // JSON Schema object
}

/// Tool trait - implemented by each tool
pub trait Tool: Send + Sync {
    /// Tool name
    fn name(&self) -> &str;

    /// Tool description
    fn description(&self) -> &str;

    /// Parameter schema (JSON Schema)
    fn parameters(&self) -> Value;

    /// Execute the tool with given arguments
    fn execute(&self, args: Value) -> ToolResult;

    /// Get full schema for this tool
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }
}

/// Registry of available tools
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Get all tool schemas (for sending to model)
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// List tool names
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool;

    impl Tool for DummyTool {
        fn name(&self) -> &str { "dummy" }
        fn description(&self) -> &str { "A dummy tool for testing" }
        fn parameters(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": ["input"]
            })
        }
        fn execute(&self, args: Value) -> ToolResult {
            let input = args.get("input").and_then(|v| v.as_str()).unwrap_or("");
            ToolResult {
                success: true,
                output: format!("Received: {}", input),
                output_file: None,
            }
        }
    }

    #[test]
    fn test_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool));

        assert!(registry.get("dummy").is_some());
        assert!(registry.get("nonexistent").is_none());
        assert_eq!(registry.names().len(), 1);
    }

    #[test]
    fn test_tool_execution() {
        let tool = DummyTool;
        let result = tool.execute(serde_json::json!({"input": "hello"}));
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway tools::registry"`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): add tool registry"
```

---

### Task 5: File Tools

**Files:**
- Create: `crates/river-gateway/src/tools/file.rs`

- [ ] **Step 1: Implement file tools (read, write, edit, glob, grep)**

```rust
use super::{Tool, ToolResult};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Read file tool
pub struct ReadTool {
    workspace: std::path::PathBuf,
}

impl ReadTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(p)
        }
    }
}

impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }

    fn description(&self) -> &str { "Read file contents" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" },
                "offset": { "type": "integer", "description": "Line number to start from (optional)" },
                "limit": { "type": "integer", "description": "Maximum lines to read (optional)" },
                "output_file": { "type": "string", "description": "Pipe output to file instead of context (optional)" }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: Value) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => self.resolve_path(p),
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: path".to_string(),
                output_file: None,
            },
        };

        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64());
        let output_file = args.get("output_file").and_then(|v| v.as_str());

        match fs::read_to_string(&path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = offset.min(lines.len());
                let end = match limit {
                    Some(l) => (start + l as usize).min(lines.len()),
                    None => lines.len(),
                };

                let result: String = lines[start..end]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{:6}│ {}", start + i + 1, line))
                    .collect::<Vec<_>>()
                    .join("\n");

                if let Some(out_path) = output_file {
                    match fs::write(out_path, &result) {
                        Ok(_) => ToolResult {
                            success: true,
                            output: format!("Output written to {}", out_path),
                            output_file: Some(out_path.to_string()),
                        },
                        Err(e) => ToolResult {
                            success: false,
                            output: format!("Failed to write output file: {}", e),
                            output_file: None,
                        },
                    }
                } else {
                    ToolResult {
                        success: true,
                        output: result,
                        output_file: None,
                    }
                }
            }
            Err(e) => ToolResult {
                success: false,
                output: format!("Failed to read file: {}", e),
                output_file: None,
            },
        }
    }
}

/// Write file tool
pub struct WriteTool {
    workspace: std::path::PathBuf,
}

impl WriteTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(p)
        }
    }
}

impl Tool for WriteTool {
    fn name(&self) -> &str { "write" }

    fn description(&self) -> &str { "Write content to file (creates or overwrites)" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, args: Value) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => self.resolve_path(p),
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: path".to_string(),
                output_file: None,
            },
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: content".to_string(),
                output_file: None,
            },
        };

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return ToolResult {
                    success: false,
                    output: format!("Failed to create directories: {}", e),
                    output_file: None,
                };
            }
        }

        match fs::write(&path, content) {
            Ok(_) => ToolResult {
                success: true,
                output: format!("Wrote {} bytes to {:?}", content.len(), path),
                output_file: None,
            },
            Err(e) => ToolResult {
                success: false,
                output: format!("Failed to write file: {}", e),
                output_file: None,
            },
        }
    }
}

/// Edit file tool (surgical string replacement)
pub struct EditTool {
    workspace: std::path::PathBuf,
}

impl EditTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(p)
        }
    }
}

impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }

    fn description(&self) -> &str { "Replace text in file" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to edit" },
                "old_string": { "type": "string", "description": "Text to find" },
                "new_string": { "type": "string", "description": "Text to replace with" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences", "default": false }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn execute(&self, args: Value) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => self.resolve_path(p),
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: path".to_string(),
                output_file: None,
            },
        };

        let old_string = match args.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: old_string".to_string(),
                output_file: None,
            },
        };

        let new_string = match args.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: new_string".to_string(),
                output_file: None,
            },
        };

        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return ToolResult {
                success: false,
                output: format!("Failed to read file: {}", e),
                output_file: None,
            },
        };

        let occurrences = content.matches(old_string).count();
        if occurrences == 0 {
            return ToolResult {
                success: false,
                output: "old_string not found in file".to_string(),
                output_file: None,
            };
        }

        if !replace_all && occurrences > 1 {
            return ToolResult {
                success: false,
                output: format!("old_string found {} times - use replace_all or make it more specific", occurrences),
                output_file: None,
            };
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        match fs::write(&path, new_content) {
            Ok(_) => ToolResult {
                success: true,
                output: format!("Replaced {} occurrence(s) in {:?}", occurrences, path),
                output_file: None,
            },
            Err(e) => ToolResult {
                success: false,
                output: format!("Failed to write file: {}", e),
                output_file: None,
            },
        }
    }
}

/// Glob tool - find files by pattern
pub struct GlobTool {
    workspace: std::path::PathBuf,
}

impl GlobTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}

impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }

    fn description(&self) -> &str { "Find files matching pattern" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g., **/*.md)" },
                "path": { "type": "string", "description": "Base directory (optional)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> ToolResult {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: pattern".to_string(),
                output_file: None,
            },
        };

        let base = args.get("path")
            .and_then(|v| v.as_str())
            .map(|p| std::path::PathBuf::from(p))
            .unwrap_or_else(|| self.workspace.clone());

        let full_pattern = base.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        match glob::glob(&pattern_str) {
            Ok(paths) => {
                let files: Vec<String> = paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();

                ToolResult {
                    success: true,
                    output: if files.is_empty() {
                        "No files found".to_string()
                    } else {
                        files.join("\n")
                    },
                    output_file: None,
                }
            }
            Err(e) => ToolResult {
                success: false,
                output: format!("Invalid glob pattern: {}", e),
                output_file: None,
            },
        }
    }
}

/// Grep tool - search file contents
pub struct GrepTool {
    workspace: std::path::PathBuf,
}

impl GrepTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}

impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }

    fn description(&self) -> &str { "Search file contents with regex" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search" },
                "path": { "type": "string", "description": "File or directory to search" },
                "glob": { "type": "string", "description": "Filter files by glob pattern (optional)" },
                "context": { "type": "integer", "description": "Lines of context around matches (optional)" },
                "output_file": { "type": "string", "description": "Pipe output to file instead of context (optional)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> ToolResult {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: pattern".to_string(),
                output_file: None,
            },
        };

        let regex = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolResult {
                success: false,
                output: format!("Invalid regex: {}", e),
                output_file: None,
            },
        };

        let search_path = args.get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.workspace.clone());

        let _context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let output_file = args.get("output_file").and_then(|v| v.as_str());

        let mut results = Vec::new();

        // Simple implementation: if it's a file, search it; if directory, search all files
        if search_path.is_file() {
            if let Ok(content) = fs::read_to_string(&search_path) {
                for (i, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        results.push(format!("{}:{}: {}", search_path.display(), i + 1, line));
                    }
                }
            }
        } else if search_path.is_dir() {
            // Walk directory
            fn walk_dir(dir: &Path, regex: &regex::Regex, results: &mut Vec<String>) {
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.is_dir() {
                            if !path.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
                                walk_dir(&path, regex, results);
                            }
                        } else if path.is_file() {
                            if let Ok(content) = fs::read_to_string(&path) {
                                for (i, line) in content.lines().enumerate() {
                                    if regex.is_match(line) {
                                        results.push(format!("{}:{}: {}", path.display(), i + 1, line));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            walk_dir(&search_path, &regex, &mut results);
        }

        let output = if results.is_empty() {
            "No matches found".to_string()
        } else {
            results.join("\n")
        };

        if let Some(out_path) = output_file {
            match fs::write(out_path, &output) {
                Ok(_) => ToolResult {
                    success: true,
                    output: format!("Output written to {} ({} matches)", out_path, results.len()),
                    output_file: Some(out_path.to_string()),
                },
                Err(e) => ToolResult {
                    success: false,
                    output: format!("Failed to write output file: {}", e),
                    output_file: None,
                },
            }
        } else {
            ToolResult {
                success: true,
                output,
                output_file: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_write_edit() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        // Write
        let write_tool = WriteTool::new(&workspace);
        let result = write_tool.execute(json!({
            "path": "test.txt",
            "content": "Hello, world!"
        }));
        assert!(result.success);

        // Read
        let read_tool = ReadTool::new(&workspace);
        let result = read_tool.execute(json!({
            "path": "test.txt"
        }));
        assert!(result.success);
        assert!(result.output.contains("Hello, world!"));

        // Edit
        let edit_tool = EditTool::new(&workspace);
        let result = edit_tool.execute(json!({
            "path": "test.txt",
            "old_string": "world",
            "new_string": "River"
        }));
        assert!(result.success);

        // Verify edit
        let result = read_tool.execute(json!({"path": "test.txt"}));
        assert!(result.output.contains("Hello, River!"));
    }

    #[test]
    fn test_glob() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        // Create some files
        fs::write(workspace.join("a.txt"), "a").unwrap();
        fs::write(workspace.join("b.txt"), "b").unwrap();
        fs::create_dir(workspace.join("sub")).unwrap();
        fs::write(workspace.join("sub/c.txt"), "c").unwrap();

        let glob_tool = GlobTool::new(&workspace);
        let result = glob_tool.execute(json!({
            "pattern": "**/*.txt"
        }));
        assert!(result.success);
        assert!(result.output.contains("a.txt"));
        assert!(result.output.contains("b.txt"));
        assert!(result.output.contains("c.txt"));
    }
}
```

- [ ] **Step 2: Add glob and regex to dependencies**

Add to workspace Cargo.toml:
```toml
glob = "0.3"
regex = "1.10"
```

Add to river-gateway Cargo.toml:
```toml
glob.workspace = true
regex.workspace = true
```

- [ ] **Step 3: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway tools::file"`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): add file tools (read, write, edit, glob, grep)"
```

---

### Task 6: Shell Tool

**Files:**
- Create: `crates/river-gateway/src/tools/shell.rs`

- [ ] **Step 1: Implement bash tool**

```rust
use super::{Tool, ToolResult};
use serde_json::{json, Value};
use std::process::Command;
use std::time::Duration;

/// Bash command execution tool
pub struct BashTool {
    workspace: std::path::PathBuf,
    timeout: Duration,
}

impl BashTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            timeout: Duration::from_secs(120),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str { "Execute shell command" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Command to execute" },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds (optional)" },
                "output_file": { "type": "string", "description": "Pipe output to file instead of context (optional)" }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, args: Value) -> ToolResult {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult {
                success: false,
                output: "Missing required parameter: command".to_string(),
                output_file: None,
            },
        };

        let output_file = args.get("output_file").and_then(|v| v.as_str());

        let output = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&self.workspace)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    format!("stderr:\n{}", stderr)
                } else {
                    format!("{}\n\nstderr:\n{}", stdout, stderr)
                };

                let success = output.status.success();

                if let Some(out_path) = output_file {
                    match std::fs::write(out_path, &combined) {
                        Ok(_) => ToolResult {
                            success,
                            output: format!("Output written to {} (exit code: {})",
                                out_path, output.status.code().unwrap_or(-1)),
                            output_file: Some(out_path.to_string()),
                        },
                        Err(e) => ToolResult {
                            success: false,
                            output: format!("Failed to write output file: {}", e),
                            output_file: None,
                        },
                    }
                } else {
                    ToolResult {
                        success,
                        output: if combined.is_empty() {
                            format!("(exit code: {})", output.status.code().unwrap_or(-1))
                        } else {
                            combined
                        },
                        output_file: None,
                    }
                }
            }
            Err(e) => ToolResult {
                success: false,
                output: format!("Failed to execute command: {}", e),
                output_file: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_bash_echo() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "echo hello"
        }));

        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[test]
    fn test_bash_failure() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "exit 1"
        }));

        assert!(!result.success);
    }

    #[test]
    fn test_bash_working_dir() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "pwd"
        }));

        assert!(result.success);
        assert!(result.output.contains(&dir.path().to_string_lossy().to_string()));
    }
}
```

- [ ] **Step 2: Update tools/mod.rs to export file and shell tools**

```rust
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
```

- [ ] **Step 3: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway tools::"`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): add bash shell tool"
```

---

### Task 7: Tool Executor

**Files:**
- Create: `crates/river-gateway/src/tools/executor.rs`

- [ ] **Step 1: Implement tool executor**

```rust
use super::{Tool, ToolRegistry, ToolResult, ToolSchema};
use river_core::ContextStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool call from the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Result of executing a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub tool_call_id: String,
    pub tool_result: ToolResult,
    pub context_status: ContextStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incoming_messages: Vec<Value>,  // Messages that arrived during execution
}

/// Executes tools and tracks context
pub struct ToolExecutor {
    registry: ToolRegistry,
    context_used: u64,
    context_limit: u64,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry, context_limit: u64) -> Self {
        Self {
            registry,
            context_used: 0,
            context_limit,
        }
    }

    /// Execute a tool call
    pub fn execute(&mut self, call: &ToolCall) -> ToolCallResponse {
        let result = match self.registry.get(&call.name) {
            Some(tool) => tool.execute(call.arguments.clone()),
            None => ToolResult {
                success: false,
                output: format!("Unknown tool: {}", call.name),
                output_file: None,
            },
        };

        // Update context tracking (rough estimate)
        self.context_used += result.output.len() as u64 / 4;  // ~4 chars per token

        ToolCallResponse {
            tool_call_id: call.id.clone(),
            tool_result: result,
            context_status: self.context_status(),
            incoming_messages: Vec::new(),
        }
    }

    /// Execute multiple tool calls
    pub fn execute_all(&mut self, calls: &[ToolCall]) -> Vec<ToolCallResponse> {
        calls.iter().map(|c| self.execute(c)).collect()
    }

    /// Get current context status
    pub fn context_status(&self) -> ContextStatus {
        ContextStatus {
            used: self.context_used,
            limit: self.context_limit,
        }
    }

    /// Get all tool schemas
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.registry.schemas()
    }

    /// Update context usage
    pub fn add_context(&mut self, tokens: u64) {
        self.context_used += tokens;
    }

    /// Reset context (on rotation)
    pub fn reset_context(&mut self) {
        self.context_used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ReadTool, WriteTool};
    use tempfile::TempDir;

    #[test]
    fn test_executor() {
        let dir = TempDir::new().unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ReadTool::new(dir.path())));
        registry.register(Box::new(WriteTool::new(dir.path())));

        let mut executor = ToolExecutor::new(registry, 65536);

        // Write a file
        let write_call = ToolCall {
            id: "call_1".to_string(),
            name: "write".to_string(),
            arguments: serde_json::json!({
                "path": "test.txt",
                "content": "Hello!"
            }),
        };

        let response = executor.execute(&write_call);
        assert!(response.tool_result.success);

        // Read it back
        let read_call = ToolCall {
            id: "call_2".to_string(),
            name: "read".to_string(),
            arguments: serde_json::json!({
                "path": "test.txt"
            }),
        };

        let response = executor.execute(&read_call);
        assert!(response.tool_result.success);
        assert!(response.tool_result.output.contains("Hello!"));
    }

    #[test]
    fn test_context_tracking() {
        let registry = ToolRegistry::new();
        let mut executor = ToolExecutor::new(registry, 1000);

        executor.add_context(500);
        assert_eq!(executor.context_status().used, 500);
        assert_eq!(executor.context_status().percent(), 50.0);

        executor.reset_context();
        assert_eq!(executor.context_status().used, 0);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway tools::executor"`

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(gateway): add tool executor with context tracking"
```

---

## Chunk 4: HTTP Server

### Task 8: Application State

**Files:**
- Create: `crates/river-gateway/src/state.rs`

- [ ] **Step 1: Implement shared state**

```rust
use crate::db::Database;
use crate::tools::{ToolExecutor, ToolRegistry};
use river_core::{AgentBirth, SnowflakeGenerator};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

/// Shared application state
pub struct AppState {
    pub config: GatewayConfig,
    pub db: Arc<Mutex<Database>>,
    pub snowflake_gen: Arc<SnowflakeGenerator>,
    pub tool_executor: Arc<RwLock<ToolExecutor>>,
}

/// Gateway configuration (runtime)
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub model_url: String,
    pub model_name: String,
    pub context_limit: u64,
    pub heartbeat_minutes: u32,
    pub agent_birth: AgentBirth,
}

impl AppState {
    pub fn new(config: GatewayConfig, db: Database, registry: ToolRegistry) -> Self {
        let executor = ToolExecutor::new(registry, config.context_limit);

        Self {
            snowflake_gen: Arc::new(SnowflakeGenerator::new(config.agent_birth)),
            db: Arc::new(Mutex::new(db)),
            tool_executor: Arc::new(RwLock::new(executor)),
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::tools::ToolRegistry;

    #[test]
    fn test_state_creation() {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
        };

        let db = Database::open_in_memory().unwrap();
        let registry = ToolRegistry::new();
        let _state = AppState::new(config, db, registry);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway state::"`

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(gateway): add application state"
```

---

### Task 9: HTTP Routes

**Files:**
- Create: `crates/river-gateway/src/api/mod.rs`
- Create: `crates/river-gateway/src/api/routes.rs`

- [ ] **Step 1: Create api/mod.rs**

```rust
//! HTTP API

mod routes;

pub use routes::create_router;
```

- [ ] **Step 2: Create api/routes.rs**

```rust
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// Incoming message request
#[derive(Deserialize)]
pub struct IncomingMessage {
    pub adapter: String,
    pub event_type: String,
    pub channel: String,
    pub author: Author,
    pub content: String,
    pub message_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
}

/// Create the router with all routes
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/incoming", post(handle_incoming))
        .route("/tools", get(list_tools))
        .route("/context/status", get(context_status))
        .with_state(state)
}

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn handle_incoming(
    State(state): State<Arc<AppState>>,
    Json(msg): Json<IncomingMessage>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        "Received message from {} in {}: {}",
        msg.author.name,
        msg.channel,
        msg.content
    );

    // TODO: Queue message and trigger tool loop
    Ok(Json(serde_json::json!({
        "status": "queued",
        "channel": msg.channel
    })))
}

async fn list_tools(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::tools::ToolSchema>> {
    let executor = state.tool_executor.read().await;
    Json(executor.schemas())
}

async fn context_status(
    State(state): State<Arc<AppState>>,
) -> Json<river_core::ContextStatus> {
    let executor = state.tool_executor.read().await;
    Json(executor.context_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::state::GatewayConfig;
    use crate::tools::ToolRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use river_core::AgentBirth;
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let config = GatewayConfig {
            workspace: PathBuf::from("/tmp/test"),
            data_dir: PathBuf::from("/tmp/test"),
            port: 3000,
            model_url: "http://localhost:8080".to_string(),
            model_name: "test".to_string(),
            context_limit: 65536,
            heartbeat_minutes: 45,
            agent_birth: AgentBirth::new(2026, 3, 16, 12, 0, 0).unwrap(),
        };

        let db = Database::open_in_memory().unwrap();
        let registry = ToolRegistry::new();
        Arc::new(AppState::new(config, db, registry))
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `nix-shell --run "cargo test -p river-gateway api::"`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): add HTTP routes"
```

---

### Task 10: Server Setup

**Files:**
- Create: `crates/river-gateway/src/server.rs`
- Modify: `crates/river-gateway/src/main.rs`

- [ ] **Step 1: Create server.rs**

```rust
use crate::api::create_router;
use crate::db::{init_db, Database};
use crate::state::{AppState, GatewayConfig};
use crate::tools::{
    BashTool, EditTool, GlobTool, GrepTool, ReadTool, ToolRegistry, WriteTool,
};
use river_core::AgentBirth;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// Server configuration from CLI args
pub struct ServerConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub model_url: Option<String>,
    pub model_name: Option<String>,
}

/// Initialize and run the gateway server
pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // Initialize database
    let db_path = config.data_dir.join("river.db");
    let db = init_db(&db_path)?;

    // Create tool registry with core tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(&config.workspace)));
    registry.register(Box::new(WriteTool::new(&config.workspace)));
    registry.register(Box::new(EditTool::new(&config.workspace)));
    registry.register(Box::new(GlobTool::new(&config.workspace)));
    registry.register(Box::new(GrepTool::new(&config.workspace)));
    registry.register(Box::new(BashTool::new(&config.workspace)));

    tracing::info!("Registered {} tools", registry.names().len());

    // Create agent birth (current time)
    let now = chrono::Utc::now();
    let agent_birth = AgentBirth::new(
        now.year() as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )?;

    // Create gateway config
    let gateway_config = GatewayConfig {
        workspace: config.workspace,
        data_dir: config.data_dir,
        port: config.port,
        model_url: config.model_url.unwrap_or_else(|| "http://localhost:8080".to_string()),
        model_name: config.model_name.unwrap_or_else(|| "default".to_string()),
        context_limit: 65536,
        heartbeat_minutes: 45,
        agent_birth,
    };

    // Create app state
    let state = Arc::new(AppState::new(gateway_config, db, registry));

    // Create router
    let app = create_router(state);

    // Bind and serve
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 2: Add chrono to dependencies**

Add to workspace:
```toml
chrono = "0.4"
```

Add to river-gateway:
```toml
chrono.workspace = true
```

- [ ] **Step 3: Update main.rs**

```rust
use clap::Parser;
use river_gateway::server::{run, ServerConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-gateway")]
#[command(about = "River Engine Gateway - Agent Runtime")]
struct Args {
    /// Workspace directory
    #[arg(short, long)]
    workspace: PathBuf,

    /// Data directory for database
    #[arg(short, long)]
    data_dir: PathBuf,

    /// Gateway port
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Model server URL
    #[arg(long)]
    model_url: Option<String>,

    /// Model name
    #[arg(long)]
    model_name: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Gateway");
    tracing::info!("Workspace: {:?}", args.workspace);
    tracing::info!("Data dir: {:?}", args.data_dir);
    tracing::info!("Port: {}", args.port);

    let config = ServerConfig {
        workspace: args.workspace,
        data_dir: args.data_dir,
        port: args.port,
        model_url: args.model_url,
        model_name: args.model_name,
    };

    run(config).await
}
```

- [ ] **Step 4: Build and verify**

Run: `nix-shell --run "cargo build -p river-gateway"`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(gateway): add server setup and main entry point"
```

---

## Chunk 5: Stub Modules

### Task 11: Create Stub Modules

Create minimal stubs for remaining modules (to be implemented in later plans).

**Files:**
- Create: `crates/river-gateway/src/config.rs`
- Create: `crates/river-gateway/src/session/mod.rs`
- Create: `crates/river-gateway/src/loop/mod.rs`

- [ ] **Step 1: Create config.rs stub**

```rust
//! Gateway configuration loading
//!
//! TODO: Implement config file loading in later plan

use std::path::Path;
use anyhow::Result;

/// Load configuration from file
pub fn load_config(_path: &Path) -> Result<()> {
    // Placeholder for config file loading
    Ok(())
}
```

- [ ] **Step 2: Create session/mod.rs stub**

```rust
//! Session management
//!
//! TODO: Implement session manager in later plan

pub mod manager;
pub mod context;
pub mod state;
```

Create stub files for each:

`session/manager.rs`:
```rust
//! Session lifecycle management
```

`session/context.rs`:
```rust
//! Context assembly for model calls
```

`session/state.rs`:
```rust
//! Session state types
```

- [ ] **Step 3: Create loop/mod.rs stub**

```rust
//! Tool loop implementation
//!
//! TODO: Implement tool loop in later plan

pub mod runner;
```

`loop/runner.rs`:
```rust
//! Tool loop runner
```

- [ ] **Step 4: Verify everything compiles**

Run: `nix-shell --run "cargo build -p river-gateway"`

- [ ] **Step 5: Run all tests**

Run: `nix-shell --run "cargo test -p river-gateway"`

- [ ] **Step 6: Final commit**

```bash
git add -A && git commit -m "feat(gateway): add stub modules for session and loop"
```

---

## Final Verification

- [ ] **Run full test suite**

Run: `nix-shell --run "cargo test"`

- [ ] **Build release**

Run: `nix-shell --run "cargo build --release -p river-gateway"`

- [ ] **Test binary runs**

Run: `nix-shell --run "./target/release/river-gateway --help"`

---

## Summary

This plan implements:

| Component | Description |
|-----------|-------------|
| Gateway crate | Binary and library structure |
| Database layer | SQLite with migrations, message CRUD |
| Tool registry | Registration and schema system |
| File tools | read, write, edit, glob, grep |
| Shell tool | bash command execution |
| Tool executor | Execution with context tracking |
| HTTP server | Health, incoming messages, tool listing |
| Stubs | Session, loop (for later plans) |

**Next plan:** Plan 3 - Memory & Embeddings (semantic search, auto-embedding, Redis integration)
