//! File tools

use river_core::RiverError;
use super::{Tool, ToolResult};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Read file tool
pub struct ReadTool {
    workspace: std::path::PathBuf,
}

impl ReadTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(p)
        }
    }
}

impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }
    fn description(&self) -> &str { "Read file contents" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" },
                "offset": { "type": "integer", "description": "Line number to start from (optional)" },
                "limit": { "type": "integer", "description": "Maximum lines to read (optional)" },
                "output_file": { "type": "string", "description": "Pipe output to file (optional)" }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let path = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: path"))?;

        let path = self.resolve_path(path);
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64());
        let output_file = args.get("output_file").and_then(|v| v.as_str());

        let content = fs::read_to_string(&path)
            .map_err(|e| RiverError::tool(format!("Failed to read file: {}", e)))?;

        let lines: Vec<&str> = content.lines().collect();
        let start = offset.min(lines.len());
        let end = match limit {
            Some(l) => (start + l as usize).min(lines.len()),
            None => lines.len(),
        };

        let result: String = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:6}│ {}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        if let Some(out_path) = output_file {
            fs::write(out_path, &result)
                .map_err(|e| RiverError::tool(format!("Failed to write output file: {}", e)))?;
            Ok(ToolResult::with_file(format!("Output written to {}", out_path), out_path))
        } else {
            Ok(ToolResult::success(result))
        }
    }
}

/// Write file tool
pub struct WriteTool {
    workspace: std::path::PathBuf,
}

impl WriteTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { self.workspace.join(p) }
    }
}

impl Tool for WriteTool {
    fn name(&self) -> &str { "write" }
    fn description(&self) -> &str { "Write content to file (creates or overwrites)" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: path"))?;
        let content = args.get("content").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content"))?;

        let path = self.resolve_path(path);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| RiverError::tool(format!("Failed to create directories: {}", e)))?;
        }

        fs::write(&path, content)
            .map_err(|e| RiverError::tool(format!("Failed to write file: {}", e)))?;

        Ok(ToolResult::success(format!("Wrote {} bytes to {:?}", content.len(), path)))
    }
}

/// Edit file tool (surgical string replacement)
pub struct EditTool {
    workspace: std::path::PathBuf,
}

impl EditTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { self.workspace.join(p) }
    }
}

impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }
    fn description(&self) -> &str { "Replace text in file" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to edit" },
                "old_string": { "type": "string", "description": "Text to find" },
                "new_string": { "type": "string", "description": "Text to replace with" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences", "default": false }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: path"))?;
        let old_string = args.get("old_string").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: old_string"))?;
        let new_string = args.get("new_string").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: new_string"))?;
        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        let path = self.resolve_path(path);
        let content = fs::read_to_string(&path)
            .map_err(|e| RiverError::tool(format!("Failed to read file: {}", e)))?;

        let occurrences = content.matches(old_string).count();
        if occurrences == 0 {
            return Err(RiverError::tool("old_string not found in file"));
        }

        if !replace_all && occurrences > 1 {
            return Err(RiverError::tool(format!(
                "old_string found {} times - use replace_all or make it more specific", occurrences
            )));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        fs::write(&path, new_content)
            .map_err(|e| RiverError::tool(format!("Failed to write file: {}", e)))?;

        Ok(ToolResult::success(format!("Replaced {} occurrence(s) in {:?}", occurrences, path)))
    }
}

/// Glob tool - find files by pattern
pub struct GlobTool {
    workspace: std::path::PathBuf,
}

impl GlobTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}

impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }
    fn description(&self) -> &str { "Find files matching pattern" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g., **/*.md)" },
                "path": { "type": "string", "description": "Base directory (optional)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let pattern = args.get("pattern").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: pattern"))?;

        let base = args.get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.workspace.clone());

        let full_pattern = base.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        let paths = glob::glob(&pattern_str)
            .map_err(|e| RiverError::tool(format!("Invalid glob pattern: {}", e)))?;

        let files: Vec<String> = paths
            .filter_map(|p| p.ok())
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        if files.is_empty() {
            Ok(ToolResult::success("No files found"))
        } else {
            Ok(ToolResult::success(files.join("\n")))
        }
    }
}

/// Grep tool - search file contents
pub struct GrepTool {
    workspace: std::path::PathBuf,
}

impl GrepTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}

impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Search file contents with regex" }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search" },
                "path": { "type": "string", "description": "File or directory to search" },
                "output_file": { "type": "string", "description": "Pipe output to file (optional)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let pattern = args.get("pattern").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: pattern"))?;
        let output_file = args.get("output_file").and_then(|v| v.as_str());

        let regex = regex::Regex::new(pattern)
            .map_err(|e| RiverError::tool(format!("Invalid regex: {}", e)))?;

        let search_path = args.get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.workspace.clone());

        let mut results = Vec::new();

        fn walk_and_search(path: &Path, regex: &regex::Regex, results: &mut Vec<String>) {
            if path.is_file() {
                if let Ok(content) = fs::read_to_string(path) {
                    for (i, line) in content.lines().enumerate() {
                        if regex.is_match(line) {
                            results.push(format!("{}:{}: {}", path.display(), i + 1, line));
                        }
                    }
                }
            } else if path.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let entry_path = entry.path();
                        let name = entry_path.file_name().map(|n| n.to_string_lossy());
                        if name.map(|n| !n.starts_with('.')).unwrap_or(false) {
                            walk_and_search(&entry_path, regex, results);
                        }
                    }
                }
            }
        }

        walk_and_search(&search_path, &regex, &mut results);

        let output = if results.is_empty() {
            "No matches found".to_string()
        } else {
            results.join("\n")
        };

        if let Some(out_path) = output_file {
            fs::write(out_path, &output)
                .map_err(|e| RiverError::tool(format!("Failed to write output: {}", e)))?;
            Ok(ToolResult::with_file(format!("{} matches written to {}", results.len(), out_path), out_path))
        } else {
            Ok(ToolResult::success(output))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_write_edit() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        // Write
        let write_tool = WriteTool::new(&workspace);
        let result = write_tool.execute(json!({"path": "test.txt", "content": "Hello, world!"})).unwrap();
        assert!(result.output.contains("Wrote"));

        // Read
        let read_tool = ReadTool::new(&workspace);
        let result = read_tool.execute(json!({"path": "test.txt"})).unwrap();
        assert!(result.output.contains("Hello, world!"));

        // Edit
        let edit_tool = EditTool::new(&workspace);
        let result = edit_tool.execute(json!({
            "path": "test.txt",
            "old_string": "world",
            "new_string": "River"
        })).unwrap();
        assert!(result.output.contains("Replaced"));

        // Verify edit
        let result = read_tool.execute(json!({"path": "test.txt"})).unwrap();
        assert!(result.output.contains("Hello, River!"));
    }

    #[test]
    fn test_glob() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        fs::write(workspace.join("a.txt"), "a").unwrap();
        fs::write(workspace.join("b.txt"), "b").unwrap();
        fs::create_dir(workspace.join("sub")).unwrap();
        fs::write(workspace.join("sub/c.txt"), "c").unwrap();

        let glob_tool = GlobTool::new(&workspace);
        let result = glob_tool.execute(json!({"pattern": "**/*.txt"})).unwrap();
        assert!(result.output.contains("a.txt"));
        assert!(result.output.contains("b.txt"));
        assert!(result.output.contains("c.txt"));
    }

    #[test]
    fn test_grep() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        fs::write(workspace.join("test.txt"), "line one\nline two\nline three").unwrap();

        let grep_tool = GrepTool::new(&workspace);
        let result = grep_tool.execute(json!({"pattern": "two"})).unwrap();
        assert!(result.output.contains("line two"));
        assert!(result.output.contains(":2:"));
    }
}
