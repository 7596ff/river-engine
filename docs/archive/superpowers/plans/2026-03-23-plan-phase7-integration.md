# Phase 7: Integration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Everything working together. Multi-turn sessions with both agent and spectator running as peer tasks. Compression triggers firing, flashes appearing in context, room notes accumulating, git commits from both authors.

**Architecture:** Full coordinator deployment. Remove old loop fallback. End-to-end testing with real conversations.

**Tech Stack:** All previous phases

**Depends on:** Phase 6 (spectator task)

---

## File Structure

**Modified files:**
- `crates/river-gateway/src/server.rs` — coordinator-only startup
- `crates/river-gateway/src/main.rs` — new CLI flags for spectator config
- `crates/river-gateway/src/lib.rs` — deprecation warnings on old loop

**New files:**
- `tests/integration/iyou_test.rs` — end-to-end integration test

**Removed (deprecated):**
- `crates/river-gateway/src/loop/` — old monolithic loop (kept but deprecated)

---

## Task 1: Remove Old Loop Fallback

- [ ] **Step 1: Make coordinator the default**

In `server.rs`, remove the `if config.use_coordinator` branch. Coordinator is now the only path.

- [ ] **Step 2: Mark old loop as deprecated**

In `crates/river-gateway/src/loop/mod.rs`, add at top:
```rust
#![deprecated(note = "Use coordinator + agent task instead. See agent/task.rs")]
```

Or rename `loop/` to `loop_legacy/` to make it clear.

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p river-gateway
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor(gateway): coordinator is now the default, old loop deprecated"
```

---

## Task 2: Full Startup Sequence

- [ ] **Step 1: Update server.rs with complete coordinator startup**

```rust
pub async fn run(config: GatewayConfig) -> Result<()> {
    // 1. Open database
    let db = Arc::new(Mutex::new(Database::open(&config.db_path())?));

    // 2. Create shared resources
    let snowflake_gen = Arc::new(SnowflakeGenerator::new(config.agent_birth));
    let flash_queue = Arc::new(FlashQueue::new(20));
    let message_queue = Arc::new(MessageQueue::new());

    // 3. Create embeddings layer
    let embeddings_dir = config.workspace.join("embeddings");
    let vector_store = VectorStore::open(&config.data_dir.join("vectors.db")).ok().map(Arc::new);

    // 4. Create sync service and run initial sync
    if let Some(ref store) = vector_store {
        if let Some(ref embedding_client) = embedding_client {
            let sync = SyncService::new(embeddings_dir.clone(), (**store).clone(), embedding_client.clone());
            let stats = sync.full_sync().await.unwrap_or_default();
            tracing::info!(updated = stats.updated, skipped = stats.skipped, "Initial embedding sync");
        }
    }

    // 5. Build tool registry
    let registry = build_tool_registry(&config, &db, ...);

    // 6. Create coordinator
    let mut coordinator = Coordinator::new();

    // 7. Spawn agent task
    let agent_task = AgentTask::new(
        AgentTaskConfig { ... },
        coordinator.bus().clone(),
        message_queue.clone(),
        agent_model_client,
        tool_executor,
        flash_queue.clone(),
        snowflake_gen.clone(),
    );
    coordinator.spawn_task("agent", |_| agent_task.run());

    // 8. Spawn spectator task
    let spectator_task = SpectatorTask::new(
        SpectatorConfig { ... },
        coordinator.bus().clone(),
        spectator_model_client,
        vector_store,
        flash_queue.clone(),
    );
    coordinator.spawn_task("spectator", |_| spectator_task.run());

    // 9. Start HTTP server
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    axum::serve(listener, app).await?;

    // 10. Graceful shutdown
    coordinator.shutdown().await;

    Ok(())
}
```

- [ ] **Step 2: Update CLI flags**

Add to `main.rs`:
```rust
/// Spectator model URL (default: same as agent model)
#[arg(long)]
spectator_model_url: Option<String>,

/// Spectator model name (default: same as agent model)
#[arg(long)]
spectator_model_name: Option<String>,
```

- [ ] **Step 3: Verify compilation and startup**

```bash
cargo build -p river-gateway
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): complete coordinator startup sequence"
```

---

## Task 3: Git Authorship

- [ ] **Step 1: Configure separate git identities**

When agent commits:
```
Author: agent <agent@river-engine>
```

When spectator commits:
```
Author: spectator <spectator@river-engine>
```

Update `GitOps` to accept an author parameter:
```rust
impl GitOps {
    pub fn commit_as(&self, message: &str, author: &str) -> Result<GitCommitResult, String> {
        // git commit --author="agent <agent@river-engine>" -m "message"
    }
}
```

- [ ] **Step 2: Agent commits its notes**

After agent writes to embeddings/, commit with agent author.

- [ ] **Step 3: Spectator commits its outputs**

After spectator writes moves, moments, or room notes, commit with spectator author.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): separate git authorship for agent and spectator"
```

