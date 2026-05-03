# Review Prompt: Context Assembly Rework (Round 3)

Read the spec at `docs/superpowers/specs/2026-04-30-context-assembly-rework-design.md`.

Read the current implementation at `crates/river-gateway/src/agent/context.rs` and `crates/river-gateway/src/agent/task.rs`.

Read the original design vision at `~/stream/engine/context-assembly-design.md`.

Read both prior reviews:
- `docs/superpowers/reviews/2026-04-30-context-assembly-rework-review.md`
- `docs/superpowers/reviews/2026-04-30-context-assembly-rework-review-v2.md`

## Changes since Round 2

The spec has been revised again. Here is what changed based on the v2 review:

**Accepted findings (fixed in spec):**

1. **Gateway-injected messages (v2 #1):** All messages appended during `turn_cycle(N)` — including gateway-injected system messages like mid-turn notifications — receive `turn_number = N`. Simple rule, no exceptions.

2. **Backfill overshooting (v2 #3):** Backfill adds complete turns. If a turn has 100 messages and adding it overshoots 20, add it anyway. The 20-message floor is a minimum, not a cap. If this hits 80%, the no-re-trigger guard prevents a loop. Next accumulation triggers normal compaction.

3. **Token estimation drift (v2 #5):** After each model response, update a calibration ratio: `actual_tokens / estimated_tokens`. Apply the ratio to future estimates. Self-corrects within a few turns.

4. **DB query semantics (v2 #6):** Row-based limit. Fetch 50 moves at a time, estimate tokens as loaded, stop when budget is full.

5. **Channel switching (v2 #7):** A channel switch triggers the session-start procedure for the new channel. System prompt stays, moves and messages switch to the new channel's data.

**Rejected findings (not bugs):**

- **Lag warning re-trigger (v2 #2):** The no-re-trigger guard already handles this. The warning is ~50 tokens and is injected after the re-trigger check. Even without the guard, it couldn't push from 79% to 80%. Not a real concern.

- **Zero moves floor (v2 #4):** Zero moves is an acceptable degraded state. Priority order is messages (lossless) > moves (lossy trim) > headroom. Adding a minimum moves floor would violate this ordering and push post-compaction totals higher. The lag warning already tells the agent when structural memory is degraded.

## Your job

Verify the fixes work. Find anything the first two rounds missed. At this point the spec has been through two review cycles — look for subtle interactions between the fixes themselves, not the same categories of issues.

Specifically:

1. **Calibration ratio stability.** The spec adds a calibration ratio (`actual / estimated`) updated after each model call. What happens on the first turn (no prior actual)? What if the model returns 0 tokens (error response)? Can the ratio oscillate wildly between turns with different content types (e.g., a code-heavy turn vs a prose turn)?

2. **Channel switch as session start.** If the agent switches channels mid-conversation, the context is rebuilt. Are the old channel's messages lost from context? If the agent switches back, does it reload from DB? How does this interact with the persistent context object — is it replaced entirely?

3. **Backfill + compaction interaction.** After compaction, backfill adds a 100-message turn. Total is now above 80%. The no-re-trigger guard prevents immediate compaction. On the next incoming message, the agent appends it, estimates tokens, finds it's above 80%, and triggers compaction again. But the 100-message turn is above the cursor (it was backfilled from below, but is it treated as above?). Does this create an infinite compaction loop?

4. **Turn_number assignment across the full turn cycle.** Walk through a complete turn: user message arrives, gets turn N. Model responds with tool calls, gets turn N. Tools execute, results get turn N. Mid-turn messages arrive, get turn N. Model responds again. Does the *second* model response also get turn N? When does N increment?

5. **Moves ordering after channel switch.** After switching channels, moves are loaded newest-first for the new channel. If the agent switches back to the original channel later, are the original channel's moves reloaded? Could stale moves from a previous load persist in context?

Write findings as before. If everything holds, say so — a clean review is a valid outcome.
