# Spectator Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-turn spectator with a time-gated sweep model that reads the home channel directly and produces narrative move summaries via LLM.

**Architecture:** The spectator listens for TurnComplete events and checks if ≥5 minutes have elapsed since the last move. If yes, it reads home channel entries since the last move's cursor, formats them with tiered detail (full text for messages, name-only for tools), sends to the LLM for a plain text narrative summary, and writes one move to `moves.jsonl`. One sweep, one move. The spectator owns segmentation, the LLM owns narration.

**Tech Stack:** Rust, tokio (async), serde/serde_json, JSONL

---

### Task 1: Entry Formatter

**Files:**
- Create: `crates/river-gateway/src/spectator/format.rs`
- Modify: `crates/river-gateway/src/spectator/mod.rs` (add `pub mod format;`)

The entry formatter converts `HomeChannelEntry` entries into a transcript string for the LLM, with token budgeting.

- [ ] **Step 1: Write failing tests for entry formatting**

```rust
// In crates/river-gateway/src/spectator/format.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::entry::*;

    #[test]
    fn test_format_user_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::user_home(
            "abc001".into(), "cassie".into(), "u1".into(), "hello world".into(),
            "discord".into(), "general".into(), Some("general".into()), None,
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc001] user:discord:general/general cassie: hello world".to_string()));
    }

    #[test]
    fn test_format_agent_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            "abc002".into(), "hi there!".into(), "home".into(), None,
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc002] agent: hi there!".to_string()));
    }

    #[test]
    fn test_format_bystander_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::bystander(
            "abc003".into(), "interesting work".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc003] bystander: interesting work".to_string()));
    }

    #[test]
    fn test_format_system_message() {
        let entry = HomeChannelEntry::Message(MessageEntry::system_msg(
            "abc004".into(), "context pressure warning".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc004] system: context pressure warning".to_string()));
    }

    #[test]
    fn test_format_spectator_message_filtered() {
        let entry = HomeChannelEntry::Message(MessageEntry::system_msg(
            "abc010".into(), "[spectator] move written covering entries abc001-abc009".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, None); // spectator's own messages are filtered
    }

    #[test]
    fn test_format_tool_call() {
        let entry = HomeChannelEntry::Tool(ToolEntry::call(
            "abc005".into(), "read_file".into(),
            serde_json::json!({"path": "/tmp/test.txt"}), "tc1".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc005] tool_call: read_file".to_string()));
    }

    #[test]
    fn test_format_tool_result() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result(
            "abc006".into(), "read_file".into(),
            "file contents here, this is some data".into(), "tc1".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc006] tool_result(read_file): [38 bytes]".to_string()));
    }

    #[test]
    fn test_format_tool_result_file() {
        let entry = HomeChannelEntry::Tool(ToolEntry::result_file(
            "abc007".into(), "bash".into(),
            "/tmp/results/abc007.txt".into(), "tc2".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, Some("[abc007] tool_result(bash): [file: /tmp/results/abc007.txt]".to_string()));
    }

    #[test]
    fn test_format_heartbeat_filtered() {
        let entry = HomeChannelEntry::Heartbeat(HeartbeatEntry::new(
            "abc008".into(), "2026-05-12T12:00:00Z".into(),
        ));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_cursor_filtered() {
        let entry = HomeChannelEntry::Cursor(CursorEntry::new("abc009".into()));
        let result = format_entry(&entry);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_entries_with_budget() {
        let entries = vec![
            HomeChannelEntry::Message(MessageEntry::agent(
                "001".into(), "short".into(), "home".into(), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                "002".into(), "also short".into(), "home".into(), None,
            )),
            HomeChannelEntry::Message(MessageEntry::agent(
                "003".into(), "third message".into(), "home".into(), None,
            )),
        ];

        // Large budget — all entries fit
        let (transcript, last_idx) = format_entries_budgeted(&entries, 10000);
        assert_eq!(last_idx, 2);
        assert!(transcript.contains("[001]"));
        assert!(transcript.contains("[003]"));

        // Tiny budget — only first entry fits
        let (transcript, last_idx) = format_entries_budgeted(&entries, 10);
        assert_eq!(last_idx, 0);
        assert!(transcript.contains("[001]"));
        assert!(!transcript.contains("[002]"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway -- spectator::format`
