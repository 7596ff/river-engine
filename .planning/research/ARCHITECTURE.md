# Architecture: Git Worktree Sync Integration

**Project:** River Engine (Dyadic Agent Orchestrator)
**Context:** Brownfield enhancement adding git worktree coordination to existing Rust agent orchestrator
**Researched:** 2026-04-06
**Confidence:** HIGH (analysis based on complete codebase inspection)

---

## Current Architecture Summary

River Engine is a **microservice-based multi-agent system** with central orchestration:

- **Orchestrator** (`river-orchestrator`): Process supervisor, registry, coordination
- **Workers** (`river-worker`): Two per dyad, implement think→act LLM loop
- **Workspace**: Currently shared filesystem, specified per dyad in config
- **Think→Act Loop**: LLM call → tool execution (read/write/bash) → persist changes → repeat
- **Tool Operations**: Direct filesystem operations on shared `workspace` path

### Current Workspace Architecture

```
workspace/
├── left/
│   ├── identity.md
│   ├── context.jsonl (LLM history - stream of consciousness)
│   └── [tool-created files]
├── right/
│   ├── identity.md
│   ├── context.jsonl
│   └── [tool-created files]
├── roles/
│   ├── actor.md
│   ├── spectator.md
├── conversations/
│   ├── adapter_channel.txt
│   └── [channel conversation files]
├── moves/
│   └── [game move history]
├── moments/
│   └── [decision points]
└── inbox/
    └── [pending notifications]
```

**Current Risk**: Shared filesystem + concurrent writes = race conditions on context.jsonl, conversation files, tool outputs

---

## Recommended: Git Worktree Architecture

### Component Boundaries

#### 1. **WorktreeManager** (new crate: `river-git`)

**Responsibility**: Lifecycle management of git worktrees per worker.

**Interface**:
```rust
pub struct WorktreeManager {
    repo_path: PathBuf,           // Base git repo (shared across dyad)
    left_worktree: PathBuf,       // ~/workspace/left (worktree)
    right_worktree: PathBuf,      // ~/workspace/right (worktree)
}

impl WorktreeManager {
    pub async fn init(repo_path: &Path) -> Result<Self>;
    pub async fn create_worktree(&self, side: Side) -> Result<PathBuf>;
    pub async fn remove_worktree(&mut self, side: Side) -> Result<()>;
    pub async fn working_tree_path(&self, side: Side) -> PathBuf;
}
```

**Lives In**: Orchestrator (created during dyad startup)
**Created By**: `spawn_dyad()` during orchestrator initialization
**Scope**: One per dyad (shared reference passed to both workers via registration response)

---

#### 2. **WorktreeSync** (module in `river-worker`)

**Responsibility**: Sync logic within worker's think→act loop.

**Interface**:
```rust
pub struct WorktreeSync {
    base_repo: PathBuf,
    my_worktree: PathBuf,
    my_side: Side,
}

impl WorktreeSync {
    // Commit current changes with generated message
    pub async fn commit_changes(&self, message: &str) -> Result<CommitInfo>;

    // Pull partner's changes (merge or rebase)
    pub async fn sync_from_partner(&self) -> Result<SyncResult>;

    // Resolve conflicts via git operations
    pub async fn resolve_conflicts(&self, strategy: ConflictStrategy) -> Result<()>;
}

#[derive(Debug)]
pub enum SyncResult {
    NoChanges,
    MergedCleanly,
    ConflictsDetected { files: Vec<String> },
}

#[derive(Debug)]
pub enum ConflictStrategy {
    OursSide,        // Prefer our changes
    TheirsSide,      // Prefer partner's changes
    Manual { files: Vec<(String, String)> },  // Explicit resolution
}
```

**Lives In**: Worker state (`SharedState`)
**Initialized**: In `main()` before `run_loop()`
**Scope**: Per worker instance

---

#### 3. **SyncCheckpoint** (in `river-worker`)

