//! Compression: moves and moments generation
//!
//! The compressor generates structural summaries of conversations:
//! - Moves: per-turn structural notes (type of exchange)
//! - Moments: arc compression (multiple moves into a narrative beat)

use crate::r#loop::ModelClient;
use chrono::Utc;
use std::path::PathBuf;

/// Compressor generates structural summaries of conversations
pub struct Compressor {
    embeddings_dir: PathBuf,
}

impl Compressor {
    pub fn new(embeddings_dir: PathBuf) -> Self {
        Self { embeddings_dir }
    }

    /// Get the moves directory path
    pub fn moves_dir(&self) -> PathBuf {
        self.embeddings_dir.join("moves")
    }

    /// Get the moments directory path
    pub fn moments_dir(&self) -> PathBuf {
        self.embeddings_dir.join("moments")
    }

    /// Update moves file for a channel after a turn
    pub async fn update_moves(
        &self,
        channel: &str,
        turn_number: u64,
        transcript_summary: &str,
        tool_calls: &[String],
        _model_client: &ModelClient,
        _spectator_identity: &str,
    ) -> Result<(), String> {
        let sanitized = channel.replace(['/', '\\', ' '], "-");
        let moves_dir = self.moves_dir();
        let moves_path = moves_dir.join(format!("{}.md", sanitized));

        // Ensure directory exists
        tokio::fs::create_dir_all(&moves_dir).await
            .map_err(|e| format!("Failed to create moves directory: {}", e))?;

        // Load existing moves
        let existing = tokio::fs::read_to_string(&moves_path).await.unwrap_or_default();

        // Classify the move type based on tool calls and summary
        let move_type = self.classify_move(transcript_summary, tool_calls);

        // Generate new move line
        let summary_truncated = if transcript_summary.len() > 80 {
            format!("{}...", &transcript_summary[..80])
        } else {
            transcript_summary.to_string()
        };

        let new_move = format!(
            "Move {}: [{}] {}\n",
            turn_number,
            move_type,
            summary_truncated.replace('\n', " ")
        );

        // Append to moves file
        let updated = format!("{}{}", existing, new_move);

        tokio::fs::write(&moves_path, &updated).await
            .map_err(|e| format!("Failed to write moves: {}", e))?;

        // Check if we should compress into a moment
        let move_count = updated.lines().filter(|l| l.starts_with("Move ")).count();
        if move_count >= 15 {
            tracing::info!(
                channel = %channel,
                moves = move_count,
                "Moves threshold reached - consider compressing into moment"
            );
        }

        tracing::debug!(
            channel = %channel,
            turn = turn_number,
            move_type = %move_type,
            "Move recorded"
        );

        Ok(())
    }

    /// Classify the type of move based on content
    fn classify_move(&self, summary: &str, tool_calls: &[String]) -> &'static str {
        let summary_lower = summary.to_lowercase();

        // Check tool usage patterns
        if tool_calls.iter().any(|t| t == "send_message") {
            return "response";
        }
        if tool_calls.iter().any(|t| t == "read" || t == "glob" || t == "grep") {
            return "exploration";
        }
        if tool_calls.iter().any(|t| t == "write" || t == "edit") {
            return "creation";
        }
        if tool_calls.iter().any(|t| t == "bash") {
            return "execution";
        }

        // Check content patterns
        if summary_lower.contains("question") || summary_lower.contains("?") {
            return "question";
        }
        if summary_lower.contains("decided") || summary_lower.contains("chose") {
            return "decision";
        }
        if summary_lower.contains("error") || summary_lower.contains("failed") {
            return "recovery";
        }
        if summary_lower.contains("wait") || summary_lower.contains("heartbeat") {
            return "pause";
        }

