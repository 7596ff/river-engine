//! Context persistence in OpenAI JSONL format.

use river_context::OpenAIMessage;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Load context from JSONL file.
pub fn load_context(path: &Path) -> Vec<OpenAIMessage> {
    if !path.exists() {
        return Vec::new();
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Failed to open context file: {}", e);
            return Vec::new();
        }
    };

    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("Failed to read line: {}", e);
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str(&line) {
            Ok(msg) => messages.push(msg),
            Err(e) => {
                tracing::warn!("Failed to parse message: {}", e);
            }
        }
    }

    messages
}

/// Append a message to context file.
pub fn append_to_context(path: &Path, message: &OpenAIMessage) -> std::io::Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    let json = serde_json::to_string(message)?;
    writeln!(file, "{}", json)?;
    Ok(())
}

/// Save full context to file (overwrites).
pub fn save_context(path: &Path, messages: &[OpenAIMessage]) -> std::io::Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(path)?;

    for message in messages {
        let json = serde_json::to_string(message)?;
        writeln!(file, "{}", json)?;
    }

    Ok(())
}

/// Clear context file.
pub fn clear_context(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Check if a message should be persisted to context.jsonl.
///
/// Only persist:
/// - Assistant messages (LLM outputs)
/// - System messages that are context pressure warnings
pub fn should_persist(message: &OpenAIMessage) -> bool {
    match message.role.as_str() {
        "assistant" => true,
        "system" => {
            // Only persist context pressure warnings
            message.content.as_ref()
                .map(|c| c.contains("Context at"))
                .unwrap_or(false)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_should_persist_assistant() {
        let msg = OpenAIMessage::assistant("I'll help you with that.");
        assert!(should_persist(&msg));
    }

    #[test]
    fn test_should_persist_system_warning() {
        let msg = OpenAIMessage::system("Context at 80%. Consider wrapping up.");
        assert!(should_persist(&msg));
    }

    #[test]
    fn test_should_not_persist_user() {
        let msg = OpenAIMessage::user("Hello");
        assert!(!should_persist(&msg));
    }

    #[test]
    fn test_should_not_persist_tool_result() {
        let msg = OpenAIMessage::tool("call_123", "result");
        assert!(!should_persist(&msg));
    }

    #[test]
    fn test_should_not_persist_regular_system() {
        let msg = OpenAIMessage::system("You are a helpful assistant.");
        assert!(!should_persist(&msg));
    }

    #[test]
    fn test_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("context.jsonl");

        let messages = vec![
            OpenAIMessage::user("hello"),
            OpenAIMessage::assistant("hi"),
        ];

        save_context(&path, &messages).unwrap();
        let loaded = load_context(&path);

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, Some("hello".into()));
        assert_eq!(loaded[1].content, Some("hi".into()));
    }

    #[test]
    fn test_append() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("context.jsonl");

        let msg1 = OpenAIMessage::user("first");
        let msg2 = OpenAIMessage::assistant("second");

        append_to_context(&path, &msg1).unwrap();
        append_to_context(&path, &msg2).unwrap();

        let loaded = load_context(&path);
        assert_eq!(loaded.len(), 2);
    }
}
