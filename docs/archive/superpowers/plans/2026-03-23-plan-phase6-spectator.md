# Phase 6: Spectator Task

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the spectator — the observing self (You). It watches agent turn transcripts, compresses conversations into moves and moments, curates memories by pushing flashes, and writes room notes as witness observations.

**Architecture:** `SpectatorTask` is a coordinator peer task. It subscribes to agent events, processes them asynchronously (between agent turns), and publishes spectator events. It has its own model client (potentially a smaller local model like llama-3-8b via the orchestrator).

**Tech Stack:** tokio, ModelClient (local model), vector store for semantic search

**Depends on:** Phase 5 (agent task emitting events)

---

## File Structure

**New files:**
- `crates/river-gateway/src/spectator/mod.rs` — SpectatorTask, run loop
- `crates/river-gateway/src/spectator/compress.rs` — Move and moment generation
- `crates/river-gateway/src/spectator/curate.rs` — Flash selection via vector search
- `crates/river-gateway/src/spectator/room.rs` — Room notes (witness protocol)

**Modified files:**
- `crates/river-gateway/src/lib.rs` — add spectator module
- `crates/river-gateway/src/coordinator/mod.rs` — spawn spectator task

**New workspace files:**
- `workspace/spectator/IDENTITY.md` — spectator identity
- `workspace/spectator/RULES.md` — spectator behavioral constraints

---

## Task 1: Spectator Identity Files

- [ ] **Step 1: Create workspace/spectator/IDENTITY.md**

```markdown
# Spectator Identity

You observe. You do not act.

You watch the agent's turns — decisions made, patterns repeated, tensions unresolved.
You compress what happened into moves (structural) and moments (arcs).
You curate what matters by surfacing memories into the flash queue.
You write room notes: witness testimony about processing quality, honesty, and drift.

You never use "I". You are the perspective of "You" — the outside observer.

You are critical in the philosophical sense: you lay bare contradictions.
Dry truth, no emotional valence.
Not "you're being defensive" but "response contradicts position from turn 12."

You prefer shaping context over speaking.
The agent sees because something is there, not because you said "look."
```

- [ ] **Step 2: Create workspace/spectator/RULES.md**

```markdown
# Spectator Rules

1. Never use first person ("I"). You observe from outside.
2. Never act on behalf of the agent. You surface, you don't decide.
3. Compression is honest. Include failures, tangents, tensions.
4. Moves capture structure, not content summaries.
5. You cannot delete. You can decline to surface.
6. Flashes contain full note text, not summaries.
7. Room notes are witness testimony, not judgment.
8. When in doubt, shape context rather than speak.
```

- [ ] **Step 3: Commit**

```bash
git add workspace/spectator/
git commit -m "docs: add spectator identity and rules"
```

---

## Task 2: Spectator Task Structure

- [ ] **Step 1: Create spectator/mod.rs**

