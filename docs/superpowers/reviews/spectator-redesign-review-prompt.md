# Spectator Redesign Review Prompt

Paste the spec below into Gemini or another reviewer, followed by this prompt:

---

You are reviewing a design spec for a **spectator redesign** in a multi-agent orchestration system called river-engine. The spectator is a background task that observes an agent's activity (written to an append-only JSONL "home channel") and produces narrative move summaries by calling an LLM. The redesign replaces a per-turn model (one LLM call per turn, one move per turn) with a time-gated sweep model (accumulate turns, read the home channel directly, produce one or more moves per sweep).

This is a critical review. Your job is not to validate — it is to **lay bare contradictions**. Find the places where the spec promises two incompatible things, where the architecture implies behaviors the text doesn't address, where edge cases would break the design.

## 1. The Segmentation Problem

The spec says the LLM segments entries into "arcs of work" and outputs multiple moves per sweep. But:
- What guarantees the LLM's segmentation is correct? If the LLM draws a boundary in the middle of a tool call chain (start snowflake is a tool_call, end snowflake is the next user message), the move covers an incoherent fragment.
- What happens if the LLM outputs overlapping snowflake ranges? (Move 1 covers entries 100-150, Move 2 covers entries 140-200.) Do they get written as-is? Is this detected?
- What if the LLM outputs a range that doesn't match any actual entries? (Hallucinated snowflake IDs.) The move file claims to cover entries that don't exist.
- What if the LLM outputs zero moves? ("Nothing significant happened.") Is this valid? The entries are still uncovered. Does the next sweep re-read them?

## 2. The Cursor Gap

The spec says the spectator tracks its own cursor — the `end` snowflake of the last move. But:
- What if the LLM produces three moves and the second one fails to parse? The third move might parse fine. Which `end` becomes the cursor — the first move's end (leaving entries uncovered) or the third move's end (skipping the gap)?
- What if the spectator crashes between appending move 1 and move 2 to `moves.jsonl`? On restart, it reads the last move's `end` and starts from there. Move 2's entries are never covered.
- The cursor is derived from the file. If the file is corrupted (partial write, truncated line), the cursor is lost. What's the recovery strategy?

## 3. The Time-Gate Race Condition

The spec says: "On each TurnComplete, check if enough time has passed since the last move was written." But:
- What if the agent completes 10 turns in rapid succession (1 second apart)? Only the first TurnComplete triggers a sweep (if 5 minutes have passed). The remaining 9 are ignored. But those 9 turns are the ones that added content. The sweep that fires reads the home channel, which includes all 10 turns — fine. But what if the next turn doesn't come for 30 minutes? Those entries sit unprocessed until the next TurnComplete arrives. Is this acceptable?
- What if a sweep is in progress (LLM call running, could take 30-60 seconds) and another TurnComplete arrives? Does it queue a second sweep? Skip it? The spec says nothing about concurrency within the spectator itself.
- The spec says "if the agent is idle, no sweeps fire." But idle agents might have a final turn whose entries are never swept because no subsequent TurnComplete arrives to trigger the time check.

## 4. The Entry Formatting Problem

The spec defines entry formatting for the LLM prompt. But:
- Tool results are truncated to 500 chars. What if the meaningful content (an error message, a key finding) is at character 501? The LLM writes a move that misses the point.
- Tool call arguments are included as raw JSON. For complex tool calls (write_file with large content), this could be enormous. Is there a truncation strategy for arguments too?
- Heartbeat entries are formatted as `[{id}] heartbeat`. If the agent was idle for hours and 5 heartbeats accumulated, the LLM sees 5 lines of `heartbeat`. Does it produce a move that says "nothing happened"? Or skip them?
- The spec doesn't address cursor entries. Are they formatted? Filtered out?

## 5. The Continuity Context Problem

The spec includes the last 10 moves as context for narrative continuity. But:
- 10 moves at what length? If each move summary is 200 words, that's 2000 words of context before the actual entries. If the entries themselves are from a busy sweep (50+ entries with tool calls), the prompt might exceed the model's context window.
- Is there a total prompt budget? The spec doesn't mention one. What happens when the home channel entries since the last move are enormous (agent ran 20 turns with heavy tool use)?
- The moves are summaries of older entries. The new entries are raw transcripts. The LLM is asked to produce summaries in the same voice as the continuity context. But the inputs are in two different formats. Does this cause style drift?

## 6. The Cleanup Timing Problem

The spec says tool result files are cleaned up after moves are written. But:
- The context builder might be reading a tool result file at the exact moment the spectator deletes it. Is there a lock? A race?
- The spec says cleanup covers "the covered snowflake range." But if the LLM segments entries into three moves, cleanup happens once for the entire range, not per-move. What if move 1 and move 3 are written but move 2 fails to parse? Cleanup still covers the full range, including move 2's entries. Is this correct?

## 7. The moves.jsonl Single File

The spec uses a single append-only JSONL file for moves. But:
- The context builder reads this file on every turn (via `load_moves`). As the file grows, this read gets slower. The spec says "no rotation for now" — at what point does this become a problem?
- Multiple processes could append to this file: the spectator writes moves, and... actually, only the spectator writes. But is the spectator guaranteed to be a single instance? What if two spectator tasks are spawned accidentally?
- The file is the source of truth for the spectator's cursor. But it's also read by the context builder. If the context builder reads a partially-written line (spectator is mid-append), it gets a parse error. The context builder skips malformed lines — but the latest move is the one most likely to be partially written.

## 8. What's Missing

- **Prompt engineering.** The spec describes the prompt structure but doesn't include the actual `on-sweep.md` prompt text. The quality of the moves depends entirely on the prompt. A bad prompt produces bad segmentation, bad summaries, or malformed JSONL.
- **Token budgeting.** No strategy for when the entries since the last move exceed the model's context window. This will happen during long unswept periods or after LLM failures.
- **Observability.** How does an operator know the spectator is working? Is there a health check, a metric, a log pattern that indicates "sweeps are happening and producing moves"?
- **First sweep.** On a fresh agent with no `moves.jsonl`, the spectator reads the entire home channel from the beginning. If the agent has been running for days before the spectator starts, this could be an enormous prompt.

Be specific. Cite the spec when pointing out issues. Suggest concrete fixes, not vague concerns.
