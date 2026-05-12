# Spectator Redesign Review — Updated Spec (V2)

**Spec Reviewed:** `docs/superpowers/specs/2026-05-12-spectator-redesign.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## Summary of Improvements

The updated specification is significantly more robust and directly addresses the most fragile elements of the previous design. By shifting the responsibility for segmentation from the LLM to the spectator, the design eliminates a major class of "hallucinated boundary" and "incoherent fragment" errors.

## 1. The Segmentation Problem (Resolved)

*   **Improvement:** The spectator now owns the entry boundaries (`start` and `end` snowflakes). The LLM is restricted to providing a plain text narrative summary of the *provided* chunk.
*   **Result:** This eliminates overlapping ranges, hallucinated snowflake IDs, and incoherent fragments. "One sweep, one move" ensures that the narrative always aligns with the underlying log segments.
*   **Zero-Move Sweeps:** Since the spectator always advances the cursor after a successful LLM call (even if the summary is brief), the "stuck" state in quiet periods is resolved.

## 2. The Cursor Gap (Resolved)

*   **Improvement:** The "one sweep, one move" policy simplifies cursor tracking. The cursor is derived from the last `end` field in `moves.jsonl`.
*   **Recovery:** On startup, reading the last move's end snowflake is a deterministic way to resume. If a sweep fails (LLM error), the cursor doesn't advance, and the next sweep automatically attempts to cover the larger window (up to the token budget).

## 3. The Time-Gate & Catch-up Strategy (Resolved)

*   **Improvement:** The introduction of a **Catch-up Loop** (Step 11) ensures that the spectator isn't strictly gated by the 5-minute timer if it's behind. This handles the "First sweep on an old agent" scenario and ensures backlogs are processed efficiently in chunks.
*   **Idle Activity:** While sweeps still only trigger on `TurnComplete`, the spec correctly notes that entries at the "tail" of the log are already visible to the agent via the raw home channel. The spectator's role is long-term compression, not real-time monitoring.

## 4. The Entry Formatting & Token Budget (Resolved)

*   **Tiered Detail:** Formatting tool calls as name-only and tool results as byte-counts drastically reduces the "Argument Bloat" and "Truncation Loss" risks. It relies on the agent's own messages to provide the narrative interpretation of tool work, which is a more token-efficient strategy for a summary-focused task.
*   **Token Budgeting:** The 16,384 token cap per sweep provides a hard bound on prompt size, preventing context window overflows and ensuring predictable cost/performance.

## 5. Observability (Resolved)

*   **Improvement:** Writing `[spectator] move written` messages to the home channel provides built-in observability. An operator or even the agent itself can see when "memory compaction" is happening.

## Remaining Areas for Caution

### 1. The `moves.jsonl` Single File
*   **Performance:** `AgentTask` still reads the entire file on every turn. While moves are infrequent (every 5+ minutes), a long-running agent will eventually accumulate thousands of moves. A "backward read" or "last N lines" strategy for `moves.jsonl` should be considered to keep context assembly fast.
*   **Partial Writes:** The risk of the Context Builder reading a partially-written line while the Spectator is appending still exists. A robust "skip malformed tail" logic is essential.

### 2. Cleanup Timing & Locking
*   **Detail:** Cleanup is performed after the move is written. While the spec notes the agent is between turns (settle gate), there is no hardware-level lock. 
*   **Risk:** If a user sends a message *exactly* as a sweep completes, the agent might wake and start a turn, attempting to read a tool result file that the spectator is currently unlinking. 

### 3. Move Consistency
*   **Detail:** The LLM is now asked for a "plain text narrative summary."
*   **Caution:** If the LLM returns an error message or a refusal in plain text (e.g., "I cannot summarize this content..."), the spectator will blindly write that as a "move." The system needs a basic heuristic or a "quality check" (e.g., minimum length, absence of refusal keywords) before committing the text to the narrative history.

## Conclusion

The redesigned spectator is architecturally sound and far more resilient than the previous version. The transition to a "Spectator Owns Boundaries" model is a major win for reliability. 
