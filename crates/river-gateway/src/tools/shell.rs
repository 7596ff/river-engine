//! Shell tool

use river_core::RiverError;
use super::{Tool, ToolResult};
use serde_json::{json, Value};
use std::process::Command;
use std::time::Duration;
use std::path::PathBuf;

/// Default command timeout (2 minutes)
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// Maximum command timeout (10 minutes)
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Bash command execution tool
pub struct BashTool {
    workspace: PathBuf,
    default_timeout: Duration,
}

impl BashTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            default_timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str { "Execute shell command" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Command to execute" },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds (optional, max 600000)" },
                "output_file": { "type": "string", "description": "Pipe output to file (optional)" }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let command = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: command"))?;

        let _timeout_ms = args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let output_file = args.get("output_file").and_then(|v| v.as_str());

        // Execute the command
        let output = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&self.workspace)
            .output()
            .map_err(|e| RiverError::tool(format!("Failed to execute command: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let combined = if stderr.is_empty() {
            stdout.to_string()
        } else if stdout.is_empty() {
            format!("stderr:\n{}", stderr)
        } else {
            format!("{}\n\nstderr:\n{}", stdout, stderr)
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let success = output.status.success();

        // Handle output file if specified
        if let Some(out_path) = output_file {
            // Validate output path is within workspace (security)
            let out_path_buf = std::path::Path::new(out_path);
            if out_path_buf.is_absolute() {
                return Err(RiverError::tool("Output file path must be relative"));
            }
            let full_out_path = self.workspace.join(out_path);

            std::fs::write(&full_out_path, &combined)
                .map_err(|e| RiverError::tool(format!("Failed to write output file: {}", e)))?;

            if success {
                Ok(ToolResult::with_file(
                    format!("Output written to {} (exit code: {})", out_path, exit_code),
                    out_path
                ))
            } else {
                Err(RiverError::tool(format!(
                    "Command failed (exit code: {}). Output written to {}",
                    exit_code, out_path
                )))
            }
        } else {
            if success {
                Ok(ToolResult::success(if combined.is_empty() {
                    format!("(exit code: {})", exit_code)
                } else {
                    combined
                }))
            } else {
                Err(RiverError::tool(format!(
                    "Command failed (exit code: {}): {}",
                    exit_code,
                    if combined.is_empty() { "(no output)" } else { &combined }
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_bash_echo() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({"command": "echo hello"})).unwrap();
        assert!(result.output.contains("hello"));
    }

    #[test]
    fn test_bash_failure() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({"command": "exit 1"}));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exit code: 1"));
    }

    #[test]
    fn test_bash_working_dir() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({"command": "pwd"})).unwrap();
        assert!(result.output.contains(&dir.path().to_string_lossy().to_string()));
    }

    #[test]
    fn test_bash_output_to_file() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "echo 'test output'",
            "output_file": "output.txt"
        })).unwrap();

        assert!(result.output_file.is_some());
        let content = std::fs::read_to_string(dir.path().join("output.txt")).unwrap();
        assert!(content.contains("test output"));
    }

    #[test]
    fn test_bash_output_file_path_validation() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "echo test",
            "output_file": "/etc/passwd"
        }));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("relative"));
    }
}
