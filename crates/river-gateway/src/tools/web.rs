//! Web tools for fetching URLs and searching
//!
//! These tools allow the agent to fetch web content and search the web.

use crate::tools::{Tool, ToolResult};
use river_core::RiverError;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Fetch URL content
pub struct WebFetchTool {
    workspace: PathBuf,
    timeout: Duration,
}

impl WebFetchTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Validate that output_file path doesn't escape workspace
    fn validate_output_path(&self, file_path: &str) -> Result<PathBuf, RiverError> {
        let path = PathBuf::from(file_path);

        // Reject absolute paths
        if path.is_absolute() {
            return Err(RiverError::tool("Absolute paths not allowed"));
        }

        let full_path = self.workspace.join(&path);
        let canonical_workspace = self.workspace.canonicalize().unwrap_or_else(|_| self.workspace.clone());

        // Resolve the path to check for escapes
        if let Ok(canonical) = full_path.canonicalize() {
            if !canonical.starts_with(&canonical_workspace) {
                return Err(RiverError::tool("Path would escape workspace"));
            }
        } else {
            // File doesn't exist yet - check parent
            if let Some(parent) = full_path.parent() {
                if parent.exists() {
                    let canonical_parent = parent.canonicalize()
                        .map_err(|_| RiverError::tool("Invalid parent directory"))?;
                    if !canonical_parent.starts_with(&canonical_workspace) {
                        return Err(RiverError::tool("Path would escape workspace"));
                    }
                }
            }
        }

        Ok(full_path)
    }

    /// Fetch URL using curl
    fn fetch_url(&self, url: &str) -> Result<String, RiverError> {
        let output = Command::new("curl")
            .args([
                "-sL",                              // Silent, follow redirects
                "--max-time", &self.timeout.as_secs().to_string(),
                "-A", "Mozilla/5.0 (compatible; RiverAgent/1.0)", // User agent
                url,
            ])
            .output()
            .map_err(|e| RiverError::tool(format!("Failed to execute curl: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RiverError::tool(format!("curl failed: {}", stderr)));
        }

        String::from_utf8(output.stdout)
            .map_err(|e| RiverError::tool(format!("Invalid UTF-8 in response: {}", e)))
    }

    /// Convert HTML to markdown using pandoc
    fn html_to_markdown(&self, html: &str) -> Result<String, RiverError> {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = Command::new("pandoc")
            .args(["-f", "html", "-t", "markdown", "--wrap=none"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    RiverError::tool("pandoc not found - install it or use raw=true")
                } else {
                    RiverError::tool(format!("Failed to execute pandoc: {}", e))
                }
            })?;

        // Write HTML to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(html.as_bytes())
                .map_err(|e| RiverError::tool(format!("Failed to write to pandoc: {}", e)))?;
        }

        let output = child.wait_with_output()
            .map_err(|e| RiverError::tool(format!("Failed to wait for pandoc: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RiverError::tool(format!("pandoc failed: {}", stderr)));
        }

        String::from_utf8(output.stdout)
            .map_err(|e| RiverError::tool(format!("Invalid UTF-8 from pandoc: {}", e)))
    }
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetch URL content"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "raw": {
                    "type": "boolean",
                    "description": "If true, return raw output without pandoc processing",
                    "default": false
                },
                "output_file": {
                    "type": "string",
                    "description": "Pipe output to file instead of context (optional)"
                }
            },
            "required": ["url"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| RiverError::tool("Missing 'url' parameter"))?;

        let raw = args["raw"].as_bool().unwrap_or(false);
        let output_file = args["output_file"].as_str();

        // Validate URL (basic check)
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(RiverError::tool("URL must start with http:// or https://"));
        }

        // Fetch the content
        let content = self.fetch_url(url)?;

        // Process with pandoc if not raw and appears to be HTML
        let processed = if raw {
            content
        } else if content.trim().starts_with("<!") || content.trim().starts_with("<html") || content.contains("<body") {
            // Looks like HTML, try to convert
            match self.html_to_markdown(&content) {
                Ok(md) => md,
                Err(e) => {
                    // Fall back to raw if pandoc fails
                    tracing::warn!("Pandoc conversion failed, returning raw: {}", e);
                    content
                }
            }
        } else {
            // Not HTML, return as-is
            content
        };

        // Output to file or return in context
        if let Some(file_path) = output_file {
            let full_path = self.validate_output_path(file_path)?;

            // Ensure parent directory exists
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| RiverError::tool(format!("Failed to create directory: {}", e)))?;
            }

            std::fs::write(&full_path, &processed)
                .map_err(|e| RiverError::tool(format!("Failed to write file: {}", e)))?;

            Ok(ToolResult::with_file(
                format!("Fetched {} ({} bytes) to {}", url, processed.len(), file_path),
                file_path.to_string(),
            ))
        } else {
            // Truncate if too large
            let max_size = 50000; // 50KB limit for context
            let output = if processed.len() > max_size {
                format!(
                    "{}\n\n[Output truncated - {} bytes total, showing first {}]",
                    &processed[..max_size],
                    processed.len(),
                    max_size
                )
            } else {
                processed
            };

            Ok(ToolResult::success(output))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_webfetch_tool_schema() {
        let dir = TempDir::new().unwrap();
        let tool = WebFetchTool::new(dir.path());

        assert_eq!(tool.name(), "webfetch");
        let params = tool.parameters();
        assert!(params["properties"]["url"].is_object());
        assert!(params["properties"]["raw"].is_object());
        assert!(params["properties"]["output_file"].is_object());
    }

    #[test]
    fn test_validate_output_path() {
        let dir = TempDir::new().unwrap();
        let tool = WebFetchTool::new(dir.path());

        // Valid path
        assert!(tool.validate_output_path("output.txt").is_ok());
        assert!(tool.validate_output_path("subdir/output.txt").is_ok());

        // Invalid: absolute path
        assert!(tool.validate_output_path("/etc/passwd").is_err());

        // Invalid: path traversal
        assert!(tool.validate_output_path("../outside.txt").is_err());
    }

    #[test]
    fn test_url_validation() {
        let dir = TempDir::new().unwrap();
        let tool = WebFetchTool::new(dir.path());

        // Invalid URL scheme
        let result = tool.execute(serde_json::json!({"url": "file:///etc/passwd"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("http"));
    }
}
