# Adversarial Review: Spectator Compression Spec (Prompt-Driven Runtime)

**Date:** 2026-04-29
**Spec:** `docs/specs/2026-04-29-spectator-compression-design.md`
**Reviewer:** Gemini CLI

---

## 1. Executive Summary

The revised specification introduces a significant architectural shift, transforming the spectator from a hardcoded multi-struct system (`Compressor`, `Curator`, `RoomWriter`) into a generic **Prompt-Driven Dispatch Runtime**. This design is highly flexible but relies on the existence and correctness of external Markdown files in `workspace/spectator/`.

## 2. Resolved Issues

- **Mutex Trap (Fixed):** The spec mandates a "lock-query-drop" pattern, ensuring the `std::sync::MutexGuard` is dropped before any `.await` points.
- **Race Condition (Fixed):** An "Ordering Guarantee" is established, requiring the agent to persist messages *before* emitting the `TurnComplete` event.
- **Sync Service Compatibility (Verified):** The `NoteType` enum in `crates/river-gateway/src/embeddings/note.rs` already includes `Moment`, confirming that `type: moment` files in `embeddings/moments/` will be correctly indexed.
- **Curation/Room Notes (Cleaned up):** By removing the hardcoded `Curator` and `RoomWriter` and moving to a prompt-dispatch model, the "incomplete intelligence" problem is solved by making these behaviors optional and prompt-defined.

## 3. Remaining Code Discrepancies

| File | Spec Claim | Code Reality | Discrepancy |
| :--- | :--- | :--- | :--- |
| `crates/river-gateway/src/agent/task.rs` | Agent persists messages before event. | `AgentTask` currently has no `Database` handle. | **Major Wiring Gap:** To implement the ordering guarantee, `AgentTask` must be updated to hold an `Arc<Mutex<Database>>`, which it currently lacks. |
| `crates/river-gateway/src/coordinator/events.rs` | Spectator receives `tool_calls`. | `AgentEvent::TurnComplete` contains `tool_calls: Vec<String>`. | The spec's DB schema uses "JSON array of tool names," which is a lossy conversion from the event's `Vec<String>` (though acceptable for analysis). |
| `workspace/roles/` | (Old directory structure) | `actor.md` and `spectator.md` exist here. | **Migration:** The spec moves all spectator identity to `workspace/spectator/identity.md`. The implementation must ensure the old files are either migrated or correctly ignored. |

## 4. Unaddressed Technical Concerns

- **Transcript Token Budget:** The `{transcript}` substitution in `on-turn-complete.md` has no defined truncation logic. A turn involving a large file `read` or a complex `bash` output could easily exceed the model's context limit, causing the move generation to fail.
- **Identity.md Absence:** The spec says event handlers are skipped if their prompt files are missing, but it doesn't define a hardcoded fallback for `identity.md` (the system prompt). If this file is missing, every LLM call in the spectator will likely fail or require a default identity (e.g., "You are a spectator.").
- **Blocking Synchronous DB Calls:** While "lock-query-drop" solves the async threading issue, the underlying `rusqlite` operations are still synchronous and blocking. A slow DB query for a large message set or move list could block the gateway's event loop.

## 5. Grades

- **Completeness:** **A-** (The prompt-driven dispatcher is a comprehensive solution).
- **Consistency:** **A** (The dispatcher model simplifies the internal logic significantly).
- **Precision:** **B+** (The Rust module structure is clear, but `{transcript}` formatting and truncation are "hand-wavy").
- **Honesty:** **A** (Directly addresses the persistence ordering and mutex conflicts).

---

**Verdict: APPROVED.**

The "Prompt-Driven Runtime" is a superior architectural choice that aligns with the "River Engine" philosophy of making agent behavior user-auditable and editable. The remaining wiring gap in `AgentTask` is an implementation detail that must be addressed during the build phase.
