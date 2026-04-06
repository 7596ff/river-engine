---
phase: 02-workspace-infrastructure
plan: 02
subsystem: protocol-orchestrator
tags: [protocol-extension, registration, worktree-path, infrastructure]
dependency_graph:
  requires:
    - "02-01: Worktree creation infrastructure"
  provides:
    - "Worker registration includes worktree_path field"
    - "Orchestrator populates side-specific worktree paths"
  affects:
    - "Workers will receive isolated worktree paths at startup"
tech_stack:
  added: []
  patterns:
    - "Protocol field extension with backward compatibility"
    - "Side-based path construction (Left → workspace/left, Right → workspace/right)"
key_files:
  created: []
  modified:
    - path: "crates/river-protocol/src/registration.rs"
      changes: "Added worktree_path field to WorkerRegistrationResponse"
    - path: "crates/river-orchestrator/src/http.rs"
      changes: "Added worktree_path field to local WorkerRegistrationResponse struct and populated it in registration handler"
decisions:
  - id: "D-02-02-01"
    summary: "Worktree path constructed in registration handler based on worker side"
    rationale: "Orchestrator knows workspace layout; workers receive explicit path without inference"
    alternatives: "Workers could infer from workspace + side, but explicit is clearer"
  - id: "D-02-02-02"
    summary: "Workspace field kept for backward compatibility"
    rationale: "Existing code may depend on workspace field; gradual migration safer than breaking change"
    alternatives: "Could remove workspace entirely, but risks breaking existing integrations"
metrics:
  duration_minutes: 2
  tasks_completed: 2
  tasks_total: 2
  files_modified: 2
  commits: 2
  completed_date: "2026-04-06"
---

# Phase 02 Plan 02: Worker Registration Worktree Path Extension Summary

**One-liner:** Extended worker registration protocol to pass isolated worktree paths (workspace/left or workspace/right) from orchestrator to workers at startup.

## What Was Built

Extended the worker registration protocol to include worktree paths, completing the handoff from orchestrator (creates worktrees) to workers (use worktrees). Workers now receive explicit paths to their isolated git worktrees instead of having to infer them from workspace + side.

### Protocol Extension (Task 1)

Modified `crates/river-protocol/src/registration.rs`:
- Added `pub worktree_path: String` field to `WorkerRegistrationResponse` struct
- Positioned after `workspace` field
- Added doc comment: "Path to worker's isolated git worktree (workspace/left or workspace/right)"
- Marked `workspace` field as "legacy, kept for backward compatibility"
- Required field, no serde attributes needed

### Orchestrator Handler Update (Task 2)

Modified `crates/river-orchestrator/src/http.rs`:
- Added `worktree_path` field to local `WorkerRegistrationResponse` struct definition
- Added worktree path construction in `handle_worker_registration`:
  ```rust
  let worktree_path = match req.worker.side {
      Side::Left => dyad_config.workspace.join("left"),
      Side::Right => dyad_config.workspace.join("right"),
  };
  ```
- Populated field in response: `worktree_path: worktree_path.to_string_lossy().to_string()`

## Deviations from Plan

None - plan executed exactly as written. Both tasks completed without issues.

## Verification Results

**Protocol compilation:**
```bash
$ cargo check -p river-protocol
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.36s
```

**Orchestrator compilation:**
```bash
$ cargo check -p river-orchestrator
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.04s
```

**Full build:**
```bash
$ cargo build -p river-protocol && cargo build -p river-orchestrator
Finished `dev` profile [unoptimized + debuginfo] target(s) in 13.77s
```

All compilation checks passed. Warnings present are pre-existing (unused imports and dead code from Plan 01).

**Field presence verification:**
```bash
$ grep "pub worktree_path: String" crates/river-protocol/src/registration.rs
    pub worktree_path: String,

$ grep "worktree_path:" crates/river-orchestrator/src/http.rs
    pub worktree_path: String,
        worktree_path: worktree_path.to_string_lossy().to_string(),
```

