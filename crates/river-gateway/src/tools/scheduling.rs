//! Scheduling tools for controlling loop timing
//!
//! These tools allow the agent to control when the next heartbeat occurs.

use crate::tools::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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
}
