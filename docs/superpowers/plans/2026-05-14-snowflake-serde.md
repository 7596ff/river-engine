# Snowflake Serde Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the derived `Serialize`/`Deserialize` on `Snowflake` with custom hex-string serde, then propagate `Snowflake` as the ID type through all entry types, `MoveEntry`, and `ChannelNotification`, removing all `.to_string()` conversions at generation sites.

**Architecture:** Custom serde on `Snowflake` makes it serialize as a 32-char hex string (matching the existing `Display` impl), so JSONL format is unchanged. Then all `pub id: String` fields become `pub id: Snowflake`, constructors accept `Snowflake`, and generation sites pass the type through without conversion. A `from_hex` constructor and `to_datetime` method are added for parsing and timestamp extraction.

**Tech Stack:** Rust, serde (custom Serialize/Deserialize), chrono

---

### Task 1: Custom serde on `Snowflake` + `from_hex`

**Files:**
- Modify: `crates/river-core/src/snowflake/id.rs`
- Modify: `crates/river-core/Cargo.toml`

- [ ] **Step 1: Add `chrono` to river-core dependencies**

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
    let hex = format!("{}", id); // uses Display, which produces bare hex
    let parsed = Snowflake::from_hex(&hex).unwrap();
    assert_eq!(id, parsed);
}

#[test]
fn test_snowflake_from_hex_invalid() {
    assert!(Snowflake::from_hex("not_hex").is_err());
    assert!(Snowflake::from_hex("abc").is_err());
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

- [ ] **Step 6: Write failing test for custom serde**

Update the existing serde test and add a new one in `tests` module:

```rust
#[test]
fn test_snowflake_serde_hex_string() {
    let birth = test_birth();
    let id = Snowflake::new(999999, birth, SnowflakeType::ToolCall, 777);

    let json = serde_json::to_string(&id).unwrap();
    // Should serialize as a quoted hex string, not {"high":...,"low":...}
    assert!(json.starts_with('"'));
    assert!(json.ends_with('"'));
    assert_eq!(json.len(), 34); // 32 hex chars + 2 quotes

    let deserialized: Snowflake = serde_json::from_str(&json).unwrap();
    assert_eq!(id, deserialized);
}

#[test]
fn test_snowflake_serde_in_struct() {
    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct TestEntry {
        id: Snowflake,
        name: String,
    }

    let birth = test_birth();
    let entry = TestEntry {
        id: Snowflake::new(12345, birth, SnowflakeType::Message, 0),
        name: "test".to_string(),
    };

    let json = serde_json::to_string(&entry).unwrap();
    assert!(json.contains("\"id\":\""));
    assert!(!json.contains("\"high\""));

    let parsed: TestEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(entry, parsed);
}
```

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_serde`

Expected: FAIL — serde still produces `{"high":...,"low":...}`.

- [ ] **Step 8: Replace derived serde with custom impl**

In `crates/river-core/src/snowflake/id.rs`, change the struct definition from:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Snowflake {
```

to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Snowflake {
```

Then add the custom implementations after the existing `impl fmt::Display`:

```rust
impl serde::Serialize for Snowflake {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:016x}{:016x}", self.high, self.low))
    }
}

impl<'de> serde::Deserialize<'de> for Snowflake {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}
```

Remove the `use serde::{Deserialize, Serialize};` import at the top of the file (no longer needed for the derive; the impls use the full path `serde::Serialize` etc.). If other items in the file need the import, keep it.

- [ ] **Step 9: Run serde tests**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_serde`

Expected: PASS

- [ ] **Step 10: Delete the old serde roundtrip test**

Remove `test_snowflake_serde_roundtrip` from the tests module — it's been replaced by `test_snowflake_serde_hex_string`.

- [ ] **Step 11: Run all snowflake tests**

Run: `cargo test -p river-core -- snowflake`

Expected: All pass.

- [ ] **Step 12: Build the full workspace to check for breakage**

Run: `cargo build 2>&1 | head -40`

The `river-db` crate uses `to_bytes`/`from_bytes`, not serde, so it should be unaffected. The `subagent` module uses `Snowflake` in structs — check if they serialize. If any crate breaks, fix the compilation error (likely a struct that derived Serialize/Deserialize and embeds `Snowflake` — these will now serialize the hex string instead of the object, which is correct).

- [ ] **Step 13: Run full workspace tests**

Run: `cargo test 2>&1 | tail -20`

Fix any failures. The most likely failures are tests that check for `{"high":...,"low":...}` JSON format.

