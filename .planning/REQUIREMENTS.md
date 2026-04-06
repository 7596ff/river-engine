# Requirements: River Engine

**Defined:** 2026-04-06
**Core Value:** Two perspectives that can disagree — the gap between them creates internal structure a single rule-follower cannot have.

## v1 Requirements

Requirements for getting River to a working, testable state with TUI.

### Stability

- [ ] **STAB-01**: Replace panics with Result types in river-discord emoji parsing
- [ ] **STAB-02**: Replace panics with Result types in river-protocol message parsing
- [ ] **STAB-03**: Replace panics with Result types in river-context assembly

### Infrastructure

- [ ] **INFRA-01**: Orchestrator creates git worktree per worker at dyad startup
- [ ] **INFRA-02**: Worktree paths passed to workers via registration

### Instructions

- [ ] **INST-01**: Workspace docs describe when agents should commit (after writes)
- [ ] **INST-02**: Workspace docs describe when agents should sync (before acting, after spectating)
- [ ] **INST-03**: Workspace docs describe conflict resolution protocol

### Testing

- [ ] **TEST-01**: Dyad boots with TUI mock adapter
- [ ] **TEST-02**: Both workers can read/write to their worktrees
- [ ] **TEST-03**: Role switching works between actor and spectator

## v2 Requirements

Deferred to future release. Tracked but not in current roadmap.

### Discord Integration

- **DISC-01**: Dyad communicates with Ground via Discord adapter
- **DISC-02**: Discord events trigger worker notifications
- **DISC-03**: Workers can send messages to Discord channels

### Observability

- **OBS-01**: Structured logging to file/external system
- **OBS-02**: Sync operation metrics (commits, merges, conflicts)

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Rust git library (river-git crate) | Agents have bash tool; instructions simpler than code |
| Multi-dyad coordination | Single dyad sufficient for v1 testing |
| Authentication/TLS | Localhost only for v1 |
| Stream cancellation for flashes | Low priority; flash queues until response completes |
| Real-time sync | Batch sync at checkpoints sufficient; agents can follow protocol |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| STAB-01 | TBD | Pending |
| STAB-02 | TBD | Pending |
| STAB-03 | TBD | Pending |
| INFRA-01 | TBD | Pending |
| INFRA-02 | TBD | Pending |
| INST-01 | TBD | Pending |
| INST-02 | TBD | Pending |
| INST-03 | TBD | Pending |
| TEST-01 | TBD | Pending |
| TEST-02 | TBD | Pending |
| TEST-03 | TBD | Pending |

**Coverage:**
- v1 requirements: 11 total
- Mapped to phases: 0
- Unmapped: 11

---
*Requirements defined: 2026-04-06*
*Last updated: 2026-04-06 after initial definition*
