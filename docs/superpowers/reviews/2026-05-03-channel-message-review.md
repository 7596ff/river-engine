# Review: Channel Message Design

**Spec Version:** 2026-05-03
**Reviewer:** Gemini CLI
**Status:** Approved with Recommendations

## Summary

The proposed design successfully addresses the broken message delivery path by simplifying the communication layer to an adapter-agnostic JSONL log system. Moving from a complex directory-based inbox to a flat, snowflake-indexed notification system is a significant improvement in maintainability and robustness.

## Findings

### Severity: Important

#### 1. Cursor Scanning Efficiency at Scale
The spec suggests scanning backward for the last `role: "agent"` entry. While elegant, this becomes a performance bottleneck for long-running channels where the agent hasn't spoken in a while.
*   **Risk:** Reading a large JSONL file from the end to find a "needle in a haystack" consumes unnecessary I/O and CPU.
*   **Recommendation:** Maintain a lightweight "cursor cache" (e.g., a simple JSON file or SQLite table) that stores the last read snowflake ID per channel. Fall back to scanning only if the cache is missing.

#### 2. "Reasonable Window" Ambiguity
For a new channel with no existing cursor, the spec suggests reading "backward some reasonable window".
*   **Risk:** Inconsistent behavior or loading too much/too little context.
*   **Recommendation:** Define a specific default (e.g., "last 50 entries") or make it a configurable parameter in `AgentTaskConfig`.

#### 3. Handling Write Failures
The inbound flow must ensure that a notification is only pushed if the log write was successful.
*   **Risk:** If the disk is full and the append fails, pushing a notification will cause the agent to wake and find no new data (or old data), potentially leading to incorrect state transitions.
*   **Recommendation:** Explicitly state that Step 5 (Push to MessageQueue) must only execute upon successful completion of Step 4 (Append entry).

### Severity: Suggestion

#### 1. JSONL Robustness
JSONL is good for recovery, but partial writes (due to system crashes) can lead to invalid JSON lines.
*   **Recommendation:** Implement a robust reader that skips invalid lines or logs a warning instead of panicking when encountering a malformed JSON line during the forward-read phase.

#### 2. Channel Cleanup Strategy
As noted in the spec's Open Questions, logs will grow indefinitely.
*   **Recommendation:** Implement a simple rotation policy where logs are moved to an `archive/` folder when they exceed a certain size (e.g., 10MB), ensuring the agent's "active" history remains performant.

#### 3. Outbound Message IDs
The spec suggests the agent generates a snowflake ID for its own message.
*   **Note:** The adapter returns a `message_id`. The log entry should store *both* the local Snowflake (for temporal ordering) and the `adapter_msg_id` (for interaction/editing). This is already in the spec's "Fields" table but should be carefully implemented to avoid confusion between the two.

## Conclusion

The design is sound and ready for implementation. The primary concern is long-term performance (cursor scanning), which can be mitigated with a simple cache.
