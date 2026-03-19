//! JSONL-based context persistence
//!
//! This module provides `ContextFile` for persisting chat messages
//! to a JSONL (JSON Lines) file in the workspace directory.

use crate::r#loop::ChatMessage;
use river_core::{RiverError, RiverResult};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const CONTEXT_FILENAME: &str = "context.jsonl";

/// A JSONL file for persisting chat messages.
///
/// Each message is stored as a single JSON line, allowing for
/// efficient append operations and resilient loading (corrupted
/// lines can be skipped).
#[derive(Debug)]
pub struct ContextFile {
    path: PathBuf,
}

impl ContextFile {
    /// Create a new empty context.jsonl file in the workspace.
    ///
    /// This will overwrite any existing file at the path.
    pub fn create(workspace: &Path) -> RiverResult<Self> {
        let path = workspace.join(CONTEXT_FILENAME);

        // Create the file (overwrites if exists)
        File::create(&path)?;

        Ok(Self { path })
    }

    /// Open an existing context.jsonl file in the workspace.
    ///
    /// Returns an error if the file does not exist.
    pub fn open(workspace: &Path) -> RiverResult<Self> {
        let path = workspace.join(CONTEXT_FILENAME);

        if !path.exists() {
            return Err(RiverError::Workspace(format!(
                "Context file does not exist: {}",
                path.display()
            )));
        }

        Ok(Self { path })
    }

    /// Create a new context.jsonl file with an initial system message summary.
    ///
    /// This is useful for initializing context after rotation.
    pub fn create_with_summary(workspace: &Path, summary: &str) -> RiverResult<Self> {
        let context_file = Self::create(workspace)?;
        let system_msg = ChatMessage::system(format!("Previous context summary: {}", summary));
        context_file.append(&system_msg)?;
        Ok(context_file)
    }

    /// Check if a context.jsonl file exists in the workspace.
    pub fn exists(workspace: &Path) -> bool {
        workspace.join(CONTEXT_FILENAME).exists()
    }

    /// Delete the context.jsonl file if it exists.
    pub fn delete(workspace: &Path) -> RiverResult<()> {
        let path = workspace.join(CONTEXT_FILENAME);

        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        Ok(())
    }

    /// Append a message to the context file as a JSONL line.
    pub fn append(&self, message: &ChatMessage) -> RiverResult<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        let json = serde_json::to_string(message)?;
        writeln!(file, "{}", json)?;

        Ok(())
    }

    /// Load all messages from the context file.
    ///
    /// Corrupted lines are logged with a warning and skipped.
    pub fn load(&self) -> RiverResult<Vec<ChatMessage>> {
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        line = line_num + 1,
                        error = %e,
                        "Failed to read line from context file, skipping"
                    );
                    continue;
                }
            };

            // Skip empty lines
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<ChatMessage>(&line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        line = line_num + 1,
                        error = %e,
                        "Corrupted line in context file, skipping"
                    );
                }
            }
        }

        Ok(messages)
    }

    /// Read the raw bytes of the context file for archiving.
    pub fn read_raw(&self) -> RiverResult<Vec<u8>> {
        Ok(std::fs::read(&self.path)?)
    }

    /// Get the path to the context file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_and_append() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        // Create a new context file
        let context_file = ContextFile::create(workspace).unwrap();

        // Append a message
        let msg = ChatMessage::user("Hello, world!");
        context_file.append(&msg).unwrap();

        // Load and verify
        let messages = context_file.load().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, Some("Hello, world!".to_string()));
    }

    #[test]
    fn test_multiple_appends() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        let context_file = ContextFile::create(workspace).unwrap();

        // Append multiple messages
        context_file.append(&ChatMessage::system("System prompt")).unwrap();
        context_file.append(&ChatMessage::user("User message")).unwrap();
        context_file.append(&ChatMessage::assistant(Some("Response".to_string()), None)).unwrap();
        context_file.append(&ChatMessage::tool("call_123", "Tool result")).unwrap();

        // Load and verify order is preserved
        let messages = context_file.load().unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[2].role, "assistant");
        assert_eq!(messages[3].role, "tool");
        assert_eq!(messages[3].tool_call_id, Some("call_123".to_string()));
    }

    #[test]
    fn test_create_with_summary() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        let context_file = ContextFile::create_with_summary(
            workspace,
            "Worked on feature X, implemented foo and bar."
        ).unwrap();

        let messages = context_file.load().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        let content = messages[0].content.as_ref().unwrap();
        assert!(content.starts_with("Previous context summary: "));
        assert!(content.contains("Worked on feature X"));
    }

    #[test]
    fn test_exists_and_delete() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        // Should not exist initially
        assert!(!ContextFile::exists(workspace));

        // Create file
        ContextFile::create(workspace).unwrap();
        assert!(ContextFile::exists(workspace));

        // Delete file
        ContextFile::delete(workspace).unwrap();
        assert!(!ContextFile::exists(workspace));

        // Delete again should not fail (idempotent)
        ContextFile::delete(workspace).unwrap();
    }

    #[test]
    fn test_open_existing() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        // Create and write some data
        let context_file = ContextFile::create(workspace).unwrap();
        context_file.append(&ChatMessage::user("Test")).unwrap();
        drop(context_file);

        // Open and read
        let context_file = ContextFile::open(workspace).unwrap();
        let messages = context_file.load().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, Some("Test".to_string()));
    }

    #[test]
    fn test_open_nonexistent_fails() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        let result = ContextFile::open(workspace);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, RiverError::Workspace(_)));
    }

    #[test]
    fn test_corrupted_line_skipped() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        let path = workspace.join(CONTEXT_FILENAME);

        // Write file with a corrupted line in the middle
        let mut file = File::create(&path).unwrap();
        writeln!(file, r#"{{"role":"user","content":"First"}}"#).unwrap();
        writeln!(file, "this is not valid json!!!").unwrap();
        writeln!(file, r#"{{"role":"user","content":"Third"}}"#).unwrap();
        drop(file);

        let context_file = ContextFile::open(workspace).unwrap();
        let messages = context_file.load().unwrap();

        // Should have 2 valid messages, corrupted line skipped
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, Some("First".to_string()));
        assert_eq!(messages[1].content, Some("Third".to_string()));
    }

    #[test]
    fn test_read_raw() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        let context_file = ContextFile::create(workspace).unwrap();
        context_file.append(&ChatMessage::user("Test message")).unwrap();

        let raw = context_file.read_raw().unwrap();
        let content = String::from_utf8(raw).unwrap();

        // Should contain the JSON line
        assert!(content.contains("Test message"));
        assert!(content.contains("user"));
        // Should end with newline (JSONL format)
        assert!(content.ends_with('\n'));
    }
}
