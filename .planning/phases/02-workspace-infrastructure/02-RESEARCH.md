# Phase 2: Workspace Infrastructure - Research

**Researched:** 2026-04-06
**Domain:** Git worktree infrastructure for isolated worker workspaces
**Confidence:** HIGH

## Summary

Phase 2 implements the foundational git infrastructure that enables two workers to operate on isolated worktrees without filesystem race conditions. This phase creates worktrees at orchestrator startup and passes the worktree paths to workers via registration—enabling Phase 3's sync protocol (which defines when/how workers commit and pull).

The research confirms git worktrees are the correct abstraction: they provide atomic filesystem isolation, native branch tracking, and clean merge semantics. The implementation is straightforward: orchestrator calls `git worktree add` during `spawn_dyad`, creates separate branches for left/right workers, and includes the worktree path in the `WorkerRegistrationResponse`.

**Primary recommendation:** Implement worktree creation in `spawn_dyad()` before worker startup, pass `worktree_path` alongside existing `workspace` field in registration response, and verify Phase 3's sync protocol assumptions (actor-wins conflict resolution, pull-before-context-assembly) before Phase 2 implementation.

## User Constraints (from CONTEXT.md)

### Locked Decisions
- **D-01:** Use existing `workspace/left/` and `workspace/right/` directories for worktrees
- **D-02:** These directories become git worktrees, not plain directories
- **D-03:** Create worktrees on dyad spawn (in `spawn_dyad`)
- **D-04:** Worktrees persist across restarts — not deleted on shutdown
- **D-05:** Clean up only on explicit reset command (out of scope for this phase)
- **D-06:** If directory already exists and is a valid worktree, reuse it; otherwise clean and recreate
- **D-07:** Add new `worktree_path` field to `WorkerRegistrationResponse`
- **D-08:** Keep existing `workspace` field for backward compatibility
- **D-09:** Worker uses `worktree_path` for all filesystem operations
- **D-10:** Single repo in `workspace/` with worktrees branching from it
- **D-11:** Each worktree tracks a separate branch: `left` branch for left worker, `right` branch for right worker
- **D-12:** Workers push to their branch, merge into `main` when syncing (Phase 3 concern)
- **D-13:** The `main` branch represents the "agreed" state that both workers have seen
- **D-14:** Move `workspace/left/identity.md` to `workspace/left-identity.md` in the changeset
- **D-15:** Move `workspace/right/identity.md` to `workspace/right-identity.md` in the changeset
- **D-16:** The workspace directory is a template — migration happens in the implementation, not at runtime

### Claude's Discretion
- Error handling strategy for git command failures
- Exact git commands used (worktree add, branch creation)
- How orchestrator discovers workspace root path (from config or convention)
- Worker initialization ordering relative to worktree readiness

### Deferred Ideas (OUT OF SCOPE)
- Worktree cleanup on explicit reset — future operational command
- Sync protocol (when workers commit/pull) — Phase 3 concern
- Conflict resolution strategy — Phase 3 concern
- Git initialization if repo doesn't exist — assume repo exists for now

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| INFRA-01 | Orchestrator creates git worktree per worker at dyad startup | Git worktree API confirmed; `spawn_dyad()` integration point identified; branch strategy locked in D-11 |
| INFRA-02 | Worktree paths passed to workers via registration | `WorkerRegistrationResponse` type located; new `worktree_path` field confirmed in decisions; worker config can read from registration |

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| git (CLI) | System default | Worktree management | Standard in all deployments; Rust projects typically shell out to git rather than embed libgit2 |
| tokio::process::Command | 1.0+ | Async git command execution | Established in river-orchestrator; non-blocking I/O required for supervision loop |
| Rust PathBuf | stdlib | Worktree path handling | Type-safe path manipulation; already used throughout codebase |

