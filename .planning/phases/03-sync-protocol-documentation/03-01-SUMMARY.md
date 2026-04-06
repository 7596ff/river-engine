---
phase: 03-sync-protocol-documentation
plan: 01
subsystem: workspace
tags: [documentation, git, sync-protocol, agent-behavior]
dependency_graph:
  requires: [phase-02-workspace-infrastructure]
  provides: [sync-protocol-docs]
  affects: [workspace-documentation, agent-instructions]
tech_stack:
  added: []
  patterns: [pr-style-workflow, file-ownership-convention, autonomous-conflict-resolution]
key_files:
  created:
    - workspace/shared/sync.md
  modified: []
decisions:
  - Use PR-style flow with squash merge to main for clean shared history
  - Agent discretion on commit granularity with guideline "after substantive changes"
  - File ownership by convention (not enforcement) to minimize conflicts
  - Autonomous conflict resolution with Ground escalation path
  - All git operations via existing bash tool (no new Rust code)
metrics:
  duration_seconds: 101
  completed_at: "2026-04-06T18:31:04Z"
  tasks_completed: 1
  tasks_total: 1
  files_created: 1
  files_modified: 0
  commits: 1
---

# Phase 03 Plan 01: Sync Protocol Documentation Summary

**Complete sync protocol documentation enabling agents to synchronize git worktrees using standard git commands via bash tool**

## What Was Built

Created `workspace/shared/sync.md` (346 lines) documenting the complete git sync protocol for workspace coordination between left and right workers. The documentation teaches agents when to commit, when to sync with their partner, how to resolve conflicts, and which files each role owns.

### Key Sections Created

1. **Overview** — Explains PR-style flow with feature branches (`left`/`right`) merging to `main`
2. **When to Commit** — Documents D-04 to D-07 decisions: commit after substantive changes, agent discretion on granularity, squash merge for clean history
3. **When to Sync** — Documents D-08 to D-11 decisions: sync at turn start, mandatory before external messages, optional at agent discretion
4. **File Ownership Convention** — Documents D-15, D-16: Actor owns notes/artifacts, Spectator owns moves/moments, both may write embeddings
5. **Conflict Resolution** — Documents D-12, D-13, D-14: autonomous resolution first, escalate to Ground with structured information if needed
6. **Common Operations** — Concrete bash command examples for: start session sync, commit changes, merge to main, pull partner's changes, resolve conflicts, abort merges
7. **Anti-Patterns** — Documents what to avoid: merging without review, vague commit messages, forgetting to pull, force pushing, leaving conflicts unresolved

### Documentation Style

Followed `workspace/shared/reference.md` pattern:
- Tables for structured information (file ownership mapping)
- Concrete bash code blocks (not pseudocode)
- Clear section headers with action-oriented language
- Agent perspective ("you commit", "your partner pulls")
- Explanations of "why" alongside "how" (e.g., "squash merge keeps shared history clean")

## Requirements Satisfied

### INST-01: When to Commit
✓ Documented in "When to Commit" section
- Commit frequently on own branch after substantive changes
- Agent judges what counts as substantive (guideline: "after meaningful writes or before transitions")
- Squash merge to main for clean shared history
- Clear commit message examples provided

### INST-02: When to Sync
✓ Documented in "When to Sync" section
- Default: sync at turn start (before acting or after spectating)
- Mandatory: sync before responding to external messages
- Optional: additional syncs at agent discretion when fresh state matters
- PR-style workflow using `git pull origin main` and `git merge --squash`

### INST-03: Conflict Resolution
✓ Documented in "Conflict Resolution" section
- Agent attempts autonomous resolution first (inspect both sides, understand intent)
- Resolve by keeping both, choosing one, or synthesizing
- Escalate to Ground if: cannot determine correct version, fundamental disagreement, prevents progress
- Escalation includes: file path, conflict markers, both sides' intent, why can't resolve, what's needed

## User Decisions Implemented

| Decision | Implementation |
|----------|----------------|
| D-01 | Created `workspace/shared/sync.md` as dedicated sync protocol file |
| D-02 | Followed `reference.md` pattern — tables, code blocks, clear structure |
| D-03 | Deferred README.md update (can be added in future plan if needed) |
| D-04 | Documented: commit frequently on own branch |
| D-05 | Documented: agent discretion with guideline "after substantive changes or before transitions" |
| D-06 | Documented: squash merge when syncing to main |
| D-07 | Documented: agent decides commit message based on what changed, with examples |
| D-08 | Documented: sync at turn start as default |
| D-09 | Documented: mandatory sync before external messages |
| D-10 | Documented: optional additional syncs at agent discretion |
| D-11 | Documented: PR-style flow using `git checkout main && git merge --squash` |
| D-12 | Documented: agent attempts autonomous conflict resolution |
| D-13 | Documented: escalate to Ground via backchannel with structured information |
| D-14 | Documented: agent discretion on escalation threshold with heuristics |
| D-15 | Documented: file ownership table mapping directories to actor/spectator |
| D-16 | Documented: ownership by convention, not enforcement |

