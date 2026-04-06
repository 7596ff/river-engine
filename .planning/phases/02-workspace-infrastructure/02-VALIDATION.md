---
phase: 02
slug: workspace-infrastructure
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-06
---

# Phase 02 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | tokio::test (async tests in same crate) + #[test] for sync code |
| **Config file** | None yet; tests create temporary directories and git repos |
| **Quick run command** | `cargo test -p river-orchestrator worktree` |
| **Full suite command** | `cargo test -p river-orchestrator && cargo test -p river-worker && cargo test -p river-protocol` |
| **Estimated runtime** | ~15 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test -p river-orchestrator worktree`
- **After every plan wave:** Run `cargo test -p river-orchestrator && cargo test -p river-worker`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 20 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 02-01-01 | 01 | 1 | INFRA-01 | T-02-01 | Validate worktree paths (no directory traversal) | unit | `cargo test -p river-orchestrator -- worktree` | ❌ W0 | ⬜ pending |
| 02-01-02 | 01 | 1 | INFRA-01 | — | N/A | integration | `cargo test -p river-orchestrator -- test_spawn_dyad_creates_worktrees` | ❌ W0 | ⬜ pending |
| 02-01-03 | 01 | 1 | INFRA-02 | — | N/A | unit | `cargo test -p river-protocol -- test_registration_response` | ✅ | ⬜ pending |
| 02-02-01 | 02 | 1 | INFRA-02 | — | N/A | integration | `cargo test -p river-worker -- test_worker_config_worktree_path` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `crates/river-orchestrator/tests/worktree_tests.rs` — Test worktree creation, validation, reuse logic
- [ ] `crates/river-orchestrator/src/supervisor.rs` — Helper functions: `ensure_worktree_exists()`, `is_valid_worktree()`
- [ ] `crates/river-protocol/src/registration.rs` — Add `worktree_path` field to `WorkerRegistrationResponse`
- [ ] `crates/river-worker/tests/config_tests.rs` — Test `worktree_path()` method and path resolution

*Existing infrastructure provides foundation; new tests cover worktree-specific behaviors.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| Worktree persists across restarts | INFRA-01 | Requires process restart | 1. Run orchestrator with dyad, 2. Stop orchestrator, 3. Start again, 4. Verify worktree reused (no "Created worktree" log) |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 20s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
