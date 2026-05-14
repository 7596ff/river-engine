//! Shell tool

use super::registry::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, error, info, warn};

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
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute shell command"
    }

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
        info!(
            workspace = %self.workspace.display(),
            "BashTool::execute called"
        );

        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                error!("BashTool: Missing required parameter 'command'");
                RiverError::tool("Missing required parameter: command")
            })?;

        let timeout_ms = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let output_file = args.get("output_file").and_then(|v| v.as_str());

        info!(
            command = %command,
            timeout_ms = timeout_ms,
            output_file = ?output_file,
            "BashTool: Executing command"
        );

        // Execute the command with timeout using tokio
        let command = command.to_string();
        let workspace = self.workspace.clone();
        let timeout = Duration::from_millis(timeout_ms);

        let output = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                debug!(command = %command, "BashTool: Starting bash process");
                let child = tokio::process::Command::new("bash")
                    .arg("-l") // Login shell: sources ~/.bash_profile for full PATH
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&workspace)
                    .output();

                match tokio::time::timeout(timeout, child).await {
                    Ok(result) => result.map_err(|e| {
                        error!(error = %e, command = %command, "BashTool: Failed to execute command");
                        RiverError::tool(format!("Failed to execute command: {}", e))
                    }),
                    Err(_) => {
                        warn!(command = %command, timeout_ms = timeout_ms, "BashTool: Command timed out");
                        Err(RiverError::tool(format!(
                            "Command timed out after {} ms",
                            timeout_ms
                        )))
                    }
                }
            })
        })?;

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

        info!(
            exit_code = exit_code,
            success = success,
            stdout_len = stdout.len(),
            stderr_len = stderr.len(),
            stdout_preview = %stdout.chars().take(200).collect::<String>(),
            "BashTool: Command completed"
        );

        if !stderr.is_empty() {
            debug!(
                stderr_preview = %stderr.chars().take(500).collect::<String>(),
                "BashTool: Command had stderr output"
            );
        }

        // Handle output file if specified
        if let Some(out_path) = output_file {
            // Security: validate output path stays within workspace
            let path = std::path::Path::new(out_path);

            // Reject absolute paths
            if path.is_absolute() {
                return Err(RiverError::tool("Output file path must be relative"));
            }

            let full_out_path = self.workspace.join(path);

            // Canonicalize and verify within workspace
            // For new files, check parent exists and is within workspace
            let check_path = if full_out_path.exists() {
                full_out_path
                    .canonicalize()
                    .map_err(|e| RiverError::tool(format!("Invalid output path: {}", e)))?
            } else {
                let parent = full_out_path
                    .parent()
                    .ok_or_else(|| RiverError::tool("Invalid output path: no parent directory"))?;
                if parent.exists() {
                    let canonical_parent = parent
                        .canonicalize()
                        .map_err(|e| RiverError::tool(format!("Invalid output path: {}", e)))?;
                    canonical_parent.join(full_out_path.file_name().unwrap_or_default())
                } else {
                    // Parent doesn't exist - just use workspace + relative path
                    self.workspace
                        .canonicalize()
                        .map_err(|e| RiverError::tool(format!("Workspace error: {}", e)))?
                        .join(path)
                }
            };

            // Verify path is within workspace
            let workspace_canonical = self
                .workspace
                .canonicalize()
                .map_err(|e| RiverError::tool(format!("Workspace error: {}", e)))?;

            if !check_path.starts_with(&workspace_canonical) {
                return Err(RiverError::tool(
                    "Output file path escapes workspace boundary",
                ));
            }

            std::fs::write(&full_out_path, &combined)
                .map_err(|e| RiverError::tool(format!("Failed to write output file: {}", e)))?;

            if success {
                Ok(ToolResult::with_file(
                    format!("Output written to {} (exit code: {})", out_path, exit_code),
                    out_path,
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
                    if combined.is_empty() {
                        "(no output)"
                    } else {
                        &combined
                    }
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_echo() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({"command": "echo hello"})).unwrap();
        assert!(result.output.contains("hello"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_failure() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({"command": "exit 1"}));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exit code: 1"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_working_dir() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({"command": "pwd"})).unwrap();
        assert!(result
            .output
            .contains(&dir.path().to_string_lossy().to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_output_to_file() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool
            .execute(json!({
                "command": "echo 'test output'",
                "output_file": "output.txt"
            }))
            .unwrap();

        assert!(result.output_file.is_some());
        let content = std::fs::read_to_string(dir.path().join("output.txt")).unwrap();
        assert!(content.contains("test output"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_output_file_path_validation() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "echo test",
            "output_file": "/etc/passwd"
        }));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("relative"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_output_file_path_traversal() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        let result = tool.execute(json!({
            "command": "echo test",
            "output_file": "../escape.txt"
        }));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("escapes workspace"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bash_timeout() {
        let dir = TempDir::new().unwrap();
        let tool = BashTool::new(dir.path());

        // Command that would take 10 seconds but timeout is 100ms
        let result = tool.execute(json!({
            "command": "sleep 10",
            "timeout": 100
        }));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
}