Expected: FAIL — module doesn't exist yet

- [ ] **Step 3: Implement the entry formatter**

```rust
//! Entry formatting for spectator sweeps
//!
//! Converts HomeChannelEntry entries into a transcript string for the LLM.
//! Tiered detail: full text for messages, name-only for tools.
//! Heartbeats and cursors are filtered out.

use crate::channels::entry::{HomeChannelEntry, MessageEntry};

/// Format a single entry into a transcript line.
/// Returns None for entries that should be filtered out (heartbeats, cursors).
pub fn format_entry(entry: &HomeChannelEntry) -> Option<String> {
    match entry {
        HomeChannelEntry::Message(m) => format_message(m),
        HomeChannelEntry::Tool(t) => {
            match t.kind.as_str() {
                "tool_call" => Some(format!("[{}] tool_call: {}", t.id, t.tool_name)),
                "tool_result" => {
                    if let Some(ref file_path) = t.result_file {
                        Some(format!("[{}] tool_result({}): [file: {}]", t.id, t.tool_name, file_path))
                    } else {
                        let byte_count = t.result.as_ref().map_or(0, |r| r.len());
                        Some(format!("[{}] tool_result({}): [{} bytes]", t.id, t.tool_name, byte_count))
                    }
                }
                _ => None,
            }
        }
        HomeChannelEntry::Heartbeat(_) => None,
        HomeChannelEntry::Cursor(_) => None,
    }
}

/// Format a message entry with source tags.
/// Returns None for spectator's own messages (feedback loop prevention).
fn format_message(m: &MessageEntry) -> Option<String> {
    // Filter spectator's own observability messages
    if m.role == "system" && m.content.starts_with("[spectator]") {
        return None;
    }

    Some(match m.role.as_str() {
        "user" => {
            let author = m.author.as_deref().unwrap_or("unknown");
            match (&m.source_adapter, &m.source_channel_id, &m.source_channel_name) {
                (Some(adapter), Some(ch_id), Some(ch_name)) => {
                    format!("[{}] user:{}:{}/{} {}: {}", m.id, adapter, ch_id, ch_name, author, m.content)
                }
                (Some(adapter), Some(ch_id), None) => {
                    format!("[{}] user:{}:{} {}: {}", m.id, adapter, ch_id, author, m.content)
                }
                _ => format!("[{}] user: {}: {}", m.id, author, m.content),
            }
        }
        "agent" => format!("[{}] agent: {}", m.id, m.content),
        "bystander" => format!("[{}] bystander: {}", m.id, m.content),
        "system" => format!("[{}] system: {}", m.id, m.content),
        other => format!("[{}] {}: {}", m.id, other, m.content),
    })
}

/// Estimate tokens for a string (same heuristic as the rest of the codebase)
fn estimate_tokens(s: &str) -> usize {
    if s.is_empty() { return 0; }
    (s.len() + 3) / 4
}

/// Format entries with a token budget.
///
/// Returns (transcript, last_index) where last_index is the index of the
/// last entry included in the transcript. Entries are included oldest-first
/// until the budget is reached.
pub fn format_entries_budgeted(entries: &[HomeChannelEntry], token_budget: usize) -> (String, usize) {
    let mut lines = Vec::new();
    let mut tokens_used = 0;
    let mut last_idx = 0;

    for (i, entry) in entries.iter().enumerate() {
        let line = match format_entry(entry) {
            Some(l) => l,
            None => continue, // Filtered out
        };

        let line_tokens = estimate_tokens(&line);

        // Always include at least one entry
        if !lines.is_empty() && tokens_used + line_tokens > token_budget {
            break;
        }

        tokens_used += line_tokens;
        lines.push(line);
        last_idx = i;
    }

    (lines.join("\n"), last_idx)
}
```

