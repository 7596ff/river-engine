---
phase: 02-workspace-infrastructure
plan: 01
subsystem: orchestrator-worktree-infrastructure
tags: [git-worktrees, process-supervision, filesystem-isolation]
dependency_graph:
  requires: []
  provides: [worktree-creation-api, isolated-worker-directories]
  affects: [spawn_dyad, worker-registration]
tech_stack:
  added: [tempfile-3.10-dev]
  patterns: [git-worktree-validation, reuse-before-create, async-git-commands]
key_files:
  created: []
  modified:
    - path: workspace/left-identity.md
      impact: Moved from workspace/left/identity.md to free directory for worktree use
    - path: workspace/right-identity.md
      impact: Moved from workspace/right/identity.md to free directory for worktree use
    - path: crates/river-orchestrator/src/supervisor.rs
      impact: Added is_valid_worktree() and ensure_worktree_exists() helpers; integrated worktree creation into spawn_dyad()
    - path: crates/river-orchestrator/Cargo.toml
      impact: Added tempfile dev dependency for test isolation
decisions:
  - id: D-06-implementation
    summary: Implemented reuse-before-create logic for worktrees
    rationale: Valid existing worktrees are reused to support orchestrator restarts without cleanup overhead
  - id: worktree-creation-timing
    summary: Worktree creation happens before supervisor lock acquisition
    rationale: Avoids holding write lock during potentially slow git operations
metrics:
  duration_seconds: 235
  duration_human: 3m 55s
  tasks_completed: 3
  tasks_total: 3
  files_modified: 4
  commits: 3
  tests_added: 6
  completed_at: "2026-04-06T17:52:26Z"
---

# Phase 02 Plan 01: Git Worktree Creation Infrastructure Summary

**One-liner:** Git worktree isolation per worker using separate branches (left/right) with reuse-on-restart logic

## What Was Built

Implemented git worktree creation infrastructure in the orchestrator's `spawn_dyad()` function. Each worker (left and right) now gets an isolated git worktree on a separate branch, created before worker processes spawn. The implementation follows D-06 reuse logic: valid existing worktrees are reused, invalid directories are cleaned and recreated.

**Key capabilities added:**
- `is_valid_worktree()` validates linked worktrees by checking .git file format
- `ensure_worktree_exists()` creates/reuses worktrees with branch management
- `spawn_dyad()` creates worktrees before spawning workers
- Identity template files relocated to workspace root

## Tasks Completed

| Task | Type | Commit | Description |
|------|------|--------|-------------|
| 1 | auto | 0d7a79f | Moved identity templates from workspace/{side}/ to workspace/{side}-identity.md |
| 2 | auto (TDD) | 880d355 | Implemented worktree helpers with full test coverage (6 tests) |
| 3 | auto | 47b429a | Integrated worktree creation into spawn_dyad before worker spawn |

## Deviations from Plan

None - plan executed exactly as written.

## Test Coverage

**New tests added (6 total):**
- `test_is_valid_worktree_returns_true_for_valid_worktree` - validates .git file detection
- `test_is_valid_worktree_returns_false_when_directory_not_exists` - validates nonexistent path handling
- `test_is_valid_worktree_returns_false_when_git_is_directory` - distinguishes main repo from linked worktree
- `test_ensure_worktree_exists_creates_worktree_when_not_exists` - verifies fresh worktree creation
- `test_ensure_worktree_exists_reuses_valid_existing_worktree` - verifies D-06 reuse logic
- `test_ensure_worktree_exists_cleans_invalid_directory` - verifies cleanup before recreation

All tests use tempfile for isolation and real git commands for integration validation.

