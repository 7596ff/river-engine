# Home Channel Implementation Plan Review

**Plan Reviewed:** `docs/superpowers/plans/2026-05-12-home-channel.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## 1. Compilation Order

The plan has a significant "bridge to nowhere" problem between Tasks 5 and 7.

*   **Task 5 vs Task 7:** Task 5 modifies `AgentTask` and removes "all channel switching logic," but `PersistentContext` (which is still in `AgentTask` at this stage) likely has dependencies on `ChannelContext` or at least assumes a single-channel world.
*   **Task 3 vs Task 5:** Task 3 builds the `home_context` builder, but Task 5 **never actually wires it into the `AgentTask`**. Task 5 steps still refer to "line 391" (the old `self.context.append` logic). This means at the end of Task 5, you are writing to the Home Channel but still building context from the old `PersistentContext` (and thus the SQL DB).
*   **The Switchover:** There is no task that changes the type of `self.context` from `PersistentContext` to something compatible with `home_context.rs`, or replaces it with a `build_context` call.

## 2. The `serde(untagged)` Problem

The use of `#[serde(untagged)]` in `ChannelEntry` (Task 1, Step 2) is a ticking time bomb for deserialization performance and correctness.

*   **Ambiguity:** As more variants are added, the risk of a `MessageEntry` being misparsed as a `ToolEntry` (if fields like `id` are present and others are optional) increases. 
*   **Performance:** Serde must attempt to deserialize into every variant until one succeeds.
*   **Recommendation:** Switch to `#[serde(tag = "type")]` now. Since this is a new feature with a new file path (`channels/home/`), you can afford the clean break from the old per-adapter JSONL format.

## 3. The Tool Name Threading Gap

Task 5, Step 4 explicitly ignores the tool name: `tool_name: "unknown".to_string()`.

*   **Impact:** This isn't just a placeholder; it's a permanent data loss in the Home Channel. The spectator and the model (during context building) will see that "something" happened but won't know which tool produced which result without complex cross-referencing.
*   **Correction:** Task 5 **must** include a refactor of `AgentTask::execute_tool_calls` to return `Vec<(String, String, String)>` (ID, Name, Result) or a `ToolResult` struct.

## 4. The Context Builder's Incomplete Tool Call Mapping

Task 3, Step 1 "hand-waves" the mapping of `ToolEntry::call` to `ChatMessage`.

*   **Complexity:** Reconstructing a `ToolCallRequest` inside a `ChatMessage::assistant` is the most error-prone part of the migration. If the `arguments` or `tool_name` don't perfectly match what the model sent, the conversation will error out on the next turn.
*   **Missing Step:** Task 3 should explicitly define how `ToolEntry` fields map to the `ToolCall` and `FunctionCall` structs used by the model client.

## 5. The `MessageEntry::user` Tag Format

Task 1, Step 3 prepends the source tag directly to the `content` string.

*   **Data Purity:** This makes the `content` field "dirty." If a future feature (like a search index or a per-adapter log viewer) wants the raw user message, it must regex-parse the tag out.
*   **Recommendation:** Add `source_adapter`, `source_channel_id`, and `source_channel_name` as optional fields on `MessageEntry`. The Context Builder (Task 3) should be responsible for formatting these into the string seen by the model.

## 6. The Write-Ahead Without Transaction

Task 6, Step 1 writes to the Home Channel via an async MPSC channel (`state.home_channel_writer.write`).

*   **Race Condition:** Because `write()` is a fire-and-forget send to the actor, the Home Channel write hasn't actually reached the disk when the code proceeds to write to the adapter log. If the gateway crashes 1ms after Task 6 Step 1, the "secondary" adapter log might actually be written while the "primary" Home Channel entry is lost in the MPSC buffer.
*   **Recommendation:** If the Home Channel is the true source of truth, `HomeChannelWriter::write` should optionally allow for an `ack` (using a oneshot channel) to ensure the write reached the WAL before continuing.

## 7. The `PersistentContext` Removal Gap

As noted in point 1, `PersistentContext` is never removed. 

*   **Intermediate State:** The plan leaves the system in a "Triple-Write" state: Home Channel, Adapter Log, and SQL DB (via `persist_turn_messages`).
*   **Fix:** Task 5 or 7 must explicitly delete `crates/river-gateway/src/agent/context.rs` and remove the `PersistentContext` field from `AgentTask`.

## 8. The SQL Removal Deferral

Task 8 (Spectator) and Task 10 (Architecture Summary) are at odds.

*   **Spectator Moves:** The current spectator writes moves to SQL. If SQL is removed, Task 8 must define where moves are stored (e.g., `channels/home/{agent}/moves.jsonl`). The plan mentions "reading spectator moves" in Task 3 but doesn't say *where* they are read from.
*   **Gap:** The plan needs a task to migrate Move storage from SQL to the filesystem.

## 9. Missing: Home Channel Initialization

The plan handles creation on write (Task 10), which is fine for new agents.

*   **Migration:** There is no "Catch-up" task for existing agents. When the new code starts, existing agents will have a SQL history but an empty Home Channel. The context builder will start from a "blank slate," effectively giving all existing agents amnesia.
*   **Fix:** Task 10 should include a one-time migration script or logic to populate the Home Channel from the SQL `messages` table for existing sessions.

## 10. Test Coverage Gaps

*   **Task 5 Logic:** The "Final Batch Check" and the loop-back to "Step 3" (Model Completion) are complex state machine changes. They require a specific integration test that simulates a message arriving *during* a model completion call.
*   **Writer Ordering:** Task 2 should include a stress test with multiple concurrent writers to verify that the MPSC/Actor pattern actually preserves order under load.
