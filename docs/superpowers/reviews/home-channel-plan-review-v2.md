# Home Channel Implementation Plan Review — V2 (Grounded & Detailed)

**Plan Reviewed:** `docs/superpowers/specs/2026-05-12-home-channel.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## Summary of Improvements

The updated implementation plan is significantly more robust and directly addresses all 10 critical points from the first review. The architecture has transitioned from "hand-waving" to "concrete," particularly regarding data types and the context builder's logic.

## 1. Serde Strategy (Resolved)

*   **Improvement:** Task 1, Step 2 now introduces `HomeChannelEntry` with `#[serde(tag = "type")]`. This is a major win for data integrity and performance.
*   **Safety:** Keeping the old `ChannelEntry` as `untagged` ensures backward compatibility for existing adapter logs while the Home Channel gets a clean, modern start.

## 2. Tool Name Threading (Resolved)

*   **Improvement:** Task 5 introduces a dedicated `ToolExecResult` struct that preserves the tool name through the execution phase.
*   **Result:** The Home Channel will now have high-quality `ToolEntry` data with correct tool names, making the log far more useful for the model and the spectator.

## 3. Context Builder Completeness (Resolved)

*   **Improvement:** Task 3, Step 1 now includes logic for reconstructing `ToolCallRequest` objects, including argument serialization and grouping multiple consecutive tool calls into a single assistant message.
*   **Correctness:** This ensures the model receives well-formed tool usage history, preventing schema validation errors.

## 4. Source Tracking & Data Purity (Resolved)

*   **Improvement:** Task 1, Step 3 adds dedicated `source_adapter` and `source_channel_id` fields to `MessageEntry`.
*   **Result:** The raw `content` field remains clean, and the Context Builder (Task 3) handles the visual formatting of tags for the model. This makes the data much easier to consume for other tools (search, UI).

## 5. Wiring and Compilation Order (Resolved)

*   **Improvement:** Task 7 now explicitly handles the switchover from `PersistentContext` to the Home Channel context builder.
*   **Refinement:** The plan avoids a "bridge to nowhere" by dual-writing during Tasks 5 and 6 and then performing a clean cut in Task 7.

## 6. SQL Removal and Move Storage (Resolved)

*   **Improvement:** Tasks 10 and 12 explicitly move spectator "moves" to a file-based storage system and remove SQL message persistence.
*   **Architecture:** This completes the transition to a purely log-centric architecture.

## Remaining Areas for Caution

### 1. The "Send-Ahead" vs "Write-Ahead" Risk
Task 6 calls the Home Channel write a "write-ahead," but `HomeChannelWriter::write` (Task 2) uses a fire-and-forget MPSC send.
*   **Risk:** If the process crashes after the `send()` to the actor but before the actor flushes to disk, the "secondary" adapter log (written synchronously in Task 8) might reach the disk while the "primary" Home Channel entry is lost. 
*   **Recommendation:** If strict WAL guarantees are required, consider adding an optional `write_sync()` method to the writer that uses a `oneshot` channel to wait for a flush confirmation.

### 2. Migration for Existing Agents
The plan handles "birth" (new agents) but doesn't include a task to migrate existing SQL history for agents already in the field.
*   **Impact:** Existing agents will experience "memory loss" upon the update, as their Home Channel will be empty while their SQL history is no longer read.
*   **Recommendation:** Add a Task 12.1 for a one-time migration utility that populates the Home Channel from the `messages` table for active sessions.

### 3. Tool Result File Permissions
Task 11 wires the writer to delete tool result files when a move is completed.
*   **Caution:** Ensure the `HomeChannelWriter` has sufficient permissions to delete files created by the `AgentTask`. In shared environments or Docker containers, this can sometimes lead to `PermissionDenied` errors if the gateway and agent run as different users.

## Conclusion

The implementation plan is now highly mature and execution-ready. The transition to a tagged Serde format and the detailed reconstruction logic in the context builder are particularly well-handled. 
