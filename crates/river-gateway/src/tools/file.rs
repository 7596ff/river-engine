//! File tools

use river_core::RiverError;
use super::{Tool, ToolResult};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

// Security constants
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB
const MAX_SEARCH_DEPTH: usize = 20;

/// Validate and resolve a path within the workspace
fn validate_path(workspace: &Path, path: &str) -> Result<PathBuf, RiverError> {
    let path = Path::new(path);

    // Reject absolute paths
    if path.is_absolute() {
        return Err(RiverError::tool("Absolute paths are not allowed"));
    }

    let full_path = workspace.join(path);

    // For new files that don't exist yet, validate parent
    let check_path = if full_path.exists() {
        full_path.canonicalize()
            .map_err(|e| RiverError::tool(format!("Invalid path: {}", e)))?
    } else {
        // For new files, check the parent directory
        let parent = full_path.parent()
            .ok_or_else(|| RiverError::tool("Invalid path: no parent directory"))?;
        if parent.exists() {
            let canonical_parent = parent.canonicalize()
                .map_err(|e| RiverError::tool(format!("Invalid path: {}", e)))?;
            canonical_parent.join(full_path.file_name().unwrap_or_default())
        } else {
            // Parent doesn't exist, we'll create it - just join with workspace
            workspace.canonicalize()
                .map_err(|e| RiverError::tool(format!("Workspace error: {}", e)))?
                .join(path)
        }
    };

    // Verify within workspace
    let workspace_canonical = workspace.canonicalize()
        .map_err(|e| RiverError::tool(format!("Workspace error: {}", e)))?;

    if !check_path.starts_with(&workspace_canonical) {
        return Err(RiverError::tool("Path escapes workspace boundary"));
    }

    Ok(full_path)
}

/// Check file size before reading
fn check_file_size(path: &Path) -> Result<(), RiverError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| RiverError::tool(format!("Cannot access file: {}", e)))?;

    if metadata.len() > MAX_FILE_SIZE {
        return Err(RiverError::tool(format!(
            "File too large: {} bytes (max: {} bytes)",
            metadata.len(), MAX_FILE_SIZE
        )));
    }
    Ok(())
}

/// Read file tool
pub struct ReadTool {
    workspace: std::path::PathBuf,
}

impl ReadTool {
    pub fn new(workspace: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace: workspace.into() }
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
        let path_str = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: path"))?;

        let path = validate_path(&self.workspace, path_str)?;
        check_file_size(&path)?;

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
            let validated_out_path = validate_path(&self.workspace, out_path)?;
            fs::write(&validated_out_path, &result)
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
        let path_str = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: path"))?;
        let content = args.get("content").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content"))?;

        let path = validate_path(&self.workspace, path_str)?;

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
        let path_str = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: path"))?;
        let old_string = args.get("old_string").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: old_string"))?;
        let new_string = args.get("new_string").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: new_string"))?;
        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        let path = validate_path(&self.workspace, path_str)?;
        check_file_size(&path)?;

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

