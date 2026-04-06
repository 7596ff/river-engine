# Feature Landscape: Git-Based Workspace Sync

**Domain:** Dyadic agent orchestrator with git worktree coordination for two worker processes
**Researched:** 2026-04-06

## Executive Summary

A robust git-based workspace sync system for River Engine must handle two concurrent worker processes (actor and spectator) each operating on their own git worktree, with explicit merge conflict handling and eventual consistency guarantees. The research distinguishes between critical infrastructure features (table stakes), reliability improvements (differentiators), and deliberately excluded scope (anti-features).

The key tension: **Low frequency of actual conflicts in this domain** (workers operate on different concerns—actor captures moves, spectator curates moments) means conflict handling needs to be robust but not the primary optimization target. Priority instead goes to predictable sync mechanics and visibility into state divergence.

---

## Table Stakes

Features without which the system fails or becomes unusable. Missing any of these prevents moving from shared filesystem to distributed worktrees.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **Worktree per worker** | Each process needs isolated filesystem root to eliminate race conditions | Low | `git worktree add` per worker on startup; clean up on shutdown |
| **Atomic commit on write** | File writes must become durable as git commits, not raw FS writes | High | Intercept all writes through persistence layer; batch or immediate? |
| **Sync before read** | Worker must see partner's latest changes before assembling context | High | Pull from main/master before context assembly in worker loop |
| **Conflict detection** | System must surface when both workers modified the same file | Medium | `git merge --no-commit` to test merge, report conflicts to orchestrator |
| **Basic conflict resolution strategy** | Deterministic handling for conflicts (not failing, not silent data loss) | High | Actor takes precedence (actor is in control); spectator's changes deferred/lost? Or merge both? |
| **Git initialization & maintenance** | Worktrees must be created, reset on startup, cleaned up on shutdown | Medium | Orchestrator handles lifecycle; each worker has consistent git state |
| **Status visibility** | Worker must know if it's in sync or diverged from partner | Medium | Query `git status` and compare HEAD with remote/master |
| **Commit metadata** | Commits must be traceable to worker and action (e.g., "actor wrote identity.md") | Low | Include worker ID and action type in commit message or metadata |

---

## Differentiators

Features that improve robustness, observability, or user confidence. System can work without these, but UX suffers and debugging becomes harder.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Rebase instead of merge** | Cleaner history, avoids merge commits cluttering the story | Medium | `git rebase` after pull to keep linear history; requires careful conflict handling |
| **Conflict markers in workspace** | When conflict occurs, leave conflict markers in place so worker can see the divergence | Medium | Instead of hard rejection, let worker inspect `<<<<<<` markers and decide |
| **Three-way merge driver** | Smarter conflict resolution for workspace files (moves, moments) using content semantics | High | Could use Mergiraf or custom driver to merge JSONL files at element level instead of line level |
| **Sync heartbeat/monitoring** | Periodic health check showing worker sync status (behind/diverged/synced) | Low | Separate coroutine polling `git status` periodically; log or expose via metrics |
| **Pre-write conflict check** | Before persisting a change, verify it won't cause conflict on next pull | High | Run test merge before committing; if conflict would occur, queue differently or notify actor |
| **Conflict advisory to LLM** | If pull encounters conflict, inject "you have unresolved conflicts" into actor's context | Medium | Pass conflict list to context assembly; let actor decide what to do |
| **Stash on sync failure** | If worker can't merge, automatically stash local changes and retry | Medium | Recoverable state; worker loses local work but doesn't crash |
| **Automatic retry with backoff** | Transient git errors (lock file, network) trigger retry logic | Low | Standard error handling; essential for reliability |
| **Sync logs in workspace** | Write sync operations (pull, merge, conflict) to a `.sync/` log for audit trail | Low | Append to `.sync/sync.log`; include timestamps, status, conflict details |
| **Spectator-preferred merge** | For certain files (moments, moves), spectator's version takes precedence (inverted from actor) | Medium | Config option per file pattern; role-aware merge logic |
| **Deferred write queue** | If sync is failing, queue writes and retry periodically instead of failing immediately | High | Requires state machine for write persistence; complex but improves resilience |

---

## Anti-Features