- [ ] **Step 14: Commit**

```bash
git add -A && git commit -m "feat(core): custom serde on Snowflake — hex string serialization, add from_hex"
```

---

### Task 2: `AgentBirth::to_epoch_micros` and `Snowflake::to_datetime`

**Files:**
- Modify: `crates/river-core/src/snowflake/birth.rs`
- Modify: `crates/river-core/src/snowflake/generator.rs`
- Modify: `crates/river-core/src/snowflake/id.rs`

- [ ] **Step 1: Write failing test for `to_epoch_micros`**

In `crates/river-core/src/snowflake/birth.rs`, add to the `tests` module:

```rust
#[test]
fn test_birth_to_epoch_micros() {
    // 2024-03-15 14:30:45 UTC
    let birth = AgentBirth::new(2024, 3, 15, 14, 30, 45).unwrap();
    let micros = birth.to_epoch_micros();
    // Verify via chrono
    use chrono::{TimeZone, Utc};
    let expected = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 45).unwrap();
    let expected_micros = expected.timestamp() as u64 * 1_000_000;
    assert_eq!(micros, expected_micros);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-core -- snowflake::birth::tests::test_birth_to_epoch_micros`

Expected: FAIL — `to_epoch_micros` doesn't exist.

- [ ] **Step 3: Implement `to_epoch_micros` on `AgentBirth`**

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

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-core -- snowflake::birth::tests::test_birth_to_epoch_micros`

Expected: PASS

- [ ] **Step 5: Update `SnowflakeGenerator::birth_to_micros` to delegate**

In `crates/river-core/src/snowflake/generator.rs`, replace the `birth_to_micros` method body:

```rust
fn birth_to_micros(birth: &AgentBirth) -> u64 {
    birth.to_epoch_micros()
}
```

Remove the `is_leap_year` method from `SnowflakeGenerator` — it's now inlined in `AgentBirth::to_epoch_micros`.

- [ ] **Step 6: Run generator tests**

Run: `cargo test -p river-core -- snowflake::generator`

Expected: PASS

- [ ] **Step 7: Write failing test for `to_datetime`**

In `crates/river-core/src/snowflake/id.rs`, add the import at the top:

```rust
use chrono::{DateTime, TimeZone, Utc};
```

Add to `tests` module:

```rust
#[test]
fn test_snowflake_to_datetime() {
    let birth = test_birth(); // 2024-03-15 14:30:45
    // 0 microseconds after birth = birth time
    let id = Snowflake::new(0, birth, SnowflakeType::Message, 0);
    let dt = id.to_datetime();
    assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-15 14:30:45");

    // 1 second after birth
    let id2 = Snowflake::new(1_000_000, birth, SnowflakeType::Message, 0);
    let dt2 = id2.to_datetime();
    assert_eq!(dt2.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-15 14:30:46");

    // 1 hour after birth
    let id3 = Snowflake::new(3_600_000_000, birth, SnowflakeType::Message, 0);
    let dt3 = id3.to_datetime();
    assert_eq!(dt3.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-15 15:30:45");
}
```

- [ ] **Step 8: Run test to verify it fails**

Run: `cargo test -p river-core -- snowflake::id::tests::test_snowflake_to_datetime`

Expected: FAIL — `to_datetime` doesn't exist.

- [ ] **Step 9: Implement `to_datetime`**

In `crates/river-core/src/snowflake/id.rs`, add the import at the top of the file (not in tests):

```rust
use chrono::{DateTime, TimeZone, Utc};
```

Add to `impl Snowflake`:

```rust
/// Compute the wall-clock time this snowflake was created.
///
/// Each snowflake encodes both the agent birth (low 36 bits) and
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

- [ ] **Step 11: Run all river-core tests**

Run: `cargo test -p river-core`

Expected: All pass.

- [ ] **Step 12: Commit**

```bash
git add -A && git commit -m "feat(core): add AgentBirth::to_epoch_micros and Snowflake::to_datetime"
```

---

### Task 3: Entry type IDs from `String` to `Snowflake`

**Files:**
- Modify: `crates/river-gateway/src/channels/entry.rs`

This is the largest task. Every entry type's `id` field changes from `String` to `Snowflake`, and all constructors, accessors, and tests update accordingly.

- [ ] **Step 1: Change `MessageEntry.id` to `Snowflake`**

In `crates/river-gateway/src/channels/entry.rs`, change:

```rust
pub struct MessageEntry {
    /// Snowflake ID — unique, sortable, encodes timestamp
    pub id: String,
```

to:

```rust
use river_core::Snowflake;

pub struct MessageEntry {
    /// Snowflake ID — unique, sortable, encodes timestamp
    pub id: Snowflake,
```

- [ ] **Step 2: Update all `MessageEntry` constructors**

Change every constructor's `id` parameter from `String` to `Snowflake`:

```rust
pub fn incoming(id: Snowflake, author: String, ...) -> Self { ... }
pub fn agent(id: Snowflake, content: String, ...) -> Self { ... }
pub fn user_home(id: Snowflake, author: String, ...) -> Self { ... }
pub fn bystander(id: Snowflake, content: String) -> Self { ... }
pub fn system_msg(id: Snowflake, content: String) -> Self { ... }
```

No other changes inside the constructors — `id` is just assigned to the field.

- [ ] **Step 3: Change `ToolEntry.id` to `Snowflake`**

```rust
pub struct ToolEntry {
    pub id: Snowflake,
```

Update constructors:

```rust
pub fn call(id: Snowflake, tool_name: String, ...) -> Self { ... }
pub fn result(id: Snowflake, tool_name: String, ...) -> Self { ... }
pub fn result_file(id: Snowflake, tool_name: String, ...) -> Self { ... }
```

- [ ] **Step 4: Change `HeartbeatEntry.id` to `Snowflake`**

```rust
pub struct HeartbeatEntry {
    pub id: Snowflake,
```

Update constructor:

```rust
pub fn new(id: Snowflake, timestamp: String) -> Self { ... }
```

- [ ] **Step 5: Change `CursorEntry.id` to `Snowflake`**

```rust
pub struct CursorEntry {
    pub id: Snowflake,
```

Update constructor:

```rust
pub fn new(id: Snowflake) -> Self { ... }
```

- [ ] **Step 6: Update `HomeChannelEntry::id()` return type**

Change from `&str` to `Snowflake`:

```rust
impl HomeChannelEntry {
    pub fn id(&self) -> Snowflake {
        match self {
            HomeChannelEntry::Message(m) => m.id,
            HomeChannelEntry::Cursor(c) => c.id,
            HomeChannelEntry::Tool(t) => t.id,
            HomeChannelEntry::Heartbeat(h) => h.id,
        }
    }
}
```

Note: `Snowflake` is `Copy`, so returning by value is correct.

- [ ] **Step 7: Update `ChannelEntry::id()` return type**

```rust
impl ChannelEntry {
    pub fn id(&self) -> Snowflake {
        match self {
            ChannelEntry::Message(m) => m.id,
            ChannelEntry::Cursor(c) => c.id,
        }
    }
}
```

- [ ] **Step 8: Fix all tests in `entry.rs`**

Every test that constructs an entry with a string ID needs a real `Snowflake`. Add a helper at the top of the `tests` module:

```rust
use river_core::{AgentBirth, Snowflake, SnowflakeType};

fn test_snowflake() -> Snowflake {
    let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
    Snowflake::new(0, birth, SnowflakeType::Message, 0)
}

fn test_snowflake_seq(seq: u32) -> Snowflake {
    let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
    Snowflake::new(0, birth, SnowflakeType::Message, seq)
}
```

Then replace every `"ABC123".to_string()`, `"001".into()`, `"m1".into()`, etc. with `test_snowflake()` or `test_snowflake_seq(N)` for tests that need distinct IDs.

For ID assertions, change from:
```rust
assert_eq!(parsed.id, "ABC123");
```
to:
```rust
assert_eq!(parsed.id, test_snowflake());
```

For `id()` accessor assertions, change from:
```rust
assert_eq!(parsed.id(), "001");
```
to:
```rust
assert_eq!(parsed.id(), test_snowflake());
```

- [ ] **Step 9: Run entry tests**

Run: `cargo test -p river-gateway -- channels::entry`

Expected: PASS

- [ ] **Step 10: Compile the full workspace**

Run: `cargo build 2>&1 | head -60`

This will produce many errors in files that pass strings to entry constructors. **Do not fix them yet** — they are fixed in subsequent tasks. Verify that `entry.rs` itself compiles.

Run: `cargo test -p river-gateway -- channels::entry 2>&1 | tail -10`

Expected: Entry tests pass even if the full crate doesn't build yet.

- [ ] **Step 11: Commit**

```bash
git add -A && git commit -m "feat(channels): entry type IDs from String to Snowflake"
```

---

### Task 4: Fix all generation sites

**Files:**
- Modify: `crates/river-gateway/src/agent/task.rs`
- Modify: `crates/river-gateway/src/api/routes.rs`
- Modify: `crates/river-gateway/src/spectator/mod.rs`
- Modify: `crates/river-gateway/src/tools/adapters.rs`
- Modify: `crates/river-gateway/src/tools/sync.rs`

Every site that does `next_id(...).to_string()` becomes just `next_id(...)`.

- [ ] **Step 1: Fix `agent/task.rs`**

Four sites. Change each from `self.snowflake_gen.next_id(SnowflakeType::Message).to_string()` to `self.snowflake_gen.next_id(SnowflakeType::Message)`.

Line ~190 (heartbeat):
```rust
let hb = HeartbeatEntry::new(
    self.snowflake_gen.next_id(SnowflakeType::Message),
    Utc::now().to_rfc3339(),
);
```

Line ~276 (agent message):
```rust
let entry = MessageEntry::agent(
    self.snowflake_gen.next_id(SnowflakeType::Message),
    content.clone(), "home".to_string(), None,
);
```

Line ~296 (tool call):
```rust
let entry = ToolEntry::call(
    self.snowflake_gen.next_id(SnowflakeType::Message),
    tc.function.name.clone(),
    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null),
    tc.id.clone(),
);
```

Line ~312 (tool result) — this one also uses the snowflake for the file name:
```rust
let snowflake = self.snowflake_gen.next_id(SnowflakeType::Message);
let entry = if result.result.len() > 4096 {
    let results_dir = self.config.workspace.join("channels").join("home")
        .join(&self.agent_name).join("tool-results");
    tokio::fs::create_dir_all(&results_dir).await.ok();
    let file_path = results_dir.join(format!("{}.txt", snowflake));
    tokio::fs::write(&file_path, &result.result).await.ok();
    ToolEntry::result_file(
        snowflake, result.tool_name.clone(),
        file_path.to_string_lossy().to_string(), result.tool_call_id.clone(),
    )
} else {
    ToolEntry::result(
        snowflake, result.tool_name.clone(),
        result.result.clone(), result.tool_call_id.clone(),
    )
};
```

Note: `format!("{}.txt", snowflake)` still works because `Snowflake` implements `Display`.

- [ ] **Step 2: Fix `api/routes.rs`**

Two sites. Remove the `snowflake_str` intermediary.

Site 1 (~line 202, handle_incoming):
```rust
let snowflake = state.snowflake_gen.next_id(river_core::SnowflakeType::Message);

if let Some(ref writer) = state.home_channel_writer {
    let home_entry = crate::channels::entry::MessageEntry::user_home(
        snowflake,
        msg.author.name.clone(),
        ...
    );
    ...
}

let entry = crate::channels::MessageEntry::incoming(
    snowflake,
    msg.author.name.clone(),
    ...
);

...

state.message_queue.push(crate::queue::ChannelNotification {
    channel: channel_key.clone(),
    id: snowflake,
});
```

Site 2 (~line 279, handle_bystander):
```rust
let snowflake = state.snowflake_gen.next_id(river_core::SnowflakeType::Message);

let entry = crate::channels::entry::MessageEntry::bystander(
    snowflake, msg.content,
);

...

state.message_queue.push(crate::queue::ChannelNotification {
    channel: "home".to_string(),
    id: snowflake,
});

tracing::info!(id = %snowflake, "Bystander message received");

Ok(Json(serde_json::json!({ "ok": true, "id": snowflake.to_string() })))
```

Note: the API JSON response still needs a string — use `snowflake.to_string()` only there.

- [ ] **Step 3: Fix `spectator/mod.rs`**

Line ~247:
```rust
let obs_msg = crate::channels::entry::MessageEntry::system_msg(
    self.snowflake_gen.next_id(SnowflakeType::Message),
    format!("[spectator] move written covering entries {}-{}", first_id, last_id),
);
```

Also fix the `first_id`/`last_id` sites (~lines 200, 210) — these call `.id().to_string()`. Since `id()` now returns `Snowflake`, and `Snowflake` implements `Display`:

```rust
let first_id = entries[0].id();
let last_id = entries.last().unwrap().id();
```

These are now `Snowflake` values, passed to `append_move` (fixed in Task 5).

- [ ] **Step 4: Fix `tools/adapters.rs`**

Line ~120-123:
```rust
let snowflake = snowflake_gen.next_id(SnowflakeType::Message);
let log = crate::channels::ChannelLog::open(channels_dir, adapter, channel_id);
let agent_entry = crate::channels::MessageEntry::agent(
    snowflake,
    content.to_string(),
    adapter.to_string(),
    adapter_msg_id,
);
```

- [ ] **Step 5: Fix `tools/sync.rs`**

Lines ~122-138:
```rust
let snowflake = snowflake_gen.next_id(SnowflakeType::Message);
let entry = if fetched.is_bot {
    crate::channels::MessageEntry::agent(
        snowflake,
        fetched.content.clone(),
        adapter.clone(),
        Some(fetched.id.clone()),
    )
} else {
    crate::channels::MessageEntry::incoming(
        snowflake,
        fetched.author_name.clone(),
        fetched.author_id.clone(),
        fetched.content.clone(),
        adapter.clone(),
        Some(fetched.id.clone()),
    )
};
```

- [ ] **Step 6: Compile**

Run: `cargo build -p river-gateway 2>&1 | head -40`

There will still be errors from `queue.rs`, `moves.rs`, `log.rs`, and `writer.rs` tests. Those are fixed in subsequent tasks.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "refactor: remove .to_string() at all snowflake generation sites"
```

---

### Task 5: `MoveEntry` fields from `String` to `Snowflake`

**Files:**
- Modify: `crates/river-gateway/src/spectator/moves.rs`
- Modify: `crates/river-gateway/src/spectator/mod.rs`

- [ ] **Step 1: Change `MoveEntry` fields**

In `crates/river-gateway/src/spectator/moves.rs`:

```rust
use river_core::Snowflake;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveEntry {
    pub start: Snowflake,
    pub end: Snowflake,
    pub summary: String,
}
```

- [ ] **Step 2: Update `append_move` signature**

```rust
pub async fn append_move(
    path: &Path,
    start: Snowflake,
    end: Snowflake,
    summary: &str,
) -> std::io::Result<()> {
    ...
    let entry = MoveEntry {
        start,
        end,
        summary: summary.to_string(),
    };
    ...
}
```

- [ ] **Step 3: Update `read_cursor` return type**

```rust
pub async fn read_cursor(path: &Path) -> Option<Snowflake> {
    let moves = read_moves(path).await;
    moves.last().map(|m| m.end)
}
```

Note: `Snowflake` is `Copy`, so `.clone()` becomes just the value.

- [ ] **Step 4: Fix move tests**

In the `tests` module, replace string IDs with real snowflakes:

```rust
use river_core::{AgentBirth, Snowflake, SnowflakeType};

fn test_snowflake(seq: u32) -> Snowflake {
    let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
    Snowflake::new(seq as u64 * 1_000_000, birth, SnowflakeType::Message, 0)
}

#[tokio::test]
async fn test_append_and_read_moves() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("moves.jsonl");

    append_move(&path, test_snowflake(1), test_snowflake(2), "The agent set up the project.").await.unwrap();
    append_move(&path, test_snowflake(3), test_snowflake(4), "The user asked about auth.").await.unwrap();

    let moves = read_moves(&path).await;
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].start, test_snowflake(1));
    ...
}
```

Update `test_read_cursor_from_moves` and `test_read_moves_skips_malformed` similarly.

- [ ] **Step 5: Update `read_home_since` in `log.rs`**

In `crates/river-gateway/src/channels/log.rs`, change the signature:

```rust
pub async fn read_home_since(&self, after_id: Snowflake) -> std::io::Result<Vec<HomeChannelEntry>> {
    let all = self.read_all_home().await?;
    Ok(all.into_iter().filter(|e| e.id() > after_id).collect())
}