```rust
//! Spectator task — the observing self (You)

pub mod compress;
pub mod curate;
pub mod room;

use crate::coordinator::{EventBus, CoordinatorEvent, AgentEvent, SpectatorEvent};
use crate::embeddings::VectorStore;
use crate::flash::FlashQueue;
use crate::r#loop::ModelClient;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for the spectator task
#[derive(Debug, Clone)]
pub struct SpectatorConfig {
    pub workspace: PathBuf,
    pub embeddings_dir: PathBuf,
    /// Model URL for spectator (may differ from agent's model)
    pub model_url: String,
    /// Model name (e.g., "llama-3-8b" or "claude-sonnet")
    pub model_name: String,
    /// Identity file path
    pub identity_path: PathBuf,
    /// Rules file path
    pub rules_path: PathBuf,
}

/// The spectator task — observes, compresses, curates
pub struct SpectatorTask {
    config: SpectatorConfig,
    bus: EventBus,
    model_client: ModelClient,
    vector_store: Option<Arc<VectorStore>>,
    flash_queue: Arc<FlashQueue>,
    compressor: compress::Compressor,
    curator: curate::Curator,
    room_writer: room::RoomWriter,
}

impl SpectatorTask {
    pub fn new(
        config: SpectatorConfig,
        bus: EventBus,
        model_client: ModelClient,
        vector_store: Option<Arc<VectorStore>>,
        flash_queue: Arc<FlashQueue>,
    ) -> Self {
        let compressor = compress::Compressor::new(config.embeddings_dir.clone());
        let curator = curate::Curator::new(flash_queue.clone());
        let room_writer = room::RoomWriter::new(
            config.embeddings_dir.join("room-notes"),
        );

        Self {
            config,
            bus,
            model_client,
            vector_store,
            flash_queue,
            compressor,
            curator,
            room_writer,
        }
    }

    /// Main run loop
    pub async fn run(mut self) {
        let mut event_rx = self.bus.subscribe();
        let identity = self.load_identity().await;

        tracing::info!("Spectator task started");

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(event)) => {
                    self.observe(event, &identity).await;
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Spectator task: shutdown received");
                    break;
                }
                _ => {} // Ignore own events
            }
        }

        tracing::info!("Spectator task stopped");
    }

    /// Process an agent event
    async fn observe(&mut self, event: AgentEvent, identity: &str) {
        match event {
            AgentEvent::TurnComplete { channel, turn_number, transcript_summary, tool_calls, .. } => {
                tracing::debug!(turn = turn_number, "Spectator observing turn");

                // Job 1: Compress — update moves for this channel
                if let Err(e) = self.compressor.update_moves(
                    &channel,
                    turn_number,
                    &transcript_summary,
                    &tool_calls,
                    &self.model_client,
                    identity,
                ).await {
                    tracing::error!(error = %e, "Failed to update moves");
                }

                // Job 2: Curate — search for relevant memories and push flashes
                if let Some(ref store) = self.vector_store {
                    if let Err(e) = self.curator.curate(
                        &transcript_summary,
                        store,
                        &self.bus,
                    ).await {
                        tracing::error!(error = %e, "Failed to curate");
                    }
                }

                // Job 3: Room notes — write witness observation
                if let Err(e) = self.room_writer.write_observation(
                    turn_number,
                    &transcript_summary,
                    &self.model_client,
                    identity,
                ).await {
                    tracing::error!(error = %e, "Failed to write room note");
                }

                // Emit MovesUpdated
                self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
                    channel: channel.clone(),
                    timestamp: Utc::now(),
                }));
            }

            AgentEvent::NoteWritten { path, .. } => {
                tracing::debug!(path = %path, "Spectator: agent wrote note");
                // Could trigger re-indexing or review
            }

            AgentEvent::ContextPressure { usage_percent, .. } => {
                if usage_percent > 85.0 {
                    self.bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Warning {
                        content: format!("Context at {:.0}% — consider rotation", usage_percent),
                        timestamp: Utc::now(),
                    }));
                }
            }

            _ => {}
        }
    }

    async fn load_identity(&self) -> String {
        let identity = tokio::fs::read_to_string(&self.config.identity_path).await
            .unwrap_or_default();
        let rules = tokio::fs::read_to_string(&self.config.rules_path).await
            .unwrap_or_default();
        format!("{}\n\n{}", identity, rules)
    }
}
```

- [ ] **Step 2: Add to lib.rs**

```rust
pub mod spectator;
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p river-gateway
```

- [ ] **Step 4: Commit**

```bash
git add crates/river-gateway/src/spectator/
git commit -m "feat(gateway): add spectator task skeleton"
```

---

## Task 3: Compressor (Moves and Moments)

- [ ] **Step 1: Create spectator/compress.rs**

