# TUI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the adapter-based TUI with a home channel viewer that reads JSONL from stdin/file and posts to the gateway's bystander endpoint.

**Architecture:** Entry types move from river-gateway to river-core with `Display` impls and `Snowflake::from_hex`/`to_datetime`. The old river-tui crate is deleted and rebuilt as a simple JSONL formatter + ratatui renderer + HTTP poster. A `TuiEntry` newtype overrides `Display` for tool entries to render collapsed one-liners.

**Tech Stack:** Rust, ratatui, crossterm, tokio, reqwest, serde/serde_json, clap, chrono, dotenvy

---

### Task 1: Add `Snowflake::from_hex` and `Snowflake::to_datetime`

**Files:**
- Modify: `crates/river-core/src/snowflake/id.rs`
- Modify: `crates/river-core/src/snowflake/generator.rs`
- Modify: `crates/river-core/Cargo.toml`

- [ ] **Step 1: Add `chrono` dependency to river-core**

In `crates/river-core/Cargo.toml`, add under `[dependencies]`:

```toml
chrono.workspace = true
```

- [ ] **Step 2: Write failing test for `from_hex`**

In `crates/river-core/src/snowflake/id.rs`, add to the `tests` module:

```rust
#[test]
fn test_snowflake_from_hex_roundtrip() {
    let birth = test_birth();
    let id = Snowflake::new(1000000, birth, SnowflakeType::Message, 42);
    let hex = format!("{}", id);
    let parsed = Snowflake::from_hex(&hex).unwrap();
    assert_eq!(id, parsed);
}

#[test]
fn test_snowflake_from_hex_invalid() {
    assert!(Snowflake::from_hex("not_hex").is_err());
    assert!(Snowflake::from_hex("abc").is_err()); // too short
    assert!(Snowflake::from_hex("").is_err());
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_from_hex`

Expected: FAIL — `from_hex` doesn't exist.

- [ ] **Step 4: Implement `from_hex`**

In `crates/river-core/src/snowflake/id.rs`, add to `impl Snowflake`:

```rust
/// Parse a Snowflake from its 32-character hex string representation.
pub fn from_hex(s: &str) -> Result<Self, String> {
    if s.len() != 32 {
        return Err(format!("expected 32 hex chars, got {}", s.len()));
    }
    let high = u64::from_str_radix(&s[..16], 16)
        .map_err(|e| format!("invalid hex in high bits: {}", e))?;
    let low = u64::from_str_radix(&s[16..], 16)
        .map_err(|e| format!("invalid hex in low bits: {}", e))?;
    Ok(Self::from_parts(high, low))
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_from_hex`

Expected: PASS

- [ ] **Step 6: Move `birth_to_micros` and `is_leap_year` to `AgentBirth`**

The `birth_to_micros` logic currently lives in `SnowflakeGenerator` as a private method. Move it to `AgentBirth` so `Snowflake::to_datetime` can use it without depending on the generator.

In `crates/river-core/src/snowflake/birth.rs`, add to `impl AgentBirth`:

```rust
/// Convert this birth to microseconds since Unix epoch.
pub fn to_epoch_micros(&self) -> u64 {
    let year = self.year() as i32;
    let month = self.month() as i32;
    let day = self.day() as i32;

    let mut days: i64 = 0;
    for y in 1970..year {
        days += if (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0) { 366 } else { 365 };
    }

    let days_in_months = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += days_in_months[m as usize] as i64;
        if m == 2 && ((year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)) {
            days += 1;
        }
    }

    days += (day - 1) as i64;

    let total_seconds =
        days as u64 * 86400 + self.hour() as u64 * 3600 + self.minute() as u64 * 60 + self.second() as u64;

    total_seconds * 1_000_000
}
```

In `crates/river-core/src/snowflake/generator.rs`, update `birth_to_micros` to delegate:

```rust
fn birth_to_micros(birth: &AgentBirth) -> u64 {
    birth.to_epoch_micros()
}
```

