# Context Assembly Rework

**Date:** 2026-04-30
**Status:** Draft (revised after Gemini review rounds 1-3)
**Authors:** Cass, Iris

---

## 1. Summary

Replace the per-turn context rebuild with a persistent context object that accumulates messages in place and compacts only when needed. Compaction is safe because it only drops messages the spectator has already compressed into moves. The system is simpler (4 config knobs instead of 8 budget slots) and lossless (no uncompressed message is ever dropped from context).

---

## 2. Current Problems

- Context is rebuilt from scratch every turn — wasteful and loses conversational flow
- Budget is split across 8 fixed-size token buckets that don't adapt to actual content
- Hot messages are token-budgeted and truncated per turn, losing messages silently
- Flashes and vector search are partially implemented warm layers that complicate the assembler
- No coordination between compaction and the spectator's compression work

---

## 3. Architecture

The context is a **persistent object** that lives for the duration of a session or until a channel switch. Built once at startup, messages are appended in place as the conversation proceeds. No per-turn rebuild.

### Internal representation

Messages in the persistent context are wrapped to carry metadata:

```rust
struct ContextMessage {
    chat_message: ChatMessage,
    turn_number: u64,
}
```

`ChatMessage` is the model API type (role, content, tool_calls). `turn_number` is gateway metadata used for compaction. When sending to the model, unwrap to `ChatMessage`. When compacting, filter by `turn_number`.

The `turn_number` field is already present on `river_db::Message` — no schema changes needed.

### Turn number assignment

Every `ContextMessage` appended during `turn_cycle(N)` receives `turn_number = N`. This includes:
- User messages drained from the `MessageQueue`
- Assistant responses from the model
- Tool call results
- Gateway-injected system messages (mid-turn message notifications, context warnings)

No exceptions. All messages within a turn share the same `turn_number`. The turn number increments once per call to `turn_cycle()`.

### Compaction trigger

Estimated tokens hit **80%** of context limit.

### Compaction procedure

1. Re-read system prompt from disk (`AGENTS.md`, `IDENTITY.md`, `RULES.md` + environment info: date, cwd, git branch/status)
2. Query spectator cursor: `SELECT MAX(turn_number) FROM moves WHERE channel = ?`. If NULL (spectator has never run), treat as 0 — no messages are droppable.
3. Drop messages by turn: drop all messages belonging to turns where `turn_number <= cursor`. Turns are atomic — all messages in a turn (user, assistant, tool calls, tool results) are dropped together or kept together. This prevents orphaned tool call/result pairs.
4. Keep all messages belonging to turns where `turn_number > cursor` (uncompressed, never dropped).
5. If remaining messages < 20, backfill complete turns from below cursor (newest first). Stop when the floor is reached or when adding the next turn would push total tokens above the compaction threshold (80%). The 20-message floor is best-effort — if backfilling a large turn would immediately re-trigger compaction, skip it. Messages below the cursor are already in moves, so skipping them does not violate the lossless guarantee.
6. Load moves from DB, newest first, 50 rows at a time. Estimate tokens as loaded. Stop when remaining space up to 40% total is filled. If 50 rows don't fill the budget, fetch more.
7. If post-compaction tokens still exceed 80% (spectator is far behind), do **not** re-trigger compaction. Accept it and continue. See §3.1 for spectator lag handling.
8. Check spectator lag (§3.1).
9. Reassemble context object, resume appending.

### 3.1 Spectator lag detection

After compaction (after the re-trigger check), check two conditions:
- Post-compaction tokens exceed **60%** of context limit (midpoint between fill target and compaction trigger)
- Agent's current turn minus spectator cursor exceeds **10 turns**

If both are true, inject a system message into the rebuilt context:

> `[System: Context compression is behind by N turns. Context is at X%. Long-term memory is degraded. Consider shorter responses or informing the user.]`

This message gets the current `turn_number` and is included in the token estimate. It is small (~50 tokens) and injected after the re-trigger check, so it cannot cause a compaction loop.

This gives the agent awareness without blocking. The agent can tell the user, adjust its behavior, or ignore it.

**No re-trigger guard:** If compaction fires and cannot reach 40%, do not re-trigger. The next compaction happens only when tokens hit 80% again through normal message accumulation.

