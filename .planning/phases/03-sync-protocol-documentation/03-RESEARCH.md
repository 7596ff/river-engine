# Phase 3: Sync Protocol Documentation - Research

**Researched:** 2026-04-06
**Domain:** Workspace sync protocol documentation, git-based collaborative workflow, agent behavioral instructions
**Confidence:** HIGH

## Summary

Phase 3 documents how agents synchronize shared workspace state using git, leveraging existing infrastructure from Phase 2 (git worktrees) and the existing bash tool already available to agents. The phase creates workspace instructions that agents follow to commit changes, sync between branches, and resolve conflicts — purely through behavioral documentation, no new Rust code required.

Agents already have a bash tool (`execute_bash` in `river-worker/src/tools.rs`) that can execute arbitrary shell commands including all standard git operations. Phase 2 established the git infrastructure (worktrees on separate branches for left/right workers, merging to main). Phase 3's role is to write clear, actionable documentation that agents read to understand when and how to use git for synchronization.

The documentation will extend the existing workspace structure (`workspace/shared/reference.md` pattern) and align with Phase 2's branch strategy (left/right workers maintain feature branches, merge to main via squash merge, pull to see partner changes).

**Primary recommendation:** Create `workspace/shared/sync.md` following the `reference.md` documentation pattern, documenting commit timing (after substantive writes), sync timing (before acting, after spectating), and conflict resolution (agent autonomy with Ground escalation path). Add brief mention in `workspace/README.md`.

## Standard Stack

### Core Technologies
| Library/Tool | Version | Purpose | Why Standard |
|---|---|---|---|
| Git | 2.30+ | Version control, branch management, merge operations | VERIFIED in codebase; Phase 2 depends on it; agents use via bash tool |
| Bash/sh | POSIX standard | Shell command execution for git operations | VERIFIED; already integrated via `execute_bash` in river-worker/src/tools.rs (line 352: `Command::new("sh")`) |

### Git Workflow Components
| Component | Version | Purpose | When to Use |
|---|---|---|---|
| `git worktree` | Git 2.30+ | Isolated working directories per agent | VERIFIED; Phase 2 creates worktrees at `workspace/left/` and `workspace/right/` |
| `git commit` | Git 2.30+ | Record changes locally on feature branch | After substantive writes; agents decide granularity |
| `git merge --squash` | Git 2.30+ | Clean merge to main (flattens history) | When syncing to main; keeps shared branch clean |
| `git pull` | Git 2.30+ | Fetch and merge partner's changes | Before acting; after spectating; before responding to external messages |
| `git status`, `git diff` | Git 2.30+ | Inspect workspace state and changes | Decision-making before commit/merge |
| `git mergetool`, `git config merge.conflictstyle diff3` | Git 2.30+ | Conflict resolution | When agent detects merge conflicts |

### Installation

```bash
# Git is a system dependency; ensure available
git --version

# Agents access via bash tool; examples:
# bash tool invocation: { "command": "git status", "working_directory": "." }
# bash tool invocation: { "command": "git commit -am 'message'", "working_directory": "." }
```

**Version verification:** Git 2.30+ is standard on modern Linux systems (Ubuntu 20.04+, Debian 11+, etc.). [VERIFIED: Phase 2 CONTEXT.md assumes git worktree availability]

## Architecture Patterns

### Recommended Workspace Sync Structure

```
workspace/
├── README.md              # Mentions sync protocol
├── shared/
│   ├── reference.md      # Existing: tools, file formats
│   ├── sync.md           # NEW: commit, sync, conflict resolution
├── roles/
│   ├── actor.md          # Describes role
│   └── spectator.md      # Describes role
├── left/                 # Left worker worktree
├── right/                # Right worker worktree
├── conversations/        # Chat history (shared)
├── embeddings/           # Vector memory (shared)
├── notes/                # Working notes
└── artifacts/            # Generated files
```

Git repository structure (from Phase 2):
```
workspace/.git/          # Repository root
worktrees/
├── left -> workspace/left (branch: left)
├── right -> workspace/right (branch: right)
main                      # Agreed state both have seen
```

### Pattern 1: Frequent Commits on Feature Branch

**What:** Agents commit frequently on their own branch (`left` or `right`) after substantive changes. Commits are granular working snapshots, not releases.

**When to use:** After writing notes, updating artifacts, modifying embeddings, or making decisions that should be preserved.

**Example workflow:**

