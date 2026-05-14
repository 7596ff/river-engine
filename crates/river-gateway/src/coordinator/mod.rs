//! Coordinator — manages peer tasks and event routing

pub mod bus;
pub mod events;

pub use bus::EventBus;
pub use events::{AgentEvent, CoordinatorEvent, SpectatorEvent};

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
        if self.shutdown {
            return;
        }
        self.shutdown = true;

        tracing::info!("Coordinator: sending shutdown signal");
        self.bus.publish(CoordinatorEvent::Shutdown);

        for task in self.tasks.drain(..) {
            tracing::info!(task = %task.name, "Waiting for task to finish");
            match tokio::time::timeout(std::time::Duration::from_secs(10), task.handle).await {
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

    /// Check if a named task is running (was spawned and not finished)
    pub fn is_running(&self, name: &str) -> bool {
        self.tasks
            .iter()
            .any(|t| t.name == name && !t.handle.is_finished())
    }
}

impl Default for Coordinator {
    fn default() -> Self {
        Self::new()
    }
}

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
        let (tx, mut result_rx) = tokio::sync::mpsc::channel::<u64>(1);

        // Subscribe before spawning tasks
        let bus_clone = coord.bus().clone();

        // Task A: publishes an agent event
        coord.spawn_task("agent", |bus| async move {
            // Small delay to ensure subscriber is ready
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
                channel: "test".into(),
                turn_number: 42,
                timestamp: chrono::Utc::now(),
            }));
        });

        // Task B: receives agent event
        coord.spawn_task("spectator", move |_| async move {
            let mut rx = bus_clone.subscribe();
            loop {
                match rx.recv().await {
                    Ok(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
                        turn_number, ..
                    })) => {
                        tx.send(turn_number).await.ok();
                        break;
                    }
                    Ok(CoordinatorEvent::Shutdown) => break,
                    _ => {}
                }
            }
        });

        // Wait for result
        let turn = tokio::time::timeout(std::time::Duration::from_secs(2), result_rx.recv()).await;

        coord.shutdown().await;

        assert_eq!(turn.unwrap(), Some(42));
    }
}