## Example Workflows Documented

### Start Session Workflow
```bash
git branch  # Verify current branch
git pull origin main  # Pull partner's changes
git log --oneline -3 main  # See recent merges
```

### Commit and Share Workflow
```bash
git status  # See what changed
git add workspace/notes/working-notes.md
git commit -m "notes: captured user feedback, three new insights"
git checkout main
git merge --squash left -m "actor: processed inbox, captured patterns"
```

### Conflict Resolution Workflow
```bash
git pull origin main  # Encounters conflict
git status  # See conflicted files
git diff workspace/embeddings/topic-x.md  # Inspect conflict markers
# Edit file to resolve
git add workspace/embeddings/topic-x.md
git commit -m "resolve: topic-x, merged actor notes with spectator curation"
```

## Integration with Phase 2 Infrastructure

- **Worktree paths**: Documentation references `workspace/left/` and `workspace/right/` from Phase 2
- **Branch structure**: Follows Phase 2 design (left/right branches merge to main)
- **Merge strategy**: Implements squash merge as established in Phase 2 context
- **Bash tool**: All git commands executable via existing `execute_bash` tool from river-worker

## Integration with Existing Workspace

- **Documentation pattern**: Follows `workspace/shared/reference.md` style (tables, code examples, clear headers)
- **File ownership**: Aligns with `workspace/roles/actor.md` and `spectator.md` role descriptions
- **Actor responsibilities**: Notes/artifacts ownership matches actor's capture role
- **Spectator responsibilities**: Moves/moments ownership matches spectator's compression role

## Readiness for Phase 4 Validation

Phase 4 (E2E testing with TUI) can now validate:
- Agents read `sync.md` and execute git commands successfully
- Commit workflow works as documented (agents commit after writes)
- Pull workflow works as documented (agents sync before acting)
- Merge workflow works as documented (squash merge to main)
- Conflict resolution follows documented protocol (autonomous then escalate)
- File ownership reduces conflicts as intended (actor/spectator boundaries respected)

All git commands in documentation are standard, executable via bash tool, require no new Rust code.

## Deviations from Plan

None — plan executed exactly as written. All must_haves satisfied:
- ✓ Workspace docs describe when to commit (substantive changes, agent discretion)
- ✓ Workspace docs describe when to sync (turn start, before external messages)
- ✓ Workspace docs describe conflict resolution (autonomous first, escalate protocol)
- ✓ Instructions executable using existing bash tool (all commands are standard git CLI)
- ✓ `workspace/shared/sync.md` exists with all required sections
- ✓ File is 346 lines (exceeds 150 line minimum)
- ✓ Contains "## When to Commit", "## When to Sync", "## Conflict Resolution"
- ✓ Contains file ownership table mapping directories to roles
- ✓ Contains 6+ `git commit` examples, 7+ `git pull` examples, 3+ `git merge` examples
- ✓ All git commands use standard flags (no custom scripts)
- ✓ Documentation follows reference.md style

## Known Stubs

None identified. Documentation is complete and actionable. All sections provide concrete bash commands agents can execute without additional clarification.

## Threat Surface Scan

No new threat surface introduced. Documentation describes use of existing bash tool with standard git commands. Threat model from plan addresses:
- T-03-01: Command injection via bash tool → accept (bash tool already uses safe `Command::new("sh")` with `.arg()`)
- T-03-04: Symlink attack via git checkout → mitigated (Phase 2 created worktrees with validated paths, sync.md doesn't instruct symlink operations)

All git operations are local filesystem operations within workspace boundaries. No network exposure, no authentication required.

## Self-Check: PASSED

### Files Created
```bash
$ [ -f "workspace/shared/sync.md" ] && echo "FOUND: workspace/shared/sync.md"
FOUND: workspace/shared/sync.md
```

### Commits Exist
```bash
$ git log --oneline --all | grep -q "4ecb781" && echo "FOUND: 4ecb781"
FOUND: 4ecb781
```

### Content Verification
- ✓ File contains all required sections (When to Commit, When to Sync, Conflict Resolution, File Ownership, Common Operations, Anti-Patterns)
- ✓ File contains 346 lines (exceeds 150 line minimum for comprehensive documentation)
- ✓ File contains 6 `git commit` examples (exceeds 3 minimum)
- ✓ File contains 7 `git pull` examples (exceeds 3 minimum)
- ✓ File contains 3 `git merge --squash` examples (meets 2 minimum)
- ✓ File contains ownership table mapping directories to actor/spectator
- ✓ All git commands use standard flags (verified: no custom scripts, all commands are POSIX standard)
- ✓ Documentation follows reference.md style (verified: tables present, code blocks present, clear headers)

All claims verified. Documentation complete and ready for agent use.