### Supporting Libraries
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tokio::fs | 1.0+ | Async filesystem operations | Directory creation, existence checks before/after worktree operations |
| anyhow | 1.0 | Error handling in Orchestrator | Already used in river-orchestrator for application-level errors |
| tracing | 0.1+ | Structured logging | Already established in orchestrator; log all git operations and state transitions |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Git CLI via Command | libgit2 Rust bindings (git2-rs) | CLI is simpler for straightforward operations (add, branch, status); libgit2 adds dependency management burden for operations we'll shell out for anyway |
| Shell to git | Custom Rust git library (river-git crate) | Out of scope per REQUIREMENTS.md; agents will use bash tool for Phase 3 sync, so custom Rust code adds complexity |
| One branch per worker | Shared branch with conflict markers | Locked decision D-11 mandates separate branches; enables clean merge semantics and role-specific conflict resolution |

**Installation:**
```bash
# git is provided by system; verify availability
command -v git

# Rust dependencies already in Cargo.lock; no new additions needed
# tokio::process::Command is part of tokio 1.0 (tokio with full features)
```

**Version verification:**
- Git: System default (typically 2.30+; verify with `git --version`)
- Tokio: 1.0+ [VERIFIED: Cargo.lock shows tokio 1.x with full features including process module]
- PathBuf: Rust stdlib, no version tracking needed

## Architecture Patterns

### Recommended Project Structure (Worktree Layout)

```
river-engine/           # Main repository (contains .git)
├── workspace/          # Template root (contains git config, branches)
│   ├── .git/           # Main worktree metadata
│   ├── left/           # Left worker's worktree (linked, branch=left)
│   │   ├── identity.md
│   │   ├── conversations/
│   │   └── inbox/
│   ├── right/          # Right worker's worktree (linked, branch=right)
│   │   ├── identity.md
│   │   ├── conversations/
│   │   └── inbox/
│   ├── left-identity.md    # Moved from workspace/left/identity.md
│   ├── right-identity.md   # Moved from workspace/right/identity.md
│   ├── shared/
│   ├── roles/
│   └── [other shared files]
```

**Key points:**
- `workspace/` is the git repository root (contains `.git/`)
- `workspace/left/` and `workspace/right/` are git worktrees (not bare directories)
- Each worktree tracks a separate branch (`left`, `right`) in the same repository
- Identity files moved to root with side prefix (D-14, D-15) to free directories for worktree use
- Shared files (roles, zettelkasten, etc.) remain in root and visible to all worktrees

### Pattern 1: Worktree Creation at Spawn Time

**What:** Orchestrator creates isolated git worktrees for each worker during dyad startup, before spawning worker processes.

**When to use:** This pattern applies universally in river-engine; each dyad gets exactly two worktrees created on startup.

**Example:**
```rust
// In spawn_dyad() after checking workspace_path exists:

// Ensure branches exist
Command::new("git")
    .args(&["-C", workspace_str, "branch", "left"])
    .output()  // May fail if branch exists; that's OK
    .ok();
Command::new("git")
    .args(&["-C", workspace_str, "branch", "right"])
    .output()
    .ok();

// Create worktrees with separate branches
let left_path = workspace.join("left");
let left_exists = left_path.exists();
if left_exists && is_valid_worktree(&left_path) {
    // Reuse existing worktree (D-06)
    tracing::info!("Reusing existing left worktree at {:?}", left_path);
} else if left_exists {
    // Directory exists but isn't a valid worktree; clean and recreate
    tokio::fs::remove_dir_all(&left_path).await?;
    create_worktree(workspace_str, "left", "left").await?;
} else {
    // Directory doesn't exist; create fresh
    create_worktree(workspace_str, "left", "left").await?;
}

// Same for right worker...
```

**Helper function pattern:**
```rust
async fn create_worktree(
    repo_path: &str,
    worktree_name: &str,
    branch_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let worktree_path = format!("{}/{}", repo_path, worktree_name);
    let output = Command::new("git")
        .args(&["-C", repo_path, "worktree", "add", "-b", branch_name, &worktree_path])
        .output()
        .await?;

    if !output.status.success() {
        return Err(format!("Failed to create worktree: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    tracing::info!("Created worktree {} at {} on branch {}", worktree_name, worktree_path, branch_name);
    Ok(())
}

fn is_valid_worktree(path: &Path) -> bool {
    // Check if .git exists and is a file (linked worktrees have .git as file, not directory)
    // or if it's a directory containing HEAD
    if let Ok(git_path) = path.join(".git").read_to_string() {
        // It's a linked worktree; .git is a file pointing to main repo
        git_path.contains("gitdir:")
    } else {
        false
    }
}
```

