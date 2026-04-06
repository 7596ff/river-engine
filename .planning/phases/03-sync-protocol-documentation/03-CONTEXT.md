# Phase 3: Sync Protocol Documentation - Context

**Gathered:** 2026-04-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Workspace instructions teach agents when and how to sync via existing bash tool (no new Rust code). This phase writes documentation that agents follow to commit, pull, merge, and resolve conflicts using git commands.

</domain>

<decisions>
## Implementation Decisions

### Documentation location
- **D-01:** Create new `workspace/shared/sync.md` as dedicated sync protocol file
- **D-02:** Follow `reference.md` pattern — technical docs with clear instructions
- **D-03:** Add brief mention in `workspace/README.md` explaining sync and linking to `sync.md`

### Commit behavior
- **D-04:** Agents commit frequently on their own branch (`left`/`right`)
- **D-05:** Use guidelines with discretion — "commit after substantive changes or before transitions" — agent judges what counts
- **D-06:** Squash merge when syncing to `main` for clean history
- **D-07:** Agent decides commit message content based on what changed

### Sync timing
- **D-08:** Sync at turn start as default behavior
- **D-09:** Mandatory sync before responding to external messages
- **D-10:** Additional syncs at agent discretion when fresh state matters
- **D-11:** "PR-style" flow using git commands locally — agent merges their branch to main, partner pulls to see changes

### Conflict resolution
- **D-12:** Agent attempts to resolve conflicts autonomously
- **D-13:** If conflict exceeds agent's confidence, escalate: notify Ground via backchannel and create artifact with conflict details
- **D-14:** Agent discretion determines what counts as a "genuine conflict" requiring escalation

### File ownership (conflict prevention)
- **D-15:** Role-based ownership structure to minimize conflicts:
  - Actor owns: `notes/`, `artifacts/`, conversation writes
  - Spectator owns: `moves/`, `moments/`, `embeddings/`
- **D-16:** Ownership is by convention — not enforced, but agents respect boundaries

### Claude's Discretion
- Exact git commands and flags in sync.md
- Commit message format examples
- How to present conflict details in escalation artifact
- Any additional sync scenarios beyond the core cases

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Phase 2 infrastructure (git worktree setup)
- `.planning/phases/02-workspace-infrastructure/02-CONTEXT.md` — D-10 through D-13 define branch strategy (left/right branches merge into main)

### Existing workspace documentation (to be extended)
- `workspace/README.md` — Will need sync mention (D-03)
- `workspace/shared/reference.md` — Existing pattern for technical docs
- `workspace/roles/actor.md` — Actor responsibilities inform ownership (D-15)
- `workspace/roles/spectator.md` — Spectator responsibilities inform ownership (D-15)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `workspace/shared/reference.md` — Template for technical documentation format
- `workspace/roles/actor.md` and `spectator.md` — Already describe role responsibilities that map to ownership

### Established Patterns
- Markdown documentation with tables for quick reference
- "How it works" explanations followed by concrete examples
- Workspace files are read by agents via the `read` tool

### Integration Points
- New `sync.md` goes in `workspace/shared/` alongside `reference.md`
- README.md addition should fit the existing philosophical tone with practical link
- Agents execute git commands via existing `bash` tool — no new tool needed

</code_context>

<specifics>
## Specific Ideas

- "PR-style" flow using pure git commands — agents conceptually open a PR by merging to main, partner reviews by syncing and seeing changes
- Frequent small commits on personal branches, squash merge to main — gives durability without cluttering shared history
- Ownership by convention, not enforcement — agents respect boundaries but can cross them when needed

</specifics>

<deferred>
## Deferred Ideas

- Git hooks or automation for commit/sync triggers — keep it behavioral/instructional for now
- Workspace cleanup/gc for old branches — operational concern for later
- Conflict resolution tools beyond bash git commands — keep minimal for v1

</deferred>

---

*Phase: 03-sync-protocol-documentation*
*Context gathered: 2026-04-06*
