# Review Prompt: Channel Message Design

Review the spec at `docs/superpowers/specs/2026-05-03-channel-message-design.md` as a design specification for a message delivery system in an AI agent runtime.

## Context

This is a Rust application (`river-engine`) with:
- A gateway (`river-gateway`) that receives messages from communication adapters (Discord, etc.) via HTTP
- An agent task that runs a wake/think/act/settle turn cycle
- A spectator (bystander) task that observes the agent
- A coordinator with an event bus connecting them
- Snowflake IDs that encode both uniqueness and temporal ordering

The spec replaces a broken message delivery path where the HTTP handler writes to an inbox file but never notifies the agent task.

## Review Criteria

1. **Internal consistency** — do any parts of the spec contradict each other?
2. **Completeness** — are there flows or edge cases the spec doesn't cover? Can an implementer build this without guessing?
3. **Ambiguity** — could any requirement be interpreted two ways?
4. **Simplicity** — is this the simplest design that solves the problem, or is there unnecessary complexity?
5. **Concurrency** — the handler and agent run on different tokio tasks. Are there race conditions in the JSONL append + queue push sequence? What happens if the agent reads the log between the append and the push? What happens if two messages arrive simultaneously?
6. **Failure modes** — what happens if the JSONL write fails? If the queue push fails? If the log file is corrupted mid-line?
7. **The cursor mechanism** — scanning backward for the last `role: agent` entry works on small logs. What happens at scale (100k+ lines)? Is there a more efficient approach that preserves the elegance of "the log is the state"?
8. **The "What This Replaces" section** — is the migration path clear? Are there dependencies that would break if the listed code is removed?

## What to produce

A structured review with findings organized by severity (critical, important, suggestion). No edits to the spec — findings only.
