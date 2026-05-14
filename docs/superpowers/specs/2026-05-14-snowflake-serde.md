# Snowflake Serde — String Serialization and Type Propagation

## Goal

Replace all `String` snowflake ID fields with the actual `Snowflake` type. Add a custom serde implementation that serializes `Snowflake` as its 32-character hex string representation, so JSONL format is unchanged and the type carries through the entire system.

## Problem

The `Snowflake` type is a 128-bit ID that encodes timestamp, agent birth, type, and sequence. It derives `Serialize`/`Deserialize`, which produces `{"high": N, "low": N}` — a JSON object with two numbers. This is useless for JSONL storage where IDs need to be strings.

The current workaround: every generation site calls `.to_string()` immediately, and all entry types store `pub id: String`. This throws away the type — you lose ordering (`Snowflake` implements `Ord`), you lose direct timestamp extraction (need to parse hex back), and you lose type safety (any string can go in an ID field).

The DB layer got this right — `Message.id`, `Move.id`, `Memory.id` are all `Snowflake`, stored as 16-byte blobs. The channel entry layer got it wrong.

## Solution

### 1. Custom serde on `Snowflake`

Replace the derived `Serialize`/`Deserialize` with a custom implementation that uses the 32-char hex string:

```rust
// Serializes as: "0000000f42400000a0e5b40100100000"
// Deserializes from the same
impl Serialize for Snowflake { ... }
impl Deserialize for Snowflake { ... }
```

This is a **breaking change** for anything that currently serializes `Snowflake` as `{"high": N, "low": N}`. Audit required:

- `river-db` stores snowflakes as 16-byte blobs via `to_bytes`/`from_bytes`, not via serde. **Not affected.**
- `subagent/types.rs` uses `Snowflake` but the subagent types are serialized for internal communication, not persisted. Verify.
- The `Snowflake` serde roundtrip test in `id.rs` will break and needs updating.

### 2. Add `Snowflake::from_hex`

Parse a 32-char hex string into a `Snowflake`. The custom `Deserialize` impl uses this internally. Also useful as a public API for the TUI and anywhere else that receives hex strings.

### 3. Add `Snowflake::to_datetime`

Compute wall-clock time from the embedded birth + timestamp. Needs `AgentBirth::to_epoch_micros` (currently private in `SnowflakeGenerator`, needs to move to `AgentBirth`).

### 4. Change entry type ID fields from `String` to `Snowflake`

All four entry types:

| Type | Field | Before | After |
|---|---|---|---|
| `MessageEntry` | `id` | `String` | `Snowflake` |
| `ToolEntry` | `id` | `String` | `Snowflake` |
| `HeartbeatEntry` | `id` | `String` | `Snowflake` |
| `CursorEntry` | `id` | `String` | `Snowflake` |

Constructors change from accepting `String` to accepting `Snowflake`. The `id()` accessor on `HomeChannelEntry` and `ChannelEntry` returns `Snowflake` (Copy type) instead of `&str`.

JSONL format is unchanged — the custom serde serializes `Snowflake` as a hex string, same as what `.to_string()` produced before.

### 5. Change `MoveEntry` fields from `String` to `Snowflake`

```rust
pub struct MoveEntry {
    pub start: Snowflake,
    pub end: Snowflake,
    pub summary: String,
}
```

`append_move` takes `Snowflake` instead of `&str`. `read_cursor` returns `Option<Snowflake>`.

### 6. Change `ChannelNotification` from `String` to `Snowflake`

```rust
pub struct ChannelNotification {
    pub channel: String,
    pub id: Snowflake,
}
```

### 7. Remove `.to_string()` calls at generation sites

Every `next_id(SnowflakeType::Message).to_string()` becomes just `next_id(SnowflakeType::Message)`. The `Snowflake` flows through as a type.

Sites:
- `api/routes.rs` — two `handle_incoming` sites
- `agent/task.rs` — four sites (agent message, tool call, tool result, heartbeat)
- `spectator/mod.rs` — one site (bystander message)
- `tools/adapters.rs` — one site
- `tools/sync.rs` — two sites

### 8. Fix cursor comparison

`read_home_since` currently compares `e.id() > after_id` as string comparison. With `Snowflake`, this becomes `e.id() > after_id` using `Snowflake`'s `Ord` implementation, which compares by timestamp first, then low bits. Same behavior (hex sorts identically to the Ord impl), but now type-safe.

### 9. Remove `parse_snowflake_id`

The `parse_snowflake_id` function in `tools/memory.rs` expects a dash-separated format (`high-low`) that doesn't match the current bare hex format. It's a dead format. Replace with `Snowflake::from_hex` where needed, or remove entirely since IDs will already be `Snowflake` values.

## What changes in JSONL

Nothing. Before:

```json
{"type":"message","id":"0000000f42400000a0e5b40100100000","role":"agent","content":"hello","adapter":"home"}
```

After: identical. The custom serde produces the same hex string the `.to_string()` call did.

## What changes in the DB

Nothing. The DB uses `to_bytes`/`from_bytes`, not serde.

## What changes in tests

- Entry type tests that construct IDs as arbitrary strings (`"001"`, `"ABC123"`, etc.) need to use real `Snowflake` values instead. No more `"001".to_string()` — must be a valid 32-char hex snowflake.
- Queue tests that use `"first"`, `"second"`, `"third"` as IDs need real snowflakes.
- The `Snowflake` serde roundtrip test needs updating (now serializes as hex string, not object).

## Dependency on TUI plan

The TUI plan (Task 1) adds `Snowflake::from_hex` and `to_datetime`. This spec supersedes that — all the snowflake work happens here, and the TUI plan's Task 1 becomes unnecessary. The TUI plan's Task 2 (move entry types to river-core) should happen after this, with the types already using `Snowflake` instead of `String`.

## Order

1. Custom serde on `Snowflake` (hex string serialization)
2. `from_hex` and `to_datetime` methods
3. `AgentBirth::to_epoch_micros` (move from generator)
4. Entry type ID fields → `Snowflake`
5. `MoveEntry` fields → `Snowflake`
6. `ChannelNotification` → `Snowflake`
7. Remove `.to_string()` at generation sites
8. Fix cursor comparison types
9. Remove `parse_snowflake_id`
10. Fix all tests