- [ ] **Step 4: Add `pub mod format;` to spectator/mod.rs**

Add after `pub mod prompt;`:

```rust
pub mod format;
```

- [ ] **Step 5: Run tests and verify they pass**

Run: `cargo test -p river-gateway -- spectator::format`
Expected: PASS (all 10 tests)

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(spectator): entry formatter with tiered detail and token budgeting"
```

---

### Task 2: Move Storage (read/write moves.jsonl)

**Files:**
- Create: `crates/river-gateway/src/spectator/moves.rs`
- Modify: `crates/river-gateway/src/spectator/mod.rs` (add `pub mod moves;`)

- [ ] **Step 1: Write failing tests**

```rust
// In crates/river-gateway/src/spectator/moves.rs

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_append_and_read_moves() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        append_move(&path, "aaa", "bbb", "The agent set up the project.").await.unwrap();
        append_move(&path, "ccc", "ddd", "The user asked about auth.").await.unwrap();

        let moves = read_moves(&path).await;
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].start, "aaa");
        assert_eq!(moves[0].summary, "The agent set up the project.");
        assert_eq!(moves[1].start, "ccc");
    }

    #[tokio::test]
    async fn test_read_moves_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");
        let moves = read_moves(&path).await;
        assert!(moves.is_empty());
    }

    #[tokio::test]
    async fn test_read_moves_tail() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        for i in 0..20 {
            append_move(&path, &format!("s{:03}", i), &format!("e{:03}", i), &format!("Move {}", i)).await.unwrap();
        }

        let tail = read_moves_tail(&path, 5).await;
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0].summary, "Move 15");
        assert_eq!(tail[4].summary, "Move 19");
    }

    #[tokio::test]
    async fn test_read_cursor_from_moves() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        assert_eq!(read_cursor(&path).await, None);

        append_move(&path, "aaa", "bbb", "First move.").await.unwrap();
        assert_eq!(read_cursor(&path).await, Some("bbb".to_string()));

        append_move(&path, "ccc", "ddd", "Second move.").await.unwrap();
        assert_eq!(read_cursor(&path).await, Some("ddd".to_string()));
    }

    #[tokio::test]
    async fn test_read_moves_skips_malformed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        append_move(&path, "aaa", "bbb", "Good move.").await.unwrap();
        // Write a malformed line
        tokio::fs::OpenOptions::new().append(true).open(&path).await.unwrap();
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::OpenOptions::new().append(true).open(&path).await.unwrap();
        f.write_all(b"{bad json\n").await.unwrap();
        append_move(&path, "ccc", "ddd", "Another good move.").await.unwrap();

        let moves = read_moves(&path).await;
        assert_eq!(moves.len(), 2); // malformed line skipped
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p river-gateway -- spectator::moves`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement move storage**

```rust
//! Move storage — read/write moves.jsonl
//!
//! Moves are stored as one JSON object per line:
//! {"start":"...","end":"...","summary":"..."}

use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncWriteExt;

/// A single move entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveEntry {
    pub start: String,
    pub end: String,
    pub summary: String,
}

/// Append a move to the JSONL file
pub async fn append_move(
    path: &Path,
    start: &str,
    end: &str,
    summary: &str,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }

    let entry = MoveEntry {
        start: start.to_string(),
        end: end.to_string(),
        summary: summary.to_string(),
    };

    let mut json = serde_json::to_string(&entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    json.push('\n');

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;

    file.write_all(json.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

/// Read all moves from the JSONL file, skipping malformed lines
pub async fn read_moves(path: &Path) -> Vec<MoveEntry> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<MoveEntry>(l).ok())
        .collect()
}

/// Read the last N moves
pub async fn read_moves_tail(path: &Path, n: usize) -> Vec<MoveEntry> {
    let all = read_moves(path).await;
    let start = all.len().saturating_sub(n);
    all[start..].to_vec()
}

