//! Home channel log writer — serialized writes for ordering guarantees

use super::entry::HomeChannelEntry;
use super::log::ChannelLog;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub enum LogWriteRequest {
    Append(HomeChannelEntry),
    Shutdown,
}

#[derive(Clone)]
pub struct HomeChannelWriter {
    tx: mpsc::Sender<LogWriteRequest>,
}

impl HomeChannelWriter {
    pub fn spawn(home_channel_path: PathBuf) -> Self {
        let (tx, mut rx) = mpsc::channel::<LogWriteRequest>(1024);

        tokio::spawn(async move {
            let log = ChannelLog::from_path(home_channel_path);
            info!("Home channel writer started");

            while let Some(req) = rx.recv().await {
                match req {
                    LogWriteRequest::Append(entry) => {
                        if let Err(e) = log.append_entry(&entry).await {
                            error!(error = %e, "Failed to write to home channel");
                        }
                    }
                    LogWriteRequest::Shutdown => {
                        info!("Home channel writer shutting down");
                        break;
                    }
                }
            }
        });

        Self { tx }
    }

    pub async fn write(&self, entry: HomeChannelEntry) {
        if let Err(e) = self.tx.send(LogWriteRequest::Append(entry)).await {
            error!(error = %e, "Failed to send to home channel writer");
        }
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(LogWriteRequest::Shutdown).await;
    }

    /// Clean up tool result files in a snowflake range after a move supersedes them.
    /// Reads the home channel, finds ToolEntry entries with result_file in the range,
    /// and deletes the files.
    pub async fn cleanup_tool_results(
        home_channel_path: &Path,
        move_start: river_core::Snowflake,
        move_end: river_core::Snowflake,
    ) {
        let log = ChannelLog::from_path(home_channel_path.to_path_buf());
        let entries = match log.read_all_home().await {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "Failed to read home channel for cleanup");
                return;
            }
        };

        let mut cleaned = 0;
        for entry in &entries {
            if let HomeChannelEntry::Tool(t) = entry {
                // Check if this entry's ID is in the snowflake range
                if t.id >= move_start && t.id <= move_end {
                    if let Some(ref file_path) = t.result_file {
                        match tokio::fs::remove_file(file_path).await {
                            Ok(()) => cleaned += 1,
                            Err(e) => {
                                // File may already be gone — not an error
                                if e.kind() != std::io::ErrorKind::NotFound {
                                    warn!(path = %file_path, error = %e, "Failed to clean up tool result file");
                                }
                            }
                        }
                    }
                }
            }
        }

        if cleaned > 0 {
            info!(cleaned, move_start = %move_start, move_end = %move_end, "Cleaned up tool result files");
        }
    }
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

    use crate::channels::entry::{HeartbeatEntry, MessageEntry, ToolEntry};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_writer_appends_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test-home.jsonl");
        let writer = HomeChannelWriter::spawn(path.clone());

        // Write a message
        let msg = MessageEntry::agent(test_snowflake_seq(1), "hello".into(), "home".into(), None);
        writer.write(HomeChannelEntry::Message(msg)).await;

        // Write a tool call
        let tool = ToolEntry::call(
            test_snowflake_seq(2),
            "bash".into(),
            serde_json::json!({"cmd": "ls"}),
            "tc1".into(),
        );
        writer.write(HomeChannelEntry::Tool(tool)).await;

        // Write a heartbeat
        let hb = HeartbeatEntry::new(test_snowflake_seq(3), "2026-05-12T12:00:00Z".into());
        writer.write(HomeChannelEntry::Heartbeat(hb)).await;

        // Shutdown and wait for writes to flush
        writer.shutdown().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Read back
        let log = ChannelLog::from_path(path);
        let entries = log.read_all_home().await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id(), test_snowflake_seq(1));
        assert_eq!(entries[1].id(), test_snowflake_seq(2));
        assert_eq!(entries[2].id(), test_snowflake_seq(3));
    }

    #[tokio::test]
    async fn test_writer_preserves_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("order-test.jsonl");
        let writer = HomeChannelWriter::spawn(path.clone());

        // Write 100 entries rapidly
        for i in 0..100 {
            let msg = MessageEntry::agent(
                test_snowflake_seq(i as u32),
                format!("msg {}", i),
                "home".into(),
                None,
            );
            writer.write(HomeChannelEntry::Message(msg)).await;
        }

        writer.shutdown().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let log = ChannelLog::from_path(path);
        let entries = log.read_all_home().await.unwrap();
        assert_eq!(entries.len(), 100);
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.id(), test_snowflake_seq(i as u32));
        }
    }
}