### 3.2 Channel switching

A channel switch takes effect at the **start of the next turn**, not mid-turn. The `switch_channel` tool returns success immediately, but the context rebuild happens before the next `turn_cycle()` begins. This ensures the current turn's tool call/result pairs are never split across channel contexts.

When the switch takes effect, the session-start procedure (§3.3) runs for the new channel. The persistent context object is replaced entirely. The system prompt is re-read from disk. Moves and messages are loaded from the new channel's data.

If the agent switches back to a previous channel, the context is rebuilt from DB — there is no caching of previous channel contexts.

### 3.3 Session start

Session start follows the same logic as compaction, sourced from DB:

1. Read system prompt from disk
2. Query spectator cursor: `MAX(turn_number)` from moves for the current channel
3. Load all messages from DB where `turn_number > cursor`
4. If fewer than 20 messages, backfill complete turns from at or below cursor (newest first) until 20 messages or until the next turn would push tokens above 80%
5. Load moves from DB, newest first, fill remaining space up to 40% total
6. Assemble context object

This is the same algorithm as compaction. The only difference is the message source: compaction filters the in-memory vector, session start queries the DB.

### Guarantees

- **Lossless:** No uncompressed message is ever dropped from context. Compaction only drops messages at or below the spectator cursor. Messages above the cursor are never touched.
- **Turn-atomic:** Compaction operates on whole turns. All messages within a turn share the same `turn_number` and are dropped or kept together. Tool call/result pairs are never separated.
- **Minimum floor:** At least 20 messages always present (backfilled from below cursor if needed). The floor is best-effort — backfill stops if the next turn would push tokens above 80%.
- **Graceful degradation:** If spectator falls behind, messages accumulate and moves get squeezed — no data loss, just less structural memory. Agent is notified when lag is significant.
- **No infinite loop:** Compaction never re-triggers itself. Backfill respects the 80% ceiling. If post-compaction exceeds 80%, it accepts the state and continues.

---

## 4. Assembly Order

```
system message (AGENTS.md + IDENTITY.md + RULES.md + environment)
[Conversation arc] (moves, single system message)
...messages... (user, assistant, tool calls, tool results)
```

Reading top to bottom: who I am → what has happened → what's happening now.

Moves are injected as a single system message, not multiple. Environment info is appended to the system prompt, not a separate message.

---

## 5. Per-Turn Cycle

Between compactions, a turn is:

1. If a channel switch is pending, run session-start for the new channel first
2. Drain incoming messages from `MessageQueue`
3. Append them to the persistent context object as `ContextMessage` (with current `turn_number`)
4. Estimate total tokens (using calibrated estimator)
5. If below 80% → call model with current context, execute tools, append results (all with same `turn_number`)
6. If at/above 80% → run compaction, then call model with rebuilt context

The agent never observes compaction mechanics. It sends and receives messages. The context object handles the rest.

### Token estimation

Base estimator: `(text.len() + 3) / 4`.

**Calibration:** After each model response, update a calibration ratio using a weighted moving average:

```
new_sample = model_response.usage.prompt_tokens / estimated_prompt_tokens
ratio = 0.7 * old_ratio + 0.3 * new_sample
```

Apply the ratio to future estimates: `calibrated = base_estimate * ratio`.

The ratio tracks `prompt_tokens` specifically — the number of tokens the model saw in the prompt we sent. This is the stable metric for context size calibration. Completion tokens are irrelevant.

On the first turn (no prior actual), use the base estimator with `ratio = 1.0`. If the model returns 0 prompt tokens (error response), do not update the ratio. The weighted average smooths oscillations between content types (code-heavy vs prose turns) and self-corrects within a few turns.

---

## 6. Configuration

```rust
pub struct ContextConfig {
    /// Total context window size in tokens
    pub limit: usize,              // default: 128_000
    /// Compaction trigger (percent of limit)
    pub compaction_threshold: f64,  // default: 0.80
    /// Post-compaction fill target (percent of limit)
    pub fill_target: f64,           // default: 0.40
    /// Minimum messages always kept in context
    pub min_messages: usize,        // default: 20
}
```