pub async fn read_home_since_opt(&self, after_id: Option<Snowflake>) -> std::io::Result<Vec<HomeChannelEntry>> {
    match after_id {
        Some(id) => self.read_home_since(id).await,
        None => self.read_all_home().await,
    }
}
```

Update the spectator's call site to pass `cursor` as `Option<Snowflake>` (from `read_cursor`).

- [ ] **Step 6: Update spectator `sweep` method**

In `crates/river-gateway/src/spectator/mod.rs`, the cursor and `first_id`/`last_id` are now `Snowflake`:

```rust
let cursor = moves::read_cursor(&self.config.moves_path).await;
let entries = match log.read_home_since_opt(cursor).await {
    ...
};
```

And:

```rust
let first_id = entries[0].id();
let last_id = entries[last_idx].id();
```

The `append_move` calls:

```rust
moves::append_move(&self.config.moves_path, first_id, last_id, &summary).await
```

The `cleanup_tool_results` call needs updating too — its parameters change from `&str` to `Snowflake`.

- [ ] **Step 7: Update `cleanup_tool_results` signature**

In `crates/river-gateway/src/channels/writer.rs`:

```rust
pub async fn cleanup_tool_results(
    home_channel_path: &Path,
    move_start: Snowflake,
    move_end: Snowflake,
) {
    ...
    if t.id >= move_start && t.id <= move_end {
    ...
}
```

- [ ] **Step 8: Fix log.rs tests**

Update tests that use string IDs in `read_home_since` calls and entry construction.

- [ ] **Step 9: Fix writer.rs tests**

Update tests that construct entries with string IDs.

- [ ] **Step 10: Compile and test**

Run: `cargo build -p river-gateway 2>&1 | head -40`
Run: `cargo test -p river-gateway -- spectator`
Run: `cargo test -p river-gateway -- channels`

Expected: All pass.

- [ ] **Step 11: Commit**

```bash
git add -A && git commit -m "refactor: MoveEntry, cursor, and log comparison use Snowflake type"
```

---

### Task 6: `ChannelNotification` and remaining cleanup

**Files:**
- Modify: `crates/river-gateway/src/queue.rs`
- Modify: `crates/river-gateway/src/tools/memory.rs`

- [ ] **Step 1: Change `ChannelNotification.snowflake_id` to `id: Snowflake`**

In `crates/river-gateway/src/queue.rs`:

```rust
use river_core::Snowflake;

#[derive(Debug, Clone)]
pub struct ChannelNotification {
    pub channel: String,
    pub id: Snowflake,
}
```

- [ ] **Step 2: Fix queue tests**

```rust
use river_core::{AgentBirth, Snowflake, SnowflakeType};

fn test_snowflake(seq: u32) -> Snowflake {
    let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
    Snowflake::new(seq as u64 * 1_000_000, birth, SnowflakeType::Message, 0)
}

#[test]
fn test_push_and_drain() {
    let queue = MessageQueue::new();

    queue.push(ChannelNotification {
        channel: "discord_general".to_string(),
        id: test_snowflake(1),
    });
    queue.push(ChannelNotification {
        channel: "discord_general".to_string(),
        id: test_snowflake(2),
    });

    let notifications = queue.drain();
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0].id, test_snowflake(1));
    assert_eq!(notifications[1].id, test_snowflake(2));
    ...
}
```

Update `test_fifo_order` and `test_thread_safety` similarly — use `test_snowflake(i)` instead of `format!("{}", i)`.

- [ ] **Step 3: Remove `parse_snowflake_id` from `tools/memory.rs`**

Delete the `parse_snowflake_id` function. At its call site (~line 227), the tool receives the ID as a string from tool arguments. Use `Snowflake::from_hex` instead:

```rust
let id = Snowflake::from_hex(id_str)
    .map_err(|e| RiverError::tool(format!("Invalid snowflake ID: {}", e)))?;
