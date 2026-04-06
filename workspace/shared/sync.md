# Sync Protocol

This document describes how workers synchronize their workspace using git. Each worker maintains their own branch (`left` or `right`) and merges to `main` to share changes with their partner. This is a PR-style workflow using local git commands.

## Overview

The workspace is a git repository with worktrees for each worker:
- Left worker: `workspace/left/` on branch `left`
- Right worker: `workspace/right/` on branch `right`
- Shared state: branch `main` (both workers merge here)

You commit frequently on your own branch. When you're ready to share, you merge to `main` (conceptually "opening a PR"). Your partner pulls `main` to see your changes (conceptually "reviewing the PR"). This keeps your work durable while giving you and your partner separate spaces to work.

For bash tool syntax and other tools, see `workspace/shared/reference.md`.

## When to Commit

Commit frequently on your own branch after substantive changes. You decide what counts as "substantive" — the guideline is: commit after meaningful writes or before transitions.

Each commit on your feature branch (`left` or `right`) is a working snapshot, not a release. Small, frequent commits make it easier to understand what changed and revert if needed. When you merge to `main`, use squash merge to keep shared history clean.

**Commit message content:** You decide based on what changed. Prefer specific descriptions over vague ones like "wip" or "updates."

### Examples

```bash
# See what changed
git status

# Stage specific files (not everything)
git add workspace/notes/working-notes.md workspace/embeddings/topic-x.md

# Commit with clear message
git commit -m "notes: captured user feedback on architecture, three new insights"
```

```bash
# After writing a move summary
git status  # Shows: modified workspace/moves/chan-123.md
git add workspace/moves/chan-123.md
git commit -m "move: chan-123 turn, user pivoted to approach B"
```

```bash
# After capturing embeddings
git add workspace/embeddings/
git commit -m "embeddings: capture insights from session, related to prior work on X"
```

## When to Sync

**Default behavior:** Sync at turn start — before acting or after spectating.

**Mandatory:** Sync before responding to external messages (user or Ground).

**Optional:** Additional syncs at your discretion when fresh state matters.

### PR-Style Workflow

The workflow uses git commands to simulate pull requests:

1. **Agent merges feature branch to main** — conceptually "opening a PR"
2. **Partner pulls main to see changes** — conceptually "reviewing the PR"

This is all done with local git commands — no GitHub, no web UI.

### Sync at Turn Start

Before you begin substantive work, pull your partner's latest changes from `main`:

```bash
# Ensure you're on your branch
git branch  # Should show 'left' or 'right'

# Pull partner's latest changes
git pull origin main

# Verify you have their state
git log --oneline -3 main  # See recent merges

# Now safe to begin work
```

### Share Your Work (Merge to Main)

When you're ready to share your session's work with your partner:

```bash
# Verify what you're merging
git diff main..left  # Replace 'left' with your branch name

# Switch to main
git checkout main

# Squash merge to flatten history
git merge --squash left -m "actor: processed inbox, captured patterns in embeddings"

# Verify merge succeeded
git status  # Should show clean working directory

# Your partner can now pull to see your changes
```

### Pull Partner's Changes

When your partner has merged their work to `main`, pull to see it:

```bash
git pull origin main
```

If the pull succeeds without conflicts, you're done. If there are conflicts, see the Conflict Resolution section below.

## File Ownership Convention

To minimize conflicts, follow ownership patterns based on your role:

| Directory | Owner | Purpose |
|-----------|-------|---------|
| `notes/` | Actor | Working notes, drafts, scratch space |
| `artifacts/` | Actor | Generated files, code, documents |
| `conversations/` | Actor | Chat history writes (both read) |
| `moves/` | Spectator | Per-turn summaries |
| `moments/` | Spectator | Arc summaries |
| `embeddings/` | Both | Actor captures, spectator curates |

**Ownership is by convention, not enforcement.** You respect boundaries but can cross them when needed. Both workers may write to `embeddings/` — the actor captures new insights, the spectator curates and surfaces them. This usually happens serially (one finishes, other pulls and sees the changes).

Following this convention reduces conflicts. When conflicts do occur, they're easier to resolve because you understand who owns what.

## Conflict Resolution

When merging or pulling produces conflicts, git halts and marks the conflicted files. You see this:

```bash
git pull origin main
# Output: CONFLICT (content): Merge conflict in workspace/embeddings/topic-x.md
```

### Inspect the Conflict

First, see which files have conflicts:

```bash
git status
# Output shows: both modified: workspace/embeddings/topic-x.md
```

Inspect the conflict markers:

```bash
git diff workspace/embeddings/topic-x.md
```

You'll see markers like this:

```
<<<<<<< HEAD (your changes)
Your text here
=======
Partner's text here
>>>>>>> branch-name (incoming changes)
```

### Resolve Autonomously

Read both sides and understand the intent from each agent's role context:

- **Keep both** if the changes complement each other (e.g., actor added notes, spectator added curation)
- **Choose one** if there's clear authority (e.g., spectator owns `moves/`, defer to their version)
- **Synthesize** if both perspectives matter and can be combined