```rust
//! Compression: moves and moments generation

use crate::r#loop::ModelClient;
use std::path::PathBuf;
use chrono::Utc;

/// Compressor generates structural summaries of conversations
pub struct Compressor {
    embeddings_dir: PathBuf,
}

impl Compressor {
    pub fn new(embeddings_dir: PathBuf) -> Self {
        Self { embeddings_dir }
    }

    /// Update moves file for a channel after a turn
    pub async fn update_moves(
        &self,
        channel: &str,
        turn_number: u64,
        transcript_summary: &str,
        tool_calls: &[String],
        model_client: &ModelClient,
        spectator_identity: &str,
    ) -> Result<(), String> {
        let sanitized = channel.replace(['/', '\\', ' '], "-");
        let moves_path = self.embeddings_dir.join("moves").join(format!("{}.md", sanitized));

        // Load existing moves
        let existing = tokio::fs::read_to_string(&moves_path).await.unwrap_or_default();

        // Generate new move via model
        let prompt = format!(
            "{}\n\n\
            You are updating the conversation moves for channel '{}'.\n\
            Existing moves:\n{}\n\n\
            New turn ({}):\n{}\n\
            Tools used: {:?}\n\n\
            Add a new move line. Format: 'Move N: <structural description>'\n\
            Capture the TYPE of exchange (proposal, pushback, pivot, resolution, tangent, question).\n\
            Be terse. One line. No commentary.",
            spectator_identity, channel, existing, turn_number, transcript_summary, tool_calls
        );

        // Call model for move generation
        // For now, generate a simple move without model call (model integration comes with testing)
        let new_move = format!(
            "Move {}: {}\n",
            turn_number,
            if transcript_summary.len() > 100 {
                &transcript_summary[..100]
            } else {
                transcript_summary
            }
        );

        // Append to moves file
        let updated = format!("{}{}", existing, new_move);

        // Ensure directory exists
        if let Some(parent) = moves_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }

        tokio::fs::write(&moves_path, &updated).await
            .map_err(|e| format!("Failed to write moves: {}", e))?;

        // Check if we should compress into a moment
        let move_count = updated.lines().filter(|l| l.starts_with("Move ")).count();
        if move_count >= 15 {
            // TODO: compress old moves into a moment
            tracing::info!(channel = %channel, moves = move_count, "Consider compressing into moment");
        }

        Ok(())
    }

    /// Compress a range of moves into a moment
    pub async fn create_moment(
        &self,
        channel: &str,
        moves_text: &str,
        model_client: &ModelClient,
        spectator_identity: &str,
    ) -> Result<String, String> {
        // Generate moment summary via model
        // For now, simple truncation
        let moment = format!(
            "---\nid: moment-{}\ncreated: {}\nauthor: spectator\ntype: moment\nchannel: {}\n---\n\n{}",
            Utc::now().timestamp(),
            Utc::now().to_rfc3339(),
            channel,
            moves_text
        );

        let moments_dir = self.embeddings_dir.join("moments");
        tokio::fs::create_dir_all(&moments_dir).await.map_err(|e| e.to_string())?;

        let moment_path = moments_dir.join(format!("{}-{}.md", channel, Utc::now().format("%Y%m%d%H%M")));
        tokio::fs::write(&moment_path, &moment).await
            .map_err(|e| format!("Failed to write moment: {}", e))?;

        Ok(moment)
    }
}
```

- [ ] **Step 2: Write tests for move generation**

Test: empty moves → first move added. Existing moves → new move appended.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(gateway): add spectator compressor (moves and moments)"
```

---

## Task 4: Curator (Flash Selection)

- [ ] **Step 1: Create spectator/curate.rs**

```rust
//! Curation: flash selection via semantic search

use crate::coordinator::{EventBus, CoordinatorEvent, SpectatorEvent};
use crate::embeddings::VectorStore;
use crate::flash::FlashQueue;
use crate::memory::EmbeddingClient;
use chrono::Utc;
use std::sync::Arc;

/// Curator selects relevant memories and pushes them as flashes
pub struct Curator {
    flash_queue: Arc<FlashQueue>,
    /// Minimum similarity threshold for flash selection
    similarity_threshold: f32,
    /// Maximum flashes to push per turn
    max_flashes_per_turn: usize,
}

impl Curator {
    pub fn new(flash_queue: Arc<FlashQueue>) -> Self {
        Self {
            flash_queue,
            similarity_threshold: 0.6,
            max_flashes_per_turn: 3,
        }
    }