        "processing"
    }

    /// Compress a range of moves into a moment
    pub async fn create_moment(
        &self,
        channel: &str,
        moves_text: &str,
        _model_client: &ModelClient,
        _spectator_identity: &str,
    ) -> Result<String, String> {
        let moments_dir = self.moments_dir();
        tokio::fs::create_dir_all(&moments_dir).await
            .map_err(|e| format!("Failed to create moments directory: {}", e))?;

        let timestamp = Utc::now();
        let sanitized = channel.replace(['/', '\\', ' '], "-");

        // Generate moment with YAML frontmatter
        let moment = format!(
            "---\nid: moment-{}\ncreated: {}\nauthor: spectator\ntype: moment\nchannel: {}\n---\n\n## Arc Summary\n\n{}\n",
            timestamp.timestamp(),
            timestamp.to_rfc3339(),
            channel,
            moves_text
        );

        let moment_path = moments_dir.join(format!(
            "{}-{}.md",
            sanitized,
            timestamp.format("%Y%m%d%H%M")
        ));

        tokio::fs::write(&moment_path, &moment).await
            .map_err(|e| format!("Failed to write moment: {}", e))?;

        tracing::info!(
            channel = %channel,
            path = %moment_path.display(),
            "Moment created"
        );

        Ok(moment)
    }

    /// Clear old moves after creating a moment
    pub async fn archive_moves(&self, channel: &str) -> Result<(), String> {
        let sanitized = channel.replace(['/', '\\', ' '], "-");
        let moves_path = self.moves_dir().join(format!("{}.md", sanitized));

        if moves_path.exists() {
            // Archive to a dated file
            let archive_path = self.moves_dir().join(format!(
                "{}-archive-{}.md",
                sanitized,
                Utc::now().format("%Y%m%d%H%M")
            ));
            tokio::fs::rename(&moves_path, &archive_path).await
                .map_err(|e| format!("Failed to archive moves: {}", e))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::time::Duration;

    fn test_model_client() -> ModelClient {
        ModelClient::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
            Duration::from_secs(30),
        ).unwrap()
    }

    #[tokio::test]
    async fn test_update_moves_creates_file() {
        let temp = TempDir::new().unwrap();
        let compressor = Compressor::new(temp.path().to_path_buf());
        let model = test_model_client();

        let result = compressor.update_moves(
            "general",
            1,
            "User asked about the weather",
            &["send_message".to_string()],
            &model,
            "spectator identity",
        ).await;

        assert!(result.is_ok());

        let moves_path = temp.path().join("moves/general.md");
        assert!(moves_path.exists());

        let content = tokio::fs::read_to_string(&moves_path).await.unwrap();
        assert!(content.contains("Move 1:"));
        assert!(content.contains("[response]"));
    }

    #[tokio::test]
    async fn test_update_moves_appends() {
        let temp = TempDir::new().unwrap();
        let compressor = Compressor::new(temp.path().to_path_buf());
        let model = test_model_client();

        // First turn
        compressor.update_moves("general", 1, "First turn", &[], &model, "").await.unwrap();
        // Second turn
        compressor.update_moves("general", 2, "Second turn", &[], &model, "").await.unwrap();

        let moves_path = temp.path().join("moves/general.md");
        let content = tokio::fs::read_to_string(&moves_path).await.unwrap();

        assert!(content.contains("Move 1:"));
        assert!(content.contains("Move 2:"));
    }

    #[tokio::test]
    async fn test_classify_move_types() {
        let temp = TempDir::new().unwrap();
        let compressor = Compressor::new(temp.path().to_path_buf());

        assert_eq!(compressor.classify_move("test", &["send_message".to_string()]), "response");
        assert_eq!(compressor.classify_move("test", &["read".to_string()]), "exploration");
        assert_eq!(compressor.classify_move("test", &["write".to_string()]), "creation");
        assert_eq!(compressor.classify_move("test", &["bash".to_string()]), "execution");
        assert_eq!(compressor.classify_move("What is this?", &[]), "question");
        assert_eq!(compressor.classify_move("error occurred", &[]), "recovery");
        assert_eq!(compressor.classify_move("just processing", &[]), "processing");
    }

    #[tokio::test]
    async fn test_create_moment() {
        let temp = TempDir::new().unwrap();
        let compressor = Compressor::new(temp.path().to_path_buf());
        let model = test_model_client();

        let moves = "Move 1: [question] User asked about X\nMove 2: [response] Answered";
        let result = compressor.create_moment("general", moves, &model, "").await;

        assert!(result.is_ok());

        let moments_dir = temp.path().join("moments");
        assert!(moments_dir.exists());

        let entries: Vec<_> = std::fs::read_dir(&moments_dir).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_moves_dir() {
        let compressor = Compressor::new(PathBuf::from("/workspace/embeddings"));
        assert_eq!(compressor.moves_dir(), PathBuf::from("/workspace/embeddings/moves"));
    }

    #[test]
    fn test_moments_dir() {
        let compressor = Compressor::new(PathBuf::from("/workspace/embeddings"));
        assert_eq!(compressor.moments_dir(), PathBuf::from("/workspace/embeddings/moments"));
    }
}
