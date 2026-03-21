//! Scheduling tools for controlling loop timing and context rotation
//!
//! These tools allow the agent to control when the next heartbeat occurs
//! and to manually trigger context rotation.

use crate::tools::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Shared state for heartbeat scheduling
///
/// The value represents minutes until next heartbeat.
/// A value of 0 means use the default from config.
#[derive(Debug)]
pub struct HeartbeatScheduler {
    /// Scheduled minutes (0 = use default)
    scheduled_minutes: AtomicU64,
    /// Default minutes from config
    default_minutes: u64,
}

impl HeartbeatScheduler {
    pub fn new(default_minutes: u32) -> Self {
        Self {
            scheduled_minutes: AtomicU64::new(0),
            default_minutes: default_minutes as u64,
        }
    }

    /// Schedule the next heartbeat in the given number of minutes
    pub fn schedule(&self, minutes: u64) {
        self.scheduled_minutes.store(minutes, Ordering::SeqCst);
    }

    /// Get the next heartbeat delay and reset the schedule
    pub fn take_delay(&self) -> Duration {
        let scheduled = self.scheduled_minutes.swap(0, Ordering::SeqCst);
        let minutes = if scheduled > 0 {
            scheduled
        } else {
            self.default_minutes
        };
        Duration::from_secs(minutes * 60)
    }

    /// Check if a custom schedule is set
    pub fn is_scheduled(&self) -> bool {
        self.scheduled_minutes.load(Ordering::SeqCst) > 0
    }

    /// Get the current scheduled minutes (0 if using default)
    pub fn scheduled_minutes(&self) -> u64 {
        self.scheduled_minutes.load(Ordering::SeqCst)
    }

    /// Get the default minutes
    pub fn default_minutes(&self) -> u64 {
        self.default_minutes
    }
}

/// Schedule the next heartbeat wake time
pub struct ScheduleHeartbeatTool {
    scheduler: Arc<HeartbeatScheduler>,
}

impl ScheduleHeartbeatTool {
    pub fn new(scheduler: Arc<HeartbeatScheduler>) -> Self {
        Self { scheduler }
    }
}

impl Tool for ScheduleHeartbeatTool {
    fn name(&self) -> &str {
        "schedule_heartbeat"
    }

    fn description(&self) -> &str {
        "Set next heartbeat wake time"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "minutes": {
                    "type": "integer",
                    "description": "Minutes until next heartbeat (1-1440)",
                    "minimum": 1,
                    "maximum": 1440
                }
            },
            "required": ["minutes"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let minutes = args["minutes"]
            .as_u64()
            .ok_or_else(|| RiverError::tool("Missing 'minutes' parameter"))?;

        // Validate range
        if minutes < 1 {
            return Err(RiverError::tool("Minutes must be at least 1"));
        }
        if minutes > 1440 {
            return Err(RiverError::tool("Minutes cannot exceed 1440 (24 hours)"));
        }

        self.scheduler.schedule(minutes);

        let output = if minutes < self.scheduler.default_minutes() {
            format!(
                "Next heartbeat scheduled in {} minutes (sooner than default {})",
                minutes,
                self.scheduler.default_minutes()
            )
        } else if minutes > self.scheduler.default_minutes() {
            format!(
                "Next heartbeat scheduled in {} minutes (later than default {})",
                minutes,
                self.scheduler.default_minutes()
            )
        } else {
            format!("Next heartbeat scheduled in {} minutes (default)", minutes)
        };

        Ok(ToolResult::success(output))
    }
}

/// Shared state for context rotation requests
///
/// When rotation is requested, the loop will transition to settling/sleeping
/// after completing the current tool calls.
#[derive(Debug)]
pub struct ContextRotation {
    /// Whether rotation has been requested
    requested: AtomicBool,
    /// Summary for the rotation (None for auto-rotation)
    summary: RwLock<Option<String>>,
}

impl ContextRotation {
    pub fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            summary: RwLock::new(None),
        }
    }

    /// Request a context rotation with summary
    pub fn request(&self, summary: String) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
        *s = Some(summary);
    }

    /// Request auto-rotation (no summary)
    pub fn request_auto(&self) {
        self.requested.store(true, Ordering::SeqCst);
        let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
        *s = None;
    }

    /// Check if rotation is requested and take the summary
    /// Returns Some(Option<String>) if rotation was requested
    /// - Some(Some(summary)) = manual rotation with summary
    /// - Some(None) = auto-rotation without summary
    /// - None = no rotation requested
    pub fn take_request(&self) -> Option<Option<String>> {
        if self.requested.swap(false, Ordering::SeqCst) {
            let mut s = self.summary.write().expect("ContextRotation RwLock poisoned");
            Some(s.take())
        } else {
            None
        }
    }

    /// Check if rotation is pending (without clearing)
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