Remove the now-redundant `is_leap_year` method from `SnowflakeGenerator` (it's inlined in `to_epoch_micros`).

- [ ] **Step 7: Write failing test for `to_datetime`**

In `crates/river-core/src/snowflake/id.rs`, add at the top:

```rust
use chrono::{DateTime, Utc};
```

Add to `tests` module:

```rust
#[test]
fn test_snowflake_to_datetime() {
    // birth = 2024-03-15 14:30:45
    let birth = test_birth();
    // 0 microseconds after birth = birth time
    let id = Snowflake::new(0, birth, SnowflakeType::Message, 0);
    let dt = id.to_datetime();
    assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-15 14:30:45");

    // 1 second (1_000_000 micros) after birth
    let id2 = Snowflake::new(1_000_000, birth, SnowflakeType::Message, 0);
    let dt2 = id2.to_datetime();
    assert_eq!(dt2.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-15 14:30:46");
}
```

- [ ] **Step 8: Run test to verify it fails**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_to_datetime`

Expected: FAIL — `to_datetime` doesn't exist.

- [ ] **Step 9: Implement `to_datetime`**

In `crates/river-core/src/snowflake/id.rs`, add the import at the top of the file:

```rust
use chrono::{DateTime, TimeZone, Utc};
```

Add to `impl Snowflake`:

```rust
/// Compute the wall-clock time this snowflake was created.
///
/// The snowflake encodes both the agent birth (low 36 bits) and
/// microseconds since birth (high 64 bits), so this needs no
/// external configuration.
pub fn to_datetime(&self) -> DateTime<Utc> {
    let birth_micros = self.birth().to_epoch_micros();
    let absolute_micros = birth_micros + self.timestamp_micros();
    let secs = (absolute_micros / 1_000_000) as i64;
    let nanos = ((absolute_micros % 1_000_000) * 1000) as u32;
    Utc.timestamp_opt(secs, nanos).single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())
}
```

- [ ] **Step 10: Run test to verify it passes**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_to_datetime`

Expected: PASS

- [ ] **Step 11: Run all snowflake tests**

Run: `cargo test -p river-core -- snowflake`

Expected: All existing tests still pass.

- [ ] **Step 12: Commit**

```bash
git add -A && git commit -m "feat(core): add Snowflake::from_hex and to_datetime, move birth_to_micros to AgentBirth"
```

---

### Task 2: Move entry types to river-core

**Files:**
- Create: `crates/river-core/src/channels/entry.rs`
- Create: `crates/river-core/src/channels/mod.rs`
- Modify: `crates/river-core/src/lib.rs`
- Modify: `crates/river-gateway/src/channels/entry.rs`
- Modify: `crates/river-gateway/src/channels/mod.rs`

- [ ] **Step 1: Create `crates/river-core/src/channels/mod.rs`**

```rust
//! Channel entry types shared across river-engine crates

pub mod entry;

pub use entry::{
    ChannelEntry, HomeChannelEntry,
    MessageEntry, ToolEntry, HeartbeatEntry, CursorEntry,
};
```

- [ ] **Step 2: Create `crates/river-core/src/channels/entry.rs`**

Copy the entire contents of `crates/river-gateway/src/channels/entry.rs` into this file. This includes all structs (`MessageEntry`, `ToolEntry`, `HeartbeatEntry`, `CursorEntry`), both enums (`ChannelEntry`, `HomeChannelEntry`), all `impl` blocks with constructors, and all tests.

- [ ] **Step 3: Update `crates/river-core/src/lib.rs`**

Add:

```rust
pub mod channels;

// Re-exports from channels module
pub use channels::{
    ChannelEntry, HomeChannelEntry,
    MessageEntry, ToolEntry, HeartbeatEntry, CursorEntry,
};
```

- [ ] **Step 4: Replace `crates/river-gateway/src/channels/entry.rs` with re-exports**

Replace the entire file contents with:

```rust
//! Channel entry types — re-exported from river-core

pub use river_core::channels::entry::*;
```

- [ ] **Step 5: Build river-gateway to check for compilation errors**

Run: `cargo build -p river-gateway 2>&1 | head -40`

Fix any import issues. The gateway's `channels/mod.rs` already re-exports from `entry`, so the re-export chain should be: `river-core::channels::entry` → `river-gateway::channels::entry` → `river-gateway::channels` → all consumers.

- [ ] **Step 6: Run all gateway tests**

Run: `cargo test -p river-gateway`

Expected: All tests pass — behavior unchanged, types just moved.

- [ ] **Step 7: Run river-core tests**

Run: `cargo test -p river-core`

Expected: All entry tests pass in their new home.

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "refactor(core): move channel entry types from gateway to river-core"
```

---

### Task 3: Add `Display` impls to entry types in river-core

**Files:**
- Modify: `crates/river-core/src/channels/entry.rs`

- [ ] **Step 1: Write failing tests for Display**

Add to the `tests` module in `crates/river-core/src/channels/entry.rs`:

```rust
use crate::snowflake::{AgentBirth, Snowflake, SnowflakeType};

fn test_snowflake_hex() -> String {
    let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
    let id = Snowflake::new(0, birth, SnowflakeType::Message, 0);
    format!("{}", id)
}

#[test]
fn test_display_agent_message() {
    let id = test_snowflake_hex();
    let entry = MessageEntry::agent(id, "hello world".into(), "home".into(), None);
    let display = format!("{}", entry);
    assert!(display.contains("[agent]"));
    assert!(display.contains("hello world"));
    assert!(display.contains("2026-05-14"));
}

