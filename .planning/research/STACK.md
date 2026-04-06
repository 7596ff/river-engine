# Technology Stack: Git Worktree Coordination for River Engine

**Project:** River Engine (Rust agent orchestrator with git worktree workspace sync)
**Researched:** 2026-04-06
**Scope:** Adding git worktree coordination to existing Rust system
**Confidence Level:** HIGH (libgit2/git2 verified), MEDIUM (worktree API coverage), MEDIUM-LOW (gitoxide worktree maturity)

## Executive Summary

For River Engine's git worktree coordination, **use the git2 crate (0.20.4+) for general git operations combined with subprocess git CLI wrapping for worktree creation/pruning**. git2 provides excellent merge conflict detection and index inspection (essential for sync operations), but worktree-specific creation operations (`git worktree add`, `git worktree prune`) are either partially exposed or require raw FFI bindings. A hybrid approach is pragmatic and aligns with ecosystem patterns observed in 2026 (Vibe Kanban, Composio). Gitoxide is the future pure-Rust alternative but lacks stable public worktree APIs in 0.81.0.

**Key decision drivers:**
1. git2 0.20.4 has mature merge/conflict handling (critical for sync)
2. `git worktree add` creation requires subprocess wrapping—git2-rs doesn't fully expose high-level bindings
3. Tokio's subprocess async APIs fit River's async runtime
4. Hybrid approach (git2 + CLI) is proven pattern in production multi-agent systems

---

## Recommended Stack

### Core Git Library

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| **git2** | 0.20.4+ | Repository operations, merge detection, conflict inspection, branch/commit management | Mature Rust bindings to libgit2 with full FFI coverage. Provides essential `merge_commits()`, `merge_trees()`, index inspection for conflict detection. Stable, well-tested in production systems. libgit2 requires 1.9.0+. |
| **libgit2** (via git2-sys) | 1.9.0+ | Underlying C library for git operations | Mature, cross-platform, handles low-level git semantics including worktree metadata and locking. |

### Worktree Management (Hybrid Approach)

| Technology | Version | Purpose | When to Use |
|------------|---------|---------|-------------|
| **tokio::process::Command** | 1.0+ | Async subprocess wrapping for git CLI | `git worktree add <name> <path>` — creation not exposed in git2-rs high-level API. Async by default, integrates seamlessly with River's Tokio runtime. Use for: add, prune, lock, unlock operations. |
| **git2** (Worktree struct) | 0.20.4+ | Worktree lookup, validation, locking | Already in git2 public API. Use for: listing existing worktrees (`worktrees()`), opening worktrees (`find_worktree()`), validation (`validate()`), locking metadata (`lock()`, `unlock()`, `is_locked()`). |

### Merge & Sync Strategy

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| **git2::Repository::merge_commits()** | 0.20.4+ | Three-way merge with conflict detection | Produces index with merged state + conflict markers. Essential for detecting when sync requires human intervention. Returns index allowing inspection of conflicted paths. |
| **git2::Index** | 0.20.4+ | Post-merge conflict inspection | Index after merge contains conflict information. Allows programmatic detection of conflicted files before attempting checkout. Required for robust sync error handling. |
| **git2::Repository::merge_trees()** | 0.20.4+ | Low-level merge without working directory mutation | For dry-run merge analysis before committing to worktree checkout. Safer for detecting conflicts upfront. |

### Async Runtime Integration

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| **tokio::process::Command** | 1.0+ (already in River) | Async subprocess execution for git CLI | River already uses Tokio. `git worktree add` wrapping must be async-safe. Use `spawn()` → `wait_with_output()` pattern. Handles signals properly, integrates with Tokio task scheduling. |
| **tokio::task::spawn_blocking()** | 1.0+ (optional) | Block git2 FFI calls from async context | If git2 merge operations become blocking bottleneck (unlikely for single worktree operations), offload to dedicated threadpool. Use only after profiling. |

---

## Installation & Integration

### Cargo.toml Addition

```toml
# In root workspace Cargo.toml or river-worker Cargo.toml

[dependencies]
git2 = "0.20"          # Merge, branch, commit operations
tokio = { version = "1.0", features = ["process"] }  # Already present
```

### No New Dependencies Required

- Tokio already in River's stack (0.8+ implied from Cargo.toml analysis)
- tokio::process::Command included in tokio with "process" feature
- git2 is pure Rust FFI (no new system dependencies beyond libgit2-sys compilation)

### System-Level Dependencies

Unchanged from current stack:
- `libgit2-dev` (for compilation)
- `pkg-config` (libgit2-sys discovery)
- Git binary must be on PATH for worktree operations (standard in most environments)

---

## Architecture: Hybrid Approach Rationale

### Why NOT Pure libgit2 for Worktree Creation

