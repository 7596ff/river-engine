# Spectator Redesign Review Prompt — Pass 2

Paste the spec below into Gemini or another reviewer, followed by this prompt:

---

You are reviewing a design spec for a **spectator redesign** in a multi-agent orchestration system called river-engine. This is the second review pass. The first pass identified issues with LLM-driven segmentation, cursor gaps, race conditions, and token budgeting. The spec has been revised to address those concerns.

Key changes since the first review:
- **One sweep, one move.** The spectator owns segmentation (what entries go in each sweep). The LLM only writes a plain text narrative summary. No structured output, no snowflake boundary negotiation, no JSONL parsing of LLM responses.
- **Tiered entry formatting.** User/agent messages in full. Tool calls show name only. Tool results show name + byte count only. Heartbeats and cursors filtered out entirely.
- **Token budget per sweep (16384 tokens).** Entries are included oldest-first until budget is reached. If more remain, the spectator sweeps again immediately.
- **Serial processing.** One sweep at a time. Events buffer during a sweep.
- **Idle stall is not a problem.** Moves are consumed as compressed history. Unswept entries at the tail are already visible to the agent through the raw home channel.
- **Observability via home channel.** The spectator writes a system message after each sweep.
- **Cursor advances only on successful writes.** One sweep = one move = one cursor advancement.

Your job is to find what's still broken. The first pass was productive — it caught real problems. This pass should be harder. Look for:

## 1. The One-Move-Per-Sweep Tradeoff

The spec chose simplicity over granularity. But:
- A 5-minute sweep covering 10 turns of varied work produces one long summary. The context builder loads the last 10 moves as history. If each move covers 10 turns, the agent has compressed history spanning 100 turns. Is this the right granularity for the context builder?
- Can a single summary meaningfully cover "configured the flake, discussed auth, fixed a race condition, and reviewed a PR"? Or does it become a wall of text that the model skims past?
- The sweep interval determines move granularity. Is 5 minutes the right default? What would 2 minutes vs 10 minutes do to move quality?

## 2. The Token Budget Edge Cases

- What happens if a single entry (e.g., a very long agent message) exceeds 16384 tokens? The spectator can't include even one entry. Does it skip the entry? Truncate it? Stall?
- The budget is estimated, not exact. Token estimation is approximate (`(len + 3) / 4`). What if the estimate says 16000 tokens but the actual prompt is 20000? Does the LLM call fail? Is there a safety margin?
- The budget covers entries only, not the full prompt. Recent moves (10 summaries) + identity + instruction also consume tokens. Is 16384 for entries alone enough, or should the budget cover the entire prompt?

## 3. The Catch-Up Loop

- On a fresh agent with a large home channel, the spectator sweeps repeatedly until caught up. Each sweep makes an LLM call. If the home channel has 1000 turns of history, that's potentially dozens of LLM calls in rapid succession. Is there rate limiting?
- During catch-up, the agent is presumably still running and producing new entries. The spectator is processing old history while new work happens. Does the new work get swept eventually, or does the catch-up loop always prioritize old entries?
- What if the LLM fails during catch-up? The spectator retries on the next TurnComplete. But if the agent is idle (done for the day), no TurnComplete arrives, and the catch-up stalls partway through history.

## 4. The Home Channel Write

The spectator writes a system message to the home channel after each sweep. But:
- The spectator's system message becomes part of the home channel. On the next sweep, the spectator reads its own previous message. Does it include `[spectator]` messages in the entries it sends to the LLM? If yes, the LLM is reading its own output. If no, there's a filter to maintain.
- The system message is written via what mechanism? The spectator doesn't own the `HomeChannelWriter`. Does it need one? Or does it write directly to the file (breaking the serialized writer pattern)?

## 5. The moves.jsonl Concurrent Access

- The `AgentTask` reads `moves.jsonl` on every turn (via `load_moves`). The spectator appends to `moves.jsonl` during sweeps. These are concurrent. The spectator appends via normal file I/O (not the HomeChannelWriter actor). What prevents a partial read?
- Is there a moment where `load_moves` reads the file, gets the last 10 moves, and one of them is a half-written line from a concurrent spectator append?

## 6. What's Still Missing

- **The actual prompt.** The spec references `on-sweep.md` but doesn't include the text. The quality of moves depends entirely on this prompt. What does it ask for? How does it instruct the LLM to handle quiet periods vs busy periods?
- **Move quality feedback.** If the LLM produces a bad summary (too vague, misses key events, hallucinates), there's no mechanism to detect or correct it. Is this acceptable?
- **Testing strategy.** How do you test the spectator? Mock the LLM? Use a real model with canned home channel entries?

Be specific. Cite the spec when pointing out issues. Focus on things that would break in production, not theoretical concerns.