**Responsibility**: Mark synchronization points in the think→act loop.

**Inserted At**: Two strategic points:
1. **After each tool execution batch** (before LLM calls for next iteration)
2. **Before role switching** (actor→spectator transition)

**Operation**:
```rust
async fn sync_checkpoint(
    state: &SharedState,
    worktree: &WorktreeSync,
    sync_policy: &SyncPolicy,
) -> Result<()> {
    // 1. Commit any uncommitted changes
    worktree.commit_changes("Checkpoint: tool results").await?;

    // 2. Pull partner's changes
    match worktree.sync_from_partner().await? {
        SyncResult::NoChanges => {},
        SyncResult::MergedCleanly => {
            // Reload context for next iteration
            state.write().await.reload_context().await?;
        },
        SyncResult::ConflictsDetected { files } => {
            // Handle merge conflicts based on policy
            handle_conflicts(worktree, files, sync_policy).await?;
            state.write().await.reload_context().await?;
        },
    }

    Ok(())
}
```

---

### Data Flow Integration

#### Startup Sequence (Modified)

```
Orchestrator:
1. Load config (includes repo_path per dyad)
2. Create WorktreeManager for each dyad
   ├─ git init <repo_path>
   └─ git config --local receive.denyCurrentBranch=updateInstead
3. Call spawn_dyad(dyad_name, config, worktree_manager)
4. spawn_dyad spawns both workers with dyad_name in registration response
5. Each worker receives workspace_path = worker's worktree path
6. Workers initialize WorktreeSync with repo_path + side
```

**New Registration Response Field**:
```rust
pub struct WorkerRegistrationResponse {
    // ... existing fields ...
    pub workspace: String,           // Path to worker's worktree
    pub git_repo_path: String,       // Shared repo path (for sync operations)
}
```

---

#### Think→Act Loop with Sync (Modified)

```
Worker Think→Act Loop:

STARTUP:
├─ Load initial state from worktree files
├─ Create WorktreeSync { base_repo, my_worktree, my_side }
└─ [opt] git add . && git commit "Initial state"

MAIN LOOP:
┌─ Get current token count
├─ LLM call with context
├─ Tool execution (read/write/bash)
│  └─ All writes go to my_worktree (exclusive access)
│
├─ ─ ─ ─ ─ SYNC CHECKPOINT #1 ─ ─ ─ ─ ─
│  ├─ git add .
│  ├─ git commit "Tool results: [tool names]"
│  └─ sync_from_partner()
│     ├─ git fetch origin
│     ├─ git merge origin/partner (or rebase)
│     └─ [if conflicts] resolve_conflicts()
│
├─ Reload context from merged files
├─ Text response handling
└─ Loop back to LLM call

ROLE SWITCH:
├─ ─ ─ ─ ─ SYNC CHECKPOINT #2 ─ ─ ─ ─ ─
│  └─ (same as checkpoint #1)
├─ Commit baton switch marker
└─ Switch loop control

EXIT:
├─ Final commit (optional: "Worker exit")
└─ Report status to orchestrator
```

---

### Git Repository Structure

```
repo/
├── .git/
│   ├── refs/heads/
│   │   ├── left (worker 1's branch)
│   │   └── right (worker 2's branch)
│   ├── objects/
│   └── config (with worktree refs)
├── .gitworktrees/
│   ├── left/
│   └── right/
├── left/  (symlink or actual worktree dir)
│   └─ [files checked out from left branch]
├── right/  (symlink or actual worktree dir)
│   └─ [files checked out from right branch]
└── main/  (optional: canonical state)
    └─ [merged state after sync]
```

**Branch Strategy**:
- `left`: Worker Left's working branch (updated only by worker Left)
- `right`: Worker Right's working branch (updated only by worker Right)
- `main`: (optional) merged canonical state, updated by orchestrator after sync
- **No shared branch**: Each worker only pushes to their own branch

