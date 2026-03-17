//! Scheduling tools for controlling loop timing and context rotation
//!
//! These tools allow the agent to control when the next heartbeat occurs
//! and to manually trigger context rotation.

use crate::tools::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

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
    /// Reason for the rotation (for logging)
    reason: RwLock<Option<String>>,
}

impl ContextRotation {
    pub fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            reason: RwLock::new(None),
        }
    }

    /// Request a context rotation
    pub fn request(&self, reason: Option<String>) {
        self.requested.store(true, Ordering::SeqCst);
        // Store reason asynchronously - use blocking for sync context
        if let Ok(mut r) = self.reason.try_write() {
            *r = reason;
        }
    }

    /// Check if rotation is requested and clear the flag
    /// Returns Some(reason) if rotation was requested (reason may be empty string)
    pub fn take_request(&self) -> Option<String> {
        if self.requested.swap(false, Ordering::SeqCst) {
            // Rotation was requested - get reason or default to empty string
            self.reason
                .try_write()
                .ok()
                .and_then(|mut r| r.take())
                .or_else(|| Some(String::new()))
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
        "Manually trigger context rotation. Call this after saving your state to thinking/current-state.md"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Reason for rotation (optional, for logging)"
                }
            },
            "required": []
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let reason = args["reason"].as_str().map(|s| s.to_string());

        self.rotation.request(reason.clone());

        let output = if let Some(r) = reason {
            format!(
                "Context rotation requested. Reason: {}. \
                Ensure you have saved your state to thinking/current-state.md before this cycle ends.",
                r
            )
        } else {
            "Context rotation requested. \
            Ensure you have saved your state to thinking/current-state.md before this cycle ends."
                .to_string()
        };

        Ok(ToolResult::success(output))
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
    fn test_context_rotation_default() {
        let rotation = ContextRotation::new();
        assert!(!rotation.is_requested());
    }

    #[test]
    fn test_context_rotation_request() {
        let rotation = ContextRotation::new();
        rotation.request(Some("Testing".to_string()));
        assert!(rotation.is_requested());
    }

    #[test]
    fn test_context_rotation_take_request() {
        let rotation = ContextRotation::new();
        rotation.request(Some("Testing".to_string()));

        let reason = rotation.take_request();
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Testing");
        assert!(!rotation.is_requested());
    }

    #[test]
    fn test_context_rotation_take_request_empty() {
        let rotation = ContextRotation::new();
        let reason = rotation.take_request();
        assert!(reason.is_none());
    }

    #[test]
    fn test_context_rotation_take_request_no_reason() {
        let rotation = ContextRotation::new();
        rotation.request(None);

        let reason = rotation.take_request();
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), ""); // Empty string when no reason given
        assert!(!rotation.is_requested());
    }

    #[test]
    fn test_rotate_context_tool() {
        let rotation = Arc::new(ContextRotation::new());
        let tool = RotateContextTool::new(rotation.clone());

        assert_eq!(tool.name(), "rotate_context");

        let result = tool.execute(serde_json::json!({"reason": "Test rotation"}));
        assert!(result.is_ok());
        assert!(rotation.is_requested());
    }

    #[test]
    fn test_rotate_context_tool_no_reason() {
        let rotation = Arc::new(ContextRotation::new());
        let tool = RotateContextTool::new(rotation.clone());

        let result = tool.execute(serde_json::json!({}));
        assert!(result.is_ok());
        assert!(rotation.is_requested());
    }
}
