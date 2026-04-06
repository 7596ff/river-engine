# Phase 3: Sync Protocol Documentation - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-06
**Phase:** 03-sync-protocol-documentation
**Areas discussed:** Documentation location, Commit triggers, Sync timing, Conflict handling

---

## Documentation location

| Option | Description | Selected |
|--------|-------------|----------|
| New workspace/shared/sync.md | Dedicated file for sync protocol — clean separation, easy to update, follows reference.md pattern | ✓ |
| Extend workspace/shared/reference.md | Add sync section to existing reference — keeps all technical docs in one place | |
| Split across role docs | Actor.md gets actor sync rules, spectator.md gets spectator rules — role-specific, but duplicates shared protocol | |

**User's choice:** New workspace/shared/sync.md (Recommended)
**Notes:** None

| Option | Description | Selected |
|--------|-------------|----------|
| Yes, add a brief mention | One paragraph in README explaining shared workspace requires sync, link to sync.md for details | ✓ |
| No, keep README philosophical | README stays about the dyad concept; reference.md and sync.md are the technical docs | |

**User's choice:** Yes, add a brief mention (Recommended)
**Notes:** README gets a brief mention with link to sync.md

---

## Commit triggers

| Option | Description | Selected |
|--------|-------------|----------|
| At natural checkpoints | Commit before sleep, before role switch, before going idle — batches related changes, clean commit history | |
| After every write | Commit immediately after each file write — maximizes durability, creates many small commits | |
| Only on role switch | Commit only when switching from actor to spectator — simple but loses work on crash | |

**User's choice:** "commit often, use multiple branches, create flattened commits for merges"
**Notes:** Agents commit frequently on personal branches (many small commits OK), squash merge when syncing to main for clean shared history

| Option | Description | Selected |
|--------|-------------|----------|
| Summary of what changed | "Left worker: updated notes, added memory entry" — brief description of the batch | |
| Standard format | "sync(left): [timestamp]" — formulaic, lets the diff speak for itself | |
| You decide | Claude determines the appropriate commit message format | ✓ |

**User's choice:** Agent decides
**Notes:** Commit message content at agent discretion

| Option | Description | Selected |
|--------|-------------|----------|
| Guidelines with discretion | "Commit after substantive changes or before transitions" — agent decides what counts | ✓ |
| Strict timing | "Commit at least every N tool calls" — mechanical but consistent | |
| Every write | "Commit after every file write" — maximum granularity | |

**User's choice:** Guidelines with discretion (Recommended)
**Notes:** None

---

## Sync timing

| Option | Description | Selected |
|--------|-------------|----------|
| Before starting new work | Actor syncs before processing messages, spectator syncs before reviewing — ensures fresh state | ✓ |
| On role switch | Sync when becoming actor or spectator — simple trigger, natural boundary | |
| On explicit trigger only | Agents sync when they choose to — maximum autonomy, risk of stale state | |

**User's choice:** Before starting new work (Recommended)
**Notes:** None

| Option | Description | Selected |
|--------|-------------|----------|
| At turn start | Sync at the beginning of each worker loop iteration — fresh state before any decisions | ✓ |
| Before external response | Sync only before responding to external messages — internal work can use stale state | ✓ |
| Agent discretion | Agent decides when fresh state matters — protocol describes what sync does, not when | ✓ |

**User's choice:** All three — "at turn start, before response, and at agent discretion"
**Notes:** Comprehensive sync timing: default at turn start, mandatory before external response, additional at discretion

| Option | Description | Selected |
|--------|-------------|----------|
| No explicit notification | Agents discover changes when they sync — simple, no coordination overhead | |
| Flash notification | Push triggers a flash to partner — "New changes available" — but not mandatory | |
| You decide | Claude determines if notification adds value | |

**User's choice:** "agents create pull requests"
**Notes:** PR-style flow — agents merge to main, partner pulls to see changes (deliberate handoff without external dependencies)

| Option | Description | Selected |
|--------|-------------|----------|
| Git commands only | Use git merge with appropriate flags — no GitHub/GitLab dependency, works locally | ✓ |
| Git + gh CLI | Use gh pr create/merge if available — supports remote repos, more ceremony | |
| You decide | Claude picks the simplest approach that achieves the review/handoff | |

**User's choice:** Git commands only (Recommended)
**Notes:** Pure git commands, no external tooling required

---

## Conflict handling

| Option | Description | Selected |
|--------|-------------|----------|
| Resolve then continue | Agent reads both versions, chooses resolution, completes merge — fully autonomous | |
| Escalate to Ground | Agent pauses and asks Ground to resolve — human decides contested content | |
| Keep both (defer) | Agent saves both versions (e.g., file.left.md, file.right.md), continues — resolution later | |

**User's choice:** "try to resolve, but if conflict arises, escalate to ground"
**Notes:** Escalation path — attempt resolution first, genuine conflicts get human oversight

| Option | Description | Selected |
|--------|-------------|----------|
| Content conflicts escalate | If both workers changed the same lines, escalate. Non-overlapping edits resolve automatically. | |
| Semantic conflicts escalate | If the changes contradict in meaning (not just lines), escalate. Agent uses judgment. | |
| Agent discretion | Agent decides based on the nature of the conflict and their confidence in the resolution | ✓ |

**User's choice:** Agent discretion
**Notes:** Agent determines what counts as genuine conflict requiring escalation

| Option | Description | Selected |
|--------|-------------|----------|
| Flash Ground via backchannel | Send message explaining conflict, provide both versions, wait for guidance | |
| Create conflict artifact | Write conflict details to a file (e.g., conflicts/pending.md), Ground reviews async | |
| You decide | Claude determines the most effective escalation method | |

**User's choice:** "notify ground as a basic instruction, the agent should know what that means. and create the artifact."
**Notes:** Both notification and artifact — immediate alert plus persistent record

| Option | Description | Selected |
|--------|-------------|----------|
| Yes, include avoidance guidelines | e.g., Actor owns notes/left/, Spectator owns moves/ — reduces collision by convention | ✓ |
| No, focus on resolution | The protocol handles conflicts when they occur; no ownership rules | |
| Brief mention only | Note that frequent commits and syncs reduce conflict likelihood — no strict ownership | |

**User's choice:** "yes, lets create file ownership structure"
**Notes:** Ownership by role to prevent conflicts

| Option | Description | Selected |
|--------|-------------|----------|
| Role-based ownership | Actor owns: notes/, artifacts/, conversations (writes). Spectator owns: moves/, moments/, embeddings/ | ✓ |
| Side-based ownership | Left worker owns: left/. Right worker owns: right/. Shared dirs have merge-on-conflict | |
| Hybrid | Some dirs by role (spectator owns moves/), some by side (each owns their identity dir) | |
| You decide | Claude determines sensible ownership based on existing workspace patterns | |

**User's choice:** Role-based ownership (Recommended)
**Notes:** Aligns with existing actor.md/spectator.md responsibilities

---

## Claude's Discretion

- Exact git commands and flags in sync.md
- Commit message format examples
- How to present conflict details in escalation artifact
- Any additional sync scenarios beyond the core cases

## Deferred Ideas

- Git hooks or automation for commit/sync triggers
- Workspace cleanup/gc for old branches
- Conflict resolution tools beyond bash git commands
