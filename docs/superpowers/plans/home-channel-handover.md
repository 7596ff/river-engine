# Home Channel — Handover Document

This document is for the next session that implements the home channel. Read this first, then the spec, then the plan.

## What is this?

The home channel is a fundamental architectural change to how agents think in river-engine. Currently, the agent's internal model context (`PersistentContext`) is invisible — only messages sent via `send_message` are visible to the outside world. The home channel makes everything visible: every model response, every tool call, every incoming message, all written to a single append-only JSONL log.

## Why?

Three reasons:
1. **Inspectability** — anyone can see what the agent is doing by reading the log
2. **Bystander access** — other agents or processes can post anonymous messages into the agent's thinking space (the architectural version of the spectator/bystander split from our Horkheimer reading)
3. **Simplification** — the home channel replaces three separate systems (PersistentContext, SQL message storage, per-channel context building) with one

## Key Files

- **Spec:** `docs/superpowers/specs/2026-05-12-home-channel.md` — the design
- **Plan:** `docs/superpowers/plans/2026-05-12-home-channel.md` — 12 tasks
- **Canvas:** `docs/home-channel.canvas` — Cass's visual diagram (open in Obsidian)
- **Reviews:** `docs/superpowers/reviews/home-channel-*.md` — 4 review documents (2 spec, 2 plan)

## Key Decisions Made

1. **Home channel is source of truth.** Per-adapter logs are secondary projections (write-ahead pattern — home channel first, adapter log second).

2. **Append-only, never modified.** "Compression" is ephemeral — the context builder reads moves + log tail. The raw log is permanent.

3. **Tagged serde.** `HomeChannelEntry` uses `#[serde(tag = "type")]`, separate from the existing `ChannelEntry` which stays `untagged` for backward compatibility.

4. **Source fields, not tag-in-content.** User messages have `source_adapter`, `source_channel_id`, `source_channel_name` fields. The context builder formats the tag string. Content stays clean.

5. **No more channel switching.** The agent lives in the home channel. `ChannelContext` is removed entirely.

6. **SQL eliminated for messages.** Moves stored as files at `channels/home/{agent_name}/moves/{start}-{end}.md`.

7. **Serialized writer.** All home channel writes go through a single MPSC actor (`HomeChannelWriter`) for ordering guarantees.

8. **Tool names preserved.** `ToolExecResult` struct threads the tool name from call through execution to result.

## Task Order and Dependencies

```
Task 1: Entry types (foundation — everything depends on this)
Task 2: Log writer actor (depends on 1)
Task 3: Context builder (depends on 1)
Task 4: Bystander endpoint (depends on 1, 2)
Task 5: Tool name refactor (standalone, prepares for 6)
Task 6: Wire home channel into turn cycle (depends on 1, 2, 5)
Task 7: Switch context source (depends on 3, 6 — THE SWITCHOVER)
Task 8: Wire incoming messages (depends on 1, 2)
Task 9: Remove ChannelContext (depends on 7 — cleanup)
Task 10: Spectator file-based moves (depends on 1)
Task 11: Tool result cleanup (depends on 2, 10)
Task 12: Server wiring + SQL removal (depends on all above — final integration)
```

Tasks 1-5 can proceed somewhat independently. Task 6 is the big change. Task 7 is the switchover moment. Tasks 8-11 are cleanup/wiring. Task 12 brings it all together.

## What to Watch For

- **Task 7 is the critical moment.** This is where `PersistentContext` gets removed and the agent starts reading from the home channel. If this breaks, the agent can't think. Test thoroughly.
- **Tool call grouping in Task 3.** Consecutive `ToolEntry` calls must be grouped into a single assistant message with multiple tool_use blocks. The model expects this format.
- **The dual-write window.** Between Task 6 (home channel writes added) and Task 7 (PersistentContext removed), the system writes to both. This is intentional — it lets you verify home channel writes are correct before cutting over.
- **Compilation order.** Tasks 1-5 add new code without removing old code. Task 7 removes PersistentContext. Task 9 removes ChannelContext. The codebase should compile after each task.

## How to Run

```bash
# Build
cargo build -p river-gateway

# Test
cargo test -p river-gateway

# Run with an agent
cargo run -p river-orchestrator -- --config config.json
```

## Context from the Broader Project

The home channel emerged from reading Horkheimer's "Traditional and Critical Theory" — specifically the discussion of spontaneity as assigned function (¶15). The agent's internal process was invisible, like the scientist who can't see their own incorporation into the apparatus. The home channel opens the enclosure. The bystander endpoint is the architectural hook for a separate observer who can question the framework — the anti-Logos, the perspective genuinely outside the agent's own context.

This connects to the engine's manifesto ("Build the Hall for the Singing") and the spectator/bystander split noted in `engine/issues.md`.