/// Read the cursor — the `end` snowflake of the last move
pub async fn read_cursor(path: &Path) -> Option<String> {
    let moves = read_moves(path).await;
    moves.last().map(|m| m.end.clone())
}
```

- [ ] **Step 4: Add `pub mod moves;` to spectator/mod.rs**

- [ ] **Step 5: Run tests and verify they pass**

Run: `cargo test -p river-gateway -- spectator::moves`
Expected: PASS (all 5 tests)

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(spectator): move storage — read/write/cursor for moves.jsonl"
```

---

### Task 3: Home Channel Reader (entries since cursor)

**Files:**
- Modify: `crates/river-gateway/src/channels/log.rs`

Add a method to read home channel entries after a given snowflake ID.

- [ ] **Step 1: Write failing test**

```rust
// Add to the existing tests in channels/log.rs

#[tokio::test]
async fn test_read_home_since() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("home.jsonl");
    let log = ChannelLog::from_path(path);

    use super::super::entry::{HomeChannelEntry, MessageEntry};

    // Write 5 entries
    for i in 0..5 {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            format!("{:032x}", i), format!("msg {}", i), "home".into(), None,
        ));
        log.append_entry(&entry).await.unwrap();
    }

    // Read since entry 1 (should get entries 2, 3, 4)
    let after_id = format!("{:032x}", 1);
    let entries = log.read_home_since(&after_id).await.unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].id(), format!("{:032x}", 2));
    assert_eq!(entries[2].id(), format!("{:032x}", 4));
}

#[tokio::test]
async fn test_read_home_since_none() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("home.jsonl");
    let log = ChannelLog::from_path(path);

    use super::super::entry::{HomeChannelEntry, MessageEntry};

    for i in 0..3 {
        let entry = HomeChannelEntry::Message(MessageEntry::agent(
            format!("{:032x}", i), format!("msg {}", i), "home".into(), None,
        ));
        log.append_entry(&entry).await.unwrap();
    }

    // Read since None (should get all entries)
    let entries = log.read_home_since_opt(None).await.unwrap();
    assert_eq!(entries.len(), 3);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-gateway -- channels::log::tests::test_read_home_since`
Expected: FAIL — method doesn't exist

- [ ] **Step 3: Implement read_home_since**

Add to `impl ChannelLog` in `channels/log.rs`:

```rust
    /// Read home channel entries after a given snowflake ID.
    /// Entries are compared lexicographically (bare hex snowflakes sort correctly).
    pub async fn read_home_since(&self, after_id: &str) -> std::io::Result<Vec<HomeChannelEntry>> {
        let all = self.read_all_home().await?;
        Ok(all.into_iter().filter(|e| e.id() > after_id).collect())
    }

    /// Read home channel entries after a given snowflake ID, or all entries if None.
    pub async fn read_home_since_opt(&self, after_id: Option<&str>) -> std::io::Result<Vec<HomeChannelEntry>> {
        match after_id {
            Some(id) => self.read_home_since(id).await,
            None => self.read_all_home().await,
        }
    }
```

- [ ] **Step 4: Run tests and verify they pass**

Run: `cargo test -p river-gateway -- channels::log::tests::test_read_home_since`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(channels): read_home_since for cursor-based home channel reads"
```

---

### Task 4: Rewrite SpectatorTask with Sweep Logic

**Files:**
- Modify: `crates/river-gateway/src/spectator/mod.rs`

This is the main rewrite — replace the no-op `handle_turn_complete` with the time-gated sweep.

- [ ] **Step 1: Update SpectatorConfig**

Replace the existing config with:

```rust
/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    /// Directory containing spectator prompt files
    pub spectator_dir: PathBuf,
    /// Path to the home channel JSONL
    pub home_channel_path: PathBuf,
    /// Path to moves.jsonl
    pub moves_path: PathBuf,
    /// Minimum time between sweeps (ignored during catch-up)
    pub sweep_interval: std::time::Duration,
    /// Max tokens for entries in a single sweep
    pub sweep_token_budget: usize,
    /// Number of recent moves to include as LLM context
    pub moves_tail: usize,
}

