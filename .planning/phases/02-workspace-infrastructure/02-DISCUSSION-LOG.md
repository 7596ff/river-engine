# Phase 2: Workspace Infrastructure - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-06
**Phase:** 02-workspace-infrastructure
**Areas discussed:** Worktree location, Worktree lifecycle, Registration payload, Git repository strategy

---

## Worktree Location

| Option | Description | Selected |
|--------|-------------|----------|
| Project-relative .worktrees/ | Create .worktrees/{dyad}-{side}/ under the workspace root. Visible, easy to inspect, persists across restarts. | |
| Temp directory | Create in system temp (e.g., /tmp/river-{dyad}-{side}). Ephemeral, auto-cleaned on reboot, less clutter. | |
| Configurable path | Add worktree_base to river.json config. Flexibility for deployment scenarios. | |

**User's choice:** Use existing `workspace/left/` and `workspace/right/` directories for worktrees
**Notes:** User clarified these directories already exist in the workspace template and are the natural place for worktrees.

---

## Worktree Lifecycle

| Option | Description | Selected |
|--------|-------------|----------|
| Create on dyad spawn, persist across restarts | Worktrees created when spawn_dyad runs. Not deleted on shutdown — allows inspecting state after crashes. Clean up only on explicit 'reset' command. | ✓ |
| Create on dyad spawn, clean on shutdown | Worktrees created on spawn, removed on graceful shutdown. Fresh start every time. Orphans may remain after crashes. | |
| Lazy creation on first worker write | Defer worktree creation until worker actually needs to write. Adds complexity but avoids setup for dyads that never run. | |

**User's choice:** Create on dyad spawn, persist across restarts
**Notes:** Persistence allows debugging and inspection after crashes.

---

## Registration Payload

| Option | Description | Selected |
|--------|-------------|----------|
| Use existing workspace field | WorkerRegistrationResponse already has a 'workspace: String' field. Change it from shared path to point to the worker's worktree path. | |
| Add worktree_path field | Add a new 'worktree_path' field alongside 'workspace'. Keeps backward compatibility, clearer naming. | ✓ |
| CLI argument | Pass --worktree /path/to/worktree when spawning worker. Worker reads from args, not registration response. | |

**User's choice:** Add worktree_path field
**Notes:** Clearer naming and backward compatibility with existing workspace field.

---

## Git Repository Strategy

| Option | Description | Selected |
|--------|-------------|----------|
| Worktrees from single repo | One main repo in workspace/. Each worker gets 'git worktree add' into workspace/left/ and workspace/right/. Shared .git, efficient storage, easy to sync. | ✓ |
| Separate clones per worker | Each worker gets independent git clone. More isolation but more disk, harder to coordinate. | |
| Bare repo with worktrees | Bare repo in workspace/.git-main/, worktrees branching from it. Cleaner separation, standard git workflow. | |

**User's choice:** Worktrees from single repo
**Notes:** Efficient storage and easy coordination between workers.

---

## Branch Strategy

| Option | Description | Selected |
|--------|-------------|----------|
| Same branch (main/master) | Both worktrees track the same branch. Simpler model — workers pull and push to same ref. Conflicts resolved at merge time. | |
| Separate branches per worker | workspace/left/ tracks 'left' branch, workspace/right/ tracks 'right' branch. Explicit divergence, merge via PR or manual step. | ✓ |
| You decide | Let Claude pick the branching strategy based on what's simpler for the sync protocol in Phase 3. | |

**User's choice:** Separate branches per worker
**Notes:** Explicit divergence model.

---

## Main Branch Relationship

| Option | Description | Selected |
|--------|-------------|----------|
| Workers merge into main | Workers push to their branch (left/right), then merge into main. Main represents 'agreed' state both workers have seen. | ✓ |
| Workers pull from partner branch | Left pulls from right branch directly, vice versa. No main branch involved in worker sync. Simpler two-way sync. | |
| You decide | Let Claude design the branch flow based on what makes the Phase 3 sync protocol cleanest. | |

**User's choice:** Workers merge into main
**Notes:** Main branch represents agreed/shared state.

---

## Existing Directory Handling

| Option | Description | Selected |
|--------|-------------|----------|
| Reuse if valid worktree | Check if directory is already a valid worktree. If yes, reuse it (no git worktree add). If not, clean and recreate. | ✓ |
| Always recreate | Delete contents and run git worktree add fresh every spawn. Clean slate, no stale state. | |
| Fail if exists | Error if directory exists and isn't empty. Force explicit cleanup before respawn. Safest but annoying. | |

**User's choice:** Reuse if valid worktree
**Notes:** Preserves state across restarts, avoids unnecessary work.

---

## Template Migration

**User's additional input:** The workspace directory is a template, so existing `identity.md` files in `workspace/left/` and `workspace/right/` need to be moved to `workspace/left-identity.md` and `workspace/right-identity.md` as part of the implementation changeset (not runtime migration).

---

## Claude's Discretion

- Error handling strategy for git command failures
- Exact git commands used (worktree add, branch creation)
- How orchestrator discovers workspace root path
- Worker initialization ordering relative to worktree readiness

## Deferred Ideas

- Worktree cleanup on explicit reset — future operational command
- Sync protocol — Phase 3
- Conflict resolution — Phase 3
- Git initialization if repo doesn't exist