Both structs have the field, and the handler populates it correctly.

## Implementation Notes

### Dual Struct Definitions

The orchestrator defines its own `WorkerRegistrationResponse` struct locally in `http.rs` rather than importing from `river-protocol`. This appears to be an existing pattern in the codebase. Both structs were updated to maintain consistency:

- `river-protocol::WorkerRegistrationResponse` (for worker deserialization)
- `river-orchestrator::http::WorkerRegistrationResponse` (for orchestrator serialization)

### Path Construction Pattern

The worktree path follows the decision from Plan 01 (D-09):
- Left worker: `workspace/left/`
- Right worker: `workspace/right/`

This matches the directory structure created by the worktree infrastructure in Plan 01.

### Backward Compatibility

The existing `workspace` field remains present and populated with the root workspace path. This ensures:
- No breaking changes for existing code that reads the workspace field
- Gradual migration path: new code uses `worktree_path`, old code continues working
- Documentation clearly marks `workspace` as legacy

## Known Stubs

None. All fields are properly populated with real values from dyad configuration.

## Threat Surface

No new threats introduced. Threat surface analysis from plan:

| Threat ID | Category | Component | Disposition | Status |
|-----------|----------|-----------|-------------|--------|
| T-02-05 | Information Disclosure | worktree_path in response | accept | Localhost HTTP only, paths not secrets |
| T-02-06 | Tampering | Worker modifying worktree_path | accept | Workers are trusted processes |
| T-02-07 | Spoofing | Malicious worker registration | mitigate | Existing dyad config validation applies |

No new trust boundaries or attack surfaces. Workers receive paths over the same localhost HTTP channel used for all registration data.

## Requirements Satisfied

- **INFRA-02:** Workers receive worktree paths via registration protocol ✓

Workers now have explicit knowledge of their isolated worktree location at startup, enabling them to operate on their own git branch without filesystem conflicts.

## Next Steps

**Plan 02-02 completes the infrastructure layer.** Workers can now:
1. Register with orchestrator (existing)
2. Receive worktree path in registration response (this plan)
3. Use worktree path for all filesystem operations (future: worker implementation)

**Immediate next:** Worker code needs to:
- Read `worktree_path` from registration response
- Use it for workspace loading instead of `workspace` field
- Document in workspace files how to sync between worktrees

**Blocked:** None. Infrastructure is ready for workers to consume.

## Self-Check: PASSED

**Created files exist:** N/A (no new files created, only modifications)

**Modified files verified:**
```bash
$ [ -f "crates/river-protocol/src/registration.rs" ] && echo "FOUND: crates/river-protocol/src/registration.rs" || echo "MISSING: crates/river-protocol/src/registration.rs"
FOUND: crates/river-protocol/src/registration.rs

$ [ -f "crates/river-orchestrator/src/http.rs" ] && echo "FOUND: crates/river-orchestrator/src/http.rs" || echo "MISSING: crates/river-orchestrator/src/http.rs"
FOUND: crates/river-orchestrator/src/http.rs
```

**Commits exist:**
```bash
$ git log --oneline | head -5
cfa5d68 feat(02-02): populate worktree_path in orchestrator registration handler
a44dbec feat(02-02): add worktree_path field to WorkerRegistrationResponse
949555a0 feat(02-01): create git worktrees for dyad workers at supervisor startup
798ec3f docs(02): capture phase context
256ce5f docs(phase-01): evolve PROJECT.md after phase completion
```

Both commits present and in correct order.

**Worktree_path field present in both structs:**
```bash
$ grep -c "pub worktree_path: String" crates/river-protocol/src/registration.rs
1
$ grep -c "pub worktree_path: String" crates/river-orchestrator/src/http.rs
1
```

**Worktree_path populated in handler:**
```bash
$ grep -c "worktree_path: worktree_path.to_string_lossy().to_string()" crates/river-orchestrator/src/http.rs
1
```

All verification checks passed.