#[test]
fn test_display_user_message() {
    let id = test_snowflake_hex();
    let mut entry = MessageEntry::user_home(
        id, "cassie".into(), "u1".into(), "hi there".into(),
        "discord".into(), "general".into(), Some("general".into()), None,
    );
    let display = format!("{}", entry);
    assert!(display.contains("[user:discord]"));
    assert!(display.contains("cassie:"));
    assert!(display.contains("hi there"));
}

#[test]
fn test_display_bystander_message() {
    let id = test_snowflake_hex();
    let entry = MessageEntry::bystander(id, "interesting".into());
    let display = format!("{}", entry);
    assert!(display.contains("[bystander]"));
    assert!(display.contains("interesting"));
}

#[test]
fn test_display_heartbeat() {
    let id = test_snowflake_hex();
    let entry = HeartbeatEntry::new(id, "2026-05-14T12:00:00Z".into());
    let display = format!("{}", entry);
    assert!(display.contains("💓"));
    assert!(display.contains("2026-05-14"));
}

#[test]
fn test_display_cursor() {
    let id = test_snowflake_hex();
    let entry = CursorEntry::new(id);
    let display = format!("{}", entry);
    assert!(display.contains("┄"));
    assert!(display.contains("read cursor"));
}

#[test]
fn test_display_tool_call() {
    let id = test_snowflake_hex();
    let entry = ToolEntry::call(
        id, "read_file".into(),
        serde_json::json!({"path": "/tmp/test.txt"}),
        "tc_001".into(),
    );
    let display = format!("{}", entry);
    assert!(display.contains("read_file"));
    // Full Display shows complete content
    assert!(display.contains("/tmp/test.txt"));
}

#[test]
fn test_display_tool_result() {
    let id = test_snowflake_hex();
    let entry = ToolEntry::result(
        id, "read_file".into(),
        "line 1\nline 2\nline 3".into(),
        "tc_001".into(),
    );
    let display = format!("{}", entry);
    // Full Display shows complete result content
    assert!(display.contains("line 1"));
    assert!(display.contains("line 3"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-core -- channels::entry::tests::test_display`

Expected: FAIL — `Display` not implemented.

- [ ] **Step 3: Implement `Display` for all entry types**

Add at the top of `crates/river-core/src/channels/entry.rs`:

```rust
use std::fmt;
use crate::snowflake::Snowflake;
```

Add after the existing `impl` blocks:

```rust
/// Extract a formatted timestamp from a snowflake hex ID string.
/// Returns "????-??-?? ??:??:??" if the ID can't be parsed.
fn format_snowflake_time(id: &str) -> String {
    match Snowflake::from_hex(id) {
        Ok(sf) => sf.to_datetime().format("%Y-%m-%d %H:%M:%S").to_string(),
        Err(_) => "????-??-?? ??:??:??".to_string(),
    }
}

impl fmt::Display for MessageEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let time = format_snowflake_time(&self.id);
        match self.role.as_str() {
            "agent" => write!(f, "{} [agent] {}", time, self.content),
            "user" => {
                let author = self.author.as_deref().unwrap_or("unknown");
                let adapter = self.source_adapter.as_deref().unwrap_or(&self.adapter);
                write!(f, "{} [user:{}] {}: {}", time, adapter, author, self.content)
            }
            "bystander" => write!(f, "{} [bystander] {}", time, self.content),
            "system" => write!(f, "{} [system] {}", time, self.content),
            other => write!(f, "{} [{}] {}", time, other, self.content),
        }
    }
}

impl fmt::Display for ToolEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let time = format_snowflake_time(&self.id);
        match self.kind.as_str() {
            "tool_call" => {
                let args = self.arguments.as_ref()
                    .map(|a| serde_json::to_string(a).unwrap_or_default())
                    .unwrap_or_default();
                write!(f, "{} 🔧 {}({})", time, self.tool_name, args)
            }
            "tool_result" => {
                if let Some(ref file) = self.result_file {
                    write!(f, "{} 🔧 {} → file: {}", time, self.tool_name, file)
                } else if let Some(ref result) = self.result {
                    write!(f, "{} 🔧 {} → {}", time, self.tool_name, result)
                } else {
                    write!(f, "{} 🔧 {} → ok", time, self.tool_name)
                }
            }
            other => write!(f, "{} [tool:{}] {}", time, other, self.tool_name),
        }
    }
}

impl fmt::Display for HeartbeatEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let time = format_snowflake_time(&self.id);
        write!(f, "{} 💓", time)
    }
}