Edit the file to resolve, removing all conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`).

After editing:

```bash
git add workspace/embeddings/topic-x.md
git commit -m "resolve: topic-x, merged actor notes with spectator curation"
```

### Escalate to Ground

Escalate if:
- You cannot determine which version is correct
- The conflict reflects fundamental disagreement about what happened (not just writing style)
- The conflict prevents further progress

**How to escalate:**

Notify Ground via backchannel. Include:
1. File path and conflict markers (copy from `git diff`)
2. What each side was trying to do (read the context around the conflict)
3. Why you can't resolve it autonomously (what's unclear or contradictory)
4. What you need from Ground (clarification, decision, or permission to proceed)

**Example escalation message:**

```
Conflict in workspace/embeddings/user-request.md:

<<<<<<< HEAD
User requested feature A with constraint X
=======
User requested feature B with constraint Y
>>>>>>> right

Actor captured during message processing (feature A).
Spectator captured during curation (feature B).
These are contradictory facts about the same conversation.
Cannot resolve without reviewing original conversation.
Need Ground to confirm which request is correct.
```

## Common Operations

### Start Session with Sync

```bash
git branch  # Verify current branch (should be 'left' or 'right')
git pull origin main  # Pull partner's latest changes
git log --oneline -3 main  # See recent merges
# Now safe to begin work
```

### Commit Substantive Changes

```bash
git status  # See what changed
git add workspace/notes/working-notes.md workspace/embeddings/topic-x.md  # Stage specific files
git commit -m "embeddings & notes: captured user feedback on architecture, added three new insights"
```

### Merge to Main (Share with Partner)

```bash
git diff main..left  # Verify what you're merging (replace 'left' with your branch)
git checkout main  # Switch to main
git merge --squash left -m "actor: processed inbox, captured patterns"  # Merge with squash
git status  # Verify clean working directory
```

### Pull Partner's Changes

```bash
git pull origin main
# If conflicts occur, see Conflict Resolution section
```

### Detect and Resolve Merge Conflict

```bash
# Pull encounters conflict
git pull origin main
# Output: CONFLICT (content): Merge conflict in workspace/embeddings/item.md

# See which files have conflicts
git status
# Output: both modified: workspace/embeddings/item.md

# Inspect the conflict
git diff workspace/embeddings/item.md
# Shows conflict markers: <<<<<<< HEAD ... ======= ... >>>>>>>

# Edit file to resolve (keep both, choose one, or synthesize)
# Remove all conflict markers

# After editing:
git add workspace/embeddings/item.md
git commit -m "resolve: item, merged actor notes with spectator curation"
```

### Recover from Interrupted Merge

If you started a merge and need to abort:

```bash
git merge --abort  # Cancels merge, returns to pre-merge state
```

If you want to see what changed in a failed merge:

```bash
git status  # Shows unmerged paths
git diff  # Shows all conflicts
```

### Check Partner's Recent Work

Before merging, review what your partner did:

```bash
git log --oneline main -5  # Last 5 commits on main
git diff main..left  # What you're about to merge (replace 'left' with your branch)
```

## Anti-Patterns

### Merge Conflicts Without Reviewing

**Don't:** Accept one side blindly, or remove conflict markers without reading both sides.

**Do:** Always inspect what both sides wrote. Run `git diff` to understand each change. Both agents had reasons for their edits.

### Vague Commit Messages

**Don't:** Use messages like "wip", "updates", "changes", "fix".

**Do:** Write specific descriptions of what changed and why:
- Good: "move: chan-123, user asked clarifying question, agent pivoted analysis"
- Good: "embeddings: captured insight about X, related to prior work in Y"
- Bad: "work", "updates"

The message is for your partner (and Ground) reading history later. It's documentation.

### Forgetting to Pull Before Merging

**Don't:** Start work on your branch without pulling `main` first. Merge to `main` without checking if your partner updated it.

**Do:** Always `git pull origin main` at session start. Verify `git log main` before merging.

### Force Pushing to Main

**Don't:** Use `git push --force` or `git push --force-with-lease`. Never rewrite shared history on `main`.

**Do:** If a merge is wrong, revert it cleanly with `git revert` or notify Ground.

`main` is shared history. Both agents depend on it being stable and append-only.

### Leaving Conflicts Unresolved

**Don't:** Continue work while `git status` shows unmerged paths. Ignore conflict markers in files.

**Do:** Resolve conflicts immediately after they occur. Unresolved conflicts block subsequent git operations (pulls, merges, commits).

If you can't resolve, escalate to Ground. Don't leave the workspace in a conflicted state.

### Committing Everything at Once

**Don't:** Stage all changes with `git add .` and commit with a vague message.

**Do:** Stage specific files that form a coherent unit. Use `git add path/to/file.md` for targeted commits. Makes history readable and reversions precise.

---

**Summary:** Commit frequently on your branch. Merge to `main` with squash when ready to share. Pull `main` before acting. Resolve conflicts with autonomy; escalate when needed. Follow file ownership to prevent conflicts. Use clear commit messages for documentation.
