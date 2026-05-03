# Review: Context Assembly Rework

**Date:** 2026-04-30
**Reviewer:** Gemini CLI
**Status:** Complete

---

## Findings

### 1. Lossless Guarantee & Failure Modes
- **Problem:** The spec claims no uncompressed message is ever dropped, but the **Session Start** procedure violates this. It specifies loading the "last 20 messages from DB." If the spectator is behind and there are, for example, 50 messages above the cursor (`turn_number > cursor`), loading only 20 at session start will lose 30 uncompressed messages forever.
- **Problem:** If the spectator has never run or the DB is empty, `MAX(turn_number)` returns `NULL`. The spec doesn't define the fallback. If treated as 0, no messages are dropped, which is safe but leads to the next issue.
- **Problem:** If the spectator crashes or lags significantly, uncompressed messages accumulate indefinitely. If they exceed the 80% threshold, compaction will fire but drop nothing (as all messages are > cursor). This leads to an infinite compaction loop or the agent operating at near-limit context until the LLM returns an error.
- **Spec Location:** Section 3 (Guarantees, Compaction Procedure), Section 7 (What Changes).
- **Question:** How does the system handle the case where `uncompressed_messages + system_prompt > 80%`? Is there a backpressure mechanism to wait for the spectator? Should Session Start load *all* messages where `turn_number > cursor`?

### 2. 40% Fill Target & Math Gaps
- **Problem:** The 40% target is a "fill up to" goal. If `system prompt + uncompressed messages` already exceed 40%, the space for moves is zero. If they exceed 80%, the system hits "Context Pressure" immediately after compaction.
- **Problem:** "Load moves from DB... trim oldest moves if needed." If moves are loaded first and then trimmed, it's inefficient for long-running sessions with thousands of moves.
- **Spec Location:** Section 3 (Compaction procedure, step 6), Section 6 (Configuration).
- **Question:** What is the priority if 40% is already exceeded? Do we drop moves entirely? What is the behavior when the context exceeds 80% post-compaction?

### 3. Message `turn_number` Tracking
- **Problem:** The current `ChatMessage` struct (in `crates/river-gateway/src/model/types.rs`) does not contain a `turn_number` field. The compaction logic "Drop messages where turn_number <= cursor" is impossible to implement without modifying this core struct or wrapping it.
- **Problem:** While `river_db::Message` has a `turn_number`, the in-memory `AssembledContext` and the `conversation` vector in `AgentTask` do not currently store it.
- **Spec Location:** Section 3 (Compaction procedure, step 3 & 4), Section 5 (Per-Turn Cycle).
- **Question:** Will `ChatMessage` be modified to include `turn_number`, or will a new wrapper type be introduced to track metadata for the persistent context object?

### 4. Session Start vs. Compaction Procedure
- **Problem:** The spec claims they are the "same procedure," but describes different message loading logic. Compaction filters an existing in-memory vector; Session Start loads a fixed number (20) from the DB.
- **Spec Location:** Section 3 (Session start).
- **Question:** Should Session Start instead be defined as: `Load all messages where turn_number > cursor` + `Load last N messages (even if below cursor) to hit min_messages floor`?

### 5. Persistent Context Object Structure
- **Problem:** Compaction procedure step 3/4 implies filtering or splicing the `Vec<ChatMessage>`. The spec doesn't address the atomicity of turns. 
- **Problem:** Tool call/result pairs: If a compaction fires mid-turn (unlikely but possible if cursor updates asynchronously), a tool call could be dropped while its result is kept if they aren't grouped.
- **Spec Location:** Section 3 (Compaction procedure).
- **Question:** Does the `persistent context object` handle "turn-based" drops to ensure tool call/result pairs are never separated during compaction?

### 6. Spectator Coordination & Lag
- **Problem:** "Compaction effectiveness depends on spectator keeping up." If the agent turn rate (e.g., 10s) exceeds the spectator compression rate (e.g., 30s), the cursor will never move fast enough to clear the 80% -> 40% gap.
- **Spec Location:** Section 3 (Guarantees).
- **Question:** At what point does the system "degrade" from lossless to "blocked"? Should the agent pause if it cannot compact below a certain safety threshold (e.g., 90%)?

### 7. Moves Loading Efficiency
- **Problem:** "Load moves from DB" — Section 8 says "all moves for current channel." For an agent with 1000+ turns, this could be hundreds of KB of text loaded only to be immediately "trimmed" to fit the 40% target.
- **Spec Location:** Section 3 (Compaction procedure, step 6).
- **Question:** Should the DB query for moves be capped by a token-aware limit or a recent-count limit *before* loading into memory?