**Test results:**
```
cargo test -p river-orchestrator -- worktree
running 6 tests
test supervisor::tests::test_is_valid_worktree_returns_true_for_valid_worktree ... ok
test supervisor::tests::test_is_valid_worktree_returns_false_when_directory_not_exists ... ok
test supervisor::tests::test_is_valid_worktree_returns_false_when_git_is_directory ... ok
test supervisor::tests::test_ensure_worktree_exists_creates_worktree_when_not_exists ... ok
test supervisor::tests::test_ensure_worktree_exists_reuses_valid_existing_worktree ... ok
test supervisor::tests::test_ensure_worktree_exists_cleans_invalid_directory ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured
```

## Technical Details

**Worktree validation pattern:**
```rust
fn is_valid_worktree(path: &Path) -> bool {
    let git_path = path.join(".git");
    if !git_path.exists() || !git_path.is_file() {
        return false;
    }

    // Linked worktrees have .git as file containing "gitdir: ..."
    if let Ok(content) = std::fs::read_to_string(&git_path) {
        return content.contains("gitdir:");
    }

    false
}
```

**Worktree creation strategy (D-06):**
1. If worktree exists AND is valid → reuse (log "Reusing existing worktree")
2. If worktree exists but is invalid → clean with `remove_dir_all`, recreate
3. If worktree doesn't exist → create fresh with `git worktree add`

**Branch management:**
- Create branch if needed (ignore error if exists): `git branch <branch_name>`
- Link worktree to existing branch: `git worktree add <path> <branch_name>`
- No `-b` flag when branch exists (avoids "already exists" error)

**spawn_dyad integration:**
- Worktree creation happens BEFORE `supervisor.write().await`
- No write lock held during git operations (prevents blocking supervision loop)
- Both left and right worktrees created sequentially before any worker spawns
- Logs confirm worktree readiness before process spawning begins

## Known Stubs

None. All worktree creation logic is fully implemented with git command integration.

## Threat Flags

None. No new security-relevant surface introduced beyond plan's threat model.

## Requirements Satisfied

- **INFRA-01**: Orchestrator creates git worktree per worker at dyad startup ✓
  - `spawn_dyad()` calls `ensure_worktree_exists()` for left and right
  - Worktrees track separate branches (left, right)
  - Worktrees persist across restarts via D-06 reuse logic

## Next Steps

1. **Phase 2 Plan 2**: Pass worktree paths to workers via registration response
   - Add `worktree_path` field to `WorkerRegistrationResponse`
   - Update orchestrator HTTP handler to populate field
   - Workers will use `worktree_path` for all I/O (conversations, inbox)

2. **Phase 3**: Define sync protocol in workspace documentation
   - When do workers commit changes?
   - How do workers pull updates?
   - Conflict resolution strategy (actor-wins per research)

3. **Phase 4**: End-to-end validation with TUI adapter
   - Verify worktrees isolate filesystem writes
   - Test restart behavior (reuse existing worktrees)
   - Validate branch separation under concurrent writes

## Self-Check: PASSED

**Files created/modified:**
- ✓ workspace/left-identity.md exists (moved from workspace/left/identity.md)
- ✓ workspace/right-identity.md exists (moved from workspace/right/identity.md)
- ✓ workspace/left/identity.md deleted (no longer exists)
- ✓ workspace/right/identity.md deleted (no longer exists)
- ✓ crates/river-orchestrator/src/supervisor.rs contains is_valid_worktree
- ✓ crates/river-orchestrator/src/supervisor.rs contains ensure_worktree_exists
- ✓ crates/river-orchestrator/src/supervisor.rs spawn_dyad calls ensure_worktree_exists
- ✓ crates/river-orchestrator/Cargo.toml includes tempfile dev dependency

**Commits exist:**
- ✓ 0d7a79f: refactor(02-01): move identity templates to workspace root
- ✓ 880d355: feat(02-01): implement worktree creation helpers with TDD
- ✓ 47b429a: feat(02-01): integrate worktree creation into spawn_dyad

**Build verification:**
```bash
cargo build -p river-orchestrator
   Compiling river-orchestrator v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.95s
```

All claims verified. Plan execution complete.