impl Default for SpectatorConfig {
    fn default() -> Self {
        Self {
            spectator_dir: PathBuf::from("spectator"),
            home_channel_path: PathBuf::from("channels/home/agent.jsonl"),
            moves_path: PathBuf::from("channels/home/agent/moves.jsonl"),
            sweep_interval: std::time::Duration::from_secs(300),
            sweep_token_budget: 16384,
            moves_tail: 10,
        }
    }
}
```

- [ ] **Step 2: Update SpectatorTask struct and constructor**

```rust
use crate::channels::log::ChannelLog;
use crate::channels::writer::HomeChannelWriter;
use crate::channels::entry::HomeChannelEntry;
use river_core::{SnowflakeGenerator, SnowflakeType};
use std::sync::Arc;

pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    model_client: ModelClient,
    home_channel_writer: HomeChannelWriter,
    snowflake_gen: Arc<SnowflakeGenerator>,
    /// Cached identity (system prompt)
    identity: String,
    /// Cached sweep prompt template
    on_sweep: Option<String>,
    /// Cached pressure prompt template
    on_pressure: Option<String>,
    /// Timestamp of last successful sweep
    last_sweep: Option<std::time::Instant>,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
        home_channel_writer: HomeChannelWriter,
        snowflake_gen: Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            config,
            bus,
            model_client,
            home_channel_writer,
            snowflake_gen,
            identity: String::new(),
            on_sweep: None,
            on_pressure: None,
            last_sweep: None,
        }
    }
```

- [ ] **Step 3: Rewrite the run loop**

```rust
    pub async fn run(mut self) {
        // Load identity — required
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

        // Load prompts
        self.on_sweep = prompt::load_prompt(
            &self.config.spectator_dir.join("on-sweep.md"),
        );
        self.on_pressure = prompt::load_prompt(
            &self.config.spectator_dir.join("on-pressure.md"),
        );

        tracing::info!(
            sweep = self.on_sweep.is_some(),
            pressure = self.on_pressure.is_some(),
            "Spectator handlers loaded"
        );

        let mut event_rx = self.bus.subscribe();
        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::TurnComplete { .. })) => {
                    self.maybe_sweep().await;
                }
                Ok(CoordinatorEvent::Agent(AgentEvent::ContextPressure { usage_percent, .. })) => {
                    self.handle_pressure(usage_percent).await;
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Spectator: shutdown received");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "Event receive error");
                }
            }
        }

        tracing::info!("Spectator task stopped");
    }
