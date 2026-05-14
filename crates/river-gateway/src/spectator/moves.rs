//! Move storage — read/write moves.jsonl
//!
//! Moves are stored as one JSON object per line:
//! {"start":"...","end":"...","summary":"..."}

use river_core::Snowflake;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncWriteExt;

/// A single move entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveEntry {
    pub start: Snowflake,
    pub end: Snowflake,
    pub summary: String,
}

/// Append a move to the JSONL file
pub async fn append_move(
    path: &Path,
    start: Snowflake,
    end: Snowflake,
    summary: &str,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }

    let entry = MoveEntry {
        start,
        end,
        summary: summary.to_string(),
    };

    let mut json = serde_json::to_string(&entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    json.push('\n');

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;

    file.write_all(json.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

/// Read all moves from the JSONL file, skipping malformed lines
pub async fn read_moves(path: &Path) -> Vec<MoveEntry> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<MoveEntry>(l).ok())
        .collect()
}

/// Read the last N moves
pub async fn read_moves_tail(path: &Path, n: usize) -> Vec<MoveEntry> {
    let all = read_moves(path).await;
    let start = all.len().saturating_sub(n);
    all[start..].to_vec()
}

/// Read the cursor — the `end` snowflake of the last move
pub async fn read_cursor(path: &Path) -> Option<Snowflake> {
    let moves = read_moves(path).await;
    moves.last().map(|m| m.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, Snowflake, SnowflakeType};

    fn test_snowflake() -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(0, birth, SnowflakeType::Message, 0)
    }

    fn test_snowflake_seq(seq: u32) -> Snowflake {
        let birth = AgentBirth::new(2026, 5, 14, 12, 0, 0).unwrap();
        Snowflake::new(seq as u64 * 1_000_000, birth, SnowflakeType::Message, seq)
    }

    use tempfile::TempDir;

    #[tokio::test]
    async fn test_append_and_read_moves() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        append_move(&path, test_snowflake_seq(1), test_snowflake_seq(2), "The agent set up the project.").await.unwrap();
        append_move(&path, test_snowflake_seq(3), test_snowflake_seq(4), "The user asked about auth.").await.unwrap();

        let moves = read_moves(&path).await;
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].start, test_snowflake_seq(1));
        assert_eq!(moves[0].summary, "The agent set up the project.");
        assert_eq!(moves[1].start, test_snowflake_seq(3));
    }

    #[tokio::test]
    async fn test_read_moves_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");
        let moves = read_moves(&path).await;
        assert!(moves.is_empty());
    }

    #[tokio::test]
    async fn test_read_moves_tail() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        for i in 0..20 {
            append_move(&path, test_snowflake_seq(i * 2), test_snowflake_seq(i * 2 + 1), &format!("Move {}", i)).await.unwrap();
        }

        let tail = read_moves_tail(&path, 5).await;
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0].summary, "Move 15");
        assert_eq!(tail[4].summary, "Move 19");
    }

    #[tokio::test]
    async fn test_read_cursor_from_moves() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        assert_eq!(read_cursor(&path).await, None);

        append_move(&path, test_snowflake_seq(1), test_snowflake_seq(2), "First move.").await.unwrap();
        assert_eq!(read_cursor(&path).await, Some(test_snowflake_seq(2)));

        append_move(&path, test_snowflake_seq(3), test_snowflake_seq(4), "Second move.").await.unwrap();
        assert_eq!(read_cursor(&path).await, Some(test_snowflake_seq(4)));
    }

    #[tokio::test]
    async fn test_read_moves_skips_malformed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("moves.jsonl");

        append_move(&path, test_snowflake_seq(1), test_snowflake_seq(2), "Good move.").await.unwrap();
        // Write a malformed line
        let mut f = tokio::fs::OpenOptions::new().append(true).open(&path).await.unwrap();
        f.write_all(b"{bad json\n").await.unwrap();
        append_move(&path, test_snowflake_seq(3), test_snowflake_seq(4), "Another good move.").await.unwrap();

        let moves = read_moves(&path).await;
        assert_eq!(moves.len(), 2); // malformed line skipped
    }
}