impl fmt::Display for CursorEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let time = format_snowflake_time(&self.id);
        write!(f, "{} ┄ read cursor", time)
    }
}

impl fmt::Display for HomeChannelEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HomeChannelEntry::Message(m) => m.fmt(f),
            HomeChannelEntry::Cursor(c) => c.fmt(f),
            HomeChannelEntry::Tool(t) => t.fmt(f),
            HomeChannelEntry::Heartbeat(h) => h.fmt(f),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p river-core -- channels::entry::tests::test_display`

Expected: PASS

- [ ] **Step 5: Run all tests**

Run: `cargo test -p river-core && cargo test -p river-gateway`

Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(core): add Display impls for all channel entry types"
```

---

### Task 4: Delete old river-tui and scaffold new crate

**Files:**
- Delete: entire `crates/river-tui/` directory
- Create: `crates/river-tui/Cargo.toml`
- Create: `crates/river-tui/src/main.rs`
- Create: `crates/river-tui/src/lib.rs`

- [ ] **Step 1: Delete the old crate**

```bash
rm -rf crates/river-tui
```

- [ ] **Step 2: Create `crates/river-tui/Cargo.toml`**

```toml
[package]
name = "river-tui"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "River Engine TUI — home channel viewer"

[[bin]]
name = "river-tui"
path = "src/main.rs"

[dependencies]
river-core = { path = "../river-core" }
ratatui.workspace = true
crossterm.workspace = true
tokio.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
clap.workspace = true
chrono.workspace = true
dotenvy = "0.15"
tracing.workspace = true
tracing-subscriber.workspace = true

[dev-dependencies]
```

- [ ] **Step 3: Create `crates/river-tui/src/lib.rs`**

```rust
//! River TUI — home channel viewer
//!
//! Reads home channel JSONL from stdin or a file, renders as a chat window,
//! and posts user input to the gateway's bystander endpoint.

pub mod config;
pub mod format;
pub mod input;
pub mod render;
pub mod post;
```

- [ ] **Step 4: Create `crates/river-tui/src/main.rs`**

```rust
use clap::Parser;

fn main() {
    println!("river-tui scaffold");
}
```

- [ ] **Step 5: Verify it builds**

Run: `cargo build -p river-tui`