impl Default for ContextRotation {
    fn default() -> Self {
        Self::new()
    }
}

/// Manually trigger context rotation
pub struct RotateContextTool {
    rotation: Arc<ContextRotation>,
}

impl RotateContextTool {
    pub fn new(rotation: Arc<ContextRotation>) -> Self {
        Self { rotation }
    }
}

impl Tool for RotateContextTool {
    fn name(&self) -> &str {
        "rotate_context"
    }

    fn description(&self) -> &str {
        "Rotate context with a summary. The summary becomes a system message in the new context, preserving continuity."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Summary of current context to carry forward. This becomes a system message in the new context."
                }
            },
            "required": ["summary"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let summary = args["summary"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing required 'summary' parameter"))?
            .to_string();

        if summary.trim().is_empty() {
            return Err(RiverError::tool("Summary cannot be empty"));
        }

        self.rotation.request(summary);

        Ok(ToolResult::success(
            "Context rotation requested. Your summary will be preserved in the new context."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_scheduler_default() {
        let scheduler = HeartbeatScheduler::new(45);
        assert_eq!(scheduler.default_minutes(), 45);
        assert!(!scheduler.is_scheduled());
    }

    #[test]
    fn test_heartbeat_scheduler_schedule() {
        let scheduler = HeartbeatScheduler::new(45);
        scheduler.schedule(10);
        assert!(scheduler.is_scheduled());
        assert_eq!(scheduler.scheduled_minutes(), 10);
    }

    #[test]
    fn test_heartbeat_scheduler_take_delay() {
        let scheduler = HeartbeatScheduler::new(45);

        // Default delay
        let delay = scheduler.take_delay();
        assert_eq!(delay, Duration::from_secs(45 * 60));

        // Scheduled delay
        scheduler.schedule(10);
        let delay = scheduler.take_delay();
        assert_eq!(delay, Duration::from_secs(10 * 60));

        // Back to default after take
        assert!(!scheduler.is_scheduled());
        let delay = scheduler.take_delay();
        assert_eq!(delay, Duration::from_secs(45 * 60));
    }

    #[test]
    fn test_schedule_heartbeat_tool() {
        let scheduler = Arc::new(HeartbeatScheduler::new(45));
        let tool = ScheduleHeartbeatTool::new(scheduler.clone());

        assert_eq!(tool.name(), "schedule_heartbeat");

        let result = tool.execute(serde_json::json!({"minutes": 10}));
        assert!(result.is_ok());
        assert_eq!(scheduler.scheduled_minutes(), 10);
    }

    #[test]
    fn test_schedule_heartbeat_validation() {
        let scheduler = Arc::new(HeartbeatScheduler::new(45));
        let tool = ScheduleHeartbeatTool::new(scheduler);

        // Too low
        let result = tool.execute(serde_json::json!({"minutes": 0}));
        assert!(result.is_err());

        // Too high
        let result = tool.execute(serde_json::json!({"minutes": 2000}));
        assert!(result.is_err());

        // Missing parameter
        let result = tool.execute(serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_context_rotation_with_summary() {
        let rotation = ContextRotation::new();
        rotation.request("Test summary".to_string());

        assert!(rotation.is_requested());

        let result = rotation.take_request();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), Some("Test summary".to_string()));
        assert!(!rotation.is_requested());
    }

    #[test]
    fn test_context_rotation_auto() {
        let rotation = ContextRotation::new();
        rotation.request_auto();

        assert!(rotation.is_requested());

        let result = rotation.take_request();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), None);
        assert!(!rotation.is_requested());
    }

    #[test]
    fn test_context_rotation_not_requested() {
        let rotation = ContextRotation::new();
        let result = rotation.take_request();
        assert!(result.is_none());
    }

    #[test]
    fn test_rotate_context_tool_requires_summary() {
        let rotation = Arc::new(ContextRotation::new());
        let tool = RotateContextTool::new(rotation.clone());

        // Missing summary should fail
        let result = tool.execute(serde_json::json!({}));
        assert!(result.is_err());

        // Empty summary should fail
        let result = tool.execute(serde_json::json!({"summary": ""}));
        assert!(result.is_err());

        // Whitespace-only summary should fail
        let result = tool.execute(serde_json::json!({"summary": "   "}));
        assert!(result.is_err());

        // Valid summary should succeed
        let result = tool.execute(serde_json::json!({"summary": "Test summary"}));
        assert!(result.is_ok());
        assert!(rotation.is_requested());
    }
}
