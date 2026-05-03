# Review Prompt: Channel Messages Implementation Plan

Review the implementation plan at `docs/superpowers/plans/2026-05-03-channel-messages.md` against the spec at `docs/superpowers/specs/2026-05-03-channel-message-design.md`.

## Context

This is a Rust workspace (`river-engine`) with multiple crates. The implementation plan has 8 tasks that replace a broken message delivery system with JSONL channel logs. The plan was written by an AI agent (Claude) that traced the existing codebase and designed the replacement with a human collaborator.

Key crate: `river-gateway` — contains the HTTP server, agent task, spectator task, tools, and the broken inbox system being replaced.

## Review Criteria

1. **Spec fidelity** — does every requirement in the spec have a corresponding task? Is anything in the plan that contradicts the spec?

2. **Task ordering** — are dependencies between tasks respected? Will any task fail because it depends on code from a later task? Can any tasks be reordered to fail faster?

3. **Compilation continuity** — after each task's commit, will `cargo check` pass? Or are there intermediate states where the codebase is broken? Note: some breakage is acceptable if explicitly acknowledged, but surprise breakage is a problem.

4. **Test coverage** — do the tests cover the key behaviors specified in the spec? Are there important paths not tested (error paths, edge cases, concurrency)?

5. **Code quality** — is the Rust idiomatic? Are there unnecessary allocations, unwraps that should be handled, or missing error propagation?

6. **Missing pieces** — the plan modifies `handle_incoming`, `turn_cycle`, `SendMessageTool`, `SpeakTool`, and removes `inbox`/`conversations`. Are there other files in the codebase that import or depend on the removed modules? The plan should account for all transitive breakage.

7. **Task 5 risk** — the agent turn cycle rewrite is the most complex change. Is the code complete enough for an implementer to work from, or are there gaps they'd need to fill?

8. **Task 6 risk** — the plan acknowledges uncertainty about the `SendMessageTool` structure. Is this acceptable, or should the plan include the actual current code and the exact diff?

## What to produce

A structured review with findings organized by severity (critical, important, suggestion). Flag any tasks that would leave the codebase in a broken state. Note any spec requirements without coverage.
