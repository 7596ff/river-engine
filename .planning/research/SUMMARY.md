# Research Summary: Git Worktree Coordination Stack

**Project:** River Engine (Rust agent orchestrator with git worktree workspace sync)
**Researched:** 2026-04-06
**Overall Confidence:** HIGH (core stack decisions) / MEDIUM (implementation edge cases)

---

## Executive Summary

River Engine should adopt **git2 0.20.4 + subprocess git CLI wrapping** for git worktree coordination. git2 provides the essential infrastructure—merge conflict detection via `merge_commits()`, index inspection for conflict analysis, branch/ref management—while subprocess wrapping handles the narrow set of worktree-specific operations not exposed in git2-rs (worktree creation, pruning). This hybrid approach is pragmatic, proven in production multi-agent systems (Vibe Kanban, Composio, Worktrunk 2026), and avoids both the cost of unsafe FFI bindings and the risk of pure-CLI error parsing.

**Key technical finding:** git2-rs exposes `WorktreeAddOptions` at the raw FFI layer, but the high-level API lacks a `Repository::worktree_add()` method. This gap is acceptable for River's use case—worktree creation is a one-time worker bootstrap operation, not a hot-path sync concern. The sync loop's critical operation—merge detection—is fully supported and mature.

**Gitoxide (0.81.0) is not recommended for v1**, despite being pure Rust. Worktree API documentation describes it as aspirational; creation functions aren't stable. Revisit in 2026 H2 when gix stabilizes worktree support as a future alternative.

---

## Key Findings

### 1. git2-rs Status & Worktree Coverage

**Finding:** git2 0.20.4 has mature libgit2 bindings with excellent merge/conflict support but incomplete worktree creation exposure.

| Operation | Available in git2-rs | Recommendation |
|-----------|---|---|
| `Repository::worktrees()` — list | ✓ Yes | Use directly |
| `Repository::find_worktree()` — open | ✓ Yes | Use directly |
| `Worktree::validate()` | ✓ Yes | Use directly |
| `Worktree::lock()` / `unlock()` | ✓ Yes | Use directly |
| `Repository::merge_commits()` — critical | ✓ Yes | Use directly |
| `Index::has_conflicts()` — critical | ✓ Yes | Use directly |
| `Repository::worktree_add()` — create | ✗ No | Wrap CLI via tokio::process |

