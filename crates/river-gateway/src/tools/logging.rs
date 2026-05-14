//! Logging tools for reading system logs
//!
//! These tools allow the agent to read system logs while respecting
//! the privacy boundary defined in the spec.

use super::registry::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::{json, Value};
use std::process::Command;

/// Read system logs via journalctl
pub struct LogReadTool {
    /// Service unit name to filter logs (e.g., "river-gateway")
    unit_name: Option<String>,
}

impl LogReadTool {
    pub fn new(unit_name: Option<String>) -> Self {
        Self { unit_name }
    }
}

impl Tool for LogReadTool {
    fn name(&self) -> &str {
        "log_read"
    }

    fn description(&self) -> &str {
        "Read system log entries"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "lines": {
                    "type": "integer",
                    "description": "Number of log lines to read (default: 50, max: 500)",
                    "default": 50
                },
                "level": {
                    "type": "string",
                    "description": "Filter by log level (debug, info, warning, error)",
                    "enum": ["debug", "info", "warning", "error"]
                },
                "component": {
                    "type": "string",
                    "description": "Filter by component name (gateway, orchestrator, discord)"
                }
            },
            "required": []
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let lines = args["lines"].as_u64().unwrap_or(50).min(500) as usize;
        let level = args["level"].as_str();
        let component = args["component"].as_str();

        // Build journalctl command
        let mut cmd = Command::new("journalctl");

        // Output as JSON for structured parsing
        cmd.arg("--output=json");

        // Limit number of lines
        cmd.arg("-n").arg(lines.to_string());

        // No pager
        cmd.arg("--no-pager");

        // Filter by unit if component specified
        if let Some(comp) = component {
            let unit = match comp {
                "gateway" => "river-gateway",
                "orchestrator" => "river-orchestrator",
                "discord" => "river-discord",
                other => other,
            };
            cmd.arg("-u").arg(format!("{}*", unit));
        } else if let Some(ref unit) = self.unit_name {
            // Default to this gateway's unit
            cmd.arg("-u").arg(format!("{}*", unit));
        } else {
            // Filter to river-* units
            cmd.arg("-u").arg("river-*");
        }

        // Filter by priority level
        if let Some(lvl) = level {
            let priority = match lvl {
                "debug" => "7",
                "info" => "6",
                "warning" => "4",
                "error" => "3",
                _ => "6",
            };
            cmd.arg("-p").arg(format!("0..{}", priority));
        }

        let output = cmd
            .output()
            .map_err(|e| RiverError::tool(format!("Failed to execute journalctl: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // journalctl returns error if no logs match - that's OK
            if stderr.contains("No entries") || output.stdout.is_empty() {
                return Ok(ToolResult::success("No log entries found matching filters"));
            }
            return Err(RiverError::tool(format!("journalctl failed: {}", stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON lines and format for output
        let mut formatted_lines = Vec::new();
        for line in stdout.lines() {
            if let Ok(entry) = serde_json::from_str::<Value>(line) {
                // Extract fields respecting privacy boundary
                let timestamp = entry["__REALTIME_TIMESTAMP"]
                    .as_str()
                    .and_then(|t| t.parse::<u64>().ok())
                    .map(|t| {
                        // Convert microseconds to ISO 8601
                        let secs = t / 1_000_000;
                        let nanos = ((t % 1_000_000) * 1000) as u32;
                        chrono::DateTime::from_timestamp(secs as i64, nanos)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                let priority = entry["PRIORITY"].as_str().unwrap_or("6");
                let level_str = match priority {
                    "0" | "1" | "2" | "3" => "ERROR",
                    "4" => "WARN",
                    "5" | "6" => "INFO",
                    "7" => "DEBUG",
                    _ => "INFO",
                };

                let unit = entry["_SYSTEMD_UNIT"].as_str().unwrap_or("unknown");

                let message = entry["MESSAGE"].as_str().unwrap_or("");

                // Privacy: redact any content that looks like message content or file paths
                let safe_message = redact_sensitive_content(message);

                formatted_lines.push(format!(
                    "{} [{}] {}: {}",
                    timestamp, level_str, unit, safe_message
                ));
            }
        }

        if formatted_lines.is_empty() {
            Ok(ToolResult::success("No log entries found matching filters"))
        } else {
            Ok(ToolResult::success(formatted_lines.join("\n")))
        }
    }
}

/// Redact potentially sensitive content from log messages
/// Per spec: logs should not contain message content, file contents, tool arguments
fn redact_sensitive_content(message: &str) -> String {
    // If the message looks like it contains user content, redact it
    // This is a simple heuristic - real implementation might need more sophistication

    let mut result = message.to_string();

    // Redact content fields (greedy match to end of string or next field)
    if result.to_lowercase().contains("content:") {
        result = regex::Regex::new(r#"(?i)content:\s*"[^"]*""#)
            .map(|re| re.replace_all(&result, "content: [REDACTED]").to_string())
            .unwrap_or(result);
        // Also handle unquoted content
        result = regex::Regex::new(r#"(?i)content:\s*[^\s,}]+"#)
            .map(|re| re.replace_all(&result, "content: [REDACTED]").to_string())
            .unwrap_or(result);
    }

    // Redact arguments that might contain sensitive data
    if result.contains("arguments:") || result.contains("args:") {
        result = regex::Regex::new(r#"(arguments|args):\s*\{[^}]*\}"#)
            .map(|re| re.replace_all(&result, "$1: [REDACTED]").to_string())
            .unwrap_or(result);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_read_tool_schema() {
        let tool = LogReadTool::new(None);
        assert_eq!(tool.name(), "log_read");

        let params = tool.parameters();
        assert!(params["properties"]["lines"].is_object());
        assert!(params["properties"]["level"].is_object());
        assert!(params["properties"]["component"].is_object());
    }

    #[test]
    fn test_redact_sensitive_content() {
        // Normal log message - no redaction
        let msg = "Starting server on port 3000";
        assert_eq!(redact_sensitive_content(msg), msg);

        // Message with content field - should redact
        let msg = "Processing content: \"hello world\"";
        let result = redact_sensitive_content(msg);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("hello world"));
    }

    #[test]
    fn test_log_read_default_params() {
        let tool = LogReadTool::new(None);
        // Just verify we can create the tool and get params
        let params = tool.parameters();
        assert_eq!(params["properties"]["lines"]["default"], 50);
    }
}