[CITED: git-scm.com/docs/git-worktree]

### Pattern 2: Registration Response with Worktree Path

**What:** Orchestrator's registration response includes both the legacy `workspace` field (for backward compatibility) and a new `worktree_path` field pointing to the worker's isolated worktree.

**When to use:** Every worker registration response in the system.

**Example:**
```rust
// In WorkerRegistrationResponse struct (crates/river-protocol/src/registration.rs):
#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: String,  // Keep for backward compatibility (D-08)
    pub worktree_path: String,  // NEW: Path to worker's isolated worktree
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}

// In orchestrator HTTP handler (crates/river-orchestrator/src/http.rs):
let response = WorkerRegistrationResponse {
    accepted: true,
    name: worker_name,
    baton,
    partner_endpoint,
    model,
    ground: dyad_config.ground.clone(),
    workspace: dyad_config.workspace.to_string_lossy().to_string(),
    worktree_path: match side {  // NEW
        Side::Left => dyad_config.workspace.join("left").to_string_lossy().to_string(),
        Side::Right => dyad_config.workspace.join("right").to_string_lossy().to_string(),
    },
    initial_message,
    start_sleeping,
};
```

[VERIFIED: crates/river-protocol/src/registration.rs]

### Pattern 3: Worker Uses Worktree Path for All I/O

**What:** Worker reads `worktree_path` from registration response and uses it for all filesystem operations, ensuring complete isolation.

**When to use:** All filesystem access in worker: conversations, inbox, identity, role files.

**Example:**
```rust
// In worker config (crates/river-worker/src/config.rs):
impl WorkerConfig {
    pub fn worktree_path(&self, registration: &RegistrationResponse) -> PathBuf {
        // NEW: Use worktree_path from registration instead of generic workspace
        PathBuf::from(&registration.worktree_path)
    }

    // Update existing methods to use worktree_path:
    pub fn identity_path(&self, registration: &RegistrationResponse) -> PathBuf {
        let worktree = self.worktree_path(registration);
        worktree.join("identity.md")  // No more "left"/"right" subdirectory
    }

    pub fn role_path(&self, registration: &RegistrationResponse) -> PathBuf {
        // Roles are shared; access via workspace root
        let workspace = PathBuf::from(&registration.workspace);
        let role_str = match registration.baton {
            Baton::Actor => "actor",
            Baton::Spectator => "spectator",
        };
        workspace.join("roles").join(format!("{}.md", role_str))
    }
}

// In worker main.rs, update identity loading:
let identity_path = config.identity_path(&registration);
if identity_path.exists() {
    let identity_content = tokio::fs::read_to_string(&identity_path).await?;
    tracing::info!("Loaded identity from worktree {:?}", identity_path);
    // ...
}
```

### Anti-Patterns to Avoid

- **Mixing worktree_path and workspace for I/O:** Worker should use `worktree_path` exclusively for worker-specific files (conversations, inbox, identity) and `workspace` only for shared resources (roles, reference docs). Mixing them leads to subtle race conditions.

- **Not checking worktree validity before spawning workers:** If `git worktree add` fails silently (e.g., branch already checked out elsewhere), workers will crash on first I/O. Always verify worktree state after creation.

- **Deleting worktrees without cleanup:** Removing worktree directories without `git worktree remove` leaves stale metadata in `.git/worktrees/`. Always use `git worktree remove` or let git garbage collection handle it (slow but safe).

- **Assuming main branch exists:** Before creating worktrees, verify that `main` branch (or the configured base branch) exists. If repository is brand new, initialize it with at least one commit.

