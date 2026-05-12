# Home Channel Design Review — Updated Spec

**Spec Reviewed:** `docs/superpowers/specs/2026-05-12-home-channel.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## Summary of Improvements

The updated specification successfully addresses the major architectural contradictions identified in the previous review. The design has transitioned from a mutable log concept to a robust, immutable **Write-Ahead Log (WAL)** pattern with ephemeral context assembly.

## 1. Source of Truth & Atomic Writes (Resolved)

*   **Improvement:** The spec now explicitly adopts a **Write-Ahead Log (WAL)** pattern. The Home Channel is written first, with adapter logs treated as secondary, asynchronous projections.
*   **Serialized Access:** The introduction of a **Log Writer Actor** solves the concurrency and interleaving risks by serializing all writes to the JSONL file.
*   **Database Removal:** The decision to remove SQL message persistence in favor of the Home Channel simplifies the architecture and eliminates a major source of distributed state conflict.

## 2. The Compression Paradox (Resolved)

*   **Improvement:** The "in-place" compression contradiction has been removed. The log is now strictly append-only and immutable.
*   **Ephemeral Assembly:** Context building is now a derived operation that combines the recent "live tail" of the Home Channel with "moves" (summaries) produced by the spectator. This mirrors high-performance log-structured systems (LSM-style).
*   **Efficiency:** By only reading the log tail after the most recent move, the I/O overhead for context building remains constant regardless of the total log size.

## 3. Batching Timing (Resolved)

*   **Improvement:** The "Final Batch Check" (Step 6) ensures that messages arriving during a pure-text turn (no tool calls) are not left in the queue. This prevents the "stalled turn" edge case.

## 4. Heartbeat Clutter (Resolved)

*   **Improvement:** Heartbeats are now transient unless they result in an actual agent action. This prevents the log from filling with 45-minute "no-op" noise while still providing a wake mechanism.

## 5. Bystander Security (Addressed)

*   **Improvement:** The requirement for bearer token authentication on the bystander endpoint is now explicitly stated. This mitigates the risk of unauthenticated prompt injection and DoS attacks.

## Remaining Areas for Caution

### 1. The Target/Focus Problem (Deferred)
While the spec acknowledges the "Target/focus hinting" issue and defers it to a separate spec, the current system still relies on the model parsing `[user:adapter:channel]` tags.
*   **Risk:** Until the "focus hinting" spec is implemented, multi-user conversations across different adapters may still lead to routing errors if the model fails to extract the correct destination for `send_message`.

### 2. Tool Result Cleanup
*   **Detail:** The spec mentions that tool result files are cleaned up when superseded by a spectator move.
*   **Caution:** This introduces a side-effect where the spectator (an observational component) must have filesystem write/delete permissions on the agent's log directory. This coupling between the spectator and the agent's private files should be carefully managed to ensure the spectator doesn't accidentally delete active results.

### 3. Move Consistency
*   **Risk:** If the spectator produces a move that is inconsistent with the raw log (e.g., skips a turn or misrepresents content), the agent's memory will be permanently corrupted from that point forward, as the context builder will prefer the move over the raw entries. The system needs a "checksum" or validation mechanism to ensure moves accurately represent the log segments they replace.

## Conclusion

The updated design is significantly more mature and technically sound. It moves the project toward a modern log-centric architecture that scales better and provides a clearer source of truth. The transition to ephemeral context building is a major architectural win.