Deliberately excluded. Including these would overcomplicate the system or solve non-problems.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Real-time collaboration (OT/CRDT)** | River's conflict rate is ~0% (actor/spectator work on different concerns). OT/CRDT complexity not justified. Git's 3-way merge is sufficient. | Accept eventual consistency; workers sync at turn boundaries, not keystroke-level |
| **Multi-dyad coordination** | Project scope is single dyad only. Multi-dyad coordination requires different conflict handling (e.g., priority-based resolution, distributed consensus) | If multi-dyad needed later, that's a different feature set; don't add now |
| **Lock-based file access** | File locks (Git LFS locking, mutex-based) introduce coordination overhead. Git's merge handles concurrent edits safely. | Use git's 3-way merge + conflict markers; locks are overkill |
| **Rebasing during actor's turn** | Actor is actively working; rebasing (which rewrites history) mid-turn creates confusion. | Only rebase between turns or when actor explicitly pauses; safer to merge-and-keep-history |
| **Automatic silent conflict resolution** | No heuristic (ours, theirs, fuzzy match) should silently pick a winner without visibility. Loss of data by accident. | Always surface conflicts; require explicit resolution (manual, or clear policy like "actor wins") |
| **Sync on every write** | Committing every keystroke kills performance. Git + filesystem have overhead. | Batch writes per turn; sync at turn boundaries or on explicit flush |
| **Distributed consensus for conflicts** | Don't implement voting or Byzantine agreement. Too complex for this use case. | Use simple rule: actor precedence, or defer to Ground (human operator) |
| **Partial sync (cherry-pick files)** | Selectively syncing some files and not others breaks workspace consistency. | Always sync full worktree state; no partial pulls |
| **Custom merge strategies for code** | Don't write a parser to merge Python/Rust syntax intelligently. Git's line-based merge works. Use Mergiraf only if JSONL merge becomes a bottleneck (unlikely). | Stick with git's default 3-way; improve if conflict rate becomes observable problem |
| **Network replication** | Don't replicate workspace over network (e.g., to backup server). Out of scope. | Local filesystem only; backup handled separately (NixOS persistence layer) |
| **Merge commit messages** | Don't auto-generate elaborate merge commit messages. Just "Merge from partner" is enough. | Simple, consistent messages; save narrative effort for workspace notes |

---

## Feature Dependencies

```
Worktree per worker
  ↓
Atomic commit on write ← requires Commit metadata
  ↓
Sync before read ← requires Conflict detection
  ↓
Basic conflict resolution strategy
  ↓ (improves)
Status visibility

Optional enhancements:
Pre-write conflict check → improves Atomic commit on write
Conflict advisory to LLM → builds on Conflict detection + Basic resolution
Sync logs → depends on all sync operations
Stash on sync failure → depends on Sync before read
Spectator-preferred merge → requires Custom resolution per file
Deferred write queue → complex state machine; optional
```

---

## MVP Recommendation

**Phase 1: Core Functionality (Table Stakes)**

Implement worktree infrastructure and basic sync in this order:

1. **Worktree per worker** — Each worker gets isolated `git worktree`. Orchestrator manages lifecycle.
2. **Atomic commit on write** — All persistence goes through git commits, not raw FS writes.
3. **Sync before read** — Worker pulls before context assembly.
4. **Conflict detection** — System detects conflicts and reports to orchestrator.
5. **Basic conflict resolution** — Actor changes take precedence; spectator's overwritten (lossy but deterministic).
6. **Commit metadata** — Worker ID + action type in every commit.
7. **Status visibility** — Worker can query if in sync or diverged.
8. **Git initialization** — Orchestrator creates, resets, cleans up worktrees.

**Phase 2: Robustness (High-Value Differentiators)**

After Phase 1 works end-to-end:

1. **Conflict advisory to LLM** — If conflict occurs, inject warning into actor's context so they're aware. This is low-complexity, high-value for debugging.
2. **Sync logs** — Audit trail of all sync operations. Invaluable for understanding why state diverged.
3. **Automatic retry with backoff** — Transient errors don't crash the system.
4. **Stash on sync failure** — Recoverable state if merge fails.

**Defer to Later:**

- Three-way merge driver (add only if JSONL conflicts become noticeable)
- Rebase (adds history complexity; not worth it for this use case)
- Spectator-preferred merge (first confirm conflict rate in practice)
- Deferred write queue (premature optimization)
- Pre-write conflict check (adds latency; conflicts are rare)

---

## Complexity Breakdown