        let base = if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
            validate_path(&self.workspace, path_str)?
        } else {
            self.workspace.clone()
        };

        let full_pattern = base.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        let workspace_canonical = self.workspace.canonicalize()
            .map_err(|e| RiverError::tool(format!("Workspace error: {}", e)))?;

        let paths = glob::glob(&pattern_str)
            .map_err(|e| RiverError::tool(format!("Invalid glob pattern: {}", e)))?;

        let files: Vec<String> = paths
            .filter_map(|p| p.ok())
            .filter(|p| {
                // Filter out paths outside workspace
                p.canonicalize()
                    .map(|cp| cp.starts_with(&workspace_canonical))
                    .unwrap_or(false)
            })
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
                "glob": { "type": "string", "description": "Filter files by glob pattern (optional)" },
                "context": { "type": "integer", "description": "Lines of context around matches (optional)" },
                "output_file": { "type": "string", "description": "Pipe output to file (optional)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let pattern = args.get("pattern").and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: pattern"))?;
        let output_file = args.get("output_file").and_then(|v| v.as_str());
        let glob_pattern = args.get("glob").and_then(|v| v.as_str());
        let context_lines = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        let regex = regex::Regex::new(pattern)
            .map_err(|e| RiverError::tool(format!("Invalid regex: {}", e)))?;

        // Compile glob pattern if provided
        let glob_matcher = if let Some(gp) = glob_pattern {
            Some(glob::Pattern::new(gp)
                .map_err(|e| RiverError::tool(format!("Invalid glob pattern: {}", e)))?)
        } else {
            None
        };

        let search_path = if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
            validate_path(&self.workspace, path_str)?
        } else {
            self.workspace.clone()
        };

        let mut results = Vec::new();

        fn walk_and_search(
            path: &Path,
            regex: &regex::Regex,
            glob_matcher: Option<&glob::Pattern>,
            context_lines: usize,
            results: &mut Vec<String>,
            depth: usize,
        ) {
            // Stop at max depth to prevent unbounded recursion
            if depth > MAX_SEARCH_DEPTH {
                return;
            }

            // Skip symlinks to prevent cycles
            if path.is_symlink() {
                return;
            }

            if path.is_file() {
                // Check glob filter if provided
                if let Some(matcher) = glob_matcher {
                    let file_name = path.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    if !matcher.matches(&file_name) {
                        return;
                    }
                }

                // Check file size before reading
                if let Ok(metadata) = fs::metadata(path) {
                    if metadata.len() > MAX_FILE_SIZE {
                        return; // Skip large files
                    }
                }

                if let Ok(content) = fs::read_to_string(path) {
                    let lines: Vec<&str> = content.lines().collect();
                    let mut matched_ranges: Vec<(usize, usize)> = Vec::new();

                    // Find all matching lines
                    for (i, line) in lines.iter().enumerate() {
                        if regex.is_match(line) {
                            let start = i.saturating_sub(context_lines);
                            let end = (i + context_lines + 1).min(lines.len());
                            matched_ranges.push((start, end));
                        }
                    }

                    // Merge overlapping ranges
                    if !matched_ranges.is_empty() {
                        let mut merged: Vec<(usize, usize)> = Vec::new();
                        let mut current = matched_ranges[0];
                        for &(start, end) in &matched_ranges[1..] {
                            if start <= current.1 {
                                current.1 = current.1.max(end);
                            } else {
                                merged.push(current);
                                current = (start, end);
                            }
                        }
                        merged.push(current);

                        // Output merged ranges
                        for (range_idx, (start, end)) in merged.iter().enumerate() {
                            if range_idx > 0 {
                                results.push("--".to_string());
                            }
                            for i in *start..*end {
                                let marker = if regex.is_match(lines[i]) { ":" } else { "-" };
                                results.push(format!("{}:{}{} {}", path.display(), i + 1, marker, lines[i]));
                            }
                        }
                    }
                }
            } else if path.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let entry_path = entry.path();
                        let name = entry_path.file_name().map(|n| n.to_string_lossy());
                        if name.map(|n| !n.starts_with('.')).unwrap_or(false) {
                            walk_and_search(&entry_path, regex, glob_matcher, context_lines, results, depth + 1);
                        }
                    }
                }
            }
        }

        walk_and_search(&search_path, &regex, glob_matcher.as_ref(), context_lines, &mut results, 0);

        let output = if results.is_empty() {
            "No matches found".to_string()
        } else {
            results.join("\n")
        };

        if let Some(out_path) = output_file {
            let validated_out_path = validate_path(&self.workspace, out_path)?;
            fs::write(&validated_out_path, &output)
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

    #[test]
    fn test_path_traversal_blocked() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        let read_tool = ReadTool::new(&workspace);

        // Test path traversal with ../
        let result = read_tool.execute(json!({"path": "../etc/passwd"}));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("escapes workspace")
            || err.to_string().contains("Invalid path")
            || err.to_string().contains("Cannot access file"),
            "Unexpected error: {}", err
        );

        // Test absolute path rejection
        let result = read_tool.execute(json!({"path": "/etc/passwd"}));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Absolute paths are not allowed"));
    }

    #[test]
    fn test_write_path_traversal_blocked() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        let write_tool = WriteTool::new(&workspace);

        // Test path traversal with ../
        let result = write_tool.execute(json!({"path": "../evil.txt", "content": "bad"}));
        assert!(result.is_err());

        // Test absolute path rejection
        let result = write_tool.execute(json!({"path": "/tmp/evil.txt", "content": "bad"}));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Absolute paths are not allowed"));
    }

    #[test]
    fn test_grep_with_context() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        fs::write(workspace.join("test.txt"), "line one\nline two\nline three\nline four\nline five").unwrap();

        let grep_tool = GrepTool::new(&workspace);
        let result = grep_tool.execute(json!({"pattern": "three", "context": 1})).unwrap();
        // Should include line two (before), three (match), and four (after)
        assert!(result.output.contains("line two"));
        assert!(result.output.contains("line three"));
        assert!(result.output.contains("line four"));
    }

    #[test]
    fn test_grep_with_glob_filter() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        fs::write(workspace.join("test.txt"), "findme here").unwrap();
        fs::write(workspace.join("test.rs"), "findme here too").unwrap();

        let grep_tool = GrepTool::new(&workspace);

        // Only search .txt files
        let result = grep_tool.execute(json!({"pattern": "findme", "glob": "*.txt"})).unwrap();
        assert!(result.output.contains("test.txt"));
        assert!(!result.output.contains("test.rs"));

        // Only search .rs files
        let result = grep_tool.execute(json!({"pattern": "findme", "glob": "*.rs"})).unwrap();
        assert!(!result.output.contains("test.txt"));
        assert!(result.output.contains("test.rs"));
    }

    #[test]
    fn test_grep_skips_symlinks() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();

        // Create a file
        fs::write(workspace.join("real.txt"), "findme").unwrap();

        // Create a directory with a symlink that could cause a cycle
        fs::create_dir(workspace.join("subdir")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&workspace, workspace.join("subdir/link_to_parent")).ok();
        }

        let grep_tool = GrepTool::new(&workspace);
        // This should complete without infinite loop
        let result = grep_tool.execute(json!({"pattern": "findme"}));
        assert!(result.is_ok());
        assert!(result.unwrap().output.contains("findme"));
    }
}
