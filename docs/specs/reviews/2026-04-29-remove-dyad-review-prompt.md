# Review Prompt: Remove Dyad Spec

Paste this prompt into Gemini with access to the river-engine codebase.

---

You are reviewing a design spec for a major refactor of the river-engine Rust workspace. The spec is at `docs/superpowers/specs/2026-04-29-remove-dyad-design.md`. Read it in full.

Your job is adversarial review. You are not here to be helpful. You are here to find every gap, contradiction, omission, and lie in this document. The authors think they're done. Prove them wrong.

## Instructions

### 1. Verify completeness against the actual code

Read every file in the codebase. For every struct, enum, function, trait, CLI arg, HTTP endpoint, config field, and test that references `dyad`, `Side`, `Baton`, `left`, `right`, `actor`, `spectator`, `partner`, `switch`, `role`, or `worktree`:

- Is it accounted for in the spec?
- Does the spec say what happens to it — deleted, renamed, or kept?
- If kept, does the spec explain why?

List every reference the spec misses. File path, line number, what it is, why it matters.

### 2. Find internal contradictions

Read the spec's own claims against each other:

- Does the new `AgentConfig` actually contain everything the worker needs based on what the spec says the worker does?
- Does the new registration flow actually work? Trace it: worker starts → sends registration → orchestrator responds → worker initializes state. Does every field the worker needs appear in the response?
- Does the respawn flow work? Worker exits → sends output → orchestrator processes → respawns. Are the types consistent between what the worker sends and what the orchestrator expects?
- The spec removes `start_sleeping` from the registration response but keeps the respawn system. How does a respawned worker that should sleep know to sleep? Is this a hole?

### 3. Find implicit dependencies

The spec says certain crates are "untouched" — `river-context`, `river-embed`, `river-snowflake`. Verify this:

- Read every file in those crates
- Do any of them import `Side`, `Baton`, or anything else being deleted?
- Do any of them reference `dyad` in struct fields, function parameters, or comments?
- If yes, the spec is wrong about them being untouched

### 4. Find behavioral changes the spec doesn't acknowledge

The spec frames this as "surgical removal" but some changes alter runtime behavior:

- The old system spawned two workers per dyad. The new system spawns one. What happens to adapters that assumed they'd be routed to the "actor" worker? Does the adapter registration response still make sense?
- The old worker could be told to sleep on startup via `start_sleeping` in the registration response. The spec removes this. What replaces it? Is there a behavioral gap?
- The old system had worktrees so two workers wouldn't collide on git. With one worker, is there still a git collision risk with the embed service or adapters writing to the same workspace?
- The old `force_summary` included `side` in its output. The new one doesn't. Does any downstream consumer depend on knowing which side produced the summary?

### 5. Find things the spec should say but doesn't

- Migration path: what happens to existing config files? Is there a migration script or is it manual?
- What happens to existing conversation files (`context.jsonl`) that live in worktree paths like `workspace/left/context.jsonl`? After refactor, the worker expects `workspace/context.jsonl`.
- The old e2e test (`e2e_dyad_boot.rs`) is deleted. The spec says to add a new e2e test. What does it actually test? "Worker registers and gets config back" is vague. What are the assertions?
- What about the `src/river-gateway/` directory? The grep found matches there. The spec doesn't mention it at all.

### 6. Grade the spec

After completing the above, assign grades:

- **Completeness** (A-F): Does it account for every change needed?
- **Consistency** (A-F): Do its own claims agree with each other?
- **Precision** (A-F): Are the changes specific enough to implement without guessing?
- **Honesty** (A-F): Does it acknowledge what it doesn't know or might break?

For each grade below B, explain exactly what would raise it.

## Output format

Structure your review as:

1. **Missed references** — table of file, line, symbol, what the spec should say
2. **Internal contradictions** — numbered list with quotes from the spec
3. **Implicit dependency failures** — which "untouched" crates are actually touched
4. **Behavioral gaps** — what changes at runtime that the spec doesn't discuss
5. **Omissions** — things the spec should address but doesn't
6. **Grades** — with justification

Be specific. Quote the spec. Quote the code. No hand-waving.