---

## Component Interaction Diagram

```
┌─────────────────────────────────────────────────────────────┐
│ Orchestrator (river-orchestrator)                           │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  WorktreeManager                                            │
│  ├─ repo_path: /data/workspace/.git                        │
│  ├─ left_worktree: /data/workspace/left                    │
│  └─ right_worktree: /data/workspace/right                  │
│      │                                                     │
│      ├─ git init /data/workspace                           │
│      ├─ git worktree add left (from left branch)           │
│      └─ git worktree add right (from right branch)         │
└─────────────────────────────────────────────────────────────┘
                          │ spawn_dyad()
        ┌─────────────────┼─────────────────┐
        │                 │                 │
┌───────▼─────────┐ ┌────▼──────────┐      │
│ Worker Left     │ │ Worker Right  │      │
├─────────────────┤ ├───────────────┤      │
│                 │ │               │      │
│ WorkerState:    │ │ WorkerState:  │      │
│ ├─ workspace:   │ │ ├─ workspace: │      │
│ │  /data/..left │ │ │  /data/.right
│ │ ├─ git_repo:  │ │ │ ├─ git_repo:│      │
│ │  /data/.git   │ │ │  /data/.git │      │
│ │ └─ side: Left │ │ │ └─ side: Rt │      │
│                 │ │               │      │
│ WorktreeSync:   │ │ WorktreeSync: │      │
│ ├─ my_branch:   │ │ ├─ my_branch: │      │
│ │  left        │ │ │  right       │      │
│ ├─ methods:     │ │ ├─ methods:   │      │
│ │ ├─ commit()   │ │ │ ├─ commit() │      │
│ │ ├─ sync()     │ │ │ ├─ sync()   │      │
│ │ └─ resolve()  │ │ │ └─ resolve()│      │
│                 │ │               │      │
│ LOOP:           │ │ LOOP:         │      │
│ ├─ LLM call     │ │ ├─ LLM call   │      │
│ ├─ Tool exec    │ │ ├─ Tool exec  │      │
│ ├─ CHECKPOINT   │ │ ├─ CHECKPOINT │      │
│ │  ├─ commit()  │ │ │  ├─ commit()│      │
│ │  └─ sync()    │ │ │  └─ sync()  │      │
│ └─ iterate      │ │ └─ iterate    │      │
└─────────────────┘ └───────────────┘      │
        │                   │               │
        └───────────────────┼───────────────┘
                            │
                    [Git worktree operations]
                    (shared .git directory)
```

---

## Data Flow: Detailed Sync Scenario

### Scenario: Context Merge After Tools

**Step 1: Worker Left executes tools**
- Tools write to `/data/workspace/left/conversations/discord_general.txt`
- Tools write to `/data/workspace/left/left/context.jsonl`
- These are only visible to left's worktree (git isolation)

**Step 2: Worker Left hits sync checkpoint**
```bash
# In worker-left process:
cd /data/workspace/left
git add .
git commit -m "Tool results: read, write (2 changes)"
# Commits to "left" branch in .git
```

**Step 3: Worker Right calls sync_from_partner() (next iteration)**
```bash
# In worker-right process:
cd /data/workspace/right

# Fetch changes from shared repo
git fetch origin  # Pull latest refs from left branch

# Merge partner's changes
git merge origin/left
# Result: context.jsonl merged using git three-way merge
```

**Step 4: Conflict detection**
```rust
// In worker-right:
match git_merge_result {
    Ok(merged) => {
        // No conflicts, context merged cleanly
        // Both workers see common ancestor + their changes
    },
    Err(conflicts) => {
        // e.g., both modified "left/context.jsonl" incompatibly
        // Reload context from "right" branch (ours)
        // Log conflict, let actor/spectator decide next step
    }
}
```