**Source:** [git2 0.20.4 docs](https://docs.rs/git2/0.20.4/git2/struct.Repository.html), [libgit2 worktree API](https://libgit2.org/docs/reference/main/worktree/index.html)

### 2. Why Subprocess Wrapping Is Appropriate

**Finding:** Creating worktrees is infrequent (once per worker), not a sync loop bottleneck. Subprocess wrapping is ecosystem standard for advanced git features unsupported by libgit2 bindings.

**Evidence:**
- **Vibe Kanban (BloopAI):** Uses hybrid approach (git2 + CLI) due to sparse-checkout limitations in libgit2
- **Worktrunk (2026):** CLI-based worktree manager, most popular in AI agent parallel workflows
- **Composio's Agent Orchestrator:** Each agent gets `git worktree` + branch + PR — uses CLI wrapping

**Tokio integration:** River already uses Tokio; `tokio::process::Command` is async-safe, handles signals properly, avoids blocking. No additional runtime overhead.

**Source:** [Vibe Kanban git integration](https://deepwiki.com/BloopAI/vibe-kanban/2.4-github-integration-and-pr-workflow), [Worktrunk](https://github.com/max-sixty/worktrunk), [Composio Agent Orchestrator](https://github.com/ComposioHQ/agent-orchestrator)

### 3. Merge Conflict Handling (Core to Sync)

**Finding:** git2 provides typed, programmatic conflict detection—essential for robust worker sync.

**API Available:**
```rust
let merge_index = repo.merge_commits(&base_commit, &worker_commit)?;
if merge_index.has_conflicts() {
    // Iterate conflicts safely, no string parsing needed
}
```

**Why this matters:** Workers sync by merging main branch into worktree. Conflicts block further operations. git2's `Index::conflicts()` allows programmatic detection without regex parsing of `git status` or `git merge --no-commit` output.

**Source:** [git2 Repository::merge_commits()](https://docs.rs/git2/0.20.4/git2/struct.Repository.html#method.merge_commits), [libgit2 merge reference](https://libgit2.org/docs/reference/main/merge/git_merge.html)

### 4. Gitoxide Not Ready (Yet)

**Finding:** Gitoxide 0.81.0 is pure Rust, aspirational alternative; worktree API not stable enough for v1.

**Status:**
- ✓ Worktree utilities available (via gix-worktree crate)
- ✓ Repository abstraction supports checkout/reset
- ✗ Worktree creation ("worktree add") not documented as stable API
- ? Merge conflict handling coverage unclear relative to libgit2

**Recommendation:** Monitor gitoxide roadmap; defer to git2 for v1. Safe to evaluate gitoxide for merge operations once gix stabilizes worktree API.

**Source:** [gitoxide GitHub](https://github.com/GitoxideLabs/gitoxide), [gix 0.81.0 docs](https://docs.rs/gix/latest/gix/), [gix-worktree](https://docs.rs/gix-worktree/)

### 5. No Hidden Complexity in Async Integration

**Finding:** Tokio subprocess wrapping is straightforward; no special concurrency concerns.

- tokio::process::Command is async-aware
- `spawn()` → `wait_with_output()` → error handling pattern is standard
- Git operations are filesystem-bound (I/O), not CPU-bound; async overhead minimal
- River already spawns worker processes; subprocess pattern proven

**Source:** [Tokio process module](https://docs.rs/tokio/latest/tokio/process/struct.Command.html), Tokio async ecosystem patterns

---

## Implications for Roadmap

### Phase Structure Recommendation

Based on stack research, the git worktree implementation has clear dependency ordering:

**Phase 1: Merge Infrastructure (0-dependency phase)**
- Add git2 0.20.4 to Cargo.toml
- Implement `MergeHandler` — wraps `Repository::merge_commits()`, `Index::has_conflicts()`
- Unit tests: merge with/without conflicts, conflict inspection
- **Output:** Typed merge result type, error handling for conflict scenarios
- **Dependencies:** None (pure git2 operations)
- **Estimated complexity:** LOW — API is straightforward

**Phase 2: Worktree Creation (subprocess wrapping)**
- Implement `WorktreeManager::create()` — wraps `git worktree add` via tokio::process::Command
- Implement `WorktreeManager::find()` — wraps `Repository::find_worktree()`
- Add validation, locking metadata operations
- Unit tests: create, list, validate, error cases (bad branch, existing path)
- **Output:** Async WorktreeManager trait, error enum for subprocess failures
- **Dependencies:** Phase 1 not strictly required (could do in parallel), but merge handling tests easier with foundation
- **Estimated complexity:** MEDIUM — subprocess error handling, edge cases

**Phase 3: Worker Sync Loop (integration)**
- Bootstrap: create worktree per worker
- Per-sync-cycle: `MergeHandler::merge()` to detect conflicts
- Conflict resolution: auto-resolve or escalate to TUI
- **Output:** Worker sync state machine
- **Dependencies:** Phase 1 + Phase 2
- **Estimated complexity:** HIGH — state management, retry logic

**Ordering rationale:**
1. Merge infrastructure first (no external dependencies, high confidence API)
2. Worktree ops parallel-able with Phase 1, but cleaner integration if Phase 1 stabilizes first
3. Sync loop is integration layer, depends on both

### Research Flags for Phases

| Phase | Topic | Flag | Reasoning |
|-------|-------|------|-----------|
| Phase 1 | Merge conflict resolution policy | RESEARCH_NEEDED | Decide: auto-resolve (ours/theirs), smart merge (gitattributes), or escalate. Impacts error handling design. |
| Phase 1 | Commit message for sync merges | RESEARCH_NEEDED | Determine: rebase-friendly format, metadata inclusion (worker ID, timestamp, conflict resolution strategy). |
| Phase 2 | Subprocess error recovery | STANDARD_PATTERN | Validate git binary at orchestrator startup; fail fast with clear error. |
| Phase 2 | Worktree cleanup/pruning strategy | RESEARCH_NEEDED | When to prune: on worker shutdown? On orchestrator restart? Locking implications? |
| Phase 3 | Conflict escalation to TUI | MINOR_RESEARCH | Determine protocol: TUI listens to conflict event, allows operator to resolve, broadcasts resolution back to worker. |
| Phase 3 | Concurrent sync safety | STANDARD_PATTERN | Each worker has exclusive worktree; merge happens within worker's repo. No additional locking needed beyond git's internal mechanisms. |

### No Critical Blockers Identified

- ✓ git2 API complete for essential operations
- ✓ Subprocess integration straightforward in Tokio context
- ✓ Libgit2 1.9.0+ available in all deployment targets
- ✓ Git binary availability standard assumption
- ✓ Error handling patterns clear and testable

**Proceed to detailed phase roadmap with confidence.**

---

## Confidence Assessment

| Area | Confidence | Evidence |
|------|------------|----------|
| **Stack Choice (git2 + CLI)** | HIGH | Verified across docs.rs, libgit2 official ref, multiple production systems (Vibe Kanban, Composio). |
| **git2 merge/conflict API** | HIGH | API stable since libgit2 1.0, no breaking changes in 0.20.4. Tested in real-world systems. |
| **Worktree API gaps** | HIGH | Confirmed: WorktreeAddOptions exists raw, no high-level binding. Workaround pattern established. |
| **Subprocess integration safety** | MEDIUM-HIGH | Pattern works in Worktrunk, Composio; Tokio subprocess proven. Edge cases exist (signal handling, git binary not on PATH) but manageable. |
| **Gitoxide alternative maturity** | MEDIUM-LOW | Pure Rust attractive; API still stabilizing. Safe to defer; no risk in git2 choice. |
| **NixOS deployment compatibility** | HIGH | Flake.nix already pins libgit2 and git; no new platform constraints. |

---

## Gaps & Unknowns

1. **Conflict resolution strategy:** River's dyadic architecture may want spectator-specific merge strategy (e.g., spectator's branch wins). Needs design decision in Phase 1.
2. **Worktree cleanup lifecycle:** Under what conditions should workers prune old worktrees? Needs policy decision in Phase 2.
3. **Merge commit message format:** Should sync merges carry metadata (worker ID, reason for sync)? Impacts commit history readability.
4. **TUI escalation protocol:** If worker detects conflict, how does it ask TUI operator to resolve? Out of scope for stack research, but impacts overall architecture.

**All gaps are design-level, not technical blockers.**

---

## Next Steps

1. **Proceed to Phase Roadmap** — Use this research to structure Git Worktree Coordination milestones
2. **Phase 1 Deep Dive:** Determine conflict resolution policy (auto vs. escalate)
3. **Phase 2 Design:** Implement WorktreeManager with subprocess error handling
4. **Phase 3 Integration:** Build worker sync state machine with conflict detection

---

## Sources

**Stack verification:**
- [git2 0.20.4 docs](https://docs.rs/git2/0.20.4/git2/)
- [libgit2 worktree API](https://libgit2.org/docs/reference/main/worktree/index.html)
- [git2-rs GitHub](https://github.com/rust-lang/git2-rs)

**Ecosystem patterns:**
- [Vibe Kanban hybrid approach](https://deepwiki.com/BloopAI/vibe-kanban/2.4-github-integration-and-pr-workflow)
- [Worktrunk git worktree manager](https://github.com/max-sixty/worktrunk)
- [Composio Agent Orchestrator](https://github.com/ComposioHQ/agent-orchestrator)

**Async integration:**
- [Tokio process module](https://docs.rs/tokio/latest/tokio/process/struct.Command.html)

**Future monitoring:**
- [gitoxide 0.81.0](https://github.com/GitoxideLabs/gitoxide)
- [gix Repository API](https://docs.rs/gix/latest/gix/struct.Repository.html)

---

*Research completed: 2026-04-06*
*Next phase: Detailed roadmap creation using STACK.md findings*