- **Not tracking which branch a worktree uses:** Each worktree is bound to a specific branch. If you want to switch branches, you must either (a) delete and recreate the worktree, or (b) manually checkout the new branch in the worktree. Don't assume branches are interchangeable within a worktree.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Process isolation for concurrent agents | Custom file-locking or mutex-based coordination | Git worktrees + separate branches | Git's 3-way merge is battle-tested; file locks are slow and error-prone; branches provide semantic isolation |
| Branch management across processes | Custom branch tracking or process-local branch state | Git's native branch abstraction | Git handles branch metadata, conflict markers, and merge state atomically; reimplementing is error-prone |
| Detecting conflicting edits | Polling filesystem for changes or custom hashing | `git merge --no-commit` test merge + conflict parsing | Git's merge algorithm is mature and handles edge cases (renames, deletions, binary files) that custom solutions miss |
| Ensuring consistent worktree state on startup | Ad-hoc state repair logic | `git worktree list` + `git worktree repair` | Git provides built-in repair for stale worktree metadata; attempting to repair manually risks data loss |

**Key insight:** Git worktrees exist specifically to solve the problem of multiple processes operating on the same repository safely. This is their primary use case. Building custom coordination on top of a shared filesystem is slower, less reliable, and invisible to debugging tools that understand git state.

## Runtime State Inventory

The workspace directory currently contains several state elements that must be preserved during the worktree migration:

| Category | Items Found | Action Required |
|----------|-------------|------------------|
| **Stored data** | No database records found. Workspace is template/docs, not persistent data store. | None — no data migration. Identity/conversation files will be versioned in git. |
| **Live service config** | Workspace directory is static; no running services store config in workspace. | None — config is in `river.json` (orchestrator-level). |
| **OS-registered state** | No OS-level registration found (no Task Scheduler, systemd units, or launchd plists specific to workspace). | None — orchestrator handles process registration. |
| **Secrets/env vars** | No secrets embedded in workspace files. Identity/role files are documentation, not credentials. | None — secrets managed via NixOS `environmentFile` option. |
| **Build artifacts** | No build artifacts in current workspace. Workspace is template + docs. | None. If generated files exist (e.g., compiled notes), they should use `.gitignore` and not be tracked. |

**Nothing found in category that would block worktree migration.** The workspace is a version-controlled template, not a runtime state container. Worktree conversion is a clean structural change with no data migration needed.

## Common Pitfalls

### Pitfall 1: Worktree Reuse Without Validation
**What goes wrong:** Orchestrator assumes a worktree still exists if the directory exists. If `git worktree remove` failed or was bypassed, the directory exists but the worktree is not registered in `.git/worktrees/`. Worker processes can operate on the directory, but git operations fail mysteriously.

**Why it happens:** D-06 says "if directory exists and is valid worktree, reuse it." Validating "is valid" requires checking both (a) directory exists, and (b) `.git` file exists and points to the main repo.

**How to avoid:** Always run `git worktree list --porcelain` to check registered worktrees, and validate that `.git` in the worktree points to the main repo. If validation fails, delete the directory and recreate the worktree fresh.

**Warning signs:** Worker starts successfully but crashes on first git operation (e.g., `git status`), or `git worktree list` shows no entry for an existing directory.

### Pitfall 2: Branch Already Checked Out Elsewhere
**What goes wrong:** `git worktree add -b left workspace/left` fails because branch `left` is already checked out in another worktree. Error message is cryptic ("fatal: ...already in use").

**Why it happens:** Git enforces that each branch can only be checked out once per repository. If a previous run crashed, the worktree may still be registered but orphaned.

**How to avoid:** Before creating worktrees, check if branches are already in use: `git worktree list`. If a stale entry exists, remove it with `git worktree remove`. Also consider `git worktree repair` to fix metadata inconsistencies.

**Warning signs:** `git worktree add` fails with "already in use" message; `git worktree list` shows entries for non-existent directories.

