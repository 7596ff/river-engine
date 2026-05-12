//! Home channel log writer — serialized writes for ordering guarantees

use super::entry::HomeChannelEntry;
use super::log::ChannelLog;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, info};

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::entry::{MessageEntry, ToolEntry, HeartbeatEntry};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_writer_appends_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test-home.jsonl");
        let writer = HomeChannelWriter::spawn(path.clone());

        // Write a message
        let msg = MessageEntry::agent("001".into(), "hello".into(), "home".into(), None);
        writer.write(HomeChannelEntry::Message(msg)).await;

        // Write a tool call
        let tool = ToolEntry::call(
            "002".into(), "bash".into(),
            serde_json::json!({"cmd": "ls"}), "tc1".into(),
        );
        writer.write(HomeChannelEntry::Tool(tool)).await;

        // Write a heartbeat
        let hb = HeartbeatEntry::new("003".into(), "2026-05-12T12:00:00Z".into());
        writer.write(HomeChannelEntry::Heartbeat(hb)).await;

        // Shutdown and wait for writes to flush
        writer.shutdown().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Read back
        let log = ChannelLog::from_path(path);
        let entries = log.read_all_home().await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id(), "001");
        assert_eq!(entries[1].id(), "002");
        assert_eq!(entries[2].id(), "003");
    }

    #[tokio::test]
    async fn test_writer_preserves_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("order-test.jsonl");
        let writer = HomeChannelWriter::spawn(path.clone());

        // Write 100 entries rapidly
        for i in 0..100 {
            let msg = MessageEntry::agent(
                format!("{:03}", i), format!("msg {}", i), "home".into(), None,
            );
            writer.write(HomeChannelEntry::Message(msg)).await;
        }

        writer.shutdown().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let log = ChannelLog::from_path(path);
        let entries = log.read_all_home().await.unwrap();
        assert_eq!(entries.len(), 100);
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.id(), format!("{:03}", i));
        }
    }
}
