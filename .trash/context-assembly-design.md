# Context Assembly Design

> How the context window is built and maintained across turns.

---

## Core idea

The context window has two fixed regions: a warm layer (moves) that holds the structural arc of the conversation, and a hot layer (messages) that holds recent exchanges. Messages accumulate until the window approaches capacity, then compaction drops the oldest messages — which are already covered by moves and don't need to be re-summarized. Nothing is discarded that isn't already captured.

The assembled context is a persistent object. It is built once at session start, then reused — new messages are appended in place as the conversation proceeds. Full rebuild only happens when compaction is triggered. All of this is transparent to the agent: it sends and receives messages normally and never observes the compaction mechanics.

---

## Budget

| Layer | Allocation |
|-------|------------|
| System | Actual size, non-negotiable |
| Moves | Up to 40% of context limit |
| Messages | 20 message cap |
| **Compaction threshold** | **~80% of context limit** |

With a 128K window: moves cap at ~51K, compaction fires around 102K total. The remaining 20% is headroom for output.

---

## Layers

### System

Identity files from the workspace root, concatenated:
- `AGENTS.md` — protocol
- `IDENTITY.md` — who the agent is
- `RULES.md` — behavioral constraints

Plus environment info: current date, cwd, platform, git branch/status.

Loaded once at session start. Non-negotiable.

### Warm: Moves

All move files from `embeddings/moves/`, sorted lexicographically (filename = timestamp = chronological order). YAML frontmatter stripped. Concatenated and injected as a single `[Conversation arc]` message.

The structural arc of the conversation — what happened, what shifted, what threads are open. Written by the spectator asynchronously.

Capped at 40% of context limit. If moves exceed the cap, oldest are trimmed.

**Reloaded at compaction.** Between compactions, the moves layer is stable. This is the sync point where new spectator moves enter context.

### Hot: Messages

The in-memory rolling buffer of conversation messages. Capped at 20 messages.

**Compaction trigger:** when total estimated tokens (system + moves + messages) exceeds ~80% of context limit.

**Compaction:** drop oldest messages where `turn_number <= cursor_turn`. The spectator cursor (in `embeddings/moves/.cursor`) records the last turn compressed into moves — those messages are already represented in the warm layer. Dropping them is safe; nothing is lost.

After compaction, reload moves (picks up any new spectator output), then continue appending.

---

## Per-turn cycle

The context starts small — system + moves + whatever messages exist — and grows as the conversation proceeds. The 80% threshold is a compaction trigger, not a fill target.

1. Append incoming message to the saved context object
2. Estimate total tokens
3. If below 80% threshold → reuse context as-is
4. If at or above threshold → compact and rebuild:
   a. Drop messages where `turn_number <= cursor_turn`
   b. Reload moves from `embeddings/moves/`
   c. Reconstruct context object from the remaining messages + updated moves
   d. If still over threshold (spectator is behind), drop further back until under

---

## Assembly order

```
system message
[Conversation arc] (moves)
...hot messages...
```

Reading top to bottom: identity → arc → what's happening now.

---

## What's deferred

- **Flashes** — high-salience moments surfaced by the spectator, injected between moves and hot
- **Retrieved** — vector search results from the embeddings layer, injected between moves and hot

---

## Configuration

```json
{
  "context": {
    "limit": 128000,
    "moves_max_percent": 40,
    "hot_max_messages": 20,
    "compaction_threshold_percent": 80
  }
}
```