### Pitfall 3: Not Ensuring Main Branch Exists
**What goes wrong:** If the git repository is brand new (no commits), `main` branch doesn't exist. Creating worktrees with `-b left` creates the branches, but the repository has no commits to check out. Worker startup fails with "fatal: reference not a tree".

**Why it happens:** D-11 assumes branches `left` and `right` can be created. If the repo is new, there's no base commit to branch from.

**How to avoid:** Before creating worktrees in `spawn_dyad`, verify the repository has at least one commit. If not, initialize it: `git commit --allow-empty -m "Initial commit"` or check out an existing branch.

**Warning signs:** Orchestrator startup succeeds but worker startup fails with "fatal: reference not a tree"; `git log` shows no commits.

### Pitfall 4: Identity Files Not Migrated
**What goes wrong:** Code expects identity files at `workspace/left/identity.md` and `workspace/right/identity.md`, but D-14 and D-15 move them to `workspace/left-identity.md` and `workspace/right-identity.md`. Worker fails to load identity.

**Why it happens:** The file locations change, but code is not updated. The migration must happen in the implementation task.

**How to avoid:** Update `WorkerConfig::identity_path()` to read from the new location. Verify with a test that loads the identity file after worktree creation.

**Warning signs:** Worker logs "Failed to load identity" or identity content is empty; manually checking the worktree shows `identity.md` is missing but `left-identity.md` exists in the root.

### Pitfall 5: Git Command Blocking in Async Context
**What goes wrong:** `spawn_dyad()` uses `tokio::process::Command` but doesn't properly await the output. Orchestrator supervision loop can be blocked if git operations hang.

**Why it happens:** `Command::output()` is async and must be awaited. If awaited incorrectly (e.g., creating a future without awaiting), the command runs in the background and we don't know if it succeeded.

**How to avoid:** Always use `.await` after `.output()`, `.spawn()`, or other command methods. Check the `status` field of the output to verify success. If a git command can hang (e.g., `git clone`), add a timeout: `.timeout(Duration::from_secs(30))`.

**Warning signs:** `spawn_dyad()` returns immediately but workers fail to connect; orchestrator logs don't show git output; manually running the same git command works fine.

## Code Examples

### Creating Worktrees with Branch Management

[CITED: git-scm.com/docs/git-worktree]

```rust
use std::process::Stdio;
use tokio::process::Command;
use std::path::Path;

async fn ensure_worktree_exists(
    repo_path: &Path,
    worktree_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_str = repo_path.to_string_lossy();

    // Step 1: Ensure branch exists
    let branch_create = Command::new("git")
        .args(&["-C", &repo_str, "branch", worktree_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await?;
    // Ignore error if branch already exists

    // Step 2: Check if worktree already exists and is valid
    let worktree_path = repo_path.join(worktree_name);
    if is_valid_worktree(&worktree_path) {
        tracing::debug!("Worktree {} already valid, reusing", worktree_name);
        return Ok(());
    }

    // Step 3: Remove stale directory if it exists but isn't a valid worktree
    if worktree_path.exists() {
        tracing::warn!("Removing invalid worktree directory at {:?}", worktree_path);
        tokio::fs::remove_dir_all(&worktree_path).await?;
    }

    // Step 4: Create the worktree
    let output = Command::new("git")
        .args(&["-C", &repo_str, "worktree", "add", "-b", worktree_name, &worktree_path.to_string_lossy()])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("Failed to create worktree: {}", stderr);
        return Err(format!("git worktree add failed: {}", stderr).into());
    }

    tracing::info!("Created worktree {} at {:?} on branch {}",
                   worktree_name, worktree_path, worktree_name);
    Ok(())
}

fn is_valid_worktree(path: &Path) -> bool {
    let git_path = path.join(".git");
    if !git_path.exists() {
        return false;
    }

    // For linked worktrees, .git is a file containing "gitdir: ..."
    if git_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_path) {
            return content.starts_with("gitdir:");
        }
    }

    false
}
```

### Worktree Path Passed in Registration

