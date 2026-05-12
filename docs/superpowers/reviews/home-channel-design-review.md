# Home Channel Design Review — Grounded Edition

**Spec Reviewed:** `docs/superpowers/specs/2026-05-12-home-channel.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## 1. The Source of Truth Contradiction

The codebase currently maintains three versions of history:
1.  **Channel Logs (`.jsonl`):** Platform-specific records of incoming/outgoing messages.
2.  **SQL Database (`river_db`):** The primary persistence layer where messages and moves are stored.
3.  **In-Memory Context (`PersistentContext`):** The agent's current working memory, built from the DB.

The spec's claim that the Home Channel is the "single source of truth" while keeping adapter logs is a step toward consolidation but introduces a **Dual-Write Vulnerability**.

*   **Code Observation:** `AgentTask::turn_cycle` reads from JSONL and then `persist_turn_messages` writes to the DB. This sequence is not atomic.
*   **Recommended Fix:** Adopt a **Write-Ahead Log (WAL)** pattern. The Home Channel should be the *only* target for incoming messages. Adapter-specific logs should be generated as secondary, asynchronous projections of the Home Channel, or eliminated entirely in favor of tagged queries on the Home Channel.

## 2. The Compression Paradox

The spec describes an "append-only log" where old entries are "compressed in place."

*   **Code Observation:** Currently, `PersistentContext::compact` drops messages below a "spectator cursor" (the max turn number of a move in the DB) and reloads moves from the DB. This works because the DB and in-memory list are mutable. A JSONL file is not.
*   **Contradiction:** "In-place" compression on a JSONL file requires a full file rewrite, which is dangerous if the agent is actively writing or reading.
*   **Recommended Fix:** Use a **Compacted Log Segment** approach (similar to Kafka or LSM trees). The spectator should write "MoveEntries" to the end of the log. The context builder should skip messages that have been superseded by a MoveEntry with a higher turn number. Periodically, a background task can "compact" the log into a new file, but the active log remains append-only.

## 3. The Batching Timing Problem

*   **Code Observation:** `AgentTask` already handles mid-turn notifications (lines 405-445) by checking for messages after tool execution and injecting them as a system message.
*   **Edge Case:** If a turn has no tool calls, mid-turn messages (arriving during the model call) are currently NOT checked before the turn completes.
*   **Recommended Fix:** The turn cycle must perform a final check for new Home Channel entries *immediately* after the model response and *before* entering the "settle" phase, regardless of whether tool calls occurred.

## 4. The Channel Switching Removal

*   **Code Observation:** Current code uses `ChannelContext` to track which adapter/channel the agent is "active" in. Removing this requires the model to be a perfect parser of its own history.
*   **Recommended Fix:** Introduce a `TargetEntry` or `ContextHint` in the Home Channel. When a message arrives from an adapter, it is written as a `MessageEntry`. The system should also write a `TargetEntry` indicating the current "focused" channel. This provides a deterministic "default" for the `send_message` tool without relying on model-side tag parsing.

## 5. The Bystander Endpoint Security

*   **Risk:** The "anonymous by design" endpoint is a DoS and prompt injection vector.
*   **Recommended Fix:** The bystander endpoint **must** require authentication. It can use the existing bearer token mechanism. "Anonymous" should mean the *author field* is omitted in the Home Channel entry, but the *caller identity* must be validated by the Gateway to prevent unauthorized writes.

## 6. The Tool Result File Pattern

*   **Maintenance:** The spec provides no cleanup logic for tool result files.
*   **Recommended Fix:** Move tool result files to a **content-addressed store (CAS)** or use a lifecycle policy. The Home Channel should reference results by a hash-based path. If a result is identical to a previous one, it reuses the file.

## 7. The Heartbeat as Home Channel Entry

*   **Clutter:** 45-minute heartbeats will fill the log with "noise" entries.
*   **Recommended Fix:** Heartbeats should only be written to the log if they *actually* result in a model turn (e.g., if the agent decides to proactively speak). Otherwise, a "No-Op Heartbeat" should be a transient event that doesn't persist to the Home Channel.

## 8. Concurrency and Locking

*   **Code Observation:** `ChannelLog::append_entry` uses `OpenOptions::append(true)`, which is atomic on Linux for small writes. However, multiple processes (Gateway, Agent, Spectator) writing to the same file still risks interleaved lines or race conditions during the "in-place" compression proposed.
*   **Recommended Fix:** Implement a **Log Writer Actor**. All writes to the Home Channel must go through a single serialized task in the Gateway to ensure ordering and prevent file lock contention.
