# Phase 4: Coordinator + Event Bus

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the coordinator that manages peer tasks (agent + spectator) and an event bus for their communication. This is the infrastructure layer — the actual tasks come in Phases 5 and 6.

**Architecture:** The coordinator owns the event bus, spawns tasks as tokio tasks, handles lifecycle (start, stop, graceful shutdown). Events are typed enums sent via tokio broadcast channels.

**Tech Stack:** tokio (broadcast, mpsc), serde

**Depends on:** Phase 3 (context assembly available for agent task)

---

## File Structure

**New files:**
- `crates/river-gateway/src/coordinator/mod.rs` — Coordinator struct, run loop
- `crates/river-gateway/src/coordinator/events.rs` — Event types (Agent↔Spectator)
- `crates/river-gateway/src/coordinator/bus.rs` — Event bus with buffering

**Modified files:**
- `crates/river-gateway/src/lib.rs` — add coordinator module

---

## Task 1: Event Types

- [ ] **Step 1: Create coordinator/events.rs**

```rust
//! Events for coordinator communication between agent and spectator

use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

/// Events emitted by the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Agent is about to start a turn
    TurnStarted {
        channel: String,
        turn_number: u64,
        timestamp: DateTime<Utc>,
    },
    /// Agent completed a turn (includes transcript summary)
    TurnComplete {
        channel: String,
        turn_number: u64,
        transcript_summary: String,
        tool_calls: Vec<String>,  // tool names called
        timestamp: DateTime<Utc>,
    },
    /// Agent wrote a note to embeddings/
    NoteWritten {
        path: String,
        timestamp: DateTime<Utc>,
    },
    /// Agent switched channels
    ChannelSwitched {
        from: String,
        to: String,
        timestamp: DateTime<Utc>,
    },
    /// Context is getting full
    ContextPressure {
        usage_percent: f64,
        timestamp: DateTime<Utc>,
    },
}

/// Events emitted by the spectator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpectatorEvent {
    /// Memory surfaced into flash queue
    Flash {
        content: String,
        source: String,
        ttl_turns: u8,
        timestamp: DateTime<Utc>,
    },
    /// Pattern or observation noticed
    Observation {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Urgent signal (context pressure, drift, etc.)
    Warning {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Moves file updated for a channel
    MovesUpdated {
        channel: String,
        timestamp: DateTime<Utc>,
    },
    /// Arc compressed into a moment
    MomentCreated {
        summary: String,
        timestamp: DateTime<Utc>,
    },
}

/// All events on the bus
#[derive(Debug, Clone)]
pub enum CoordinatorEvent {
    Agent(AgentEvent),
    Spectator(SpectatorEvent),
    /// System-level events
    Shutdown,
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p river-gateway
```

- [ ] **Step 3: Commit**

```bash
git add crates/river-gateway/src/coordinator/
git commit -m "feat(gateway): add coordinator event types"
```

---

## Task 2: Event Bus

- [ ] **Step 1: Create coordinator/bus.rs**

```rust
//! Event bus for peer task communication

use super::events::CoordinatorEvent;
use tokio::sync::broadcast;
use std::sync::Arc;
use tokio::sync::RwLock;

const DEFAULT_CAPACITY: usize = 256;

/// Event bus for coordinator communication
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<CoordinatorEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_CAPACITY);
        Self { sender }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event to all subscribers
    pub fn publish(&self, event: CoordinatorEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.sender.send(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<CoordinatorEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::events::*;

    #[tokio::test]
    async fn test_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
            channel: "general".into(),
            turn_number: 1,
            timestamp: chrono::Utc::now(),
        }));

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, CoordinatorEvent::Agent(AgentEvent::TurnStarted { .. })));
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(CoordinatorEvent::Shutdown);

        assert!(matches!(rx1.recv().await.unwrap(), CoordinatorEvent::Shutdown));
        assert!(matches!(rx2.recv().await.unwrap(), CoordinatorEvent::Shutdown));
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(gateway): add event bus with broadcast channels"
```

