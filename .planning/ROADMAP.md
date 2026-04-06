# River Engine v1 Roadmap

**Project:** River Engine
**Milestone:** v1 - Working, testable state with TUI mock adapter
**Created:** 2026-04-06
**Granularity:** Standard (5-8 phases)

---

## Phases

- [ ] **Phase 1: Error Handling Foundation** - Replace panics with Result types across three crates
- [ ] **Phase 2: Workspace Infrastructure** - Git worktrees at dyad startup, worker registration
- [ ] **Phase 3: Sync Protocol Documentation** - Workspace instructions for agent commit/pull/resolve
- [ ] **Phase 4: E2E Testing with TUI** - Dyad boot, worktree read/write, role switching

---

## Phase Details

### Phase 1: Error Handling Foundation

**Goal:** All critical code paths return Result types instead of panicking, providing stable foundation for testing.

**Depends on:** Nothing (foundational)

**Requirements:** STAB-01, STAB-02, STAB-03

**Success Criteria** (what must be TRUE):
1. Discord emoji parsing errors return Result, no panics on invalid emojis
2. River protocol message parsing errors return Result, no panics on malformed messages
3. Context assembly errors return Result, no panics on missing workspace files
4. All three crates compile and pass existing tests with error handling

**Plans:** 3 plans in 1 wave

Plans:
- [ ] 01-01-PLAN.md — Discord emoji parsing error handling (DiscordAdapterError, parse_emoji Result)
- [ ] 01-02-PLAN.md — Protocol message parsing error handling (ConversationError, parse_message_line Result)
- [ ] 01-03-PLAN.md — Context timestamp parsing error handling (ContextError extensions, parse_now/extract_timestamp Result)

---

### Phase 2: Workspace Infrastructure

**Goal:** Workers own isolated git worktrees created by orchestrator at startup, passed via registration.

**Depends on:** Phase 1

**Requirements:** INFRA-01, INFRA-02

**Success Criteria** (what must be TRUE):
1. Orchestrator creates unique git worktree for each worker (two per dyad) at startup
2. Worktree paths passed to workers in registration payload
3. Workers receive worktree path and can use it in context assembly
4. No shared filesystem access between workers (all I/O isolated to own worktree)

**Plans:** TBD

---

### Phase 3: Sync Protocol Documentation

**Goal:** Workspace instructions teach agents when and how to sync via existing bash tool (no new Rust code).

**Depends on:** Phase 2

**Requirements:** INST-01, INST-02, INST-03

**Success Criteria** (what must be TRUE):
1. Workspace docs describe commit protocol (when: after writes, what: changed files)
2. Workspace docs describe sync protocol (when: before acting, after spectating; how: pull with conflict handling)
3. Workspace docs describe merge conflict resolution (agent tooling, manual review steps)
4. Instructions are executable by agents using existing bash tool (no new commands required)

**Plans:** TBD

---

### Phase 4: E2E Testing with TUI

**Goal:** Dyad boots, workers communicate via worktrees, TUI adapter validates actor/spectator loop.

**Depends on:** Phase 3

**Requirements:** TEST-01, TEST-02, TEST-03

**Success Criteria** (what must be TRUE):
1. Dyad boots with TUI mock adapter (orchestrator, two workers, adapter running)
2. Both workers can read and write to their isolated worktrees
3. Role switching works: actor writes, spectator reads, roles reverse
4. Messages flow through protocol: actor thinks/acts, spectator observes, baton switches

**Plans:** TBD

---

## Progress

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Error Handling Foundation | 0/3 | Planning complete | - |
| 2. Workspace Infrastructure | 0/2 | Not started | - |
| 3. Sync Protocol Documentation | 0/2 | Not started | - |
| 4. E2E Testing with TUI | 0/3 | Not started | - |

---

**Coverage:** 11/11 v1 requirements mapped (100%)

Requirement mapping:
- STAB-01, STAB-02, STAB-03 → Phase 1
- INFRA-01, INFRA-02 → Phase 2
- INST-01, INST-02, INST-03 → Phase 3
- TEST-01, TEST-02, TEST-03 → Phase 4