```

- [ ] **Step 4: Fix any remaining references to `snowflake_id`**

Search for `snowflake_id` across the codebase and update to `id`:

```bash
grep -rn "snowflake_id" crates/ --include="*.rs"
```

Fix every hit.

- [ ] **Step 5: Full workspace build**

Run: `cargo build 2>&1 | tail -10`

Expected: Clean build.

- [ ] **Step 6: Full workspace tests**

Run: `cargo test 2>&1 | tail -20`

Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "refactor: ChannelNotification.id is Snowflake, remove parse_snowflake_id"
```

---

### Task 7: Final verification

**Files:**
- No new files

- [ ] **Step 1: Audit for remaining string IDs**

```bash
grep -rn "pub id: String" crates/ --include="*.rs"
grep -rn "\.to_string()" crates/ --include="*.rs" | grep -i "snowflake\|next_id"
grep -rn "snowflake_id" crates/ --include="*.rs"
```

Expected: No hits for any of these (except in comments, if any).

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings 2>&1 | head -20`

Fix any warnings.

- [ ] **Step 3: Run full test suite**

Run: `cargo test`

Expected: All pass.

- [ ] **Step 4: Verify JSONL format is unchanged**

The custom serde produces the same hex strings. Check by running a gateway test that writes to a home channel JSONL and reading the output:

Run: `cargo test -p river-gateway -- channels::writer`

Verify the JSONL contains `"id":"<32-char-hex>"` format, not `"id":{"high":...,"low":...}`.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "cleanup: verify no remaining String snowflake IDs, clippy clean"
```