---

## Task 3: Coordinator

- [ ] **Step 1: Create coordinator/mod.rs**

```rust
//! Coordinator — manages peer tasks and event routing

pub mod events;
pub mod bus;

pub use events::{AgentEvent, SpectatorEvent, CoordinatorEvent};
pub use bus::EventBus;

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Task handle with metadata
struct PeerTask {
    name: String,
    handle: JoinHandle<()>,
}

/// The coordinator manages agent and spectator as peer tasks
pub struct Coordinator {
    bus: EventBus,
    tasks: Vec<PeerTask>,
    shutdown: bool,
}

impl Coordinator {
    pub fn new() -> Self {
        Self {
            bus: EventBus::new(),
            tasks: Vec::new(),
            shutdown: false,
        }
    }

    /// Get a reference to the event bus
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Spawn a named task that receives events
    pub fn spawn_task<F, Fut>(&mut self, name: impl Into<String>, f: F)
    where
        F: FnOnce(EventBus) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let name = name.into();
        let bus = self.bus.clone();
        let handle = tokio::spawn(async move {
            f(bus).await;
        });
        self.tasks.push(PeerTask { name, handle });
    }

    /// Graceful shutdown: send shutdown event, wait for tasks
    pub async fn shutdown(&mut self) {
        if self.shutdown { return; }
        self.shutdown = true;

        tracing::info!("Coordinator: sending shutdown signal");
        self.bus.publish(CoordinatorEvent::Shutdown);

        for task in self.tasks.drain(..) {
            tracing::info!(task = %task.name, "Waiting for task to finish");
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                task.handle
            ).await {
                Ok(Ok(())) => tracing::info!(task = %task.name, "Task finished"),
                Ok(Err(e)) => tracing::error!(task = %task.name, error = %e, "Task panicked"),
                Err(_) => tracing::warn!(task = %task.name, "Task timed out, aborting"),
            }
        }
    }

    /// Number of active tasks
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
}

impl Default for Coordinator {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Add to lib.rs**

```rust
pub mod coordinator;
```

- [ ] **Step 3: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_coordinator_lifecycle() {
        let mut coord = Coordinator::new();

        coord.spawn_task("test-task", |bus| async move {
            let mut rx = bus.subscribe();
            loop {
                match rx.recv().await {
                    Ok(CoordinatorEvent::Shutdown) => break,
                    _ => {}
                }
            }
        });

        assert_eq!(coord.task_count(), 1);
        coord.shutdown().await;
    }

    #[tokio::test]
    async fn test_event_flow_between_tasks() {
        let mut coord = Coordinator::new();
        let (tx, mut result_rx) = tokio::sync::mpsc::channel(1);

        // Task A: publishes an agent event
        coord.spawn_task("agent", |bus| async move {
            bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
                channel: "test".into(),
                turn_number: 1,
                timestamp: chrono::Utc::now(),
            }));
        });

        // Task B: receives agent event
        coord.spawn_task("spectator", {
            let bus = coord.bus().clone();
            move |_| async move {
                let mut rx = bus.subscribe();
                if let Ok(CoordinatorEvent::Agent(AgentEvent::TurnStarted { turn_number, .. })) = rx.recv().await {
                    tx.send(turn_number).await.ok();
                }
            }
        });

        // Wait for result
        let turn = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            result_rx.recv()
        ).await;

        coord.shutdown().await;

        // The event should have flowed from agent to spectator
        assert!(turn.is_ok());
    }
}
```

- [ ] **Step 4: Verify, commit**

```bash
cargo test -p river-gateway coordinator
git add -A && git commit -m "feat(gateway): add coordinator with peer task management"
```

---

## Summary

Phase 4 builds the coordination infrastructure:
1. **Event types** — Agent events, Spectator events, system events
2. **Event bus** — tokio broadcast channel with publish/subscribe
3. **Coordinator** — spawns named tasks, graceful shutdown

Total: 3 tasks, ~15 steps. No agent or spectator logic yet — just the plumbing.
