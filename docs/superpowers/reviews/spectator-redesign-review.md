# Spectator Redesign Review

**Spec Reviewed:** `docs/superpowers/specs/2026-05-12-spectator-redesign.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## 1. The Segmentation Problem

The strategy of letting the LLM segment entries into "arcs" by outputting snowflake ranges is highly fragile.

*   **Incoherence:** The LLM has no semantic understanding of "tool call chains" as atomic units. It might easily draw a boundary between a `tool_call` and its corresponding `tool_result`, or between multiple steps of a complex file edit. This leaves moves covering incoherent fragments of history.
*   **Validation:** The spec does not define how to handle overlapping ranges (Move 1: 100-150, Move 2: 140-200) or hallucinated snowflake IDs. If the LLM outputs a range that doesn't correspond to actual entries, the "coverage" reported by the move is false.
*   **Zero-Move Sweeps:** If the LLM determines "nothing happened" and outputs zero moves, the entries remain "uncovered" from the spectator's perspective. On the next sweep, these same entries will be re-read, potentially leading to a "stuck" state where the spectator repeatedly processes a quiet period without ever advancing its cursor.

## 2. The Cursor Gap

Tracking the cursor as the `end` snowflake of the last move in `moves.jsonl` introduces data loss scenarios.

*   **Partial Failures:** If a sweep produces 5 moves but the 3rd one fails to parse, what happens to the cursor? If the spectator uses the 5th move's `end`, the entries corresponding to the failed 3rd move are effectively "skipped" and never summarized.
*   **Append Failures:** If the spectator crashes mid-append (e.g., after writing 1 of 3 moves), the restart logic will resume from the end of move 1. Moves 2 and 3 are lost forever.
*   **Corruption:** The cursor is derived from the file tail. If `moves.jsonl` is truncated or contains a partial line, the cursor is lost. There is no fallback (e.g., a dedicated `cursor.json` file) to recover the last successfully processed snowflake.

## 3. The Time-Gate Race Condition

The "Sweep on TurnComplete + Time Gate" logic creates several "dead zones."

*   **Idle Stall:** If an agent finishes a flurry of work and then goes idle, the final turns may sit "unswept" for hours because no new `TurnComplete` event arrives to trigger the 5-minute check. The spectator is only "awake" when the agent is "active," leaving the tail of most conversations unsummarized.
*   **Concurrency:** The spec does not address what happens if a sweep is already in progress (waiting for an LLM response) when another `TurnComplete` arrives. Does it queue? If it skips, and that turn was the last one, the agent enters the "Idle Stall" described above.
*   **Event Skipping:** Only the *first* `TurnComplete` after the 5-minute mark triggers a sweep. If 10 turns happen in 10 seconds, the subsequent 9 events are ignored. While the sweep will read the current log, the "trigger" is decoupled from the actual data volume.

## 4. The Entry Formatting Problem

*   **Truncation Loss:** Truncating tool results at 500 characters is a major data quality risk. Key information (error messages, search results) often appears late in a large output. The LLM will summarize based on incomplete data, leading to "hallucinated" or missing narrative details in the moves.
*   **Argument Bloat:** While results are truncated, `arguments_json` for tool calls are not. A `write_file` call with 200KB of content will flood the prompt and likely cause a context window overflow or an expensive model call for no narrative gain.
*   **Heartbeat/Cursor Noise:** The spec doesn't specify if `CursorEntry` is filtered. If heartbeats accumulate (e.g., 45-minute idle intervals), the transcript will be cluttered with `[{id}] heartbeat` lines that the LLM must pay tokens to process.

## 5. The Continuity Context Problem

*   **Token Budgeting:** Including the last 10 moves *and* the new transcript has no upper bound. A busy period followed by a long sweep could easily exceed a 32K or 128K context window. The spec lacks a strategy for "shedding load" if the entries since the last sweep are too numerous.
*   **Style Drift:** The LLM is given two different formats: summaries (narrative) and raw entries (structured logs). Asking it to produce a summary that matches the "voice" of the moves based on raw logs is prone to style drift as the context of *how* the previous moves were written is lost.

## 6. The Cleanup Timing Problem

*   **Race Condition:** The `AgentTask` (Context Builder) reads the home channel and its linked tool result files. The Spectator deletes these files after writing moves. If the Agent is in the middle of a turn while the Spectator completes a sweep, the Agent may attempt to read a file that the Spectator just unlinked.
*   **Cleanup Granularity:** Cleanup is performed on the "covered snowflake range." If Move 2 of 3 fails to parse, but moves 1 and 3 succeed, the "range" still covers the entries for Move 2. The tool results for Move 2 are deleted, even though no move summary exists to replace them.

## 7. The `moves.jsonl` Single File

*   **Scaling:** `AgentTask` reads the *entire* `moves.jsonl` on every turn to extract the last N summaries. As the file grows over months, this read/parse operation becomes a performance bottleneck for every agent turn.
*   **Partial Reads:** The Context Builder reads `moves.jsonl` concurrently with the Spectator appending to it. If the Context Builder reads while the Spectator has only written half a line, the Builder will encounter a parse error. Since the Builder "skips malformed lines," it will effectively ignore the most recent (and most relevant) move.

## 8. What's Missing

*   **Observability:** No mention of how an operator monitors spectator health. A "stalled" spectator (e.g., due to repeated LLM parse errors) will leave the agent's memory increasingly cluttered without any visible warning.
*   **First Sweep Volume:** A fresh spectator on an old agent will attempt to sweep the entire history in one go. This will almost certainly exceed token limits and fail. The design lacks a "backfill" strategy or a "start from now" option.
*   **Prompt Engineering:** The spec refers to `on-sweep.md` but provides no template. The entire logic of "segmentation via snowflake range" depends on the LLM's ability to follow a complex formatting instruction perfectly—a task where even top-tier models frequently fail.
