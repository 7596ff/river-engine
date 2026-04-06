# Phase 2: Workspace Infrastructure - Context

**Gathered:** 2026-04-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Workers own isolated git worktrees created by orchestrator at startup, passed via registration. This phase sets up the infrastructure; sync protocol (when/how workers commit and pull) is Phase 3.

</domain>

<decisions>
## Implementation Decisions

### Worktree location
- **D-01:** Use existing `workspace/left/` and `workspace/right/` directories for worktrees
- **D-02:** These directories become git worktrees, not plain directories

### Worktree lifecycle
- **D-03:** Create worktrees on dyad spawn (in `spawn_dyad`)
- **D-04:** Worktrees persist across restarts — not deleted on shutdown
- **D-05:** Clean up only on explicit reset command (out of scope for this phase)
- **D-06:** If directory already exists and is a valid worktree, reuse it; otherwise clean and recreate

### Registration payload
- **D-07:** Add new `worktree_path` field to `WorkerRegistrationResponse`
- **D-08:** Keep existing `workspace` field for backward compatibility
- **D-09:** Worker uses `worktree_path` for all filesystem operations

### Git repository strategy
- **D-10:** Single repo in `workspace/` with worktrees branching from it
- **D-11:** Each worktree tracks a separate branch: `left` branch for left worker, `right` branch for right worker
- **D-12:** Workers push to their branch, merge into `main` when syncing (Phase 3 concern)
- **D-13:** The `main` branch represents the "agreed" state that both workers have seen

### Template migration
- **D-14:** Move `workspace/left/identity.md` to `workspace/left-identity.md` in the changeset
- **D-15:** Move `workspace/right/identity.md` to `workspace/right-identity.md` in the changeset
- **D-16:** The workspace directory is a template — migration happens in the implementation, not at runtime

### Claude's Discretion
- Error handling strategy for git command failures
- Exact git commands used (worktree add, branch creation)
- How orchestrator discovers workspace root path (from config or convention)
- Worker initialization ordering relative to worktree readiness

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Git worktree mechanics
- `.planning/research/FEATURES.md` — Feature landscape for git-based workspace sync, including table stakes, differentiators, and anti-features

### Codebase patterns
- `crates/river-orchestrator/src/supervisor.rs` — `spawn_worker` and `spawn_dyad` functions where worktree creation will be added
- `crates/river-protocol/src/registration.rs` — `WorkerRegistrationResponse` struct where `worktree_path` field will be added

### Existing workspace structure
- `workspace/left/identity.md` — Current identity file location (to be moved)
- `workspace/right/identity.md` — Current identity file location (to be moved)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `Supervisor::spawn_worker()` — spawns worker process, will need modification to create worktree first
- `spawn_dyad()` — orchestrates dyad startup, natural place to add worktree initialization
- `WorkerRegistrationResponse` — already has `workspace: String` field, new `worktree_path` adds alongside

### Established Patterns
- Process spawning via `tokio::process::Command`
- Registration via HTTP POST to `/register` endpoint
- Shared state via `Arc<RwLock<T>>`

### Integration Points
- `spawn_dyad()` in supervisor.rs — add worktree creation before worker spawning
- `/register` endpoint in http.rs — include `worktree_path` in response
- Worker startup — read `worktree_path` from registration response

</code_context>

<specifics>
## Specific Ideas

- Worktrees use separate branches (left/right) merging into main — this sets up a clear model for the Phase 3 sync protocol
- Reusing existing worktrees avoids startup overhead and preserves state across restarts
- Identity files move to workspace root with side prefix — keeps them accessible while freeing the directories for git worktree use

</specifics>

<deferred>
## Deferred Ideas

- Worktree cleanup on explicit reset — future operational command
- Sync protocol (when workers commit/pull) — Phase 3 concern
- Conflict resolution strategy — Phase 3 concern
- Git initialization if repo doesn't exist — assume repo exists for now

</deferred>

---

*Phase: 02-workspace-infrastructure*
*Context gathered: 2026-04-06*
