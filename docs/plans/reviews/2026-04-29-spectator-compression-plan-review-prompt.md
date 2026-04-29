# Review Prompt: Spectator Compression Implementation Plan

Paste this prompt into Gemini with access to the river-engine codebase.

---

You are reviewing an implementation plan for reworking the spectator compression pipeline in a Rust workspace called river-engine. The plan is at `docs/plans/2026-04-29-spectator-compression.md`. The spec it implements is at `docs/specs/2026-04-29-spectator-compression-design.md`. Read both in full.

Your job is adversarial review. You are not here to confirm the plan looks reasonable. You are here to find every line of code that will not compile, every import that doesn't exist, every method signature that doesn't match the codebase, and every step that assumes something about the code that isn't true. The authors wrote this plan from partial file reads. You have the full codebase. Use it.

## Instructions

### 1. Verify every code block against the actual codebase

For every code block in the plan, check:

- **Do the imports exist?** If the plan writes `use river_db::Database;`, verify that `Database` is actually exported from `river_db`. If it writes `use crate::r#loop::context::ChatMessage;`, verify that `ChatMessage` has the methods being called (`::system()`, `::user()`, etc.) and the fields being accessed.
- **Do the method signatures match?** If the plan calls `self.model_client.complete(&messages, &[])`, verify that `ModelClient::complete` takes `&[ChatMessage]` and `&[ToolSchema]`. Check the actual parameter types.
- **Do the struct fields exist?** If the plan constructs a `Message { ..., turn_number: 1, ... }`, verify that every other field in the current `Message` struct is accounted for. A missing field means the code won't compile.
- **Do the SQL column positions match?** The plan changes `from_row` to read `turn_number` from a specific column index. Verify the column order in the SQL migration matches the index used in `from_row`. One off-by-one and every message read is corrupted.

List every code block that will not compile as written. Quote the plan, quote the codebase, explain the mismatch.

### 2. Trace the deletion impact

The plan deletes `compress.rs`, `curate.rs`, and `room.rs`. For each deleted file:

- Grep the entire codebase for every symbol exported from that file (`Compressor`, `Curator`, `RoomWriter`, `Compressor::new`, etc.)
- List every file that imports or references these symbols
- Verify the plan accounts for updating or removing every reference

If the plan deletes a file but doesn't update a file that imports from it, that's a compilation error the plan doesn't mention.

### 3. Verify the server.rs wiring

The plan's Task 7 says to update `server.rs` but gives incomplete code ("read the file to find the right insertion point"). This is where things break.

- Read the full `server.rs` from top to bottom
- Identify every line that references the old `SpectatorTask`, `SpectatorConfig`, `Compressor`, `Curator`, `RoomWriter`, `FlashQueue`, or `VectorStore` as passed to the spectator
- For each reference, determine exactly what the plan needs to change
- List every change the plan misses

The old `SpectatorTask::new()` takes `(SpectatorConfig, EventBus, ModelClient, Option<Arc<VectorStore>>, Arc<FlashQueue>)`. The new one takes `(SpectatorConfig, EventBus, ModelClient, Arc<Mutex<Database>>)`. Every argument mismatch is a compilation error.

### 4. Verify the AgentTask changes

The plan's Task 7 and Task 8 modify `agent/task.rs` but give incomplete code. Read the full `agent/task.rs`:

- Does `AgentTask` currently have a `db` field? If not, what needs to change in `new()`?
- Does `AgentTask` currently persist messages to the database? If not, how does it persist them? If through a different mechanism, does the plan account for this?
- Where exactly is `TurnComplete` emitted? What code runs before and after it? Is the plan's ordering guarantee actually achievable by moving one call, or does it require restructuring the turn cycle?
- Does `AgentTask::new()` get called anywhere besides `server.rs`? If so, every call site needs updating.

### 5. Find type mismatches across task boundaries

Tasks are written to be independent, but they share types. Check:

- Does the `Move` struct in Task 2 match exactly how it's used in Task 4 (`format_moves`) and Task 6 (`handle_turn_complete`)?
- Does the `Message` struct after Task 1's changes match how it's used in Task 4 (`format_transcript`, `fallback_summary`) and Task 6?
- The plan uses `river_core::AgentBirth::now()` in Task 6 to create a snowflake generator for move insertion. Does `AgentBirth::now()` exist? What does it return? Should the spectator be creating its own generator or sharing the one from `server.rs`?
- The plan uses `ChatMessage::system()` and `ChatMessage::user()` in `call_model()`. Do these constructors exist with these signatures? What do they return?

### 6. Find missing test updates

The plan deletes old spectator modules but the old tests live in `spectator/mod.rs`. The plan rewrites `mod.rs` but:

- Are there integration tests in `tests/` that reference old spectator types?
- Are there tests in other modules that construct `SpectatorConfig::from_workspace()` or `SpectatorTask::new()` with the old signature?
- Does the plan's Task 9 ("fix any failing tests") actually enumerate what needs fixing, or is it a hand-wave?

### 7. Grade the plan

- **Compilability** (A-F): If you execute every code block in order, does the project compile at each commit point?
- **Completeness** (A-F): Does every spec requirement have a corresponding task with code?
- **Accuracy** (A-F): Do the code blocks match the actual codebase's types, signatures, and patterns?
- **Independence** (A-F): Can each task be executed without knowledge not present in the plan?

For each grade below B, list exactly what's wrong and what would fix it.

## Output format

1. **Code blocks that won't compile** — numbered list with plan quote, codebase quote, explanation
2. **Deletion impact gaps** — table of deleted symbol, file that references it, whether plan accounts for it
3. **server.rs wiring gaps** — line-by-line diff of what needs to change vs what the plan says
4. **AgentTask gaps** — specific missing changes
5. **Type mismatches** — cross-task inconsistencies
6. **Missing test updates** — tests that will fail but aren't addressed
7. **Grades** — with justification

Be specific. Quote the plan. Quote the code. No hand-waving.