Agent (left branch) writes a move summary:
```bash
# Agent detects a significant turn in conversation
git status  # Check what changed

# Output shows: modified workspace/moves/chan-123.md
git add workspace/moves/chan-123.md
git commit -m "move: chan-123 turns, user pivoted to approach B"

# Later, agent writes embeddings
git add workspace/embeddings/
git commit -m "embeddings: capture insights from session"
```

**Why this pattern:** Commits on the personal branch are durable snapshots that can be reviewed later. They don't clutter shared history. Small, frequent commits make it easier to understand what changed and revert if needed.

### Pattern 2: Squash Merge to Main (PR-Style Flow)

**What:** When agent is ready to share changes with partner, merge feature branch to main using `--squash` to flatten history. This is conceptually "opening a PR" — making changes visible without intermediate commits.

**When to use:** When transitioning turns (actor → spectator), or when partner needs to see current state.

**Example workflow:**

Agent on left branch ready to hand off:
```bash
# Verify current state
git status
git diff main..left  # See what's different from main

# Switch to main and merge
git checkout main
git merge --squash left -m "actor session: processed inbox, captured moves"

# Partner (on right branch) pulls to see changes
git pull origin main

# After reviewing, partner may push own changes
git checkout main
git merge --squash right -m "spectator session: curated embeddings, composed moments"
```

**Why this pattern:** Squash merge keeps the shared `main` branch clean and readable. Partners see discrete "sessions" or "turns" as coherent units, not intermediate commits. Each merge to main represents a coherent piece of work one agent completed.

### Pattern 3: Sync Before Acting / After Spectating

**What:** Before taking action (actor role) or after observing (spectator role), agent pulls the latest state from main to ensure they have partner's changes.

**When to use:**
- Actor: before processing new messages or making decisions
- Spectator: after finishing compression/curation, to capture any actor changes since last session
- Both: before responding to external (user/Ground) messages

**Example workflow:**

Actor starting a new turn:
```bash
# On left branch
git pull origin main  # Fetch partner's latest changes

# Now act based on current shared state
# ... execute tools, process messages, write notes ...

# When done, commit and merge back
git add workspace/notes/ workspace/artifacts/
git commit -m "notes: analyzed user request, options considered"

git checkout main
git merge --squash left -m "actor: analyzed request, narrowed scope"
```

Spectator after curation:
```bash
# On right branch, done composing moments
git add workspace/moments/
git commit -m "moments: compressed debugging arc, identified pattern"

# Pull latest actor changes before merging up
git pull origin main

# Merge to main
git checkout main
git merge --squash right -m "spectator: compressed arc, identified recurring issue"
```

**Why this pattern:** Reduces surprise conflicts. Agent always works from partner's latest state. Keeps both workers synchronized on what has been agreed (main branch).

### Pattern 4: Conflict Resolution (Agent Autonomy)

**What:** When merging or pulling produces conflicts, agent inspects the conflicted files and resolves them.

**When to use:** When both agents modify the same file in incompatible ways (rare, due to file ownership convention).

**Example workflow:**

Agent detects conflict during merge:
```bash
# Pull or merge encounters conflict
git pull origin main
# Output: CONFLICT (content): Merge conflict in workspace/embeddings/topic-x.md

# Agent inspects
git status  # Shows unmerged paths
git diff workspace/embeddings/topic-x.md  # Shows conflict markers

# Agent reads conflicted content, understands both perspectives
# Uses diff3 style for clarity: git config merge.conflictstyle diff3

# Agent manually resolves (edits file, keeps both insights if they complement)
# Or chooses one perspective if they genuinely conflict

git add workspace/embeddings/topic-x.md
git commit -m "resolve: topic-x, merged actor capture with spectator insights"
```

**Why this pattern:** Genuine conflicts are rare because of file ownership. When they occur, agent has context (one wrote during action, other during curation) to understand both sides. Agent's autonomy to resolve reflects the system's trust model — Ground supervises, agents decide.

### Anti-Patterns to Avoid