**Status of git2-rs worktree bindings (as of 0.20.4):**
- ✓ `Repository::worktrees()` — list existing worktrees
- ✓ `Repository::find_worktree()` — open worktree by name
- ✓ `Worktree::validate()`, `lock()`, `unlock()`, `prune()` — metadata operations
- ✗ `Repository::worktree_add()` — **NOT exposed in high-level API**
  - `WorktreeAddOptions` struct exists (raw FFI available)
  - But no public Repository method calls `git_worktree_add` safely
  - Would require unsafe raw FFI binding

**Real-world precedent:** Vibe Kanban (multi-agent git coordination) uses hybrid approach due to sparse-checkout incompatibility in libgit2 (reports sparse-excluded files as "deleted"). git2-rs has similar limitations with advanced git features.

### Why Subprocess Wrapping is Acceptable Here

1. **Frequency:** Worktree creation is infrequent (once per worker boot, not per sync cycle)
2. **Error handling:** Simple exit code checking; git CLI errors are clear
3. **Async compatibility:** Tokio handles subprocess overhead well
4. **Ecosystem standard:** Both Worktrunk (2026 git worktree manager) and Composio's orchestrator use CLI wrapping for worktree ops

---

## API Pattern for River Integration

### Merge Detection (git2-based, no subprocess)

```rust
// Pseudocode for sync operation
let repo = git2::Repository::open(&worktree_path)?;
let base_oid = repo.find_reference("refs/heads/main")?
    .target()
    .ok_or("No commit")?;
let worker_oid = repo.head()?.target().ok_or("Detached HEAD")?;

// Three-way merge: find common ancestor
let merge_index = repo.merge_trees(
    &repo.find_commit(base_oid)?.tree()?,
    &repo.find_commit(worker_oid)?.tree()?,
    &repo.find_commit(base_oid)?.tree()?,  // ancestor
)?;

// Detect conflicts
if merge_index.has_conflicts() {
    return Err("Merge has conflicts — needs human resolution");
}
```

### Worktree Creation (subprocess-based)

```rust
// Pseudocode for worker bootstrap
use tokio::process::Command;

async fn create_worktree(
    parent_repo_path: &Path,
    worktree_name: &str,
    worktree_path: &Path,
    branch_name: &str,
) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(parent_repo_path)
        .arg("worktree")
        .arg("add")
        .arg("--detach")  // or --track main
        .arg(worktree_path)
        .arg(branch_name)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {}", stderr));
    }
    Ok(())
}
```

### Worktree Lookup & Metadata (git2-based)

```rust
// Pseudocode for checking worktree health
let repo = git2::Repository::open(&parent_repo_path)?;
let wt = repo.find_worktree(&worktree_name)?;

if wt.is_locked() {
    eprintln!("Worktree locked: {:?}", wt.lock_reason()?);
}
wt.validate()?;  // Checks if worktree files exist, HEAD valid
```

---

## Alternatives Considered

| Approach | Recommendation | Reasoning |
|----------|---|---|
| **Pure gitoxide (gix 0.81.0)** | Not recommended for v1 | Pure Rust, no FFI overhead, but worktree creation API not stable; documented as aspirational. Revisit in 2026 H2 when gix stabilizes worktree support. Safe for merge operations (alternative to git2). |
| **Pure libgit2 with raw FFI** | Not recommended | Requires unsafe code, maintenance burden, error handling complexity. Only justified if performance critical; unlikely for worktree ops. |
| **git CLI exclusively (no git2)** | Not recommended | Loses merge conflict detection capability; would need `git merge --no-commit` + manual parsing of merge status. git2 provides typed, inspectable conflict API. |
| **gitoxide for git2 operations, CLI for worktree** | Not recommended for v1 | Splits responsibility; gix not feature-complete for merge operations. Stick with single library (git2) + CLI for narrow set of worktree ops. |

---

## Merge Conflict Handling Strategy

This is the critical operation that justifies git2 dependency:

1. **Detection:** `merge_index.has_conflicts()` returns bool
2. **Inspection:** Iterate `merge_index.conflicts()?` to find conflicted paths
3. **Resolution Options:**
   - Auto-resolve: Use git2's `merge_opts.favor` to pick "ours" or "theirs" (configurable per sync policy)
   - Manual: Report to orchestrator; wait for human judgment via TUI
   - Smart merge: Use git2 attributes system (`Repository::get_attr()`) to mark "ours-win" files in `.gitattributes`

**Why git2 for this:** Provides typed access to conflict data; avoids string parsing of `git status` or `git diff` output.

---

## Testing & Validation

### Unit Tests (in river-worker)

- `test_merge_no_conflicts()` — merge_commits succeeds, no conflicts
- `test_merge_with_conflicts()` — merge_commits detects conflicts, index reports conflicted paths
- `test_worktree_not_found()` — find_worktree fails appropriately
- `test_async_worktree_add()` — subprocess wrapping handles spawn/wait correctly

### Integration Tests

- Spin up two worktrees from same parent
- One worker writes file A, commits
- Other worker writes file B, commits
- Sync attempts merge (should succeed, no conflicts)
- One worker writes same file, different content
- Sync detects conflict, blocks further sync until resolved

