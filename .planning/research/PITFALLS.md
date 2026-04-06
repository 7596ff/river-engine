# Domain Pitfalls: Git-Based Sync + Panic Refactoring

**Domain:** Agent orchestrator with git worktree isolation and error path refactoring
**Researched:** 2026-04-06
**Confidence:** HIGH (git concurrency patterns well-documented; Rust error handling best practices verified)

---

## Critical Pitfalls

Mistakes that cause silent data corruption, lost commits, or system-wide crashes.

### Pitfall 1: Concurrent Git Operations Without Lock Serialization

**What goes wrong:**
Two workers simultaneously run `git pull`, `git merge`, or `git commit` on their worktrees. Git's `index.lock` file protects each worktree independently, BUT the shared `.git/refs/` directory is not protected. Both processes write to the same branch pointers, causing:
- Lost commits (one worker's push is silently overwritten)
- Corrupted branch refs (one worker sees wrong commit hash)
- Fork state: worker A thinks it's on commit X, worker B has pushed Y, but branch pointer is corrupted to Z

**Why it happens:**
Developers assume worktrees provide full isolation. They don't. Worktrees isolate the working directory and index, but share the object database and branch refs in `.git/`. Git is fundamentally single-process-per-repository, not single-process-per-worktree.

**Consequences:**
- Spectator worker merges changes from actor, but the merge actually overwrites unrelated commits
- Next sync loses work silently
- Discovered during testing: "Wait, where did my changes go?"
- In production: dyad diverges, Ground has to manually reconcile state

**Prevention:**
1. **Implement per-worker lock around all git operations** — before any git command, acquire a lock that's stored in orchestrator or shared state (e.g., Redis key or lock file in shared `.git/` directory)
2. **Use git with `--no-optional-locks`** where safe, but NOT for write operations (pull, merge, commit)
3. **Serialize git writes through orchestrator** — don't let workers call git directly. Make orchestrator run `git fetch`, `git merge`, and return success/failure to worker
4. **Add timestamps to branch refs** — write metadata to each branch about which worker last modified it, detect stale writes

**Detection:**
- Warning sign: `git pull` returns success but worker sees different commits than expected
- Warning sign: orchestrator logs show divergent branch pointers for same branch
- Catch in testing: verify both workers see same commits after sync

**Phase mapping:**  Phase 2 (Git Integration). Implement lock mechanism before workers touch git directly.

---

### Pitfall 2: Git State Corruption From Process Crash During Merge

**What goes wrong:**
Worker A starts `git merge spectator/state` but crashes mid-merge (OOM, panic, signal 9). The merge is incomplete:
- `.git/index.lock` left behind (next git command fails with "lock file exists")
- Merge conflict markers in working files, not staged
- Branch pointer partially updated (detached state or corrupted ref)
- When orchestrator respawns worker B on same worktree, B sees corrupted state and either crashes or skips sync thinking nothing changed

**Why it happens:**
Git operations are multi-step. If any step fails after creating `index.lock` but before deleting it, recovery requires human intervention. Panic crashes don't give the process time to clean up.

**Consequences:**
- Orchestrator health check sees worker is down, respawns it
- New worker can't run git commands (lock file exists)
- New worker fails to pull state changes, acts on stale context
- Dyad gets progressively more out of sync
- Discovery mode: both workers crashed, worktree is locked, sync is broken

**Prevention:**
1. **Wrap ALL git operations in error handling that cleans up lock files** — if git fails, run `git reset --hard` to abort merge and unlock
2. **Use git with explicit timeout** (`timeout 30s git merge ...`) to prevent hangs
3. **Ensure panic handler calls cleanup** — register signal handler (SIGTERM, SIGINT) that runs `git merge --abort` before exiting
4. **Before attempting git ops, check for and report stale locks** — on startup, if `.git/index.lock` exists, log error and fail fast (don't try to use corrupted state)
5. **Implement pre-merge validation** — run `git diff --check` before merge to detect merge conflicts early, refuse to merge if conflicts exist (force manual resolution)

**Detection:**
- Warning sign: worker crashes, orchestrator respawns it, new worker logs "fatal: Unable to create '.git/index.lock': File exists"
- Catch: Add test that kills worker mid-merge (SIGKILL), verify next spawn can recover
- Monitor: Track lock file age; if lock exists longer than timeout duration, alert

**Phase mapping:**  Phase 2 (Git Integration). Design cleanup protocol before first merge attempt. Phase 4 (Conflict Handling) should add graceful abort strategies.

---

### Pitfall 3: Panic Crashes During Tool Execution or Merge Handling

**What goes wrong:**
Worker is executing bash tool or parsing merged files, hits unexpected input, calls `.unwrap()`, panics. Process dies before:
- Committing state to git (uncommitted changes lost)
- Sending status to orchestrator (orchestrator thinks worker is hung, respawns it)
- Releasing locks (next worker can't sync)
- Rolling back partial tool execution (bash script half-completed)

**Why it happens:**
Current codebase has 456+ unwrap/expect calls. Error handling wasn't prioritized during initial development. These are time bombs — they work until they don't.

**Consequences:**
- Worker crashes on malformed JSON in tool response (line 212-213 in tools.rs)
- Orchestrator respawns worker, loses context of what it was trying to do
- If crash happened during merge, state is corrupted (see Pitfall 2)
- In production: user sees dyad go silent, restart, lose conversation history
- Hard to debug: no error message, just "worker-1 exited with code 101"

**Prevention:**
1. **Blanket refactoring: replace unwrap() with ? in all Result-returning functions** (Phase 1)
   - Files to prioritize: `tools.rs` (highest risk), `workspace_loader.rs`, `llm.rs`, `worker_loop.rs`
   - Use `cargo fix --allow-staged` to auto-convert where safe
2. **Add explicit error context using anyhow::Context** — chain errors with `.context("doing X")` so panic info isn't lost
3. **Never use unwrap_or_default() for critical paths** — use explicit error handling instead
4. **Add test for each unwrap-removal** — e.g., if removing unwrap from JSON parsing, add test with malformed JSON
5. **Create panic-free wrapper for tools execution** — catch any panic, log it, return Err instead of crashing

**Detection:**
- Warning sign: worker exits with panic, not graceful error
- Catch: Run with RUST_BACKTRACE=1, see panic at unwrap site
- Monitor: Count worker crashes per day; if crashes increase, high panic risk

**Phase mapping:**  Phase 1 (Error Paths). Block: must complete before testing with real LLM (LLM will generate unexpected input).

---

### Pitfall 4: Merge Conflicts Not Detected or Silently Resolved Incorrectly

**What goes wrong:**
Actor modifies `workspace/memo.md`, spectator modifies the same file in the same location. Workers sync:
- `git merge` reports 3-way merge success (no conflict markers in file)
- OR merge reports conflict but worker doesn't handle it and commits with conflict markers still in file
- OR merge resolves conflict by keeping one version, silently deleting the other worker's work

This happens because:
1. Worker doesn't check `git status` for unmerged paths after merge
2. Worker commits a merge with conflict markers still present (valid Git, invalid semantics)
3. `git merge` with auto-resolve strategy (default) silently keeps one side if both modified same region

**Why it happens:**
Developers assume git merge either succeeds cleanly or reports a conflict. It doesn't always. Git's 3-way merge can "succeed" and still produce nonsensical output if the base version is far from both current versions.

**Consequences:**
- Conversation history has corrupted lines with conflict markers (`<<<<<<<`)
- Context assembly parser hits malformed file, crashes (unwrap on regex capture)
- Or spectator reads corrupted state, LLM hallucinates responses based on corrupted context
- Lost work: one worker's edits are overwritten by merge auto-resolve

**Prevention:**
1. **After every merge, check for conflicts before continuing:**
   ```rust
   git merge actor/state
   if git status --porcelain | grep "^UU" (unmerged) {
     return Err("Merge conflict detected, manual resolution required")
   }
   ```
2. **Never auto-resolve merge; force explicit conflict handling** — use `git merge --no-commit --no-ff` and validate result before committing
3. **Add merge pre-check** — run `git merge --no-commit --no-ff actor/state` to test merge, then abort if conflicts detected
4. **Validate file format AFTER merge** — parse `.md` files to detect conflict markers, fail if found
5. **Implement conflict resolution strategy explicitly** — define rules: "spectator changes take precedence" or "use larger diff" or "escalate to Ground"

**Detection:**
- Warning sign: `git log --oneline` shows merge commit, but no corresponding merge commit on other worker
- Catch: Test merge of two divergent branches, verify conflict is detected or properly resolved
- Monitor: Check conversation files for `<<<<<<<` markers, alert if found

**Phase mapping:**  Phase 4 (Conflict Handling). Don't attempt two-worker sync without this.

---

## Moderate Pitfalls

Issues that cause data loss or reduced functionality, recoverable but require intervention.

### Pitfall 5: Stale Lock Files Left by Previous Worker Instance

**What goes wrong:**
Worker 1 crashes. Orchestrator respawns Worker 2 in same worktree. Git tries to write, but `.git/index.lock` exists from Worker 1's crash. Git fails with:
```
fatal: Unable to create '.git/index.lock': File exists
```
Worker 2 is now stuck. It can't pull, can't commit, can't merge.

**Why it happens:**
When git exits abnormally, it doesn't clean up lock files. The lock is a safety mechanism — Git won't touch index while locked — but stale locks are unrecoverable without intervention.

**Prevention:**
1. **On worktree startup, detect and remove stale locks:**
   ```rust
   if Path::new(".git/index.lock").exists() {
     log_error("Stale lock detected, removing");
     std::fs::remove_file(".git/index.lock")?;
   }
   ```
2. **Validate git is healthy after removing lock** — run `git status` and verify it succeeds
3. **Only remove lock if no other process is using the worktree** — check orchestrator's process table first

**Detection:**
- Warning sign: Worker logs "fatal: Unable to create" after respawn
- Catch: Test crash + respawn in same worktree, verify lock is handled gracefully

**Phase mapping:**  Phase 2 (Git Integration). Implement as part of worktree setup.

---

### Pitfall 6: Error Context Lost During Panic Refactoring

**What goes wrong:**
During refactoring, developer removes `unwrap()` and replaces with `?`:
```rust
// Before
let value = json_obj.get("key").unwrap();

// After
let value = json_obj.get("key")?;
```
Now error is propagated, but caller doesn't add context. Error bubbles up without explanation of what was being parsed. Ground sees:
```
Error: key not found
```
No info on which file, which LLM response, which tool call.

**Why it happens:**
Quick refactoring doesn't add context layers. Each error needs context at the point where it occurs, not just at the top level.

**Consequences:**
- Debugging is harder (no trace of where error originated)
- Ground can't diagnose LLM hallucination vs. tool failure vs. git corruption
- Error logs are useless for understanding failures

**Prevention:**
1. **Every refactored Result must include .context() or .map_err():**
   ```rust
   let value = json_obj.get("key")
     .context("Parsing LLM response for tool_call field")?;
   ```
2. **Use typed errors, not just strings** — define custom error enum with variants for each failure mode
3. **Chain context all the way up** — each layer adds context about what it was doing
4. **Test that error messages are informative** — in error path tests, verify the message explains the failure

**Detection:**
- Warning sign: Error log with no context about which file/worker/request
- Catch: Run tests with expected errors, verify error message is useful to human reader

**Phase mapping:**  Phase 1 (Error Paths). Build error handling framework early, enforce in code review.

---

### Pitfall 7: Partial Commits / Incomplete Git State After Tool Execution

**What goes wrong:**
Worker executes bash tool that modifies files:
1. Worker calls `git add .` (stages changes)
2. Worker calls `git commit -m "..."`
3. But worker crashes before returning success to orchestrator
4. Next sync assumes commit succeeded (worker reported "OK"), but commit wasn't actually made
5. Or commit was made, but on wrong branch

OR:

1. Worker modifies multiple files with bash tool
2. Worker crashes after modifying file A, before modifying file B
3. Next worker sees partial state (file A modified, file B not)
4. Dyad is now inconsistent

**Why it happens:**
Bash tool execution and git commits are separate operations. If they're not atomic (commit contains all the tool's effects, or tool doesn't run at all), partial state is possible.

**Consequences:**
- Dyad sees inconsistent workspace state
- Spectator's context assembly pulls partially-modified files
- LLM response is based on inconsistent state
- Eventually detected when spectator tries to parse file and hits inconsistency

**Prevention:**
1. **Require idempotent bash tools** — tools should be safe to run twice, not break if partially executed
2. **Commit after EVERY tool execution** — don't accumulate changes across multiple tools before committing
3. **Use git transactions (pseudo)** — if tool modifies multiple files, group them into one commit
4. **Add pre-commit validation** — before committing, validate all modified files are in expected state
5. **Log commit SHA before returning to caller** — return the commit hash, not just "OK"

**Detection:**
- Warning sign: Worker reports success, but next worker's `git log --oneline` doesn't show expected commit
- Catch: Test bash tool execution with crash injection (kill -9 mid-execution)

**Phase mapping:**  Phase 3 (Tool Execution Safety). Required before tools run in production.

---

## Minor Pitfalls

Issues that reduce clarity or cause minor inefficiencies.

### Pitfall 8: No Rollback Strategy for Failed Syncs

**What goes wrong:**
Actor and spectator sync:
1. Spectator pulls actor's changes successfully
2. Spectator attempts to merge, merge fails (conflicts or corruption)
3. Spectator now has actor's commits in its history, but can't use them
4. Spectator is stuck in "mid-merge" state

To recover, someone has to manually `git reset --hard` and try again.

**Prevention:**
1. **Use `git merge --no-commit` to test merge before committing** — test result, then commit if valid
2. **Keep track of pre-merge commit hash** — if merge fails, can reset to pre-merge state
3. **Implement rollback-on-error** — if merge fails, run `git merge --abort` immediately

**Detection:**
- Warning sign: Worker is in detached HEAD after failed merge
- Catch: Test merge of conflicting branches, verify worker recovers

**Phase mapping:**  Phase 4 (Conflict Handling).

---

### Pitfall 9: No Metrics on Git Operation Performance

**What goes wrong:**
As conversation history grows, `git merge` takes longer (Git walks history). Worker A takes 5s to merge, Worker B takes 15s. If merge timeout is 10s, Worker B fails intermittently. Ground doesn't know why.

**Prevention:**
1. **Add timing instrumentation to all git operations** — log duration of pull, merge, commit
2. **Alert if git operation exceeds threshold** — if merge takes >10s, log warning
3. **Profile with large conversation histories** — test merge performance with 100MB+ history

**Detection:**
- Warning sign: Worker logs timeout, but retry succeeds (timing-dependent)
- Monitor: Plot merge duration over time, watch for growth

**Phase mapping:**  Phase 3 (Sync) or later, during testing.

---

### Pitfall 10: Panic Stack Traces Don't Point to Root Cause

**What goes wrong:**
After refactoring panics to Results, a panic still occurs somewhere. The stack trace is deep and hard to read. Ground sees:
```
thread 'tokio-runtime' panicked at 'called `Result::unwrap()` on an `Err` value'
<backtrace here>
```
No information about WHAT was being processed or WHY it failed.

**Prevention:**
1. **Use `RUST_BACKTRACE=1` or better yet, structured error types** — don't rely on panic messages
2. **In tests, use assertion libraries that provide context** — don't just assert, provide message
3. **Add logging at critical boundaries** — before parsing LLM responses, before git ops
4. **Never panic from async code without context** — wrap with `catch_unwind` + logging

**Detection:**
- Warning sign: Panic message has no context about what failed
- Catch: In tests, simulate errors (malformed input, OOM) and verify error context is preserved

**Phase mapping:**  Phase 1 (Error Paths). Enforce in code review.

---

## Phase-Specific Warnings

| Phase | Topic | Likely Pitfall | Mitigation |
|-------|-------|---------------|-----------|
| **Phase 1: Error Paths** | Panic refactoring | Incomplete coverage (456+ unwraps means some will be missed) | Use cargo-clippy to find all unwrap sites, refactor systematically, add test for each |
| **Phase 1: Error Paths** | Error context | Lost context during propagation | Enforce `.context()` in code review, use typed errors |
| **Phase 2: Git Integration** | Lock serialization | Concurrent git writes corrupt refs | Implement orchestrator-level lock BEFORE first merge attempt |
| **Phase 2: Git Integration** | Stale locks | Worker crashes leave locks, next worker can't run git | Handle stale locks on startup |
| **Phase 3: Sync Protocol** | Partial commits | Tool execution and commits are not atomic | Test crash scenarios with bash tools |
| **Phase 3: Sync Protocol** | State validation | After sync, no validation that merged state is sensible | Add file format validation after merge |
| **Phase 4: Conflict Handling** | Auto-resolve | Git auto-resolves "successfully" but produces nonsense | Test merge of conflicting files, verify conflicts are detected |
| **Phase 4: Conflict Handling** | Rollback | Failed merge leaves worker in bad state | Implement explicit rollback (merge --abort) on error |
| **Testing** | Crash injection | All pitfalls require crash-during-operation tests | Use `timeout` + `kill -9` + respawn to test recovery |
| **Testing** | Large history | Performance issues hidden until conversation grows | Test with 100MB+ git histories |

---

## Sources

- [Git Worktree Concurrent Operations Race Conditions](https://github.com/github/spec-kit/issues/1476)
- [Git Index Lock File Documentation](https://devtoolbox.dedyn.io/blog/git-index-lock-file-exists-fix-guide)
- [Git Single-Process Limitation with Worktrees](https://github.com/kaeawc/auto-worktree/issues/176)
- [Git Worktrees Complete Guide 2026](https://devtoolbox.dedyn.io/blog/git-worktrees-complete-guide)
- [Rust Error Handling Best Practices](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- [Refactoring to Improve Error Handling](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)
- [Rust Error Handling Propagation](https://www.lpalmieri.com/posts/error-handling-rust/)
- [Testing Error Paths in Rust](https://nrc.github.io/error-docs/rust-errors/testing.html)
- [Merge Conflict Resolution with AI](https://www.graphite.com/guides/ai-code-merge-conflict-resolution)
- [Workspace Isolation in Multi-Agent Systems](https://docs.openclaw.ai/concepts/agent-workspace)
- [Git Stash Merge Conflict Handling](https://oneuptime.com/blog/post/2026-01-24-git-stash-effectively/view)
- [Concurrent Git Lock Prevention Strategies](https://devtoolbox.dedyn.io/blog/git-index-lock-file-exists-fix-guide)
- [Agent Workspace Isolation and Corruption Prevention](https://northflank.com/blog/how-to-sandbox-ai-agents)

---

*Research complete: 2026-04-06*