---

## Task 4: Compression Triggers

- [ ] **Step 1: Implement configurable compression triggers**

In spectator task, add trigger logic:

```rust
/// Should we run full compression this turn?
fn should_compress(&self, turn_number: u64, context_pressure: Option<f64>) -> bool {
    // Every N turns
    if turn_number % 10 == 0 { return true; }
    // On context pressure
    if let Some(pressure) = context_pressure {
        if pressure > 80.0 { return true; }
    }
    false
}
```

- [ ] **Step 2: Wire periodic compression**

When `should_compress()` returns true, spectator:
1. Counts moves for all channels
2. If any channel has >15 moves, creates a moment
3. Trims old moves (keep recent 3)

- [ ] **Step 3: Test compression trigger**

Simulate 20 turns, verify moment created around turn 15-20.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): add compression triggers for moves→moments"
```

---

## Task 5: End-to-End Testing

- [ ] **Step 1: Create integration test**

```rust
// tests/integration/iyou_test.rs

#[tokio::test]
async fn test_full_iyou_session() {
    // 1. Create coordinator with agent + spectator
    // 2. Send 5 messages
    // 3. Verify:
    //    - Agent responded to each
    //    - Spectator wrote moves for the channel
    //    - Flash queue received entries (if applicable)
    //    - Room notes file exists
    // 4. Shutdown gracefully
}
```

- [ ] **Step 2: Test channel switching**

```rust
#[tokio::test]
async fn test_channel_switching() {
    // Send messages to channel A, then channel B
    // Verify moves files exist for both channels
    // Verify flashes persist across channels
}
```

- [ ] **Step 3: Test context pressure → compression**

```rust
#[tokio::test]
async fn test_context_pressure_triggers_compression() {
    // Simulate high context usage
    // Verify spectator emits warning
    // Verify moment created if enough moves
}
```

- [ ] **Step 4: Manual session test**

Run the gateway with a real adapter. Have a 20+ turn conversation. Inspect:
- `workspace/embeddings/moves/` — moves files with structural summaries
- `workspace/embeddings/room-notes/` — session observations
- `workspace/embeddings/moments/` — compressed arcs (if triggered)
- Git log — commits from both agent and spectator

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "test(gateway): add I/You integration tests"
```

---

## Task 6: Qualitative Review

This task has no code — it's the human assessment.

- [ ] **Step 1: Review moves quality**

Read the moves files. Do they capture the structure of conversation? Can you understand the arc without reading the full transcript?

- [ ] **Step 2: Review room notes**

Read the spectator's observations. Are they useful? Terse? Third-person? Do they notice things the transcript alone doesn't reveal?

- [ ] **Step 3: Review flash relevance**

Were the surfaced memories relevant? Did they appear at the right time?

- [ ] **Step 4: Check spectator voice**

Does the spectator maintain "You" perspective? Is it critical in the philosophical sense? Does it shape context rather than speak?

- [ ] **Step 5: Document findings**

Write findings to `docs/superpowers/reviews/2026-XX-XX-iyou-review.md`.

---

## Success Criteria (from master spec)

### A. Functional ✓
- [ ] Sync service embeds files, vectors appear
- [ ] Context assembles from hot/warm/cold layers
- [ ] Spectator runs, events flow, flashes appear
- [ ] Moves and moments generate
- [ ] Git tracks with correct authorship
- [ ] 100+ turns without crash

### B. Behavioral ✓
- [ ] Mention topic → related notes surface
- [ ] Cross-session memory works
- [ ] Moves capture structure
- [ ] Flashes are timely
- [ ] Channel switching works

### C. Qualitative (the goal)
- [ ] Compression is honest
- [ ] Retrieval feels relevant
- [ ] Agent coherent over long sessions
- [ ] Spectator voice is right
- [ ] Room notes are useful witness testimony

---

## Summary

Phase 7 ties everything together:
1. **Remove old loop** — coordinator is the sole path
2. **Full startup** — coordinator spawns agent + spectator
3. **Git authorship** — separate authors for agent and spectator
4. **Compression triggers** — automatic moves→moments
5. **Integration tests** — end-to-end verification
6. **Qualitative review** — human judgment on the result

Total: 6 tasks, ~20 steps. This is where theory meets practice.