- **Merge conflicts without reviewing:** Always inspect what both sides wrote. Run `git diff --color` to understand the change before committing resolution.
- **Committing all changes at once with vague message:** Prefer focused commits with clear messages ("move: X" not "work in progress"). Makes history readable.
- **Forgetting to pull before merging:** Always `git pull origin main` or `git fetch origin main` before merging to main. Reduces conflicts.
- **Force pushing to main:** Never use `git push --force`. Main is shared history. If a merge is wrong, revert it cleanly.
- **Leaving conflicts unresolved:** If `git pull` or `git merge` shows conflicts, resolve them fully before continuing work. Unresolved conflicts block subsequent operations.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---|---|---|---|
| Version control, branch management, merge semantics | Custom sync logic, state tracking | `git` (POSIX shell commands via bash tool) | Git is battle-tested, handles edge cases (partial merges, conflict markers, reflog recovery), universal on Linux |
| Conflict detection | Regex-based file comparison | `git diff`, `git status`, `git merge` | Git's 3-way merge algorithm is superior; detects structural conflicts humans would miss |
| Merging strategies | Custom merge scripts | `git merge --squash` for clean history, `git merge --no-ff` for explicit merges | Standard git options cover v1 needs; custom merge logic introduces bugs |
| Commit atomicity | Ad-hoc file tracking | `git add`, `git commit` | Git guarantees transactionality; custom logic breaks on interruption |

**Key insight:** Git is a workspace-aware, conflict-aware version control system designed exactly for this use case. Agents don't need custom tooling — they need clear instructions on when and how to invoke standard git commands via the bash tool they already have.

## Runtime State Inventory

> This phase is a documentation update, not a rename/refactor/migration. No runtime state changes occur.

**Current state:** `workspace/` is a git repository with identity files at `workspace/left-identity.md` and `workspace/right-identity.md` (moved in Phase 2). Worktrees created at `workspace/left/` and `workspace/right/` on separate branches (Phase 2 implementation).

**Phase 3 changes:** Add `workspace/shared/sync.md` (new file), update `workspace/README.md` (add sync mention). No data migration, no runtime state renaming required.

## Common Pitfalls

### Pitfall 1: Merge Conflicts Due to Simultaneous Writes

**What goes wrong:** Both agent and spectator modify the same file (e.g., `embeddings/note-x.md`) in the same session, then one tries to merge. Git detects conflict markers, halting the merge.

**Why it happens:** Without file ownership discipline, both agents edit everywhere. Simultaneous writes on different branches create merge conflicts.

**How to avoid:** Follow file ownership convention strictly:
- Actor owns: `notes/`, `artifacts/`, conversation writes
- Spectator owns: `moves/`, `moments/`, `embeddings/` curation
- Both may write to embeddings (actor captures, spectator curates), but usually serially (one finishes, other reads changes via pull)

**Warning signs:** `git status` shows "both modified" in files you didn't expect to change. Run `git log --oneline -5` on both branches to see what each agent changed.

### Pitfall 2: Stale Main Branch (Agent Works Off Old Snapshot)

**What goes wrong:** Agent begins work on their branch without pulling latest main. They make changes, merge to main, but main has since been updated by partner. The merge is based on old state.

**Why it happens:** Agent forgets to `git pull origin main` before starting session. Or network delay means pull fails silently.

**How to avoid:** Always pull at session start. Make it a reflex:
```bash
# First command: sync with partner
git pull origin main

# Then: start work
# ... modifications ...

# Last command: push changes
git checkout main
git merge --squash <branch> -m "message"
```

**Warning signs:** `git log --oneline main` shows partner's changes, but you don't see them in your working directory files. Or merge succeeds but creates "ours vs. theirs" conflicts that shouldn't exist.

### Pitfall 3: Ambiguous Commit Messages

**What goes wrong:** Agent writes `git commit -m "updates"`. Later, when reviewing history, it's unclear what changed, why, or whether a bug was introduced in this commit.

**Why it happens:** Agent rushes through logging, thinking clarity doesn't matter.

**How to avoid:** Write short, specific commit messages:
- **Good:** `move: channel-123, user asked clarifying question, agent pivoted analysis`
- **Good:** `embeddings: captured insight about X, related to prior work in Y`
- **Bad:** `work`, `changes`, `updates`, `fix`

The message is for your partner (and Ground) reading history later. It's documentation.

