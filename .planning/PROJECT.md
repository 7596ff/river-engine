# River Engine

## What This Is

An agent orchestrator implementing dyadic architecture — two AI instances alternate between actor and spectator roles, witnessing each other. The spectator sees patterns the actor cannot see about themselves. Built in Rust with NixOS deployment, designed for a human operator (Ground) who has full access and final say.

## Core Value

Two perspectives that can disagree. The gap between them is the point — it creates internal structure that a single rule-follower cannot have.

## Requirements

### Validated

<!-- Shipped and confirmed valuable. Inferred from existing codebase. -->

- ✓ Orchestrator supervises workers and adapters — existing
- ✓ Workers run think→act LLM loop with tool execution — existing
- ✓ Role switching (baton swap) between actor and spectator — existing
- ✓ Discord adapter translates external events to River protocol — existing
- ✓ TUI mock adapter for local testing — existing
- ✓ Context assembly builds LLM messages from workspace files — existing
- ✓ 12 tools (read, write, bash, speak, switch_roles, flash, sleep, etc.) — existing
- ✓ Registration and service discovery via orchestrator — existing
- ✓ Health checks and process respawn with backoff — existing
- ✓ Snowflake ID generation for messages — existing
- ✓ Vector embedding service with SQLite storage — existing
- ✓ Error paths return Result types instead of panicking — Phase 1 complete
- ✓ Orchestrator creates git worktrees at dyad startup — Phase 2 complete
- ✓ Workers receive worktree paths via registration protocol — Phase 2 complete

### Active

<!-- Current scope. Building toward these. -->

- [ ] Workspace instructions tell agents when/how to sync via bash tool
- [ ] Agents follow sync protocol using existing bash tool (no new Rust code)
- [ ] Dyad boots and runs end-to-end with TUI mock adapter

### Out of Scope

<!-- Explicit boundaries. Includes reasoning to prevent re-adding. -->

- Discord integration for v1 — test locally with TUI first, add Discord after core loop verified
- Multi-dyad coordination — single dyad sufficient for initial testing
- Authentication/TLS — localhost only for v1, security layer added later
- Logging/monitoring infrastructure — defer until core functionality stable
- Stream cancellation for flashes — low priority, flash queues until LLM response completes

## Context

The project is ~90% implemented. The Rust crates form a supervision tree:
- `river-orchestrator` spawns and monitors all processes
- `river-worker` runs the agent loop (two instances per dyad)
- `river-discord` and `river-tui` are adapters
- `river-context` assembles LLM prompts from workspace
- `river-embed` provides vector search for the zettelkasten memory system

The architecture is philosophically grounded in a response to the Chinese Room problem. Searle's thought experiment has one rule-follower. River has two processes that can disagree — the spectator can ask "was that genuine?" in a way the actor cannot ask of themselves.

Current code assumes shared filesystem for workspace. The goal is git worktrees — each worker owns their worktree, syncs via git, handles conflicts. This eliminates race conditions and makes the coordination explicit.

**Key insight:** Agents have a bash tool. Git sync is behavioral, not code — agents follow instructions in workspace docs to commit, pull, and resolve conflicts. No new Rust crate needed.

## Constraints

- **Stack**: Rust 2021, Tokio async runtime, Axum HTTP — established, not changing
- **Deployment**: NixOS modules exist, systemd integration — maintain compatibility
- **LLM Protocol**: OpenAI-compatible API — workers already implement this
- **Testing**: Must work with TUI mock adapter before Discord — reduces variables

## Key Decisions

<!-- Decisions that constrain future work. Add throughout project lifecycle. -->

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Git worktrees for workspace isolation | Eliminates filesystem race conditions, git handles merge semantics | ✓ Phase 2 |
| Instructions not code for git sync | Agents have bash tool; behavioral protocol simpler than Rust code | — Pending |
| Fix panics before testing | Crashes on unexpected input make debugging harder | ✓ Phase 1 |
| TUI testing before Discord | Fewer moving parts, faster iteration | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd-transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd-complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-04-06 after Phase 2 completion*
