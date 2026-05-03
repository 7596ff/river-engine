# Review: Context Assembly Rework (Round 3)

**Date:** 2026-04-30
**Reviewer:** Gemini CLI
**Status:** Complete

---

## Overview
The revised specification (v3) is robust and addresses the critical architectural gaps identified in previous rounds. The introduction of the `ContextMessage` wrapper, the unified turn-atomic rule, and the calibrated token estimator provide a solid foundation for the persistent context object.

This round focuses on subtle interactions between the new mechanisms.

---

## Findings

### 1. The "Backfill Jitter" Loop
- **Problem:** A performance regression exists where compaction fires every single turn.
- **Scenario:**
    1. The spectator cursor is at Turn 50. Turn 50 is a massive "dump" (e.g., a large `ls -R` or log file) containing 80 messages and 60K tokens.
    2. The agent is at Turn 60. Turns 51-60 contain only 10 messages.
    3. Compaction fires. Step 3 drops Turn 50 (<= cursor).
    4. Step 5 sees only 10 messages remaining, so it backfills Turn 50 from below the cursor.
    5. The context is now 60K + current turns, exceeding the 80% threshold.
    6. The "no-re-trigger" guard prevents a loop *within the turn*, but on the very next turn, the estimator still sees >80%, triggering compaction again.
- **Result:** The gateway performs a full compaction, drop, and backfill every turn until the spectator cursor moves past Turn 50.
- **Recommendation:** The 20-message floor should be "best effort" or the backfill should be capped to prevent immediately re-crossing the compaction threshold.

### 2. Calibration Ratio Stability
- **Problem:** The ratio `actual / estimated` is highly reactive if updated every turn. 
- **Scenario:** Turn N is code-heavy (high characters, low tokens). The ratio drops to 0.6. Turn N+1 is a dense JSON tool result or prose in a different language (low characters, high tokens). The 0.6 ratio is applied, making the estimate dangerously optimistic, potentially causing the LLM to hit its hard limit before the 80% trigger fires.
- **Recommendation:** Use a **Weighted Moving Average** (e.g., `0.7 * old_ratio + 0.3 * new_sample`) to smooth out oscillations, and start with a conservative default (e.g., 1.1) to favor safety over efficiency on the first turn.

### 3. Mid-Turn Channel Switching
- **Problem:** The spec handles channel switches as a "Session Start" but doesn't explicitly define the timing if a switch happens mid-turn.
- **Scenario:** An agent is in a tool loop. Tool 1 causes a channel switch. Tool 2 is then called.
- **Question:** Is the persistent context object replaced *immediately* when `set_channel_context` is called, or at the end of the turn? 
- **Recommendation:** Replacement must be immediate. If a tool call triggers a switch, all subsequent tool calls in that turn and the final assistant response must observe the *new* channel's context (or at least a cleared context) to prevent context leakage between channels.

### 4. Zero Token Response Handling
- **Problem:** "If the model returns 0 tokens (error), do not update the ratio."
- **Question:** What constitutes an "error"? If the model returns a valid but extremely short response (e.g., a single emoji or "OK"), `prompt_tokens` is still high, but `completion_tokens` is low. 
- **Clarification:** The calibration should specifically use `usage.prompt_tokens` against the `estimated_tokens` of the prompt sent. This is the only stable metric for context calibration.

### 5. Moves Budget Circularity (Resolved with Batches)
- **Observation:** The "fetch 50 rows at a time" batching strategy successfully resolves the circularity issue from v2. It allows the gateway to "fill up" the budget without knowing the total row count in advance.

---

## Conclusion
The design is ready for implementation once the **Backfill Jitter** and **Ratio Smoothing** concerns are addressed. The "Turn-Atomic" and "Lossless" promises are well-guarded.