**Step 5: Reload context**
```rust
// In worker-right after successful merge:
let mut state = state.write().await;

// Reload from merged files
state.reload_context(&workspace_path).await?;

// Next LLM call sees:
// - Partner's tool outputs (merged)
// - Own previous context
// - New context from merged state
```

---

## Build Order & Dependencies

### Phase 1: Foundation (must complete first)
1. **New crate: `river-git`**
   - Wraps `git2-rs` or uses `Command` subprocess
   - Exports `WorktreeManager`, `WorktreeSync` types
   - Tests: Unit tests for git operations in isolation
   - **Deliverable**: Pure git manipulation, no integration

2. **Add to workspace `Cargo.toml`**
   - Add `git2` v0.29+ (or use subprocess git)
   - Add `river-git` crate to members

3. **Extend `river-protocol`**
   - Add `git_repo_path: String` to `WorkerRegistrationResponse`
   - Version bump for compatibility

### Phase 2: Orchestrator Integration
4. **Modify `river-orchestrator`**
   - `supervisor.rs`: Extend `spawn_dyad()` to create `WorktreeManager`
   - `http.rs`: Add git_repo_path to worker registration response
   - `config.rs`: Add `repo_path: PathBuf` to `DyadConfig` (parse from config)
   - Tests: Mock git operations, verify worktree creation

### Phase 3: Worker Integration
5. **Modify `river-worker`**
   - `state.rs`: Add `WorktreeSync` to `WorkerState`, add methods to reload context
   - `main.rs`: Initialize `WorktreeSync` from registration response
   - `worker_loop.rs`: Insert `sync_checkpoint()` calls at two points:
     - After tool execution, before next LLM call
     - Before role switching
   - `tools.rs`: (No changes) Tools already write to state.workspace
   - Tests: Integration tests with real git repo

6. **Error Handling**
   - Add `GitError` enum to `river-adapter` or new in tools
   - Propagate merge conflicts to worker as `ToolResult::Error`
   - Worker decides: abort, retry with strategy, or escalate to Ground

### Phase 4: Testing & Validation
7. **End-to-end tests**
   - TUI mock adapter (existing)
   - Simulate both workers making changes, merging
   - Verify conflict detection and resolution

---

## Architecture Patterns

### Pattern 1: Exclusive Worktree Ownership

**What**: Each worker owns one worktree, never touches partner's.

**When**: Always enforced by git worktree design.

**Code**:
```rust
// In worker:
let my_worktree = state.workspace.clone();  // /data/workspace/left
let my_branch = "left";  // Only this worker commits to "left" branch

// Sync code:
git merge origin/right  // Merge from partner's branch
// Result: git enforces one writer per branch
```

**Benefit**: Eliminates write-write conflicts on individual worktrees.

---

### Pattern 2: Checkpoint-Driven Sync

**What**: Sync at specific, well-defined points in the loop (not continuous).

**When**: After tool batches, before role changes, before context assembly.

**Code**:
```rust
// In worker_loop.rs:
loop {
    let response = llm.call(&messages).await?;

    for tool_call in response.tool_calls {
        execute_tool(&tool_call).await?;
    }

    // ← CHECKPOINT: Sync before next iteration
    worktree.commit_and_sync().await?;

    // Continue with next LLM call...
}
```

**Benefit**: Predictable sync points reduce surprise conflicts; forces explicit handling.

---

### Pattern 3: Three-Way Merge with Git

**What**: Use git's merge algorithm, not custom conflict resolution.

**When**: sync_from_partner() pulls and merges.

**Code**:
```rust
// In WorktreeSync:
pub async fn sync_from_partner(&self) -> Result<SyncResult> {
    self.run_git(&["fetch", "origin"]).await?;

    match self.run_git(&["merge", "origin/partner"]).await {
        Ok(_) => Ok(SyncResult::MergedCleanly),
        Err(e) if e.is_conflict => {
            let conflicted = self.get_conflicted_files().await?;
            Ok(SyncResult::ConflictsDetected { files: conflicted })
        },
        Err(e) => Err(e),
    }
}
```

