---
phase: 4
slug: e2e-testing-with-tui
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-06
---

# Phase 4 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo test (Rust native) |
| **Config file** | `Cargo.toml` workspace config |
| **Quick run command** | `cargo test -p river-orchestrator --test e2e` |
| **Full suite command** | `cargo test --workspace` |
| **Estimated runtime** | ~30 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test -p river-orchestrator --test e2e`
- **After every plan wave:** Run `cargo test --workspace`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 30 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 04-01-01 | 01 | 1 | TEST-01 | — | N/A | integration | `cargo test dyad_boots` | ❌ W0 | ⬜ pending |
| 04-01-02 | 01 | 1 | TEST-02 | — | N/A | integration | `cargo test worktree_io` | ❌ W0 | ⬜ pending |
| 04-01-03 | 01 | 1 | TEST-03 | — | N/A | integration | `cargo test role_switching` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `crates/river-orchestrator/tests/e2e.rs` — E2E test harness module
- [ ] `crates/river-orchestrator/tests/mock_llm.rs` — Mock LLM server for deterministic responses
- [ ] `crates/river-orchestrator/tests/helpers.rs` — Shared test fixtures (process spawning, polling)

*If none: "Existing infrastructure covers all phase requirements."*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| TUI baton display | D-09 | Visual rendering | Launch TUI, verify header shows actor/spectator state |

*If none: "All phase behaviors have automated verification."*

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 30s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
