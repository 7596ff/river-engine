//! Context management — rotation and status

use super::registry::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

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

/// Get current context window usage
pub struct ContextStatusTool {
    context_limit: u64,
    context_used: Arc<AtomicU64>,
}

impl ContextStatusTool {
    pub fn new(context_limit: u64, context_used: Arc<AtomicU64>) -> Self {
        Self {
            context_limit,
            context_used,
        }
    }
}

impl Tool for ContextStatusTool {
    fn name(&self) -> &str {
        "context_status"
    }

    fn description(&self) -> &str {
        "Get current context window usage"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(&self, _args: Value) -> Result<ToolResult, RiverError> {
        let used = self.context_used.load(Ordering::Relaxed);
        let limit = self.context_limit;
        let percent = if limit > 0 {
            (used as f64 / limit as f64) * 100.0
        } else {
            0.0
        };
        let remaining = limit.saturating_sub(used);

        let output = serde_json::json!({
            "used": used,
            "limit": limit,
            "remaining": remaining,
            "percent": format!("{:.1}%", percent),
            "near_limit": percent >= 90.0
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&output).unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_context_status_tool() {
        let context_used = Arc::new(AtomicU64::new(5000));
        let tool = ContextStatusTool::new(10000, context_used);

        assert_eq!(tool.name(), "context_status");
        assert_eq!(tool.description(), "Get current context window usage");

        let result = tool.execute(serde_json::json!({})).unwrap();
        assert!(result.output.contains("50.0%"));
    }
}