| Feature | Effort | Why |
|---------|--------|-----|
| **Worktree per worker** | Low | Standard git API; single-threaded orchestrator |
| **Atomic commit on write** | High | Requires wrapping all persistence; choose sync timing (per-turn vs per-write) |
| **Sync before read** | Medium | Pull + error handling; must not block worker loop |
| **Conflict detection** | Medium | Run test merge; parse conflict markers |
| **Basic conflict resolution** | Medium | Actor-wins strategy is simple; harder if needs semantics |
| **Status visibility** | Low | Query `git status`, compare HEAD with remote |
| **Commit metadata** | Low | Format string in commit message |
| **Git initialization** | Medium | Error handling for race conditions; ensure clean state |
| **Conflict advisory to LLM** | Low | Add conflict list to context assembly |
| **Sync logs** | Low | Append to file; structured format (JSONL or TSV) |
| **Automatic retry** | Low | Standard exponential backoff pattern |
| **Stash on failure** | Low | `git stash` API call; recover state after merge fails |

---

## Key Questions for Phase-Specific Research

1. **Atomic commit timing:** Per-turn or per-write? Batching writes improves performance but delays consistency.
   - Per-turn: Simple, matches turn boundaries naturally.
   - Per-write: Immediate consistency; requires careful ordering to avoid commit storms.

2. **Spectator's role in conflicts:** If spectator's changes get overwritten by actor, does spectator re-apply them next turn?
   - Today: Actor wins (lossy). Spectator loses curation work.
   - Alternative: Merge both (requires smarter resolution); preserves spectator's work but risks incoherence.

3. **Conflict rate hypothesis:** We expect ~0% actual conflicts (different workspaces, different concerns). If observed rate is higher, architecture needs rethinking.

4. **Merge conflict markers in LLM context:** If actor is presented with `<<<<<<` markers in their context, do they understand them? May need explicit instruction.

---

## Implementation Constraints

- **Rust + Tokio:** Async I/O required; all git operations must be non-blocking.
- **Two processes only:** No multi-dyad logic; keep coordination simple.
- **Local filesystem:** No network sync; worktrees are on same machine.
- **TUI testing first:** Must validate with local mock adapter before Discord integration.
- **NixOS deployment:** Git and filesystem operations must work in Nix sandbox; avoid assumptions about /tmp or home directories.

---

## Success Criteria for Feature Validation

By end of Phase 1:

- [ ] Two workers can start with isolated worktrees
- [ ] Changes committed by one worker are visible to the other after sync
- [ ] Conflicts are detected and reported (not silently dropped)
- [ ] Sync happens before context assembly; worker sees partner's changes
- [ ] Commits are traceable (worker ID + action in message)
- [ ] System doesn't lose data (even if conflict handling is lossy, it's explicit)
- [ ] TUI adapter test run completes without sync-related crashes

By end of Phase 2:

- [ ] Conflict advisory appears in LLM context
- [ ] Sync logs are generated and readable for debugging
- [ ] Transient git errors don't crash the system
- [ ] Stash-on-failure recovery works without data loss
- [ ] Observed conflict rate is measured and documented

---

## Sources

- [Git Worktrees Documentation](https://git-scm.com/docs/git-worktree) — Official git worktree API
- [Using Git Worktrees for Concurrent Development - Ken Muse](https://www.kenmuse.com/blog/using-git-worktrees-for-concurrent-development/) — Practical patterns for multi-process coordination
- [How to Fix Merge Conflict Errors in Git - OneUptime](https://oneuptime.com/blog/post/2026-01-24-git-merge-conflict-errors/view) — Current (2026) conflict resolution strategies
- [Building Real-Time Collaboration: OT vs CRDT - Tiny Cloud](https://www.tiny.cloud/blog/real-time-collaboration-ot-vs-crdt/) — Why OT/CRDT overkill for low-conflict scenarios
- [GitHub Blog: Git's Database Internals](https://github.blog/open-source/git/gits-database-internals-iv-distributed-synchronization/) — Distributed sync semantics
- [Coordination Avoidance in Database Systems - Berkeley AmpLab](https://amplab.cs.berkeley.edu/wp-content/uploads/2014/10/p168-bailis.pdf) — Theoretical basis for avoiding unnecessary coordination
- [Memory in LLM-based Multi-agent Systems - TechRxiv](https://www.techrxiv.org/users/1007269/articles/1367390) — Conflict resolution patterns in multi-agent systems
