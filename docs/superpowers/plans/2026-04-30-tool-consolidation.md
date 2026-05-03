# Tool Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the `river-tools` crate and consolidate all tool code into `river-gateway/src/tools/`, reorganizing by concern.

**Architecture:** Move 8 source files from `river-tools` into `river-gateway/src/tools/`. Split `scheduling.rs` into `context.rs` + `heartbeat.rs`. Split `communication.rs` into `adapters.rs` + `communication.rs` + move `ContextStatusTool` to `context.rs`. Update `mod.rs` to declare all submodules. Delete `crates/river-tools/`.

**Tech Stack:** Rust, Cargo workspaces

**Spec:** `docs/superpowers/specs/2026-04-30-tool-consolidation-design.md`

---

### Task 1: Move river-tools source files into gateway

**Files:**
- Copy: `crates/river-tools/src/registry.rs` → `crates/river-gateway/src/tools/registry.rs`
- Copy: `crates/river-tools/src/executor.rs` → `crates/river-gateway/src/tools/executor.rs`
- Copy: `crates/river-tools/src/file.rs` → `crates/river-gateway/src/tools/file.rs`
- Copy: `crates/river-tools/src/shell.rs` → `crates/river-gateway/src/tools/shell.rs`
- Copy: `crates/river-tools/src/web.rs` → `crates/river-gateway/src/tools/web.rs`
- Copy: `crates/river-tools/src/model.rs` → `crates/river-gateway/src/tools/model.rs`
- Copy: `crates/river-tools/src/logging.rs` → `crates/river-gateway/src/tools/logging.rs`

- [ ] **Step 1: Copy files**

```bash
cd ~/river-engine
cp crates/river-tools/src/registry.rs crates/river-gateway/src/tools/registry.rs
cp crates/river-tools/src/executor.rs crates/river-gateway/src/tools/executor.rs
cp crates/river-tools/src/file.rs crates/river-gateway/src/tools/file.rs
cp crates/river-tools/src/shell.rs crates/river-gateway/src/tools/shell.rs
cp crates/river-tools/src/web.rs crates/river-gateway/src/tools/web.rs
cp crates/river-tools/src/model.rs crates/river-gateway/src/tools/model.rs
cp crates/river-tools/src/logging.rs crates/river-gateway/src/tools/logging.rs
```

- [ ] **Step 2: Fix imports in copied files**

All copied files use `use crate::registry::{Tool, ToolResult}` (which was `crate` within `river-tools`). These need to become `use super::registry::{Tool, ToolResult}` or `use crate::tools::registry::{Tool, ToolResult}`.

In each copied file, replace:
```
use crate::registry::{Tool, ToolResult};
```
with:
```
use super::registry::{Tool, ToolResult};
```

Affected files: `executor.rs`, `file.rs`, `shell.rs`, `web.rs`, `model.rs`, `logging.rs`.

Also in `executor.rs`, replace:
```
use crate::registry::{ToolRegistry, ToolResult, ToolSchema};
```
with:
```
use super::registry::{ToolRegistry, ToolResult, ToolSchema};
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/tools/registry.rs crates/river-gateway/src/tools/executor.rs crates/river-gateway/src/tools/file.rs crates/river-gateway/src/tools/shell.rs crates/river-gateway/src/tools/web.rs crates/river-gateway/src/tools/model.rs crates/river-gateway/src/tools/logging.rs
git commit -m "copy: move river-tools source files into river-gateway/src/tools/"
```

---

### Task 2: Split scheduling.rs into context.rs and heartbeat.rs

**Files:**
- Create: `crates/river-gateway/src/tools/context.rs`
- Create: `crates/river-gateway/src/tools/heartbeat.rs`
- Source: `crates/river-tools/src/scheduling.rs` (do NOT copy this file directly)

- [ ] **Step 1: Create heartbeat.rs**

Write `crates/river-gateway/src/tools/heartbeat.rs` with `HeartbeatScheduler` and `ScheduleHeartbeatTool` from lines 1-133 and tests 250-312 of `scheduling.rs`.