### Subprocess Error Handling

- `git worktree add` with invalid branch name → capture stderr, return Result
- `git worktree add` with existing path → detect and fail gracefully
- Signal handling → ensure tokio::process::Command respects Tokio task cancellation

---

## Version Stability & Support

| Library | Current | Min Supported | Why |
|---------|---------|---|---|
| git2 | 0.20.4 | 0.20.0 | Latest stable; no breaking changes expected before 1.0. Worktree API stable. |
| libgit2 | 1.9.0+ | 1.9.0 | Required by git2-sys; version includes full worktree support. |
| tokio | 1.0+ | 1.0 (already in River) | process module stable since 1.0. |
| gitoxide | 0.81.0 | N/A | For future; not recommended for v1 (API still stabilizing). |

---

## Known Limitations & Workarounds

### git2 Worktree API Gap

**Limitation:** `Repository::worktree_add()` not exposed
**Workaround:** Use `tokio::process::Command` to wrap `git worktree add`
**Mitigation:** Small set of operations (add, prune); easy to test and maintain
**Future:** Consider contributing bindings to git2-rs if this becomes bottleneck

### Subprocess Dependency on Git Binary

**Limitation:** Requires `git` binary on PATH
**Workaround:** Validate presence at orchestrator startup; fail with clear error message
**Mitigation:** Standard in all deployment environments; NixOS flake.nix already pins git

### Merge Conflict Resolution

**Limitation:** git2 detects conflicts but doesn't auto-resolve intelligently (no 3-way merge strategy selection)
**Workaround:** Use `merge_opts.favor(git2::MergeFileOptions::FAVOR_OURS)` for deterministic resolution policy
**Mitigation:** Document conflict handling in worker loop; allow TUI to resolve interactively
**Future:** Implement conflict resolution strategy per River dyad behavior (e.g., "spectator's branch wins")

---

## Deployment Considerations

### NixOS Integration

Current flake.nix should be updated:
```nix
# Add to buildInputs if not present (likely already there)
libgit2
git
```

No Cargo.lock regeneration needed beyond `cargo update git2`.

### CI/CD

- Tests must run with `git` available (already standard)
- libgit2 headers needed at compile time (addressed by libgit2-sys)

### Production Monitoring

- Track `git worktree add` invocation duration (async, but should be <100ms)
- Log subprocess failures: capture stderr from `git worktree` operations
- Monitor merge conflict frequency: high frequency suggests sync policy needs adjustment

---

## Confidence Assessment

| Component | Confidence | Reasoning |
|-----------|------------|-----------|
| **git2 0.20.4** | HIGH | Verified on docs.rs; stable API; widely used in production. Merge/conflict operations confirmed. |
| **git2 worktree operations** | MEDIUM | `find_worktree()`, `validate()`, `lock()` confirmed in API. Creation gap documented and workarounded. |
| **Subprocess worktree add** | MEDIUM | Pattern verified in Vibe Kanban, Worktrunk, Composio (2026 systems). Tokio process integration straightforward. |
| **libgit2 1.9.0 features** | HIGH | Verified on libgit2.org; worktree API documented and stable. |
| **Merge conflict handling** | HIGH | git2::Index conflict inspection API confirmed; patterns match libgit2 semantics. |
| **gitoxide future roadmap** | MEDIUM-LOW | Gitoxide 0.81.0 has worktree utilities but not stable creation API. Aspirational for future; not ready for v1. |

---

## Sources

### Official Documentation
- [git2 0.20.4 docs](https://docs.rs/git2/0.20.4/git2/) — Repository, Worktree, Index APIs
- [libgit2 worktree reference](https://libgit2.org/docs/reference/main/worktree/index.html) — git_worktree_add, git_worktree_list, locking
- [Tokio process module](https://docs.rs/tokio/latest/tokio/process/struct.Command.html) — async subprocess wrapping
- [git2-rs GitHub](https://github.com/rust-lang/git2-rs) — Rust bindings source
- [gitoxide repository](https://github.com/GitoxideLabs/gitoxide) — Pure Rust alternative (monitored)

### Ecosystem & Patterns
- [Vibe Kanban git integration](https://deepwiki.com/BloopAI/vibe-kanban/2.4-github-integration-and-pr-workflow) — Hybrid approach justification
- [Worktrunk (2026)](https://github.com/max-sixty/worktrunk) — Production git worktree manager in Rust
- [Composio Agent Orchestrator](https://github.com/ComposioHQ/agent-orchestrator) — Multi-agent git coordination reference

### API References
- [gix 0.81.0 Repository](https://docs.rs/gix/latest/gix/struct.Repository.html) — Future alternative (not recommended v1)
- [gix-worktree 0.81.0](https://docs.rs/gix-worktree/latest/gix_worktree/) — Utility crate (confirmed limited scope)

---

*Research completed 2026-04-06. Ready for roadmap integration.*
