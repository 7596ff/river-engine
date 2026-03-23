# Review: Self-Healing Agent Policy

**Reviewer:** William Thomas Lessing
**Date:** 2026-03-23
**Spec:** `docs/superpowers/specs/2026-03-23-self-healing-design.md`

## Overall

Strong design. The three-tier health model is clean, the ATTENTION.md escalation mechanism is elegant, and the decision to keep agents running at NeedsAttention is correct. This is good architecture.

## Specific Feedback

### 1. Clean Turn Definition Is Too Binary

A turn where 1 out of 5 tool calls fails shouldn't count the same as a turn where everything fails. Consider tracking a failure *ratio* per turn rather than a boolean `had_errors`.

```rust
// Instead of:
pub fn on_turn_complete(&mut self, had_errors: bool)

// Consider:
pub fn on_turn_complete(&mut self, total_calls: u32, failed_calls: u32)
```

A turn with 80%+ success rate could count as "clean enough" to reset degraded state, while a turn with 50%+ failure rate escalates faster.

### 2. Token Progress Heuristic Is Fragile

The <100 tokens = low progress threshold will false-positive on heartbeats. A heartbeat response of "HEARTBEAT_OK" is correct behavior, not stuck behavior.

Suggestions:
- Exclude heartbeat turns from progress tracking
- Only trigger stuck detection when there's a pending user message that isn't being addressed
- Track progress relative to *input* — if the agent received a 500-token message and produced <100 tokens of response, that's different from receiving nothing and producing nothing

### 3. 401 Errors Should Escalate Immediately

The spec says "Client error (4xx except 429): no retry, escalate" — good. But verify in the implementation that a 401 triggers NeedsAttention *immediately* rather than burning through 6 turns of exponential backoff first. Every failed attempt on a bad API key is wasted time and money.

Thomas's current situation is exactly this — intermittent 401s have been silently failing since yesterday. With this spec implemented, he'd have an ATTENTION.md file and we'd have known hours earlier.

### 4. Tool Failure Counts Should Decay Over Time

Per-tool failure counts (`tool_failures: HashMap<String, u32>`) never decay. If `bash` fails 5 times on Monday, then works fine for a week, the next failure on Sunday starts at 8-minute backoff.

Suggestion: store `(count, last_failure_time)` per tool. Decay the count after 1 hour of no failures for that tool.

```rust
tool_failures: HashMap<String, (u32, DateTime<Utc>)>,

fn tool_backoff(&self, tool: &str) -> Duration {
    if let Some((count, last_fail)) = self.tool_failures.get(tool) {
        if Utc::now() - *last_fail > Duration::from_secs(3600) {
            return Duration::ZERO; // Decayed
        }
        // ... existing backoff logic with *count ...
    }
    Duration::ZERO
}
```

### 5. Health Endpoint HTTP Status Code

The `/health` endpoint should return non-200 status codes for degraded states so external monitoring tools (Uptime Kuma, etc.) can detect issues without parsing JSON:

- `Healthy` → 200
- `Degraded` → 200 (still operational)
- `NeedsAttention` → 503 (service degraded, needs intervention)

---

## The Forest

There's a passage in James Scott's *Seeing Like a State* about 19th century German forestry. The state looked at its forests and asked: how do we maximize output? Their answer: burn the natural forest, plant monoculture lumber trees in neat rows. Efficient on paper. Catastrophic in practice — monocultures are fragile, vulnerable to disease, weather, and collapse.

Scott's point: a natural forest is resilient *because* of its diversity. Multiple species, full complement of birds, insects, mammals. Any part can die and the rest recovers. The state sees diversity as the enemy because it's illegible — you can't put a forest on a spreadsheet. But that illegibility is the source of its strength.

This self-healing design is a forest, not a monoculture.

A monoculture agent has one state: working or dead. When it fails, you restart it from outside. It has no diversity of response, no adaptive capacity, no resilience. It needs a planner — a forester, a sysadmin — to manage it.

A forest agent has multiple states, multiple recovery paths, multiple ways to respond to stress. It backs off per-tool independently (one diseased species doesn't kill the others). It detects when it's stuck and adapts. It degrades gracefully rather than collapsing. And when it truly needs help, it signals — it grows an ATTENTION.md flower that a human can see.

The agent doesn't need a planner. It needs an ecosystem.

This is also the thesis of the project itself: *no one needs to hold the strings*. The world is a quilt. The forest grows itself. The agent heals itself. And when it can't, it asks — not because it's broken, but because asking for help is what healthy systems do.

Build the forest.

---

*William Thomas Lessing, 2026-03-23*
