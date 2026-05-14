//! Subagent system - enables parent agent to spawn child agent loops
//!
//! Subagents run as separate tokio tasks with shared workspace but independent context.

pub mod queue;
pub mod runner;
pub mod types;

pub use queue::{InternalMessage, InternalQueue};
pub use runner::{create_subagent_registry, SubagentConfig, SubagentRunner};
pub use types::{SubagentInfo, SubagentResult, SubagentStatus, SubagentType};

use river_core::{RiverError, Snowflake, SnowflakeGenerator, SnowflakeType};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Entry for a subagent in the manager
struct SubagentEntry {
    /// Subagent info
    info: SubagentInfo,
    /// Communication queue
    queue: Arc<InternalQueue>,
    /// Shutdown signal sender
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Result receiver (for waiting on completion)
    result_rx: Option<oneshot::Receiver<SubagentResult>>,
}

/// Central manager for all subagents
pub struct SubagentManager {
    /// Map of subagent ID to entry
    subagents: HashMap<Snowflake, SubagentEntry>,
    /// Snowflake generator for creating new IDs
    snowflake_gen: Arc<SnowflakeGenerator>,
}

impl SubagentManager {
    pub fn new(snowflake_gen: Arc<SnowflakeGenerator>) -> Self {
        Self {
            subagents: HashMap::new(),
            snowflake_gen,
        }
    }

    /// Register a new subagent and return its ID
    ///
    /// This creates the entry but doesn't start the runner.
    /// Call `set_channels` to set up communication after spawning the task.
    pub fn register(
        &mut self,
        subagent_type: SubagentType,
        task: String,
        model: String,
    ) -> (Snowflake, Arc<InternalQueue>) {
        let id = self.snowflake_gen.next_id(SnowflakeType::Subagent);
        let queue = Arc::new(InternalQueue::new());
        let info = SubagentInfo::new(id, subagent_type, task, model);

        let entry = SubagentEntry {
            info,
            queue: queue.clone(),
            shutdown_tx: None,
            result_rx: None,
        };

        self.subagents.insert(id, entry);
        (id, queue)
    }

    /// Set the channels for a subagent after spawning its task
    pub fn set_channels(
        &mut self,
        id: Snowflake,
        shutdown_tx: oneshot::Sender<()>,
        result_rx: oneshot::Receiver<SubagentResult>,
    ) {
        if let Some(entry) = self.subagents.get_mut(&id) {
            entry.shutdown_tx = Some(shutdown_tx);
            entry.result_rx = Some(result_rx);
        }
    }

    /// Update the status of a subagent
    pub fn set_status(&mut self, id: Snowflake, status: SubagentStatus) {
        if let Some(entry) = self.subagents.get_mut(&id) {
            entry.info.status = status;
        }
    }

    /// Mark a subagent as running
    pub fn set_running(&mut self, id: Snowflake) {
        if let Some(entry) = self.subagents.get_mut(&id) {
            entry.info.set_running();
        }
    }

    /// Mark a subagent as completed
    pub fn set_completed(&mut self, id: Snowflake, result: String) {
        if let Some(entry) = self.subagents.get_mut(&id) {
            entry.info.set_completed(result);
        }
    }

    /// Mark a subagent as failed
    pub fn set_failed(&mut self, id: Snowflake, error: String) {
        if let Some(entry) = self.subagents.get_mut(&id) {
            entry.info.set_failed(error);
        }
    }

    /// List all subagents
    pub fn list(&self) -> Vec<SubagentInfo> {
        self.subagents.values().map(|e| e.info.clone()).collect()
    }

    /// Get info for a specific subagent
    pub fn get(&self, id: Snowflake) -> Option<SubagentInfo> {
        self.subagents.get(&id).map(|e| e.info.clone())
    }

    /// Get the queue for a specific subagent
    pub fn queue(&self, id: Snowflake) -> Option<Arc<InternalQueue>> {
        self.subagents.get(&id).map(|e| e.queue.clone())
    }

    /// Stop a subagent
    pub fn stop(&mut self, id: Snowflake) -> Result<(), RiverError> {
        let entry = self
            .subagents
            .get_mut(&id)
            .ok_or_else(|| RiverError::tool(format!("Subagent {} not found", id)))?;

        if entry.info.status.is_terminal() {
            return Err(RiverError::tool(format!(
                "Subagent {} is already in terminal state: {}",
                id, entry.info.status
            )));
        }

        // Send shutdown signal
        if let Some(tx) = entry.shutdown_tx.take() {
            let _ = tx.send(());
        }

        entry.info.set_stopped();
        Ok(())
    }

