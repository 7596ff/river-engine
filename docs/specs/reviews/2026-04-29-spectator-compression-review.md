# Adversarial Review: Spectator Compression Spec (Final)

**Date:** 2026-04-29
**Spec:** `docs/specs/2026-04-29-spectator-compression-design.md`
**Reviewer:** Gemini CLI

---

## 1. Executive Summary

The revised specification successfully addresses the critical technical flaws identified in previous reviews, specifically the **Mutex Trap** and the **Message/Move Race Condition**. By explicitly defining a "lock-query-drop" pattern and an "ordering guarantee" for persistence, the spec now provides a technically sound foundation for implementation.

## 2. Resolved Issues

- **Mutex Trap (Fixed):** The spec now explicitly mandates that the `std::sync::MutexGuard` be dropped before any `.await` points. This prevents compilation errors and deadlocks in the async runtime.
- **Race Condition (Fixed):** The spec adds an ordering guarantee requiring the agent to persist messages *before* emitting the `TurnComplete` event, ensuring the spectator always finds the data it needs.
- **Internal Contradictions (Resolved):** The role of `classify_move()` is clarified (removed), and the fallback logic is replaced with a simpler role/tool-based summary.
- **Turn Number Consistency (Fixed):** `turn_number` is now correctly defined as `NOT NULL` in the schema and a mandatory `u64` in the Rust struct.

## 3. Remaining Code Discrepancies

| File | Spec Claim | Code Reality | Discrepancy |
| :--- | :--- | :--- | :--- |
| `crates/river-gateway/src/spectator/mod.rs` | Identity loaded from `spectator/AGENTS.md`, etc. | Workspace uses `roles/spectator.md` and `roles/actor.md`. | **Existing Bug Propagation:** The spec maintains the current code's incorrect assumption about identity file paths. While this is an existing issue, the spec misses the opportunity to align with the actual workspace structure. |
| `crates/river-gateway/src/coordinator/events.rs` | `tool_calls` stored as "JSON array". | `AgentEvent::TurnComplete` uses `Vec<String>`. | Minor terminology mismatch between the internal Rust representation and the SQLite storage format. |

## 4. Unaddressed Technical Concerns

- **Robust Parsing:** The regex `turns:\s*(\d+)\s*-\s*(\d+)` remains fragile. If the LLM produces Markdown formatting (e.g., `turns: **12-34**`), the parser will fail. The fallback to the "entire range" prevents data loss but may lead to overlapping or poorly bounded moments.
- **Sync Service Compatibility:** It remains unverified if the `SyncService`'s `Note::parse` logic (in `crates/river-gateway/src/embeddings/note.rs`) will accept `type: moment` in the YAML frontmatter. If the parser is strict about `type: note`, moments will not be indexed as intended.
- **Prompt Hot-reloading:** Loading prompts only once at startup remains a limitation for developer experience, though not a functional blocker.

## 5. Grades

- **Completeness:** **A-** (Now covers cross-crate changes to `messages` and `agent/task.rs`).
- **Consistency:** **A** (Internal contradictions are resolved).
- **Precision:** **A** (Clear SQL and explicit threading/ordering rules).
- **Honesty:** **A** (Directly addresses the previously ignored Mutex conflict).

---

**Verdict: APPROVED.**

The specification is now technically robust and ready for implementation. The remaining concerns regarding file paths and parsing robustness are minor compared to the core architectural fixes and can be resolved during the implementation phase.
