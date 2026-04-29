# Adversarial Review: Spectator Compression Implementation Plan

**Reviewer:** Gemini CLI (Adversarial Agent)
**Date:** 2026-04-29
**Status:** **REJECTED (Critical Compilability & Logic Gaps)**

---

## 1. Code blocks that won't compile

### 1.1 `AgentBirth::now()` does not exist
*   **Plan Quote (Task 6):** `let gen = river_core::SnowflakeGenerator::new(river_core::AgentBirth::now());`
*   **Codebase Quote (`crates/river-core/src/snowflake/birth.rs`):** No `fn now()` exists.
*   **Explanation:** `AgentBirth` only provides `new(...)` and `from_raw(...)`. The generator must be initialized with the agent's actual birth date (retrieved from the DB or config) to maintain ID consistency.

### 1.2 `from_row` Column Mismatch
*   **Plan Quote (Task 1):** `turn_number: row.get::<_, i64>(7)? as u64, created_at: row.get(8)?`
*   **Codebase Quote (`crates/river-db/src/migrations/001_messages.sql`):** 
    ```sql
    created_at INTEGER NOT NULL, -- Currently index 7
    metadata TEXT                -- Currently index 8
    ```
*   **Explanation:** The plan gives conflicting instructions. It says "add [turn_number] after the metadata TEXT line" but then writes code assuming it's at index 7 (between `name` and `created_at`). If added at the end, `created_at` remains at 7. If added in the middle, index 8 becomes `created_at`. This will result in `Rusqlite` errors or data corruption during deserialization.

### 1.3 `ChatMessage` Constructor Signature
*   **Plan Quote (Task 6):** `ChatMessage::system(self.identity.clone())`
*   **Codebase Quote (`crates/river-gateway/src/loop/context.rs`):** 
    ```rust
    pub fn system(content: impl Into<String>) -> Self { ... }
    ```
*   **Explanation:** This actually compiles (lucky guess by the author), but the plan also calls `complete(&messages, &[])`. In `crates/river-gateway/src/loop/model.rs`, `complete` takes `&[ChatMessage]` and `&[ToolSchema]`. The plan passes an empty slice `&[]` which is correct, but requires `use crate::tools::ToolSchema` which is missing from the `mod.rs` rewrite.

---

## 2. Deletion Impact Gaps

The plan deletes `compress.rs`, `curate.rs`, and `room.rs`, but fails to account for the following references:

| Deleted Symbol | Reference File | Plan Status |
| :--- | :--- | :--- |
| `Compressor` | `crates/river-gateway/tests/iyou_test.rs` | **Ignored** (Will break build) |
| `RoomWriter` | `crates/river-gateway/tests/iyou_test.rs` | **Ignored** (Will break build) |
| `Curator` | `crates/river-gateway/tests/iyou_test.rs` | **Ignored** (Will break build) |
| `SpectatorConfig::from_workspace` | `crates/river-gateway/tests/iyou_test.rs` | **Ignored** (Will break build) |

---

## 3. `server.rs` Wiring Gaps

*   **Missing Imports:** The plan updates `SpectatorConfig` instantiation but doesn't add `use std::time::Duration;` or verify `SpectatorConfig` fields.
*   **Unused Arguments:** The new `SpectatorTask::new` signature removes `vector_store` and `flash_queue`. `server.rs` currently calculates these values; they will now be unused variables unless explicitly removed.
*   **Session ID Mismatch:** The plan hardcodes `session_id: "primary"`, but the codebase constant `PRIMARY_SESSION_ID` in `crates/river-gateway/src/session/mod.rs` is `"main"`.

---

## 4. `AgentTask` Gaps (CRITICAL)

*   **Missing Persistence Logic:** Task 8 ("Agent persist-before-emit") is a hand-wave. `AgentTask` currently **does not have a database handle** and **does not persist messages**.
*   **Codebase Reality:** Message persistence currently lives in `AgentLoop::persist_messages` (the legacy loop). `AgentTask` (the new task) is essentially a stateless relay that lacks the logic to write to the DB.
*   **Task 8 Failure:** The plan says "The exact code depends on how the agent currently persists messages." Since it *doesn't* persist them, the plan fails to provide the required ~50 lines of conversion and insertion logic needed to make the spectator actually work.

---

## 5. Type Mismatches

*   **Move Struct Inconsistency:** Task 2 defines `Move` with `tool_calls: Option<String>`, but Task 6 tries to insert it using `serde_json::to_string(tool_calls)`. While technically compatible, the `tool_calls` argument in `TurnComplete` is `Vec<String>`, which should probably be handled more explicitly in the DB layer.
*   **Snowflake Types:** The spectator creates a new `SnowflakeGenerator` for every turn. This is inefficient and risks collision if the system clock is manipulated, as it loses sequence state between calls. It should share the generator from `server.rs`.

---

## 6. Missing Test Updates

*   **`iyou_test.rs` is Dead:** This file contains 5+ integration tests that directly instantiate `Compressor` and `RoomWriter`. The plan completely ignores this file.
*   **`spectator/mod.rs` Tests:** The plan deletes the old tests but doesn't replace them with integration tests that verify the new prompt-driven behavior with a mock model.

---

## 7. Grades

*   **Compilability: F** — Will fail immediately on `AgentBirth::now()` and `iyou_test.rs` references.
*   **Completeness: D** — Fails to implement the actual persistence in `AgentTask` that the entire feature depends on.
*   **Accuracy: C** — Column indices in `from_row` are likely to be off-by-one based on the migration instructions.
*   **Independence: F** — Task 8 is impossible to execute without doing 2-3 turns of research not in the plan.

## Recommendation

**Rewrite Task 7 and 8 entirely.** `AgentTask` needs a full `persist_messages` implementation ported from `AgentLoop` before the spectator can be considered functional. Update `iyou_test.rs` or delete it.
