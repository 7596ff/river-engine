//! Subagent type definitions

use river_core::Snowflake;
use serde::{Deserialize, Serialize};

/// Type of subagent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentType {
    /// One-shot task worker that terminates when model returns no tool calls
    TaskWorker,
    /// Long-running agent that waits for messages or shutdown
    LongRunning,
}

impl SubagentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubagentType::TaskWorker => "task_worker",
            SubagentType::LongRunning => "long_running",
        }
    }
}

impl std::fmt::Display for SubagentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Status of a subagent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    /// Subagent is starting up
    Starting,
    /// Subagent is actively running
    Running,
    /// Subagent completed successfully
    Completed,
    /// Subagent failed with an error
    Failed,
    /// Subagent was stopped by parent
    Stopped,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubagentStatus::Starting => "starting",
            SubagentStatus::Running => "running",
            SubagentStatus::Completed => "completed",
            SubagentStatus::Failed => "failed",
            SubagentStatus::Stopped => "stopped",
        }
    }

    /// Check if the subagent is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Stopped
        )
    }
}

impl std::fmt::Display for SubagentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Information about a subagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    /// Unique ID (Snowflake type 0x04)
    pub id: Snowflake,
    /// Type of subagent
    pub subagent_type: SubagentType,
    /// Current status
    pub status: SubagentStatus,
    /// Task description
    pub task: String,
    /// Model being used
    pub model: String,
    /// Creation timestamp (Unix seconds)
    pub created_at: i64,
    /// Completion timestamp (Unix seconds)
    pub completed_at: Option<i64>,
    /// Result text (if completed successfully)
    pub result: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
}

impl SubagentInfo {
    pub fn new(
        id: Snowflake,
        subagent_type: SubagentType,
        task: String,
        model: String,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Self {
            id,
            subagent_type,
            status: SubagentStatus::Starting,
            task,
            model,
            created_at: now,
            completed_at: None,
            result: None,
            error: None,
        }
    }

    /// Mark as running
    pub fn set_running(&mut self) {
        self.status = SubagentStatus::Running;
    }

    /// Mark as completed with result
    pub fn set_completed(&mut self, result: String) {
        self.status = SubagentStatus::Completed;
        self.result = Some(result);
        self.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        );
    }

    /// Mark as failed with error
    pub fn set_failed(&mut self, error: String) {
        self.status = SubagentStatus::Failed;
        self.error = Some(error);
        self.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        );
    }

    /// Mark as stopped
    pub fn set_stopped(&mut self) {
        self.status = SubagentStatus::Stopped;
        self.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        );
    }
}

/// Result returned when a subagent completes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    /// The subagent ID
    pub id: Snowflake,
    /// Final status
    pub status: SubagentStatus,
    /// Result text (if completed successfully)
    pub result: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
}

impl From<&SubagentInfo> for SubagentResult {
    fn from(info: &SubagentInfo) -> Self {
        Self {
            id: info.id,
            status: info.status,
            result: info.result.clone(),
            error: info.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snowflake() -> Snowflake {
        // Create a simple test snowflake using from_parts
        // For tests, we just need a unique ID - the internal structure doesn't matter
        Snowflake::from_parts(1, 0x0400000000000001) // type 0x04 = Subagent
    }

    #[test]
    fn test_subagent_type_display() {
        assert_eq!(SubagentType::TaskWorker.as_str(), "task_worker");
        assert_eq!(SubagentType::LongRunning.as_str(), "long_running");
    }

    #[test]
    fn test_subagent_status_terminal() {
        assert!(!SubagentStatus::Starting.is_terminal());
        assert!(!SubagentStatus::Running.is_terminal());
        assert!(SubagentStatus::Completed.is_terminal());
        assert!(SubagentStatus::Failed.is_terminal());
        assert!(SubagentStatus::Stopped.is_terminal());
    }

    #[test]
    fn test_subagent_info_lifecycle() {
        let mut info = SubagentInfo::new(
            test_snowflake(),
            SubagentType::TaskWorker,
            "Test task".to_string(),
            "gpt-4".to_string(),
        );

        assert_eq!(info.status, SubagentStatus::Starting);
        assert!(info.result.is_none());
        assert!(info.error.is_none());

        info.set_running();
        assert_eq!(info.status, SubagentStatus::Running);

        info.set_completed("Done!".to_string());
        assert_eq!(info.status, SubagentStatus::Completed);
        assert_eq!(info.result, Some("Done!".to_string()));
        assert!(info.completed_at.is_some());
    }

    #[test]
    fn test_subagent_info_failed() {
        let mut info = SubagentInfo::new(
            test_snowflake(),
            SubagentType::TaskWorker,
            "Test task".to_string(),
            "gpt-4".to_string(),
        );

        info.set_failed("Something went wrong".to_string());
        assert_eq!(info.status, SubagentStatus::Failed);
        assert_eq!(info.error, Some("Something went wrong".to_string()));
        assert!(info.completed_at.is_some());
    }

    #[test]
    fn test_subagent_result_from_info() {
        let mut info = SubagentInfo::new(
            test_snowflake(),
            SubagentType::TaskWorker,
            "Test task".to_string(),
            "gpt-4".to_string(),
        );
        info.set_completed("Result".to_string());

        let result = SubagentResult::from(&info);
        assert_eq!(result.status, SubagentStatus::Completed);
        assert_eq!(result.result, Some("Result".to_string()));
    }

    #[test]
    fn test_subagent_type_serde() {
        let json = serde_json::to_string(&SubagentType::TaskWorker).unwrap();
        assert_eq!(json, "\"task_worker\"");

        let parsed: SubagentType = serde_json::from_str("\"long_running\"").unwrap();
        assert_eq!(parsed, SubagentType::LongRunning);
    }

    #[test]
    fn test_subagent_status_serde() {
        let json = serde_json::to_string(&SubagentStatus::Running).unwrap();
        assert_eq!(json, "\"running\"");

        let parsed: SubagentStatus = serde_json::from_str("\"completed\"").unwrap();
        assert_eq!(parsed, SubagentStatus::Completed);
    }
}