**Benefit**: Battle-tested algorithm; respects both workers' intents; supports strategies.

---

### Pattern 4: Reload-on-Merge

**What**: After successful merge, reload context from workspace files.

**When**: After `sync_from_partner()` succeeds.

**Code**:
```rust
pub async fn reload_context(&mut self, workspace: &Path) -> Result<()> {
    let context_path = workspace.join("left/context.jsonl");
    self.llm_history = load_context(&context_path);

    let channels = load_channels(workspace, &self.current_channels).await?;
    self.channel_contexts = channels;

    Ok(())
}
```

**Benefit**: Guarantees LLM sees merged state; avoids stale context bugs.

---

## Scalability Considerations

| Concern | At 1 Dyad | At 10 Dyads | At 100+ Dyads |
|---------|-----------|------------|--------------|
| **Git repos** | 1 shared per dyad | 10 separate repos | 100+ repos on different mounts |
| **Worktree count** | 2 per dyad (4 total) | 20 worktrees | 200+ worktrees |
| **Sync time** | ~100ms (small context) | ~500ms (larger contexts) | May need async commits |
| **Disk I/O** | Negligible | Monitor for contention | Consider batch commits |
| **Conflict rate** | Rare (different files) | Low (context files isolated per side) | Manageable if checkpoints are frequent |

**Scaling Strategy**:
- **1-10 dyads**: Single machine, local filesystem
- **10-100 dyads**: Separate git repos per dyad (reduce contention), possibly SSDs
- **100+ dyads**: Consider git server (Gitea/Gogs) for remote sync, offload disk I/O

---

## Error Handling & Conflict Resolution

### Git Operation Failures

| Failure | Example | Recovery |
|---------|---------|----------|
| **Repo corrupted** | `git fsck` fails | Manual intervention (orchestrator alerts Ground) |
| **Merge conflict** | Both sides modified context.jsonl | Detected, worker decides strategy |
| **Stale worktree** | `.git/index.lock` exists | Retry with backoff, or kill stale process |
| **Disk full** | `git commit` fails | Return error to loop, pause loop |

### Merge Conflict Strategies

```rust
pub enum ConflictStrategy {
    OursSide,         // Keep my changes, discard theirs
    TheirsSide,       // Keep theirs, discard mine
    Abort,            // Don't merge, report to loop
    Manual { files: Vec<(String, String)> },  // Explicit resolution (future)
}
```