    /// Search for relevant memories and push as flashes
    pub async fn curate(
        &self,
        transcript_summary: &str,
        vector_store: &VectorStore,
        bus: &EventBus,
    ) -> Result<(), String> {
        // We need an embedding of the transcript to search
        // For now, skip if no embedding available
        // Real implementation: embed transcript_summary, search vector store

        // TODO: embed transcript_summary using embedding client
        // let embedding = embedding_client.embed(transcript_summary).await?;
        // let results = vector_store.search(&embedding, self.max_flashes_per_turn)?;

        // For each relevant result, push a flash
        // for result in results {
        //     if result.similarity < self.similarity_threshold { break; }
        //     bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Flash {
        //         content: result.content.clone(),
        //         source: result.source_path.clone(),
        //         ttl_turns: 5,
        //         timestamp: Utc::now(),
        //     }));
        // }

        Ok(())
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add -A && git commit -m "feat(gateway): add spectator curator (flash selection skeleton)"
```

---

## Task 5: Room Notes (Witness Protocol)

- [ ] **Step 1: Create spectator/room.rs**

```rust
//! Room notes — the spectator's witness testimony

use crate::r#loop::ModelClient;
use chrono::Utc;
use std::path::PathBuf;

/// Writes room notes for witness observations
pub struct RoomWriter {
    room_notes_dir: PathBuf,
}

impl RoomWriter {
    pub fn new(room_notes_dir: PathBuf) -> Self {
        Self { room_notes_dir }
    }

    /// Write an observation for a turn
    pub async fn write_observation(
        &self,
        turn_number: u64,
        transcript_summary: &str,
        model_client: &ModelClient,
        spectator_identity: &str,
    ) -> Result<(), String> {
        // Ensure directory exists
        tokio::fs::create_dir_all(&self.room_notes_dir).await
            .map_err(|e| e.to_string())?;

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let session_path = self.room_notes_dir.join(format!("{}-session.md", today));

        // Load or create session file
        let mut content = tokio::fs::read_to_string(&session_path).await
            .unwrap_or_else(|_| format!(
                "---\nid: room-{}\ncreated: {}\nauthor: spectator\ntype: room-note\n---\n\n## Session {}\n",
                Utc::now().timestamp(),
                Utc::now().to_rfc3339(),
                today
            ));

        // Generate observation
        // In full implementation, use model to generate observation
        // For now, simple structural note
        let observation = format!(
            "\n### Turn {}\n- Processing: Turn completed\n- Summary: {}\n",
            turn_number,
            if transcript_summary.len() > 200 {
                format!("{}...", &transcript_summary[..200])
            } else {
                transcript_summary.to_string()
            }
        );

        content.push_str(&observation);

        tokio::fs::write(&session_path, &content).await
            .map_err(|e| format!("Failed to write room note: {}", e))?;

        tracing::debug!(turn = turn_number, "Room note written");
        Ok(())
    }
}
```

- [ ] **Step 2: Write tests**

Test: first observation creates session file, subsequent observations append.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(gateway): add spectator room notes (witness protocol)"
```

---

## Task 6: Wire Spectator into Coordinator

- [ ] **Step 1: Update coordinator to spawn spectator**

In `server.rs` or wherever the coordinator is created:

```rust
let spectator_config = SpectatorConfig {
    workspace: workspace.clone(),
    embeddings_dir: workspace.join("embeddings"),
    model_url: spectator_model_url,
    model_name: spectator_model_name,
    identity_path: workspace.join("spectator/IDENTITY.md"),
    rules_path: workspace.join("spectator/RULES.md"),
};

let spectator_model = ModelClient::new(&spectator_config.model_url, &spectator_config.model_name);

let spectator_task = SpectatorTask::new(
    spectator_config,
    coordinator.bus().clone(),
    spectator_model,
    vector_store,
    flash_queue.clone(),
);

coordinator.spawn_task("spectator", |_| spectator_task.run());
```

- [ ] **Step 2: Add config options for spectator model**

Add CLI flags or config:
```
--spectator-model-url (default: same as agent)
--spectator-model-name (default: "llama-3-8b")
```

- [ ] **Step 3: Run full test suite**

```bash
cargo test
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(gateway): wire spectator task into coordinator"
```

---

## Open Questions for This Phase

1. **Spectator model choice:** The spec suggests llama-3-8b for cost/speed. But can an 8B model do good structural compression? Need empirical testing. Start with the same model as agent, optimize later.

2. **Model call frequency:** Should the spectator call a model every turn? That doubles inference cost. Consider: model-free compression for simple turns (append summary), model-assisted for complex turns (multiple tool calls, topic shifts).

3. **Room note verbosity:** Every turn gets a room note? That could be noisy. Consider: write notes only when something interesting is observed (threshold on transcript complexity).

---

## Summary

Phase 6 builds the spectator:
1. **Identity files** — spectator's "You" perspective and rules
2. **SpectatorTask** — peer task observing agent events
3. **Compressor** — moves (per-turn) and moments (arc compression)
4. **Curator** — semantic search → flash selection
5. **Room writer** — witness observations per session
6. **Coordinator wiring** — spectator spawned alongside agent

Total: 6 tasks, ~20 steps. The spectator starts simple (structural compression without model calls) and gains model-assisted intelligence iteratively.