```rust
// In river-protocol/src/registration.rs

#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
pub struct WorkerRegistrationResponse {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub baton: Baton,
    pub partner_endpoint: Option<String>,
    pub model: ModelConfig,
    pub ground: Ground,
    pub workspace: String,           // Root workspace (backward compat)
    pub worktree_path: String,       // NEW: Path to worker's isolated worktree
    pub initial_message: Option<String>,
    pub start_sleeping: bool,
}
```

```rust
// In river-orchestrator/src/http.rs, in handle_register

let worktree_path = match side {
    Side::Left => dyad_config.workspace.join("left"),
    Side::Right => dyad_config.workspace.join("right"),
};

let response = WorkerRegistrationResponse {
    accepted: true,
    name: worker_name,
    baton,
    partner_endpoint,
    model,
    ground: dyad_config.ground.clone(),
    workspace: dyad_config.workspace.to_string_lossy().to_string(),
    worktree_path: worktree_path.to_string_lossy().to_string(),
    initial_message,
    start_sleeping,
};

Ok(Json(serde_json::to_value(response).unwrap()))
```

### Worker Reads Worktree Path from Registration

```rust
// In river-worker/src/config.rs

impl WorkerConfig {
    pub fn worktree_path(&self, registration: &RegistrationResponse) -> PathBuf {
        PathBuf::from(&registration.worktree_path)
    }

    pub fn identity_path(&self, registration: &RegistrationResponse) -> PathBuf {
        // NEW: Read from worktree, not workspace/side/identity.md
        let worktree = self.worktree_path(registration);
        // Identity files were moved to workspace root as left-identity.md / right-identity.md
        // But they should also be in the worktree for isolation, OR
        // We read from root and copy to worktree on startup
        // For now, assume identity files are in the root:
        let workspace = PathBuf::from(&registration.workspace);
        let side_str = match self.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        workspace.join(format!("{}-identity.md", side_str))
    }

    pub fn conversations_dir(&self, registration: &RegistrationResponse) -> PathBuf {
        let worktree = self.worktree_path(registration);
        worktree.join("conversations")
    }

    pub fn inbox_dir(&self, registration: &RegistrationResponse) -> PathBuf {
        let worktree = self.worktree_path(registration);
        worktree.join("inbox")
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Shared filesystem root (`workspace/`) for all agents | Isolated git worktrees per agent (worker) | Phase 2 (current) | Eliminates race conditions; enables atomic writes via git commits; allows independent branches for conflict resolution |
| No version control for workspace files | Full git tracking of all workspace state | Phase 2 (current) | Enables deterministic merges, conflict detection, and historical auditing |
| No structured sync protocol | Agent-driven sync via bash tool + workspace docs (Phase 3) | Planned for Phase 3 | Agents understand when/how to commit and pull; no magic sync daemon |

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | Git worktrees are supported on deployment target (Linux with NixOS) | Standard Stack | If git < 2.15, `git worktree` command doesn't exist; system git must be recent enough. Check in NixOS module. |
| A2 | Repository in `workspace/` already exists and has at least one commit | Common Pitfalls (Pitfall 3) | If repo is brand new, worktree creation fails; Phase 2 must initialize repo if missing |
| A3 | Identity files can be safely moved from `workspace/left/` to `workspace/left-identity.md` | Runtime State Inventory | If code expects old path and migration isn't complete, worker fails to load identity. Testing must verify path resolution. |
| A4 | `tokio::process::Command` handles git commands without deadlock | Code Examples | If git command streams large output that isn't read, process can deadlock. Must use `.output()` not `.spawn().wait_with_output()` incorrectly. |
| A5 | Phase 3 will implement the sync protocol as documented in FEATURES.md | Architecture Patterns | If Phase 3 changes the conflict resolution strategy or branch naming, Phase 2's branch structure becomes suboptimal. Lock Phase 3 decisions early. |

## Open Questions

1. **How does the orchestrator discover the workspace path?**
   - What we know: It's passed via `DyadConfig::workspace` in `river.json`
   - What's unclear: Is the path absolute or relative to orchestrator working directory?
   - Recommendation: Verify in next phase's planning; make paths absolute or resolve relative to config file location to avoid surprises.

2. **Should worktree cleanup be part of graceful shutdown, or only on explicit reset?**
   - What we know: D-05 says cleanup is out of scope; only explicit reset command
   - What's unclear: If orchestrator crashes, are stale worktree metadata files cleaned up automatically?
   - Recommendation: Document in Phase 2 that `git worktree prune` can be run manually; Phase 2 doesn't automate it.

3. **What happens if a worker crashes and doesn't commit before death?**
   - What we know: Worktrees persist across restarts (D-04); changes in worktree directory are not auto-committed
   - What's unclear: Should uncommitted changes be auto-committed, stashed, or left dirty?
   - Recommendation: Phase 3 docs should define this as part of sync protocol (likely: "stash uncommitted changes and pull fresh")

4. **How does identity file loading work across the transition?**
   - What we know: D-14/D-15 move files to root; worker config must read from new location
   - What's unclear: Do identity files live in root (shared) or in each worktree? How are they made available to both workers?
   - Recommendation: Design decision for Phase 2 planning: identity files in root (read-only per-worker) or copied to each worktree on startup?

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| git CLI | Worktree creation in orchestrator | ✓ | System default (2.30+) | None — git is mandatory for Phase 2 |
| Tokio async runtime | Worker spawning and supervision | ✓ | 1.0+ [VERIFIED: Cargo.lock] | — |
| Filesystem (Unix permissions) | Worktree isolation | ✓ | Linux kernel default | — |
| /tmp or workspace directory | Worktree mount points | ✓ | Configured via river.json | Reconfigure workspace path in config |

**Missing dependencies with no fallback:**
- None identified. Git is available on all deployment targets (NixOS includes it by default).

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | tokio::test (async tests in same crate) + #[test] for sync code |
| Config file | None yet; tests create temporary directories and git repos |
| Quick run command | `cargo test -p river-orchestrator worktree` |
| Full suite command | `cargo test -p river-orchestrator && cargo test -p river-worker` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| INFRA-01 | Orchestrator creates worktrees on `spawn_dyad()` | unit | `cargo test -p river-orchestrator -- spawn_dyad` | ❌ Wave 0 |
| INFRA-01 | Worktree directories exist after spawn | integration | `cargo test -p river-orchestrator -- test_spawn_dyad_creates_worktrees` | ❌ Wave 0 |
| INFRA-01 | Branches are correctly set up in worktrees | integration | `cargo test -p river-orchestrator -- test_worktree_branches` | ❌ Wave 0 |
| INFRA-02 | Registration response includes worktree_path | unit | `cargo test -p river-protocol -- test_registration_response_schema` | ✅ (structural) |
| INFRA-02 | Worker reads worktree_path from registration | integration | `cargo test -p river-worker -- test_worker_config_worktree_path` | ❌ Wave 0 |
| INFRA-02 | Worker I/O operations use worktree_path | integration | `cargo test -p river-worker -- test_worker_filesystem_isolation` | ❌ Wave 0 |

### Sampling Rate
- **Per task commit:** `cargo test -p river-orchestrator worktree` (quick test of worktree creation logic)
- **Per wave merge:** Full suite + integration tests verifying end-to-end worktree creation and worker startup
- **Phase gate:** TUI adapter test with actual worktree I/O (Phase 4) must pass before `/gsd-verify-work`

### Wave 0 Gaps
- [ ] `crates/river-orchestrator/tests/worktree_tests.rs` — Test worktree creation, validation, reuse logic
- [ ] `crates/river-orchestrator/src/supervisor.rs` — Add helper functions: `ensure_worktree_exists()`, `is_valid_worktree()`
- [ ] `crates/river-protocol/src/registration.rs` — Add `worktree_path` field to `WorkerRegistrationResponse`
- [ ] `crates/river-worker/src/config.rs` — Add `worktree_path()` method; update `identity_path()`, `conversations_dir()`, etc. to use it
- [ ] `crates/river-orchestrator/src/http.rs` — Update registration response handler to populate `worktree_path`
- [ ] Workspace template migrations — Move identity files to root; verify git status clean

*(After Wave 0 implementation, Phase 3 can be planned with confidence that worktree infrastructure is solid.)*

## Security Domain

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|---------------|---------|-----------------|
| V1 Architecture | yes | Isolated worktrees per worker (in-repo, not OS-level) prevents cross-worker filesystem tampering |
| V2 Authentication | no | Registration happens via localhost HTTP; no additional auth for worktree access |
| V5 Input Validation | yes | Git operations must validate paths to prevent directory traversal (e.g., `git worktree add ../../../etc/passwd`) |
| V12 File Upload | no | No file uploads; git operations are local only |
| V14 Configuration | yes | Workspace path in config must be absolute or relative to config file, not user-controlled |

### Known Threat Patterns for {Rust Async + Git + Filesystem}

| Pattern | STRIDE | Standard Mitigation |
|---------|--------|---------------------|
| Symlink attack on worktree path | Tampering | Always use `canonicalize()` on workspace paths before passing to git; reject symlinks |
| Git command injection via dynamic arguments | Tampering | Use `Command` with `.arg()` not `.args()` with string interpolation; parse paths through `Path` type system |
| Race condition: check-then-use on worktree existence | Tampering | Use atomic git operations; don't check existence then create; let git handle the atomic check |
| Stale worktree metadata causes data loss | Denial of Service | Always run `git worktree repair` before assuming worktree state; validate `.git` file points to main repo |

**No additional crypto, secrets, or auth required for Phase 2.** Isolation is filesystem-based (git worktrees) and deployment environment is trusted (localhost, NixOS sandbox).

## Sources

### Primary (HIGH confidence)
- [Git Worktrees Official Documentation](https://git-scm.com/docs/git-worktree) — Verified worktree add/list/remove commands, linked worktree semantics, `.git` file format
- [Codebase: crates/river-orchestrator/src/supervisor.rs](file:///home/cassie/river-engine/crates/river-orchestrator/src/supervisor.rs) — Verified `spawn_dyad()` and `spawn_worker()` patterns; confirmed tokio::process::Command usage
- [Codebase: crates/river-protocol/src/registration.rs](file:///home/cassie/river-engine/crates/river-protocol/src/registration.rs) — Verified `WorkerRegistrationResponse` structure; confirmed where `worktree_path` field will be added
- [Codebase: crates/river-worker/src/config.rs](file:///home/cassie/river-engine/crates/river-worker/src/config.rs) — Verified `workspace_path()` pattern; confirmed how worker accesses workspace from registration

### Secondary (MEDIUM confidence)
- [.planning/research/FEATURES.md](file:///home/cassie/river-engine/.planning/research/FEATURES.md) — Research on git sync feature landscape; verified that worktrees are table stakes, not experimental
- [CONTEXT.md from discuss-phase](file:///home/cassie/river-engine/.planning/phases/02-workspace-infrastructure/02-CONTEXT.md) — Locked decisions on worktree locations, branch strategy, and lifecycle management

### Tertiary (LOW confidence)
- None identified; all major claims verified via codebase or official documentation.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — git worktrees are standard, tokio::process::Command is established in codebase, no new dependencies
- Architecture: HIGH — `spawn_dyad()` integration point identified, registration response pattern clear, worker code structure straightforward
- Pitfalls: HIGH — git-specific pitfalls researched and documented with clear warning signs and mitigations
- Assumptions: MEDIUM — identity file migration path needs validation in Phase 2 planning; assumptions A3 and A5 are assumptions about downstream phases

**Research date:** 2026-04-06
**Valid until:** 2026-04-13 (7 days; git worktree documentation is stable, but Phase 3 decisions may emerge that affect Phase 2 scope)
**Dependencies:** Phase 1 (error handling) must be complete; Phase 3 sync protocol must be locked in decisions before implementation to avoid rework