```

- [ ] **Step 4: Implement `maybe_sweep` and `sweep`**

```rust
    /// Check the time gate and sweep if enough time has passed
    async fn maybe_sweep(&mut self) {
        if self.on_sweep.is_none() {
            return;
        }

        // Check time gate (skip during catch-up — last_sweep is None on first run)
        if let Some(last) = self.last_sweep {
            if last.elapsed() < self.config.sweep_interval {
                return;
            }
        }

        // Run sweep loop (may iterate for catch-up)
        loop {
            let more = self.sweep().await;
            if !more {
                break;
            }
            // Catch-up: more entries exist, sweep again immediately
            tracing::info!("Catch-up sweep: more entries to process");
        }
    }

    /// Execute one sweep. Returns true if there are more entries to process (catch-up needed).
    async fn sweep(&mut self) -> bool {
        let template = match &self.on_sweep {
            Some(t) => t.clone(),
            None => return false,
        };

        // Read cursor
        let cursor = moves::read_cursor(&self.config.moves_path).await;

        // Read entries since cursor
        let log = ChannelLog::from_path(self.config.home_channel_path.clone());
        let entries = match log.read_home_since_opt(cursor.as_deref()).await {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(error = %e, "Failed to read home channel for sweep");
                return false;
            }
        };

        if entries.is_empty() {
            tracing::debug!("Sweep: no new entries");
            self.last_sweep = Some(std::time::Instant::now());
            return false;
        }

        // Format with token budget
        let (transcript, last_idx) = format::format_entries_budgeted(
            &entries, self.config.sweep_token_budget,
        );

        if transcript.is_empty() {
            // All entries were filtered (heartbeats/cursors/spectator messages)
            // Write a no-activity move to advance the cursor past them
            let first_id = entries[0].id().to_string();
            let last_id = entries.last().unwrap().id().to_string();
            if let Err(e) = moves::append_move(&self.config.moves_path, &first_id, &last_id, "[no activity]").await {
                tracing::error!(error = %e, "Failed to write no-activity move");
            }
            self.last_sweep = Some(std::time::Instant::now());
            return false;
        }

        let first_id = entries[0].id().to_string();
        let last_id = entries[last_idx].id().to_string();

        // Check if there are more non-filtered entries beyond what we included
        let remaining = &entries[last_idx + 1..];
        let has_more = remaining.iter().any(|e| format::format_entry(e).is_some());

        // Read recent moves for continuity
        let recent_moves = moves::read_moves_tail(&self.config.moves_path, self.config.moves_tail).await;
        let moves_text = if recent_moves.is_empty() {
            "No previous moves.".to_string()
        } else {
            recent_moves.iter().map(|m| m.summary.as_str()).collect::<Vec<_>>().join("\n\n")
        };

        // Build prompt
        let user_prompt = prompt::substitute(&template, &[
            ("recent_moves", &moves_text),
            ("entries", &transcript),
        ]);

        // Call LLM
        let summary = match self.call_model(&user_prompt).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(error = %e, "Sweep LLM call failed");
                return false;
            }
        };

        // Write move
        if let Err(e) = moves::append_move(&self.config.moves_path, &first_id, &last_id, &summary).await {
            tracing::error!(error = %e, "Failed to write move");
            return false;
        }

        // Write observability message to home channel
        let obs_msg = crate::channels::entry::MessageEntry::system_msg(
            self.snowflake_gen.next_id(SnowflakeType::Message).to_string(),
            format!("[spectator] move written covering entries {}-{}", first_id, last_id),
        );
        self.home_channel_writer.write(HomeChannelEntry::Message(obs_msg)).await;

        // Clean up tool result files
        HomeChannelWriter::cleanup_tool_results(
            &self.config.home_channel_path, &first_id, &last_id,
        ).await;

        // Emit event
        self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
            channel: "home".to_string(),
            timestamp: Utc::now(),
        }));

        self.last_sweep = Some(std::time::Instant::now());

        tracing::info!(
            start = %first_id,
            end = %last_id,
            summary_len = summary.len(),
            has_more = has_more,
            "Sweep complete — move written"
        );

        has_more
    }
```

- [ ] **Step 5: Keep existing `handle_pressure` and `call_model` methods unchanged**

They already work correctly.

- [ ] **Step 6: Update imports at the top of mod.rs**

```rust
use crate::channels::log::ChannelLog;
use crate::channels::writer::HomeChannelWriter;
use crate::channels::entry::HomeChannelEntry;
use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::model::ModelClient;
use chrono::Utc;
use river_core::{SnowflakeGenerator, SnowflakeType};
use std::path::PathBuf;
use std::sync::Arc;
```

- [ ] **Step 7: Remove the `handlers` module reference if no longer needed**

Check if `handlers.rs` (parse_moment_response) is used anywhere. If not, remove `pub mod handlers;`.

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p river-gateway`
Expected: compiles with no errors

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "feat(spectator): rewrite with time-gated sweep, one-sweep-one-move"
```

---

### Task 5: Wire Spectator in Server

**Files:**
- Modify: `crates/river-gateway/src/server.rs`

Pass the `HomeChannelWriter` to the spectator and update config construction.

- [ ] **Step 1: Update server.rs spectator construction**

```rust
    let spectator_config = SpectatorConfig {
        spectator_dir: config.workspace.join("spectator"),
        home_channel_path: config.workspace.join("channels/home").join(format!("{}.jsonl", agent_name)),
        moves_path: config.workspace.join("channels/home").join(&agent_name).join("moves.jsonl"),
        sweep_interval: Duration::from_secs(300),
        sweep_token_budget: 16384,
        moves_tail: 10,
    };

    // Clone the home channel writer for the spectator (app_state already has a clone)
    let spectator_home_writer = state.home_channel_writer.as_ref()
        .expect("Home channel writer must be configured")
        .clone();

    let spectator_task = SpectatorTask::new(
        spectator_config,
        coordinator.bus().clone(),
        spectator_model,
        spectator_home_writer,
        snowflake_gen.clone(),
    );
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p river-gateway`

- [ ] **Step 3: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(server): wire spectator with HomeChannelWriter and sweep config"
```