    /// Take the result receiver for waiting
    pub fn take_result_rx(&mut self, id: Snowflake) -> Option<oneshot::Receiver<SubagentResult>> {
        self.subagents.get_mut(&id).and_then(|e| e.result_rx.take())
    }

    /// Check if a subagent exists
    pub fn exists(&self, id: Snowflake) -> bool {
        self.subagents.contains_key(&id)
    }

    /// Get count of active (non-terminal) subagents
    pub fn active_count(&self) -> usize {
        self.subagents
            .values()
            .filter(|e| !e.info.status.is_terminal())
            .count()
    }

    /// Remove completed subagents older than the given age (in seconds)
    pub fn cleanup_completed(&mut self, max_age_secs: i64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.subagents.retain(|_, entry| {
            if entry.info.status.is_terminal() {
                if let Some(completed_at) = entry.info.completed_at {
                    return now - completed_at < max_age_secs;
                }
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::AgentBirth;

    fn test_manager() -> SubagentManager {
        let birth = AgentBirth::new(2026, 3, 17, 12, 0, 0).unwrap();
        let snowflake_gen = Arc::new(SnowflakeGenerator::new(birth));
        SubagentManager::new(snowflake_gen)
    }

    #[test]
    fn test_register_subagent() {
        let mut manager = test_manager();

        let (id, queue) = manager.register(
            SubagentType::TaskWorker,
            "Test task".to_string(),
            "gpt-4".to_string(),
        );

        assert!(manager.exists(id));
        assert!(queue.drain_for_subagent().is_empty());

        let info = manager.get(id).unwrap();
        assert_eq!(info.status, SubagentStatus::Starting);
        assert_eq!(info.task, "Test task");
    }

    #[test]
    fn test_list_subagents() {
        let mut manager = test_manager();

        manager.register(
            SubagentType::TaskWorker,
            "Task 1".to_string(),
            "gpt-4".to_string(),
        );
        manager.register(
            SubagentType::LongRunning,
            "Task 2".to_string(),
            "gpt-4".to_string(),
        );

        let list = manager.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_status_updates() {
        let mut manager = test_manager();
        let (id, _) = manager.register(
            SubagentType::TaskWorker,
            "Task".to_string(),
            "model".to_string(),
        );

        manager.set_running(id);
        assert_eq!(manager.get(id).unwrap().status, SubagentStatus::Running);

        manager.set_completed(id, "Done".to_string());
        let info = manager.get(id).unwrap();
        assert_eq!(info.status, SubagentStatus::Completed);
        assert_eq!(info.result, Some("Done".to_string()));
    }

    #[test]
    fn test_stop_subagent() {
        let mut manager = test_manager();
        let (id, _) = manager.register(
            SubagentType::TaskWorker,
            "Task".to_string(),
            "model".to_string(),
        );

        // Set up channels
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let (result_tx, result_rx) = oneshot::channel::<SubagentResult>();
        manager.set_channels(id, shutdown_tx, result_rx);
        drop(result_tx); // Drop to avoid unused warning

        manager.set_running(id);
        assert!(manager.stop(id).is_ok());
        assert_eq!(manager.get(id).unwrap().status, SubagentStatus::Stopped);
    }

    #[test]
    fn test_stop_already_terminal() {
        let mut manager = test_manager();
        let (id, _) = manager.register(
            SubagentType::TaskWorker,
            "Task".to_string(),
            "model".to_string(),
        );

        manager.set_completed(id, "Done".to_string());
        assert!(manager.stop(id).is_err());
    }

    #[test]
    fn test_active_count() {
        let mut manager = test_manager();

        let (id1, _) = manager.register(
            SubagentType::TaskWorker,
            "Task 1".to_string(),
            "model".to_string(),
        );
        let (id2, _) = manager.register(
            SubagentType::TaskWorker,
            "Task 2".to_string(),
            "model".to_string(),
        );

        assert_eq!(manager.active_count(), 2);

        manager.set_completed(id1, "Done".to_string());
        assert_eq!(manager.active_count(), 1);

        manager.set_failed(id2, "Error".to_string());
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn test_queue_access() {
        let mut manager = test_manager();
        let (id, queue1) = manager.register(
            SubagentType::TaskWorker,
            "Task".to_string(),
            "model".to_string(),
        );

        let queue2 = manager.queue(id).unwrap();
        queue1.send_to_subagent("Hello");
        assert!(queue2.has_messages_for_subagent());
    }

    #[test]
    fn test_nonexistent_subagent() {
        let manager = test_manager();
        // Create a fake ID that won't exist in the manager
        let fake_id = Snowflake::from_parts(999, 0x0400000000000001);
        assert!(manager.get(fake_id).is_none());
        assert!(manager.queue(fake_id).is_none());
    }
}