Four knobs. No per-layer token budgets. The budget is dynamic based on what's actually in moves and messages.

Derived thresholds (not configured):
- Spectator lag warning: midpoint of `fill_target` and `compaction_threshold` (default: 60%)
- Turn lag threshold: 10 turns (hardcoded)

---

## 7. What Changes

### Replaced

| Current | New |
|---------|-----|
| `ContextBudget` (8 fixed slots) | `ContextConfig` (4 knobs) |
| Per-turn full rebuild | Persistent context object with append |
| Per-layer token truncation | Compaction at 80%, rebuild to 40% |
| Messages taken from tail, token-budgeted | Messages accumulate, compacted by spectator cursor |
| `ChatMessage` in context vector | `ContextMessage` wrapper with `turn_number` |
| Fixed token estimator | Calibrated estimator (WMA from model prompt_tokens feedback) |

### Rewritten

| File | Changes |
|------|---------|
| `agent/context.rs` | `ContextAssembler` rewritten — persistent context, `ContextMessage` wrapper, compaction logic, cursor query, spectator lag detection, calibrated token estimation, channel switching |
| `agent/task.rs` | Turn cycle simplified — append + threshold check instead of full assembly each turn. Pending channel switch applied at turn start. |

### Removed

- `ContextBudget` struct and its 8 fields
- `warm_flashes` budget slot
- `warm_retrieved` budget slot
- Per-turn context rebuild logic

### Deferred (not in this spec)

- **Flashes** — spectator-surfaced memories, injected between moves and messages
- **Retrieved** — vector search results, injected between moves and messages

These can be added later as additional layers between moves and messages without changing the compaction logic.

---

## 8. Data Dependencies

### Reads from DB

- `messages` table: messages by `turn_number` for session start and compaction backfill. Existing queries sufficient.
- `moves` table: `MAX(turn_number) WHERE channel = ?` for cursor. Moves by channel, ordered by `turn_number DESC`, for loading newest-first with budget cap.

### Reads from disk

- `workspace/AGENTS.md`, `workspace/IDENTITY.md`, `workspace/RULES.md` — at session start and each compaction

### No new tables or schema changes

The `messages` table has `turn_number`. The `moves` table has `turn_number` and `channel`. The cursor is `MAX(turn_number)`, not stored state.

### New DB query needed

`get_moves_newest_first(channel, limit)` — returns moves ordered by `turn_number DESC` with a row limit (default 50). Caller fetches in batches, estimating tokens as loaded, stopping when budget is full.

---

## 9. Testing

### Unit tests

- Token estimation (existing, keep)
- Calibration ratio: WMA update, apply, first-turn default (1.0), zero-token skip, smoothing across content types
- `ContextMessage` wrapper: wrap/unwrap roundtrip
- Compaction with cursor at various positions (0, mid, current)
- Compaction with NULL cursor (spectator never ran) — keeps all messages
- Turn-atomic drops: verify all messages in a turn share `turn_number`, dropped/kept together
- Gateway-injected messages get current `turn_number`
- Backfill when fewer than 20 messages above cursor
- Backfill stops when next turn would exceed 80% (no jitter loop)
- Moves loading newest-first with budget cap (50-row batches)
- Post-compaction above 80% does not re-trigger
- Spectator lag detection: warning injected when above 60% and 10+ turns behind
- Lag warning does not trigger re-compaction
- Assembly order verification (system → moves → messages)
- Channel switch: pending flag, applied at next turn start, context rebuilt

### Integration tests

- Session start: loads messages from DB above cursor + backfill, assembles at ~40%
- Accumulation: messages grow context toward 80%
- Compaction fires: drops below-cursor turns, reloads moves, rebuilds to ~40%
- Spectator behind: all messages kept, moves squeezed, warning injected
- Spectator caught up: compaction drops old messages, context returns to ~40%
- Empty moves: works with no spectator output (messages only)
- Long session: 100+ turns, moves loaded newest-first, oldest trimmed
- Channel switch: context rebuilt with new channel data, old channel data reloaded on switch-back
- Channel switch mid-turn: switch deferred to next turn start
- Calibration drift: verify ratio self-corrects over several turns with mixed content
- Backfill jitter: large turn below cursor does not cause compaction every turn
