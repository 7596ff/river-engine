---
phase: 01
slug: error-handling-foundation
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-06
---

# Phase 01 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo test (Rust native) |
| **Config file** | Cargo.toml (workspace root) |
| **Quick run command** | `cargo test --lib -q` |
| **Full suite command** | `cargo test --all` |
| **Estimated runtime** | ~15 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib -q`
- **After every plan wave:** Run `cargo test --all`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 15 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 01-01-01 | 01 | 1 | STAB-01 | — | N/A | unit | `cargo test -p river-discord --lib` | ⬜ W0 | ⬜ pending |
| 01-01-02 | 01 | 1 | STAB-02 | — | N/A | unit | `cargo test -p river-protocol --lib` | ⬜ W0 | ⬜ pending |
| 01-02-01 | 02 | 1 | STAB-03 | — | N/A | unit | `cargo test -p river-context --lib` | ⬜ W0 | ⬜ pending |
| 01-03-01 | 03 | 2 | STAB-01,02,03 | — | N/A | integration | `cargo test --all` | ✅ | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] Error type tests for each crate's new error enum
- [ ] Test cases for malformed input handling (emoji, protocol, context)

*Existing Rust test infrastructure covers framework needs.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| — | — | — | — |

*All phase behaviors have automated verification.*

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 15s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