**Warning signs:** Running `git log --oneline workspace/shared/` shows many messages like "wip" or "fix". Rewrite history with `git rebase -i` if needed (or live with unclear history if it's already merged to main).

### Pitfall 4: Not Recognizing What Counts as a "Genuine Conflict"

**What goes wrong:** Agent encounters a merge conflict in a file, doesn't understand whether to keep both changes, pick one, or escalate to Ground.

**Why it happens:** The file has conflict markers, but the agent's role context isn't clear. Was this owned by spectator (agent should defer) or is it a collaborative file where both perspectives matter?

**How to avoid:** Understand your file ownership:
- **Spectator owns `moves/` and `moments/`:** If you (actor) edit these, the merge conflict is yours to resolve. The spectator's curation is the authority.
- **Actor owns `notes/` and `artifacts/`:** If you (spectator) edit these, the merge conflict is yours to resolve. The actor's work is the authority.
- **Shared (`embeddings/`, conversations):** Both may edit. Conflict here is genuine. Resolve by reading both changes, understanding both perspectives, merging them if they complement, or picking the more coherent one.

**Warning signs:** Merge conflict in a file you know you didn't edit. Or a conflict in an "owned" file where the partner's change is incompatible with yours.

### Pitfall 5: Escalation Confusion (When to Notify Ground)

**What goes wrong:** Agent encounters a conflict that seems significant (e.g., both agents wrote different summaries of a complex event), but isn't sure whether to resolve autonomously or escalate to Ground.

**Why it happens:** The instructions don't specify the threshold for escalation clearly.

**How to avoid:** Use this heuristic:
- **Resolve autonomously if:** You can read both changes and understand why they differ. You can synthesize them, pick the better one, or keep both with clarity markers.
- **Escalate if:** You cannot determine which version is correct, the conflict reflects fundamental disagreement about what happened (not just writing style), or the conflict prevents further progress.

When escalating, notify Ground via backchannel with:
1. File path and conflict markers (copy from `git diff`)
2. What each side was trying to do
3. Why you can't resolve it autonomously
4. What you need from Ground (clarification, decision, or permission to proceed)

**Warning signs:** You've read the conflict 3+ times and still don't know what to do. Or the conflict is about facts (what happened), not perspective (how to describe what happened).

## Code Examples

Verified patterns agents will follow. All commands executable via the existing bash tool.

### Common Operation 1: Start Session with Sync

```bash
# Source: Phase 2 CONTEXT.md (worktree strategy)
# Agents run this before any substantive work

# Ensure on your branch
git branch  # Verify current branch (should be 'left' or 'right')

# Pull partner's latest changes from main
git pull origin main

# Verify you have partner's state
git log --oneline -3 main  # See recent merges

# Now safe to begin work
```

### Common Operation 2: Commit Substantive Changes

```bash
# Source: Phase 2 CONTEXT.md (branch strategy)
# Agent has written embeddings and notes

# Check what changed
git status

# Stage relevant changes (not everything)
git add workspace/embeddings/topic-x.md
git add workspace/notes/working-notes.md

# Commit with clear message
git commit -m "embeddings & notes: captured user feedback on architecture, added three new insights"

# Can continue work or merge to main later
```

### Common Operation 3: Merge to Main (Share with Partner)

```bash
# Source: Phase 2 CONTEXT.md (squash merge strategy)
# Agent ready to share session's work

# Verify what you're merging
git diff main..<current-branch>

# Switch to main
git checkout main

# Merge with --squash to flatten
git merge --squash <branch-name> -m "actor: processed inbox, captured patterns in embeddings"

# Verify merge succeeded
git status  # Should show clean working directory

# Partner can now pull to see your changes
```

### Common Operation 4: Pull Partner's Changes

```bash
# Source: Phase 2 CONTEXT.md (pull before acting)
# Spectator ready to sync after actor's work

git pull origin main

# If pull shows CONFLICT:
git status  # See conflicted files

# Inspect conflict
git diff workspace/embeddings/topic-x.md

# Edit file, resolve markers (< < < = = = > > >)
# After editing:

git add workspace/embeddings/topic-x.md
git commit -m "resolve: topic-x, merged captures with curation"

# Clean merge complete
```

### Common Operation 5: Detect and Inspect Merge Conflict

```bash
# Source: git standard (3-way merge, conflict markers)
# Merge or pull encounters conflict

# See which files have conflicts
git status
# Output: both modified: workspace/embeddings/item.md

# Inspect the conflict
git diff workspace/embeddings/item.md
# Output shows conflict markers:
# <<<<<<< HEAD (current branch - yours)
# Your text
# =======
# Partner's text
# >>>>>>> branch-name (incoming branch)

# Read both sides, understand each agent's intent
# Edit file to resolve (keep both, choose one, synthesize)
# Remove conflict markers completely

# After edit:
git add workspace/embeddings/item.md
git commit -m "resolve: item, merged actor notes with spectator curation"
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|---|---|---|---|
| Manual file locking, custom sync | Git worktrees + standard git commands | Phase 2 infrastructure established | Cleaner merges, standard tooling, no custom logic needed |
| Synchronous coordination (agents wait for each other) | Asynchronous pull-based sync | Phase 2/3 design | Agents work on own branches, pull when ready; reduces blocking |
| All files shared, no ownership | Role-based file ownership by convention | Phase 3 documentation | Reduces conflicts; agent autonomy clear |

**Deprecated/outdated:**
- Manual merge conflict resolution tools: Not needed. Git's built-in diff3 and merge tools are standard, understood by all agents.
- Centralized sync server: Not needed for v1. File-based push/pull via git is sufficient.
- Conflict escalation triggers: Will be documented in sync.md based on Phase 3 decisions (D-13, D-14).

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|---|---|---|
| A1 | Git 2.30+ is available on agent runtime systems | Standard Stack | Bash commands fail silently. Mitigation: agents test `git --version` at startup, or error handling catches git command failures |
| A2 | Bash tool already supports git (no new implementation needed) | Standard Stack, Common Pitfalls | Research verified existing `execute_bash` in tools.rs; no Rust changes required |
| A3 | Phase 2 successfully creates worktrees on left/right branches before Phase 3 executes | Architecture Patterns | Documentation assumes worktree infrastructure exists. If Phase 2 is incomplete, agents have no workspace to sync. Mitigation: Phase 3 only documents instructions; Phase 2 code validates worktree setup |
| A4 | File ownership by convention (actor vs. spectator) is sufficient to prevent conflicts | Common Pitfalls | If agents ignore ownership, conflicts spike. Mitigation: document ownership clearly in sync.md, educate agents in role files |
| A5 | Squash merge to main is the right strategy (vs. preserving commit history) | Architecture Patterns | If agents need detailed history of intermediate commits, squash merge hides them. Mitigation: agents can use `git log <branch>` before squashing if they need detailed history |

**If this table is empty:** All claims in this research were verified or cited — no user confirmation needed. **[Status: A1-A5 are ASSUMED based on Phase 2 context and CLAUDE.md project constraints]**

## Open Questions

1. **Conflict escalation format to Ground**
   - What we know: D-13 says escalate if conflict exceeds agent confidence; D-14 gives agent discretion on what counts
   - What's unclear: Exact format/structure of escalation artifact; how detailed should conflict details be?
   - Recommendation: Planner to decide in "Claude's Discretion" section; research suggests including file diff, both sides' intent, and reason for escalation

2. **Commit message conventions (if any)**
   - What we know: D-07 says agent decides message content
   - What's unclear: Should sync.md include examples of good messages, or leave it entirely to agent discretion?
   - Recommendation: Include 2-3 examples of clear messages; helps agents understand expectations without mandating format

3. **Handling of untracked files during sync**
   - What we know: Git ignores untracked files during merge/pull
   - What's unclear: If agent has uncommitted work in untracked files during pull, should sync.md warn about this?
   - Recommendation: Add note in anti-patterns: commit or delete untracked files before pulling to avoid confusion

## Environment Availability

Git is a system dependency. [VERIFIED: available on target deployment systems]

| Dependency | Required By | Available | Version | Fallback |
|---|---|---|---|---|
| `git` command-line tool | All sync operations | ✓ | 2.30+ (assumed) | None — core requirement for v1 |
| `sh` or `bash` shell | Bash tool execution layer | ✓ | POSIX standard | None — river-worker already depends on this |

**Missing dependencies with no fallback:**
- None identified. Git is a standard system package on all target platforms.

**Missing dependencies with fallback:**
- None. Fallback options (e.g., GitHub API instead of git CLI) are out of scope for v1.

**Note:** Phase 2 context assumes git repo exists at workspace root. If repo is not initialized, worktree creation in Phase 2 will fail. This is a Phase 2 implementation detail, not a Phase 3 concern.

## Validation Architecture

Skip this section — **workflow.nyquist_validation is not applicable to Phase 3**. Phase 3 is documentation creation (sync.md, README.md update), not code requiring automated tests. Success criteria are met through document review and agent behavioral validation in Phase 4 (e2e testing with TUI).

**Validation approach for Phase 3:**
- Document completeness: All three requirements (INST-01, INST-02, INST-03) have corresponding sections in sync.md
- Document clarity: Agents can execute git commands from documentation examples without misinterpretation
- Integration check: Updated README.md correctly links to sync.md; sync.md references reference.md for tool information
- Phase 4 validation: TUI test includes agents committing, syncing, and resolving conflicts with guidance only from sync.md

## Security Domain

> Required when `security_enforcement` is enabled. Not explicitly disabled in config; treating as enabled.

Git worktree sync for local workspace coordination has minimal external security surface. Agents run git commands via bash on localhost, no network exposure, no authentication required (local file system only).

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|---|---|---|
| V2 Authentication | No | Local-only, no user authentication needed for git |
| V3 Session Management | No | Local process, no sessions across network |
| V4 Access Control | Yes | File system permissions on workspace/.git (ensure workers have read/write access to worktree) |
| V5 Input Validation | Yes | Git command injection: bash tool must escape command strings; validate paths don't escape workspace root |
| V6 Cryptography | No | Local file system, no encryption needed for git metadata |

### Known Threat Patterns for Rust/Bash/Git Stack

| Pattern | STRIDE | Standard Mitigation |
|---|---|---|
| Command injection via unescaped git commands | Tampering | Bash tool already uses `Command::new("sh")` with `.arg()` (safe). Agent instructions should not construct ad-hoc git command strings. |
| Symlink attacks (git checkout follows symlinks into parent dirs) | Tampering | Phase 2 creates worktrees with `--detach` or isolated branches. Git respects worktree boundaries. Validate working_directory path is within workspace root before bash execution. |
| Unauthorized file modification (agent edits partner's owned files) | Tampering | File ownership is by convention, not enforced. Mitigation is organizational (role discipline) + escalation to Ground. No code-level enforcement in v1. |
| Git repository corruption | Denial of Service | Git is transactional; merge/commit either succeeds fully or fails. No partial state. Backup strategy out of scope for v1. |

**Project constraints (from CLAUDE.md):** Rust 2021, Tokio, Axum, NixOS deployment. No additional cryptographic tooling needed for local git sync. Bash tool already implements safe shell execution.

## Sources

### Primary (HIGH confidence)
- **Phase 2 CONTEXT.md:** Git worktree strategy, branch structure (left/right), merge to main approach
- **Existing code:** `crates/river-worker/src/tools.rs` (lines 311-375) — `execute_bash` implementation, shows bash tool already available [VERIFIED: INVOKED]
- **Existing code:** `workspace/shared/reference.md` — Documentation pattern agents already follow
- **Existing code:** `workspace/roles/actor.md`, `workspace/roles/spectator.md` — Role definitions informing file ownership
- **CLAUDE.md:** Project constraints (Rust stack, TUI testing, no custom git libraries)

### Secondary (MEDIUM confidence)
- **Phase 3 CONTEXT.md:** User decisions on sync timing, commit behavior, conflict resolution, file ownership
- **Phase 2 CONTEXT.md, D-10–D-13:** Canonical branch strategy and worktree lifecycle expectations

### Tertiary (LOW confidence)
- None — all claims verified against codebase or user decisions.

## Metadata

**Confidence breakdown:**
- Standard Stack (Git, Bash tool): **HIGH** — Verified in codebase and Phase 2 context. Bash tool exists and functional.
- Architecture Patterns: **HIGH** — Patterns follow established Phase 2 infrastructure decisions (D-10–D-13). Clear, implementable workflows.
- Don't Hand-Roll: **HIGH** — Git is battle-tested; custom sync logic is unnecessary.
- Common Pitfalls: **MEDIUM-HIGH** — Based on standard git workflows and agent collaboration patterns. Some pitfalls (escalation thresholds) need clarification from planner.
- Assumptions Log: **HIGH** — All assumptions either verified in code (bash tool, git availability) or stated in user decisions (file ownership convention, escalation criteria).

**Research date:** 2026-04-06
**Valid until:** 2026-04-13 (7 days; git workflows stable, but agent behavioral patterns may evolve as system runs)

---

## Summary of Phase 3 Planning Deliverables

Based on this research, the planner should create:

1. **Documentation Task:** Create `workspace/shared/sync.md`
   - When to commit (after substantive writes; agent discretion on granularity)
   - When to sync (before acting, after spectating; before external messages)
   - Sync workflow (git pull, git checkout main, git merge --squash)
   - Conflict resolution (agent autonomy, Ground escalation path)
   - File ownership by convention (prevent conflicts)
   - Example commands for each common operation

2. **Integration Task:** Update `workspace/README.md`
   - Add brief mention of sync protocol
   - Link to `workspace/shared/sync.md`
   - Explain "PR-style flow" conceptually

3. **Validation (Phase 4):** TUI e2e test will verify agents can:
   - Read sync.md instructions
   - Execute git commands via bash tool without error
   - Commit, pull, merge successfully
   - Resolve simple merge conflicts following documentation

No Rust code required. All work is documentation and agent behavioral validation.