**Recommended Default**:
- **For context.jsonl**: `OursSide` (preserve actor/spectator's thinking)
- **For conversation files**: `TheirsSide` (prefer partner's messages, assume they're authoritative)
- **For tool outputs**: Three-way merge (usually works, rarely conflicts)

---

## Key Design Decisions

| Decision | Rationale | Tradeoff |
|----------|-----------|----------|
| **Worktree per worker** | Eliminates filesystem race conditions, exploits git semantics | ~10-20ms overhead per commit (acceptable) |
| **Sync at checkpoints** (not continuous) | Predictable, allows explicit error handling, reduces merge complexity | Worker might see stale partner state briefly (acceptable—spectator role) |
| **Git's three-way merge** | Battle-tested, respects both intents, deterministic | Conflicts still possible if both modify same file (rare with isolation) |
| **Branch per worker** | Simple ownership model, no shared branches | Requires explicit merge strategy (not implicit) |
| **No remote server (v1)** | Simpler deployment, faster for localhost | Doesn't scale past ~100 dyads without infrastructure changes |

---

## Failure Modes & Mitigations

### Failure 1: Worker Crash Mid-Commit

**Symptom**: Worktree left in detached HEAD state, or index.lock stuck.

**Mitigation**:
```rust
// On worker startup:
if worktree_is_broken() {
    // Clean up stale locks
    std::fs::remove_file(git_dir.join("index.lock"))?;
    // Attempt recovery
    git_reset_hard_to_head()?;
}
```

---

### Failure 2: Both Workers Modify Same File

**Symptom**: Merge conflicts on every sync.

**Mitigation**:
- Isolation by design: context.jsonl per side, conversations per adapter/channel
- If conflicts occur frequently: escalate to Ground (spectator role's job)

---

### Failure 3: Disk Corruption (Rare)

**Symptom**: `git fsck` fails.

**Mitigation**:
- Automated health check: orchestrator periodically runs `git fsck` on all repos
- Alert Ground, recommend manual recovery (re-initialize repo)
- Log all operations for audit trail

---

## Integration with Existing Components

### Worker State (`river-worker/src/state.rs`)

**Add**:
```rust
pub struct WorkerState {
    // ... existing fields ...
    pub worktree_sync: Option<WorktreeSync>,  // Initialized in main()
    pub git_repo_path: PathBuf,
}
```

### Worker Loop (`river-worker/src/worker_loop.rs`)

**Modify**:
```rust
pub async fn run_loop(
    state: SharedState,
    config: &WorkerConfig,
    client: &reqwest::Client,
) -> WorkerOutput {
    // ... setup ...

    loop {
        // ... existing LLM + tool execution ...

        // NEW: Sync checkpoint
        if let Err(e) = sync_checkpoint(&state, config).await {
            tracing::warn!("Sync failed: {}", e);
            // Continue loop anyway (spectator can see failure in context)
        }

        // ... continue ...
    }
}
```

### Orchestrator (`river-orchestrator`)

**Add to config loading**:
```rust
pub struct DyadConfig {
    pub workspace: PathBuf,
    pub repo_path: PathBuf,  // NEW: Where .git lives
    // ... rest ...
}
```

**Add to spawn_dyad()**:
```rust
async fn spawn_dyad(
    // ... args ...
    worktree_mgr: &mut WorktreeManager,
) -> Result<()> {
    // Initialize git repo if needed
    worktree_mgr.init_repo().await?;

    // Spawn workers (they'll receive worktree paths)
    supervisor.spawn_worker(...)?;
    supervisor.spawn_worker(...)?;
}
```

---

## Testing Strategy

### Unit Tests (river-git crate)

```rust
#[tokio::test]
async fn test_worktree_creation() {
    let repo = tempdir();
    let mgr = WorktreeManager::init(repo.path()).await.unwrap();
    let left_path = mgr.working_tree_path(Side::Left).await;
    assert!(left_path.exists());
}

#[tokio::test]
async fn test_commit_and_merge() {
    let repo = tempdir();
    let mgr = WorktreeManager::init(repo.path()).await.unwrap();

    // Worker left writes
    write_file(mgr.working_tree_path(Side::Left).await, "file.txt", "left data");
    mgr.commit(Side::Left, "Message").await.unwrap();

    // Worker right syncs
    mgr.sync(Side::Right).await.unwrap();
    let content = read_file(mgr.working_tree_path(Side::Right).await, "file.txt");
    assert_eq!(content, "left data");
}
```

### Integration Tests (river-worker)

```rust
#[tokio::test]
async fn test_worker_sync_checkpoint() {
    // Spawn both workers with shared git repo
    let (left, right) = spawn_test_dyad().await;

    // Left writes via tool
    left.execute_tool("write", ...);
    left.sync_checkpoint().await.unwrap();

    // Right syncs
    right.sync_checkpoint().await.unwrap();

    // Verify right sees left's changes
    let content = right.read("file.txt").await.unwrap();
    assert_eq!(content, left_expected);
}
```

### E2E Tests (with TUI adapter)

```rust
#[tokio::test]
async fn test_dyad_full_cycle() {
    // Use existing TUI mock adapter
    let dyad = spawn_dyad_with_tui().await;

    // Simulate: left responds, right spectates
    dyad.left.llm_loop_once().await;
    dyad.right.observe().await;

    // Verify sync happened, both see changes
    assert_sync_successful(&dyad).await;
}
```

---

## Roadmap Implications

### Suggested Phase Structure

#### **Phase 1: Git Foundation (Week 1-2)**
- [ ] Create `river-git` crate with `WorktreeManager`, `WorktreeSync`
- [ ] Add git operations (init, commit, merge) using git2-rs or subprocess
- [ ] Unit tests for all git operations
- **Delivers**: Library-level git functionality, no integration

#### **Phase 2: Orchestrator Integration (Week 2-3)**
- [ ] Modify orchestrator to create worktrees during `spawn_dyad()`
- [ ] Extend worker registration response with `git_repo_path`
- [ ] Tests: Mock git, verify worktree creation
- **Delivers**: Orchestrator sets up repos, workers receive paths

#### **Phase 3: Worker Sync (Week 3-4)**
- [ ] Add `WorktreeSync` to `WorkerState`
- [ ] Insert `sync_checkpoint()` in worker loop at two points
- [ ] Implement conflict detection and basic resolution
- [ ] Tests: Integration tests with real git repo
- **Delivers**: Workers can sync, conflicts detected

#### **Phase 4: E2E Testing & Polish (Week 4-5)**
- [ ] End-to-end tests with TUI adapter
- [ ] Error handling (failed commits, stale worktrees)
- [ ] Logging and observability
- [ ] Documentation
- **Delivers**: Stable, tested worktree sync

---

## Observed Patterns in Existing Codebase

### 1. **State Pattern** (Arc<RwLock<T>>)
Already established: `SharedState` for worker, `SharedRegistry` for orchestrator. Sync's worktree instance should follow same pattern.

### 2. **Tool Execution Pattern** (ToolCall → ToolResult)
Existing tools already write to `state.workspace` directly. No changes needed—worktree is transparent to tools.

### 3. **Error Propagation** (Result<T, CustomError>)
Use `Result<T, SyncError>` consistent with existing `ToolError`, `SupervisorError`.

### 4. **Async/Await throughout**
All I/O is async (tokio). Sync operations should use `tokio::process::Command` for `git` subprocess calls.

### 5. **Registration-Based Discovery**
Workers register with orchestrator on startup, receiving config. Use same pattern for git_repo_path.

---

## Summary

**Git worktree sync integrates cleanly because**:

1. **Ownership is clear**: Each worker owns one worktree, one branch
2. **Sync points are explicit**: Happens at checkpoints, not continuously
3. **State is already structured**: Existing code separates left/right context, conversations per channel
4. **Error handling exists**: Tools already return Result; sync can do the same
5. **Async runtime is established**: Tokio already used; git operations fit naturally

**Build order**: Foundation → Orchestrator → Worker → Testing

**Testing strategy**: Unit (git ops) → Integration (worker loop) → E2E (TUI)

**Confidence**: HIGH — Architecture is complete, dependencies are clear, patterns exist to follow.

---

## Questions Likely to Arise in Phase-Specific Research

1. **Conflict resolution strategy**: Should we prefer actor/spectator changes, or use git markers?
   → Answer: Phase 3 research (depends on role semantics)

2. **Git-over-HTTP for distributed teams**: Needed for remote orchestrators?
   → Answer: Phase 4+ (out of scope for v1 localhost)

3. **Worktree cleanup on worker crash**: How to garbage-collect orphaned worktrees?
   → Answer: Phase 2 research (orchestrator health checks)

4. **Performance: is sync too slow?**: Benchmark at 100 workers/dyads
   → Answer: Phase 4 research (load testing)

5. **Git history size**: Does context.jsonl grow unbounded?
   → Answer: Addressed by existing `compact_conversations()`, not a sync issue

---

*Architecture analysis: 2026-04-06*
*Researcher: Claude Code (Haiku)*
