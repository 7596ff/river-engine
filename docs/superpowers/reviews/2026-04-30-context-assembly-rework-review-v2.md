# Review: Context Assembly Rework (Round 2)

**Date:** 2026-04-30
**Reviewer:** Gemini CLI
**Status:** Complete

---

## Resolved Findings from Round 1
- **Lossless Guarantee (Session Start):** Resolved. The new procedure loads all messages where `turn_number > cursor`, ensuring no uncompressed messages are lost at startup.
- **NULL Cursor Handling:** Resolved. NULL is explicitly treated as 0 (keep everything), which is safe.
- **`turn_number` Tracking:** Resolved. The `ContextMessage` wrapper provides the necessary metadata for in-memory management without polluting the model API types.
- **Moves Loading Efficiency:** Resolved. Newest-first loading with a budget cap prevents fetching exhaustive history.

---

## New Findings & Open Questions

### 1. Turn-Atomic Drops for Gateway-Injected Messages
- **Problem:** The spec mandates that compaction operates on whole turns. However, the gateway injects system messages mid-turn (e.g., "Messages received during tool execution" in `task.rs:388`, or the new "60% Lag Warning").
- **Question:** What `turn_number` do these injected messages receive? If they are assigned `current_turn`, they stay above the cursor. If they are assigned a dummy or previous turn number, they might be dropped while the turn they describe is kept.
- **Spec Location:** Section 3 (Compaction procedure, step 3).

### 2. 60% Lag Warning: Re-trigger Loop
- **Problem:** The lag warning is a system message injected *after* compaction.
- **Question:** Does this message count toward token estimation *immediately*? If compaction fails to reach 40% (e.g., hits 79%) and then injects a large lag warning, could it push the total to 80.1% and trigger an immediate (though guarded) secondary compaction attempt?
- **Spec Location:** Section 3.1 (Spectator lag detection).

### 3. Backfill Semantics: "Until at least 20"
- **Problem:** "Backfill complete turns... until at least 20 messages." This is ambiguous for large turns.
- **Scenario:** Current context has 5 messages above cursor. Newest turn below cursor has 100 messages (e.g., extensive `ls -R` output).
- **Question:** Does "until at least 20" mean we add the *entire* 100-message turn (resulting in 105 messages), or do we skip it because it's "too much"? If we add it, we might immediately hit the 80% compaction threshold again.
- **Spec Location:** Section 3 (Compaction procedure, step 5).

### 4. Moves Budget vs. Minimum Floor
- **Problem:** The spec says "fill remaining space up to 40% total with moves."
- **Question:** If uncompressed messages + system prompt already take 39.5% of the limit, only 0.5% (~640 tokens) is allocated to moves. Is "zero moves" or "near-zero moves" an acceptable state? Should there be a `min_moves_tokens` floor (e.g., 2K) to ensure the agent always has *some* structural memory of the past, even if it pushes the post-compaction total to 45%?
- **Spec Location:** Section 3 (Compaction procedure, step 6).

### 5. Token Estimation Drift Calibration
- **Problem:** The rough estimator (`len/4`) is used for the 80% trigger. The spec says the actual count from the model "can be used to calibrate future estimates."
- **Question:** How is this calibration stored? If the model returns 100,000 tokens but the estimator says 70,000 (30% error), the agent might exceed the model's hard limit before the 80% trigger fires. Should the 80% trigger be checked against `MAX(estimated, last_actual_normalized)`?
- **Spec Location:** Section 5 (Token estimation).

### 6. New DB Query Circularity
- **Problem:** `get_moves_newest_first(channel, limit)`.
- **Question:** What is the unit of `limit`? If it's a row count, the gateway doesn't know how many rows equal the remaining token budget. If it's a token limit, the DB would need to estimate tokens.
- **Recommendation:** Use a row-based limit (e.g., 50 moves) for the initial fetch, and if tokens permit, fetch more in batches until the 40% target is reached.
- **Spec Location:** Section 8 (New DB query needed).

### 7. Persistent Object Lifecycle & Channel Switching
- **Problem:** The spec says the context lives for the "duration of a session."
- **Question:** What happens when the agent switches channels (e.g., Discord `#general` to `#dev`)? Since moves and messages are channel-specific, the "persistent" object must be cleared and rebuilt from the DB. Is a channel switch treated as a "Session Start"?
- **Spec Location:** Section 3 (Architecture).