```rust
//! Heartbeat scheduling — controls when the agent wakes next

use super::registry::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Shared state for heartbeat scheduling
///
/// The value represents minutes until next heartbeat.
/// A value of 0 means use the default from config.
#[derive(Debug)]
pub struct HeartbeatScheduler {
    /// Scheduled minutes (0 = use default)
    scheduled_minutes: AtomicU64,
    /// Default minutes from config
    default_minutes: u64,
}

impl HeartbeatScheduler {
    pub fn new(default_minutes: u32) -> Self {
        Self {
            scheduled_minutes: AtomicU64::new(0),
            default_minutes: default_minutes as u64,
        }
    }

    pub fn schedule(&self, minutes: u64) {
        self.scheduled_minutes.store(minutes, Ordering::SeqCst);
    }

    pub fn take_delay(&self) -> Duration {
        let scheduled = self.scheduled_minutes.swap(0, Ordering::SeqCst);
        let minutes = if scheduled > 0 {
            scheduled
        } else {
            self.default_minutes
        };
        Duration::from_secs(minutes * 60)
    }

    pub fn is_scheduled(&self) -> bool {
        self.scheduled_minutes.load(Ordering::SeqCst) > 0
    }

    pub fn scheduled_minutes(&self) -> u64 {
        self.scheduled_minutes.load(Ordering::SeqCst)
    }

    pub fn default_minutes(&self) -> u64 {
        self.default_minutes
    }
}

/// Schedule the next heartbeat wake time
pub struct ScheduleHeartbeatTool {
    scheduler: Arc<HeartbeatScheduler>,
}

impl ScheduleHeartbeatTool {
    pub fn new(scheduler: Arc<HeartbeatScheduler>) -> Self {
        Self { scheduler }
    }
}

impl Tool for ScheduleHeartbeatTool {
    fn name(&self) -> &str {
        "schedule_heartbeat"
    }

    fn description(&self) -> &str {
        "Set next heartbeat wake time"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "minutes": {
                    "type": "integer",
                    "description": "Minutes until next heartbeat (1-1440)",
                    "minimum": 1,
                    "maximum": 1440
                }
            },
            "required": ["minutes"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let minutes = args["minutes"]
            .as_u64()
            .ok_or_else(|| RiverError::tool("Missing 'minutes' parameter"))?;

        if minutes < 1 {
            return Err(RiverError::tool("Minutes must be at least 1"));
        }
        if minutes > 1440 {
            return Err(RiverError::tool("Minutes cannot exceed 1440 (24 hours)"));
        }

        self.scheduler.schedule(minutes);

        let output = if minutes < self.scheduler.default_minutes() {
            format!(
                "Next heartbeat scheduled in {} minutes (sooner than default {})",
                minutes,
                self.scheduler.default_minutes()
            )
        } else if minutes > self.scheduler.default_minutes() {
            format!(
                "Next heartbeat scheduled in {} minutes (later than default {})",
                minutes,
                self.scheduler.default_minutes()
            )
        } else {
            format!("Next heartbeat scheduled in {} minutes (default)", minutes)
        };

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_scheduler_default() {
        let scheduler = HeartbeatScheduler::new(45);
        assert_eq!(scheduler.default_minutes(), 45);
        assert!(!scheduler.is_scheduled());
    }

    #[test]
    fn test_heartbeat_scheduler_schedule() {
        let scheduler = HeartbeatScheduler::new(45);
        scheduler.schedule(10);
        assert!(scheduler.is_scheduled());
        assert_eq!(scheduler.scheduled_minutes(), 10);
    }

    #[test]
    fn test_heartbeat_scheduler_take_delay() {
        let scheduler = HeartbeatScheduler::new(45);

        let delay = scheduler.take_delay();
        assert_eq!(delay, Duration::from_secs(45 * 60));

        scheduler.schedule(10);
        let delay = scheduler.take_delay();
        assert_eq!(delay, Duration::from_secs(10 * 60));

        assert!(!scheduler.is_scheduled());
        let delay = scheduler.take_delay();
        assert_eq!(delay, Duration::from_secs(45 * 60));
    }

    #[test]
    fn test_schedule_heartbeat_tool() {
        let scheduler = Arc::new(HeartbeatScheduler::new(45));
        let tool = ScheduleHeartbeatTool::new(scheduler.clone());

        assert_eq!(tool.name(), "schedule_heartbeat");

        let result = tool.execute(serde_json::json!({"minutes": 10}));
        assert!(result.is_ok());
        assert_eq!(scheduler.scheduled_minutes(), 10);
    }

    #[test]
    fn test_schedule_heartbeat_validation() {
        let scheduler = Arc::new(HeartbeatScheduler::new(45));
        let tool = ScheduleHeartbeatTool::new(scheduler);

        let result = tool.execute(serde_json::json!({"minutes": 0}));
        assert!(result.is_err());

        let result = tool.execute(serde_json::json!({"minutes": 2000}));
        assert!(result.is_err());

        let result = tool.execute(serde_json::json!({}));
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Create context.rs**

Write `crates/river-gateway/src/tools/context.rs` with `ContextRotation`, `RotateContextTool` from lines 135-244 and tests 314-368 of `scheduling.rs`, plus `ContextStatusTool` from lines 360-412 and its test at line 996 of `communication.rs`.

```rust
//! Context management — rotation and status