Expected: Compiles (with warnings about unused modules — they don't exist yet).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor(tui): delete old adapter-based TUI, scaffold new home channel viewer"
```

---

### Task 5: CLI config and bystander POST client

**Files:**
- Create: `crates/river-tui/src/config.rs`
- Create: `crates/river-tui/src/post.rs`

- [ ] **Step 1: Write `crates/river-tui/src/config.rs`**

```rust
//! CLI args and configuration

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-tui")]
#[command(about = "River Engine TUI — home channel viewer")]
pub struct Args {
    /// Agent name
    #[arg(long)]
    pub agent: String,

    /// Gateway URL
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    pub gateway_url: String,

    /// Path to JSONL file to tail (reads stdin if not given)
    #[arg(long)]
    pub file: Option<PathBuf>,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub agent: String,
    pub gateway_url: String,
    pub file: Option<PathBuf>,
    pub auth_token: Option<String>,
}

impl TuiConfig {
    pub fn from_args(args: Args) -> Self {
        let auth_token = std::env::var("RIVER_AUTH_TOKEN").ok();
        Self {
            agent: args.agent,
            gateway_url: args.gateway_url,
            file: args.file,
            auth_token,
        }
    }

    /// The bystander endpoint URL
    pub fn bystander_url(&self) -> String {
        format!("{}/home/{}/message", self.gateway_url, self.agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bystander_url() {
        let config = TuiConfig {
            agent: "iris".into(),
            gateway_url: "http://localhost:3000".into(),
            file: None,
            auth_token: None,
        };
        assert_eq!(config.bystander_url(), "http://localhost:3000/home/iris/message");
    }
}
```

- [ ] **Step 2: Write failing test for bystander POST**

Create `crates/river-tui/src/post.rs`:

```rust
//! Bystander endpoint HTTP client

use reqwest::Client;
use std::time::Duration;

pub struct BystanterClient {
    client: Client,
    url: String,
    auth_token: Option<String>,
}

impl BystanterClient {
    pub fn new(url: String, auth_token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, url, auth_token }
    }

    /// Post a message to the bystander endpoint.
    /// Returns Ok(()) on success, Err(message) on failure.
    pub async fn post(&self, content: &str) -> Result<(), String> {
        let body = serde_json::json!({ "content": content });
        let mut req = self.client.post(&self.url).json(&body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        let resp = req.send().await.map_err(|e| format!("request failed: {}", e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("gateway returned {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_construction() {
        let client = BystanterClient::new(
            "http://localhost:3000/home/iris/message".into(),
            Some("test-token".into()),
        );
        assert_eq!(client.url, "http://localhost:3000/home/iris/message");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-tui`

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(tui): add CLI config and bystander POST client"
```

---

### Task 6: TuiEntry newtype and HomeChannelFormatter

**Files:**
- Create: `crates/river-tui/src/format.rs`

- [ ] **Step 1: Write failing tests for TuiEntry formatting**

Create `crates/river-tui/src/format.rs`:

```rust
//! TUI-specific entry formatting
//!
//! TuiEntry wraps HomeChannelEntry with collapsed tool rendering.
//! HomeChannelFormatter handles stateful tool call pairing.

use river_core::channels::entry::{HomeChannelEntry, ToolEntry};
use river_core::snowflake::Snowflake;
use std::collections::HashMap;
use std::fmt;

/// Newtype for TUI-specific Display.
/// Delegates to river-core Display for most types.
/// Overrides ToolEntry to render collapsed one-liners.
pub struct TuiEntry(pub HomeChannelEntry);

/// A formatted line ready for display.
#[derive(Debug, Clone)]
pub struct FormattedLine {
    pub text: String,
}

/// Stateful formatter that pairs tool calls with results.
pub struct HomeChannelFormatter {
    /// Pending tool calls waiting for results, keyed by tool_call_id
    pending_calls: HashMap<String, PendingCall>,
}

#[derive(Debug)]
struct PendingCall {
    tool_name: String,
    args_summary: String,
    timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::channels::entry::*;
    use river_core::snowflake::{AgentBirth, SnowflakeType};

    fn test_snowflake_hex() -> String {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        let id = Snowflake::new(0, birth, SnowflakeType::Message, 0);
        format!("{}", id)
    }

    #[test]
    fn test_tui_entry_message_delegates() {
        let id = test_snowflake_hex();
        let entry = HomeChannelEntry::Message(
            MessageEntry::agent(id, "hello".into(), "home".into(), None),
        );
        let tui = TuiEntry(entry.clone());
        // TuiEntry should produce same output as inner Display for messages
        assert_eq!(format!("{}", tui), format!("{}", entry));
    }

    #[test]
    fn test_tui_entry_heartbeat_delegates() {
        let id = test_snowflake_hex();
        let entry = HomeChannelEntry::Heartbeat(
            HeartbeatEntry::new(id, "2026-05-14T12:00:00Z".into()),
        );
        let tui = TuiEntry(entry.clone());
        assert_eq!(format!("{}", tui), format!("{}", entry));
    }

    #[test]
    fn test_formatter_tool_call_then_result() {
        let id1 = test_snowflake_hex();
        let id2 = test_snowflake_hex();
        let mut fmt = HomeChannelFormatter::new();

        let call = HomeChannelEntry::Tool(ToolEntry::call(
            id1, "read_file".into(),
            serde_json::json!({"path": "src/main.rs"}),
            "tc_001".into(),
        ));
        let lines = fmt.push(call);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("🔧 read_file"));
        assert!(!lines[0].text.contains("→")); // no result yet

        let result = HomeChannelEntry::Tool(ToolEntry::result(
            id2, "read_file".into(),
            "fn main() {}\n".repeat(100),
            "tc_001".into(),
        ));
        let lines = fmt.push(result);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("→ 100 lines"));
    }

    #[test]
    fn test_formatter_tool_result_file() {
        let id1 = test_snowflake_hex();
        let id2 = test_snowflake_hex();
        let mut fmt = HomeChannelFormatter::new();

        let call = HomeChannelEntry::Tool(ToolEntry::call(
            id1, "bash".into(),
            serde_json::json!({"command": "ls"}),
            "tc_002".into(),
        ));
        fmt.push(call);

        let result = HomeChannelEntry::Tool(ToolEntry::result_file(
            id2, "bash".into(),
            "tool-results/abc123.txt".into(),
            "tc_002".into(),
        ));
        let lines = fmt.push(result);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("→ tool-results/abc123.txt"));
    }

    #[test]
    fn test_formatter_orphan_result() {
        let id = test_snowflake_hex();
        let mut fmt = HomeChannelFormatter::new();

        let result = HomeChannelEntry::Tool(ToolEntry::result(
            id, "read_file".into(),
            "some content\nmore content".into(),
            "tc_orphan".into(),
        ));
        let lines = fmt.push(result);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("🔧 read_file → 2 lines"));
    }

    #[test]
    fn test_formatter_message_passthrough() {
        let id = test_snowflake_hex();
        let mut fmt = HomeChannelFormatter::new();

        let msg = HomeChannelEntry::Message(
            MessageEntry::agent(id, "hello".into(), "home".into(), None),
        );
        let lines = fmt.push(msg);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("[agent]"));
        assert!(lines[0].text.contains("hello"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-tui -- format::tests`

Expected: FAIL — `TuiEntry` Display and `HomeChannelFormatter` not implemented.

- [ ] **Step 3: Implement `TuiEntry` Display**

Add to `crates/river-tui/src/format.rs`:

```rust
impl fmt::Display for TuiEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            // Tool entries get collapsed rendering — but standalone
            // TuiEntry Display is only used outside the formatter for
            // simple cases. The formatter handles tool call pairing.
            HomeChannelEntry::Tool(t) => {
                let time = snowflake_time(&t.id);
                match t.kind.as_str() {
                    "tool_call" => {
                        let args = summarize_args(&t.arguments);
                        write!(f, "{} 🔧 {}({})", time, t.tool_name, args)
                    }
                    "tool_result" => {
                        let result_summary = summarize_result(t);
                        write!(f, "{} 🔧 {} → {}", time, t.tool_name, result_summary)
                    }
                    _ => write!(f, "{}", self.0),
                }
            }
            // Everything else delegates to river-core Display
            other => write!(f, "{}", other),
        }
    }
}

fn snowflake_time(id: &str) -> String {
    match Snowflake::from_hex(id) {
        Ok(sf) => sf.to_datetime().format("%Y-%m-%d %H:%M:%S").to_string(),
        Err(_) => "????-??-?? ??:??:??".to_string(),
    }
}

fn summarize_args(args: &Option<serde_json::Value>) -> String {
    match args {
        Some(v) => {
            let s = serde_json::to_string(v).unwrap_or_default();
            if s.len() > 60 {
                format!("{}…", &s[..57])
            } else {
                s
            }
        }
        None => String::new(),
    }
}

fn summarize_result(t: &ToolEntry) -> String {
    if let Some(ref file) = t.result_file {
        file.clone()
    } else if let Some(ref result) = t.result {
        let lines = result.lines().count();
        if lines > 1 {
            format!("{} lines", lines)
        } else if result.is_empty() {
            "ok".to_string()
        } else if result.len() > 80 {
            format!("{}…", &result[..77])
        } else {
            result.clone()
        }
    } else {
        "ok".to_string()
    }
}
```

- [ ] **Step 4: Implement `HomeChannelFormatter`**

Add to `crates/river-tui/src/format.rs`:

```rust
impl HomeChannelFormatter {
    pub fn new() -> Self {
        Self {
            pending_calls: HashMap::new(),
        }
    }

    /// Push an entry and get back formatted lines.
    ///
    /// For tool calls: stores the call and emits a line without an arrow.
    /// For tool results: pairs with stored call and emits a combined line.
    /// For everything else: emits the line immediately via TuiEntry Display.
    pub fn push(&mut self, entry: HomeChannelEntry) -> Vec<FormattedLine> {
        match &entry {
            HomeChannelEntry::Tool(t) if t.kind == "tool_call" => {
                let time = snowflake_time(&t.id);
                let args = summarize_args(&t.arguments);
                let text = format!("{} 🔧 {}({})", time, t.tool_name, args);
                self.pending_calls.insert(t.tool_call_id.clone(), PendingCall {
                    tool_name: t.tool_name.clone(),
                    args_summary: args,
                    timestamp: time,
                });
                vec![FormattedLine { text }]
            }
            HomeChannelEntry::Tool(t) if t.kind == "tool_result" => {
                let result_summary = summarize_result(t);
                let text = if let Some(call) = self.pending_calls.remove(&t.tool_call_id) {
                    format!("{} 🔧 {}({}) → {}",
                        call.timestamp, call.tool_name, call.args_summary, result_summary)
                } else {
                    // Orphan result — no matching call
                    let time = snowflake_time(&t.id);
                    format!("{} 🔧 {} → {}", time, t.tool_name, result_summary)
                };
                vec![FormattedLine { text }]
            }
            _ => {
                let text = format!("{}", TuiEntry(entry));
                vec![FormattedLine { text }]
            }
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-tui -- format::tests`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(tui): TuiEntry newtype and HomeChannelFormatter with tool call pairing"
```

---

### Task 7: Input reader (stdin and file tailing)

**Files:**
- Create: `crates/river-tui/src/input.rs`

- [ ] **Step 1: Write `crates/river-tui/src/input.rs`**

```rust
//! JSONL input reader — reads from stdin or tails a file

use river_core::channels::entry::HomeChannelEntry;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Read JSONL entries from stdin or a file and send parsed entries to the channel.
pub async fn run_reader(
    file: Option<PathBuf>,
    tx: mpsc::UnboundedSender<HomeChannelEntry>,
) {
    if let Some(path) = file {
        read_file(path, tx).await;
    } else {
        read_stdin(tx).await;
    }
}

async fn read_stdin(tx: mpsc::UnboundedSender<HomeChannelEntry>) {
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HomeChannelEntry>(&line) {
            Ok(entry) => {
                if tx.send(entry).is_err() {
                    break; // receiver dropped
                }
            }
            Err(e) => {
                tracing::warn!("skipping malformed JSONL line: {}", e);
            }
        }
    }
}

async fn read_file(path: PathBuf, tx: mpsc::UnboundedSender<HomeChannelEntry>) {
    use tokio::fs::File;
    use tokio::time::{sleep, Duration};

    let file = match File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("failed to open {}: {}", path.display(), e);
            return;
        }
    };

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF — wait and try again (tail behavior)
                sleep(Duration::from_millis(100)).await;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<HomeChannelEntry>(trimmed) {
                    Ok(entry) => {
                        if tx.send(entry).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("skipping malformed JSONL line: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("read error: {}", e);
                break;
            }
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p river-tui`

Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(tui): JSONL input reader for stdin and file tailing"
```

---

### Task 8: Ratatui rendering

**Files:**
- Create: `crates/river-tui/src/render.rs`

- [ ] **Step 1: Write `crates/river-tui/src/render.rs`**

```rust
//! Ratatui terminal rendering

use crate::format::{FormattedLine, HomeChannelFormatter};
use crate::post::BystanterClient;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use river_core::channels::entry::HomeChannelEntry;
use std::io::stdout;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Run the TUI. Ensures terminal cleanup on all exit paths.
pub async fn run(
    agent: String,
    mut entry_rx: mpsc::UnboundedReceiver<HomeChannelEntry>,
    client: Arc<BystanterClient>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let result = run_inner(agent, &mut entry_rx, client).await;

    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);

    result
}

async fn run_inner(
    agent: String,
    entry_rx: &mut mpsc::UnboundedReceiver<HomeChannelEntry>,
    client: Arc<BystanterClient>,
) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut formatter = HomeChannelFormatter::new();
    let mut lines: Vec<FormattedLine> = Vec::new();
    let mut input = String::new();
    let mut scroll_offset: u16 = 0;
    let mut follow_tail = true;
    let mut status_error: Option<String> = None;

    loop {
        // Calculate input height (expands with content)
        let input_line_count = {
            let width = terminal.size()?.width.saturating_sub(4) as usize; // border + prompt
            if width == 0 { 1 } else {
                let display_len = input.len() + 2; // "> " prefix
                (display_len / width.max(1) + 1).max(1) as u16
            }
        };
        let input_height = input_line_count + 2; // +2 for borders

        // Draw
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),              // log
                    Constraint::Length(1),            // status bar
                    Constraint::Length(input_height), // input
                ])
                .split(frame.area());

            // --- Log ---
            let prefix_width = 20; // "YYYY-MM-DD HH:MM:SS " = 20 chars
            let log_lines: Vec<Line> = lines.iter().flat_map(|fl| {
                let content_lines: Vec<&str> = fl.text.lines().collect();
                if content_lines.is_empty() {
                    vec![Line::from(Span::raw(""))]
                } else {
                    content_lines.iter().enumerate().map(|(i, line)| {
                        if i == 0 {
                            Line::from(Span::raw(line.to_string()))
                        } else {
                            // Indent continuation lines
                            let indent = " ".repeat(prefix_width);
                            Line::from(Span::raw(format!("{}{}", indent, line)))
                        }
                    }).collect::<Vec<_>>()
                }
            }).collect();

            let log_widget = Paragraph::new(log_lines.clone())
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });

            let inner_height = chunks[0].height.saturating_sub(2);
            let total_lines = log_lines.len() as u16;
            if follow_tail && total_lines > inner_height {
                scroll_offset = total_lines.saturating_sub(inner_height);
            }

            let log_widget = log_widget.scroll((scroll_offset, 0));
            frame.render_widget(log_widget, chunks[0]);

            // --- Status bar ---
            let mut status_spans = vec![
                Span::raw(" [river] "),
                Span::styled(&agent, Style::default().fg(Color::Cyan)),
            ];
            if let Some(ref err) = status_error {
                status_spans.push(Span::styled(
                    format!(" | {}", err),
                    Style::default().fg(Color::Red),
                ));
            }
            let status_widget = Paragraph::new(Line::from(status_spans))
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(status_widget, chunks[1]);

            // --- Input ---
            let input_widget = Paragraph::new(format!("> {}", input))
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });
            frame.render_widget(input_widget, chunks[2]);
        })?;

        // Event loop
        tokio::select! {
            // Terminal input
            poll_result = tokio::task::spawn_blocking(|| {
                event::poll(std::time::Duration::from_millis(50)).unwrap_or(false)
            }) => {
                if !poll_result.unwrap_or(false) {
                    continue;
                }
                let evt = tokio::task::block_in_place(|| event::read())?;
                match evt {
                    Event::Key(key) => {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Enter, _) if !input.is_empty() => {
                                let content = std::mem::take(&mut input);
                                let c = client.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = c.post(&content).await {
                                        tracing::error!("bystander post failed: {}", e);
                                    }
                                });
                                status_error = None;
                                follow_tail = true;
                            }
                            (KeyCode::Char(c), _) => { input.push(c); }
                            (KeyCode::Backspace, _) => { input.pop(); }
                            (KeyCode::Up, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(1);
                            }
                            (KeyCode::Down, _) => {
                                scroll_offset = scroll_offset.saturating_add(1);
                                let total = lines.len() as u16;
                                if scroll_offset >= total { follow_tail = true; }
                            }
                            (KeyCode::PageUp, _) => {
                                follow_tail = false;
                                scroll_offset = scroll_offset.saturating_sub(10);
                            }
                            (KeyCode::PageDown, _) => {
                                scroll_offset = scroll_offset.saturating_add(10);
                                let total = lines.len() as u16;
                                if scroll_offset >= total { follow_tail = true; }
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {} // re-render
                    _ => {}
                }
            }
            // New entry from reader
            entry = entry_rx.recv() => {
                match entry {
                    Some(e) => {
                        let new_lines = formatter.push(e);
                        lines.extend(new_lines);
                    }
                    None => break, // reader closed
                }
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p river-tui`

Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(tui): ratatui rendering with expanding input, scroll, tool call pairing"
```

---

### Task 9: Wire main.rs

**Files:**
- Modify: `crates/river-tui/src/main.rs`

- [ ] **Step 1: Write the complete `main.rs`**

```rust
use clap::Parser;
use river_tui::config::{Args, TuiConfig};
use river_tui::post::BystanterClient;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args = Args::parse();
    let config = TuiConfig::from_args(args);

    // Log to file — stdout is owned by ratatui
    let log_file = std::fs::File::create("river-tui.log")?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("Starting river-tui for agent: {}", config.agent);

    let client = Arc::new(BystanterClient::new(
        config.bystander_url(),
        config.auth_token.clone(),
    ));

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn input reader
    let file = config.file.clone();
    tokio::spawn(async move {
        river_tui::input::run_reader(file, tx).await;
    });

    // Run TUI (blocks until Ctrl-C)
    river_tui::render::run(config.agent, rx, client).await?;

    Ok(())
}
```

- [ ] **Step 2: Build the final binary**

Run: `cargo build -p river-tui`

Expected: Compiles with no errors.

- [ ] **Step 3: Run all tests in workspace**

Run: `cargo test -p river-core && cargo test -p river-gateway && cargo test -p river-tui`

Expected: All pass.

- [ ] **Step 4: Smoke test**

Create a test JSONL file and run the TUI:

```bash
cat > /tmp/test-home.jsonl << 'EOF'
{"type":"message","id":"0000000000000000a000000000000000","role":"agent","content":"hello world","adapter":"home"}
{"type":"heartbeat","id":"0000000000000001a000000000000000","kind":"heartbeat","timestamp":"2026-05-14T12:00:01Z"}
EOF

river-tui --agent test --file /tmp/test-home.jsonl
```

Verify the TUI renders entries. Ctrl-C to exit.

Note: the test snowflake IDs above won't have valid birth encodings, so timestamps will show as `????-??-?? ??:??:??`. That's correct behavior for unparseable IDs.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(tui): wire main.rs — complete home channel viewer"
```

---

### Task 10: Cleanup

**Files:**
- Modify: `crates/river-tui/src/lib.rs`

- [ ] **Step 1: Remove unused module declarations from lib.rs**

Verify `lib.rs` only declares modules that exist. The current `lib.rs` declares `config`, `format`, `input`, `render`, `post`. Verify all five files exist.

Run: `ls crates/river-tui/src/`

Expected: `config.rs`, `format.rs`, `input.rs`, `lib.rs`, `main.rs`, `post.rs`, `render.rs`

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p river-tui -- -D warnings 2>&1 | head -20`

Fix any warnings.

- [ ] **Step 3: Run full workspace build**

Run: `cargo build`

Expected: Clean build, all crates compile.

- [ ] **Step 4: Run full workspace tests**

Run: `cargo test`

Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "cleanup(tui): clippy fixes, verify full workspace builds"
```