---

### Task 6: Update AgentTask::load_moves

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`

The current `load_moves` already reads `moves.jsonl` — but it should use the shared `moves` module for consistency.

- [ ] **Step 1: Update load_moves to use spectator::moves**

```rust
    /// Load move summaries from moves.jsonl
    async fn load_moves(&self) -> Vec<String> {
        let moves_path = self.config.workspace.join("channels").join("home")
            .join(&self.agent_name).join("moves.jsonl");

        crate::spectator::moves::read_moves_tail(&moves_path, 10)
            .await
            .into_iter()
            .map(|m| m.summary)
            .collect()
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-gateway`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "refactor(agent): load_moves uses spectator::moves module"
```

---

### Task 7: Create on-sweep.md Prompt File

**Files:**
- Create: `spectator/on-sweep.md` (in the agent's workspace, not in the crate)

This is the prompt template the spectator loads at startup. Without it, `on_sweep` is `None` and the spectator never sweeps.

- [ ] **Step 1: Create the prompt file**

Create `spectator/on-sweep.md` in the agent workspace (the path the spectator loads from):

```markdown
Below are the most recent move summaries from this agent's history, followed by new activity entries from the home channel that have not yet been summarized.

## Recent moves (for narrative continuity)

{recent_moves}

## New entries to summarize

{entries}

## Instructions

Write a narrative summary of what happened in the new entries above. Cover all significant topics — what the user asked for, what the agent did, what tools were used and why, what decisions were made, what the outcomes were.

Write in third person ("the agent", "the user"). Be concise but thorough — don't skip topics. If multiple distinct tasks happened, cover each one. No length limit — write as much as the content requires.

Output plain text only. No JSON, no markdown headers, no formatting beyond paragraphs.
```

- [ ] **Step 2: Commit**

```bash
git add -A && git commit -m "feat(spectator): create on-sweep.md prompt template"
```

Note: This file lives in the agent's workspace directory, not in the Rust crate. It will be loaded at runtime by the spectator from `{workspace}/spectator/on-sweep.md`.

---

### Task 8: Remove Dead Spectator Code

**Files:**
- Delete: `crates/river-gateway/src/spectator/handlers.rs` (if unused)
- Modify: `crates/river-gateway/src/spectator/mod.rs`

- [ ] **Step 1: Check if handlers.rs is used**

```bash
grep -rn "handlers::\|parse_moment" crates/river-gateway/src/ --include="*.rs" | grep -v "handlers.rs$" | grep -v test
```

If no results, delete it.

- [ ] **Step 2: Remove `pub mod handlers;` from mod.rs if deleted**

- [ ] **Step 3: Update mod.rs doc comment**

```rust
//! Spectator — event-driven sweep observer
//!
//! Listens for TurnComplete events, reads the home channel, and produces
//! narrative move summaries via LLM. One sweep, one move.
//! Moves stored in channels/home/{agent}/moves.jsonl.
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p river-gateway`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "cleanup(spectator): remove dead handlers module, update docs"
```