use super::registry::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Shared state for context rotation requests
#[derive(Debug)]
pub struct ContextRotation {
    requested: AtomicBool,
    summary: RwLock<Option<String>>,
}

impl ContextRotation {
    pub fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            summary: RwLock::new(None),
        }
    }

    pub fn request(&self, summary: String) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
        *s = Some(summary);
    }

    pub fn request_auto(&self) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
        *s = None;
    }

    pub fn take_request(&self) -> Option<Option<String>> {
        if self.requested.swap(false, Ordering::SeqCst) {
            let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
            Some(s.take())
        } else {
            None
        }
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

impl Default for ContextRotation {
    fn default() -> Self {
        Self::new()
    }
}

/// Manually trigger context rotation
pub struct RotateContextTool {
    rotation: Arc<ContextRotation>,
}

impl RotateContextTool {
    pub fn new(rotation: Arc<ContextRotation>) -> Self {
        Self { rotation }
    }
}

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

/// Get current context window usage
pub struct ContextStatusTool {
    context_limit: u64,
    context_used: Arc<AtomicU64>,
}

impl ContextStatusTool {
    pub fn new(context_limit: u64, context_used: Arc<AtomicU64>) -> Self {
        Self {
            context_limit,
            context_used,
        }
    }
}

impl Tool for ContextStatusTool {
    fn name(&self) -> &str {
        "context_status"
    }

    fn description(&self) -> &str {
        "Get current context window usage"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(&self, _args: Value) -> Result<ToolResult, RiverError> {
        let used = self.context_used.load(Ordering::Relaxed);
        let limit = self.context_limit;
        let percent = if limit > 0 {
            (used as f64 / limit as f64) * 100.0
        } else {
            0.0
        };
        let remaining = limit.saturating_sub(used);

        let output = serde_json::json!({
            "used": used,
            "limit": limit,
            "remaining": remaining,
            "percent": format!("{:.1}%", percent),
            "near_limit": percent >= 90.0
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&output).unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!rotation.is_requested());
    }

    #[test]
    fn test_context_rotation_not_requested() {
        let rotation = ContextRotation::new();
        let result = rotation.take_request();
        assert!(result.is_none());
    }

    #[test]
    fn test_rotate_context_tool_requires_summary() {
        let rotation = Arc::new(ContextRotation::new());
        let tool = RotateContextTool::new(rotation.clone());

        let result = tool.execute(serde_json::json!({}));
        assert!(result.is_err());

        let result = tool.execute(serde_json::json!({"summary": ""}));
        assert!(result.is_err());

        let result = tool.execute(serde_json::json!({"summary": "   "}));
        assert!(result.is_err());

        let result = tool.execute(serde_json::json!({"summary": "Test summary"}));
        assert!(result.is_ok());
        assert!(rotation.is_requested());
    }

    #[test]
    fn test_context_status_tool() {
        let context_used = Arc::new(AtomicU64::new(5000));
        let tool = ContextStatusTool::new(10000, context_used);

        let result = tool.execute(serde_json::json!({})).unwrap();
        assert!(result.output.contains("50.0%"));
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/tools/heartbeat.rs crates/river-gateway/src/tools/context.rs
git commit -m "feat: split scheduling into context.rs and heartbeat.rs, move ContextStatusTool"
```

---

### Task 3: Split communication.rs into adapters.rs and communication.rs

**Files:**
- Create: `crates/river-gateway/src/tools/adapters.rs`
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Create adapters.rs**

Extract `AdapterConfig` (lines 18-27), `AdapterRegistry` (lines 31-62), `send_to_adapter()` (lines 64-175), and `ListAdaptersTool` (lines 299-358) from `communication.rs` into a new `crates/river-gateway/src/tools/adapters.rs`.

Keep the same imports these items need:
```rust
//! Adapter infrastructure — registry, config, shared send logic

use crate::conversations::{Author, WriteOp};
use super::registry::{Tool, ToolResult};
use river_core::RiverError;
use river_adapter::Feature;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
```

Then paste `AdapterConfig`, `AdapterRegistry`, `send_to_adapter()`, and `ListAdaptersTool` with their impls and any tests that belong to them.

- [ ] **Step 2: Update communication.rs**

Remove `AdapterConfig`, `AdapterRegistry`, `send_to_adapter()`, `ListAdaptersTool`, and `ContextStatusTool` from `communication.rs`. Add import:
```rust
use super::adapters::{AdapterRegistry, send_to_adapter};
```

Remove the `use std::collections::{HashMap, HashSet}` import if no longer needed (it was for `AdapterRegistry`).

- [ ] **Step 3: Verify it compiles**

```bash
cd ~/river-engine && cargo check 2>&1 | head -20
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/tools/adapters.rs crates/river-gateway/src/tools/communication.rs
git commit -m "refactor: extract adapter infrastructure from communication.rs into adapters.rs"
```

---

### Task 4: Rewrite tools/mod.rs

**Files:**
- Modify: `crates/river-gateway/src/tools/mod.rs`

- [ ] **Step 1: Replace mod.rs contents**

Replace the entire file with submodule declarations and re-exports. No more `river_tools` imports.

```rust
//! Tool system — all agent capabilities

// Core
pub mod registry;
pub mod executor;

// Pure tools
pub mod file;
pub mod shell;
pub mod web;
pub mod logging;

// Stateful tools
pub mod model;
pub mod context;
pub mod heartbeat;
pub mod memory;

// Gateway-integrated tools
pub mod adapters;
pub mod communication;
pub mod subagent;
pub mod sync;

// Re-export core types
pub use registry::{Tool, ToolRegistry, ToolSchema, ToolResult};
pub use executor::{ToolExecutor, ToolCall, ToolCallResponse};

// Re-export tools
pub use file::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
pub use shell::BashTool;
pub use web::{WebFetchTool, WebSearchTool};
pub use logging::LogReadTool;
pub use model::{ModelManagerConfig, ModelManagerState, RequestModelTool, ReleaseModelTool, SwitchModelTool};
pub use context::{ContextRotation, RotateContextTool, ContextStatusTool};
pub use heartbeat::{HeartbeatScheduler, ScheduleHeartbeatTool};
pub use adapters::{AdapterConfig, AdapterRegistry, ListAdaptersTool};
pub use communication::{SendMessageTool, SpeakTool, SwitchChannelTool, TypingTool, ReadChannelTool};
pub use memory::{EmbedTool, MemorySearchTool, MemoryDeleteTool, MemoryDeleteBySourceTool};
pub use subagent::{
    SpawnSubagentTool, ListSubagentsTool, SubagentStatusTool, StopSubagentTool,
    InternalSendTool, InternalReceiveTool, WaitForSubagentTool
};
pub use sync::SyncConversationTool;
```

- [ ] **Step 2: Fix imports in gateway tool files**

In `communication.rs`, `memory.rs`, `subagent.rs`, `sync.rs`, replace:
```rust
use river_tools::{Tool, ToolResult};
```
with:
```rust
use super::registry::{Tool, ToolResult};
```

- [ ] **Step 3: Verify it compiles**

```bash
cd ~/river-engine && cargo check 2>&1 | head -20
```

Expected: success (or errors to fix before continuing).

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/tools/mod.rs crates/river-gateway/src/tools/communication.rs crates/river-gateway/src/tools/memory.rs crates/river-gateway/src/tools/subagent.rs crates/river-gateway/src/tools/sync.rs
git commit -m "refactor: rewrite tools/mod.rs, remove river-tools imports"
```

---

### Task 5: Remove river-tools dependency from gateway and delete the crate

**Files:**
- Modify: `crates/river-gateway/Cargo.toml`
- Delete: `crates/river-tools/` (entire directory)

- [ ] **Step 1: Remove river-tools from gateway Cargo.toml**

Remove the line:
```toml
river-tools = { path = "../river-tools" }
```

- [ ] **Step 2: Delete the river-tools crate**

```bash
rm -rf ~/river-engine/crates/river-tools
```

- [ ] **Step 3: Build and test**

```bash
cd ~/river-engine && cargo build 2>&1 | tail -5
cd ~/river-engine && cargo test 2>&1 | tail -10
```

Expected: build succeeds, all tests pass.

- [ ] **Step 4: Verify no river_tools references remain**

```bash
cd ~/river-engine && grep -r "river.tools\|river_tools" --include="*.rs" --include="*.toml" crates/
```

Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "remove: delete river-tools crate, consolidate all tools into river-gateway"
```

---

### Task 6: Final verification

- [ ] **Step 1: Full test suite**

```bash
cd ~/river-engine && cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 2: Verify final tools/ structure**

```bash
ls ~/river-engine/crates/river-gateway/src/tools/
```

Expected:
```
adapters.rs
communication.rs
context.rs
executor.rs
file.rs
heartbeat.rs
logging.rs
memory.rs
mod.rs
model.rs
registry.rs
shell.rs
subagent.rs
sync.rs
web.rs
```

15 files (14 modules + mod.rs). One concern each.
