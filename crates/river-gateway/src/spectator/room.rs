//! Room notes — the spectator's witness testimony
//!
//! Room notes are per-session observations written by the spectator.
//! They provide a running commentary on the agent's processing quality,
//! patterns, and potential issues.

use crate::r#loop::ModelClient;
use chrono::Utc;
use std::path::PathBuf;

/// Writes room notes for witness observations
pub struct RoomWriter {
    room_notes_dir: PathBuf,
}

impl RoomWriter {
    pub fn new(room_notes_dir: PathBuf) -> Self {
        Self { room_notes_dir }
    }

    /// Get the room notes directory
    pub fn dir(&self) -> &PathBuf {
        &self.room_notes_dir
    }

    /// Get the path for today's session file
    pub fn session_path(&self) -> PathBuf {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.room_notes_dir.join(format!("{}-session.md", today))
    }

    /// Write an observation for a turn
    pub async fn write_observation(
        &self,
        turn_number: u64,
        transcript_summary: &str,
        _model_client: &ModelClient,
        _spectator_identity: &str,
    ) -> Result<(), String> {
        // Ensure directory exists
        tokio::fs::create_dir_all(&self.room_notes_dir).await
            .map_err(|e| format!("Failed to create room notes directory: {}", e))?;

        let session_path = self.session_path();
        let now = Utc::now();
        let today = now.format("%Y-%m-%d").to_string();

        // Load or create session file
        let mut content = tokio::fs::read_to_string(&session_path).await
            .unwrap_or_else(|_| format!(
                "---\nid: room-{}\ncreated: {}\nauthor: spectator\ntype: room-note\n---\n\n## Session {}\n",
                now.timestamp(),
                now.to_rfc3339(),
                today
            ));

        // Analyze the turn for notable patterns
        let observation = self.generate_observation(turn_number, transcript_summary);

        content.push_str(&observation);

        tokio::fs::write(&session_path, &content).await
            .map_err(|e| format!("Failed to write room note: {}", e))?;

        tracing::debug!(turn = turn_number, "Room note written");
        Ok(())
    }

    /// Generate an observation for a turn
    fn generate_observation(&self, turn_number: u64, transcript_summary: &str) -> String {
        let time = Utc::now().format("%H:%M:%S").to_string();

        // Truncate summary for display
        let summary_display = if transcript_summary.len() > 150 {
            format!("{}...", &transcript_summary[..150].replace('\n', " "))
        } else {
            transcript_summary.replace('\n', " ")
        };

        // Detect patterns worth noting
        let mut notes = Vec::new();

        if transcript_summary.contains("error") || transcript_summary.contains("failed") {
            notes.push("Recovery pattern observed");
        }
        if transcript_summary.contains("heartbeat") {
            notes.push("Idle turn (heartbeat)");
        }
        if transcript_summary.len() > 500 {
            notes.push("Dense turn (long summary)");
        }
        if transcript_summary.matches("tool").count() > 5 {
            notes.push("High tool activity");
        }

        let notes_str = if notes.is_empty() {
            String::new()
        } else {
            format!("\n  - Notes: {}", notes.join(", "))
        };

        format!(
            "\n### Turn {} ({})\n- Summary: {}{}\n",
            turn_number,
            time,
            summary_display,
            notes_str
        )
    }

    /// Write a custom observation (not tied to a turn)
    pub async fn write_custom(
        &self,
        title: &str,
        content: &str,
    ) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.room_notes_dir).await
            .map_err(|e| format!("Failed to create room notes directory: {}", e))?;

        let session_path = self.session_path();
        let time = Utc::now().format("%H:%M:%S").to_string();
        let now = Utc::now();
        let today = now.format("%Y-%m-%d").to_string();

        let mut existing = tokio::fs::read_to_string(&session_path).await
            .unwrap_or_else(|_| format!(
                "---\nid: room-{}\ncreated: {}\nauthor: spectator\ntype: room-note\n---\n\n## Session {}\n",
                now.timestamp(),
                now.to_rfc3339(),
                today
            ));

        let note = format!(
            "\n### {} ({})\n{}\n",
            title,
            time,
            content
        );

        existing.push_str(&note);

        tokio::fs::write(&session_path, &existing).await
            .map_err(|e| format!("Failed to write custom room note: {}", e))?;

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
    async fn test_write_observation_creates_file() {
        let temp = TempDir::new().unwrap();
        let writer = RoomWriter::new(temp.path().to_path_buf());
        let model = test_model_client();

        let result = writer.write_observation(
            1,
            "User asked about the weather, agent responded with forecast",
            &model,
            "spectator identity",
        ).await;

        assert!(result.is_ok());
        assert!(writer.session_path().exists());
    }

    #[tokio::test]
    async fn test_write_observation_appends() {
        let temp = TempDir::new().unwrap();
        let writer = RoomWriter::new(temp.path().to_path_buf());
        let model = test_model_client();

        writer.write_observation(1, "First turn", &model, "").await.unwrap();
        writer.write_observation(2, "Second turn", &model, "").await.unwrap();

        let content = tokio::fs::read_to_string(writer.session_path()).await.unwrap();
        assert!(content.contains("Turn 1"));
        assert!(content.contains("Turn 2"));
    }

    #[tokio::test]
    async fn test_observation_detects_patterns() {
        let temp = TempDir::new().unwrap();
        let writer = RoomWriter::new(temp.path().to_path_buf());
        let model = test_model_client();

        writer.write_observation(1, "An error occurred during processing", &model, "").await.unwrap();

        let content = tokio::fs::read_to_string(writer.session_path()).await.unwrap();
        assert!(content.contains("Recovery pattern"));
    }

    #[tokio::test]
    async fn test_write_custom() {
        let temp = TempDir::new().unwrap();
        let writer = RoomWriter::new(temp.path().to_path_buf());

        writer.write_custom("Pattern Detected", "Repeated questioning observed").await.unwrap();

        let content = tokio::fs::read_to_string(writer.session_path()).await.unwrap();
        assert!(content.contains("Pattern Detected"));
        assert!(content.contains("Repeated questioning"));
    }

    #[test]
    fn test_generate_observation() {
        let temp = TempDir::new().unwrap();
        let writer = RoomWriter::new(temp.path().to_path_buf());

        let obs = writer.generate_observation(5, "Normal processing turn");
        assert!(obs.contains("Turn 5"));
        assert!(obs.contains("Normal processing"));
    }

    #[test]
    fn test_generate_observation_truncates_long_summary() {
        let temp = TempDir::new().unwrap();
        let writer = RoomWriter::new(temp.path().to_path_buf());

        let long_summary = "x".repeat(200);
        let obs = writer.generate_observation(1, &long_summary);
        assert!(obs.contains("..."));
    }
}
