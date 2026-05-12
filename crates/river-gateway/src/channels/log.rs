//! Channel log — JSONL file operations
//!
//! One JSONL file per channel at channels/{adapter}_{channel_id}.jsonl
//! Handles append, read-from-cursor, and malformed line skipping.
//!
//! Uses tokio::fs for async I/O — this module is called from async contexts
//! (agent task, HTTP handler) and must not block the executor.

use super::entry::{ChannelEntry, HomeChannelEntry};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Manages a single channel's JSONL log file
pub struct ChannelLog {
    path: PathBuf,
}

/// Sanitize a string for use in a filename — alphanumeric, dash, underscore only
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

impl ChannelLog {
    /// Open a channel log at the standard path: {channels_dir}/{adapter}_{channel_id}.jsonl
    pub fn open(channels_dir: &Path, adapter: &str, channel_id: &str) -> Self {
        let filename = format!("{}_{}.jsonl", sanitize(adapter), sanitize(channel_id));
        Self {
            path: channels_dir.join(filename),
        }
    }

    /// Open a channel log at an explicit path (for testing)
    pub fn from_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Append a serialized entry as a single JSONL line
    pub async fn append_entry(&self, entry: &impl serde::Serialize) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }

        let mut json = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        json.push('\n');

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;

        // Single write_all for atomic-like behavior — avoids corrupted entries
        // if the process crashes between separate write calls
        file.write_all(json.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Read all entries from the log, skipping malformed lines
    pub async fn read_all(&self) -> std::io::Result<Vec<ChannelEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = tokio::fs::File::open(&self.path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut entries = Vec::new();
        let mut line_num = 0usize;

        while let Some(line) = lines.next_line().await? {
            line_num += 1;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ChannelEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!(
                        line_num = line_num,
                        error = %e,
                        path = %self.path.display(),
                        "Skipping malformed JSONL line"
                    );
                }
            }
        }

        Ok(entries)
    }

    /// Read all entries from a home channel log (tagged serde), skipping malformed lines
    pub async fn read_all_home(&self) -> std::io::Result<Vec<HomeChannelEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = tokio::fs::File::open(&self.path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut entries = Vec::new();
        let mut line_num = 0usize;

        while let Some(line) = lines.next_line().await? {
            line_num += 1;
            if line.trim().is_empty() { continue; }
            match serde_json::from_str::<HomeChannelEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!(line_num, error = %e, path = %self.path.display(), "Skipping malformed home channel line");
                }
            }
        }
        Ok(entries)
    }

    /// Read home channel entries after a given snowflake ID.
    /// Entries are compared lexicographically (bare hex snowflakes sort correctly).
    pub async fn read_home_since(&self, after_id: &str) -> std::io::Result<Vec<HomeChannelEntry>> {
        let all = self.read_all_home().await?;
        Ok(all.into_iter().filter(|e| e.id() > after_id).collect())
    }

    /// Read home channel entries after a given snowflake ID, or all entries if None.
    pub async fn read_home_since_opt(&self, after_id: Option<&str>) -> std::io::Result<Vec<HomeChannelEntry>> {
        match after_id {
            Some(id) => self.read_home_since(id).await,
            None => self.read_all_home().await,
        }
    }

    /// Read new entries since the agent's last cursor position.
    ///
    /// Scans backward for the last role:agent entry, returns everything after it.
    /// If no agent entry exists, returns the last `default_window` entries.
    pub async fn read_since_cursor(&self, default_window: usize) -> std::io::Result<Vec<ChannelEntry>> {
        let all = self.read_all().await?;

        // Find the last agent entry (message or cursor)
        let last_agent_idx = all.iter().rposition(|e| e.is_agent());

        match last_agent_idx {
            Some(idx) => {
                // Return everything after the cursor
                Ok(all[idx + 1..].to_vec())
            }
            None => {
                // No cursor — return last N entries
                let start = all.len().saturating_sub(default_window);
                Ok(all[start..].to_vec())
            }
        }
    }

    /// Get the path to this channel log
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::entry::{CursorEntry, MessageEntry};
    use tempfile::TempDir;

    fn test_log(dir: &TempDir) -> ChannelLog {
        ChannelLog::open(dir.path(), "discord", "general")
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("discord"), "discord");
        assert_eq!(sanitize("my-channel"), "my-channel");
        assert_eq!(sanitize("guild/channel"), "guild_channel");
        assert_eq!(sanitize("a:b:c"), "a_b_c");
    }

    #[test]
    fn test_channel_log_path() {
        let dir = TempDir::new().unwrap();
        let log = ChannelLog::open(dir.path(), "discord", "general");
        assert!(log.path().ends_with("discord_general.jsonl"));
    }

    #[tokio::test]
    async fn test_append_and_read() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        let entry = MessageEntry::incoming(
            "001".to_string(),
            "cassie".to_string(),
            "u1".to_string(),
            "hello".to_string(),
            "discord".to_string(),
            None,
        );
        log.append_entry(&entry).await.unwrap();

        let entries = log.read_all().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_agent());
    }

    #[tokio::test]
    async fn test_read_empty_log() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);
        let entries = log.read_all().await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_read_since_cursor_with_agent_message() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        // 3 messages from others, then agent speaks, then 2 more from others
        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(), "hi".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "002".into(), "bob".into(), "b1".into(), "hey".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "carol".into(), "c1".into(), "sup".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::agent(
            "004".into(), "hello everyone".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "005".into(), "alice".into(), "a1".into(), "nice".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "006".into(), "bob".into(), "b1".into(), "cool".into(), "discord".into(), None,
        )).await.unwrap();

        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id(), "005");
        assert_eq!(new[1].id(), "006");
    }

    #[tokio::test]
    async fn test_read_since_cursor_with_cursor_entry() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(), "hi".into(), "discord".into(), None,
        )).await.unwrap();
        log.append_entry(&CursorEntry::new("002".into())).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "bob".into(), "b1".into(), "hey".into(), "discord".into(), None,
        )).await.unwrap();

        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id(), "003");
    }

    #[tokio::test]
    async fn test_read_since_cursor_no_agent_entry() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        // 100 messages, no agent entry
        for i in 0..100 {
            log.append_entry(&MessageEntry::incoming(
                format!("{:03}", i), "user".into(), "u1".into(),
                format!("msg {}", i), "discord".into(), None,
            )).await.unwrap();
        }

        // Default window of 50
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 50);
        assert_eq!(new[0].id(), "050");
        assert_eq!(new[49].id(), "099");
    }

    #[tokio::test]
    async fn test_malformed_line_skipped() {
        let dir = TempDir::new().unwrap();
        let log = test_log(&dir);

        // Write a valid entry
        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(), "hi".into(), "discord".into(), None,
        )).await.unwrap();

        // Write a malformed line directly using std::fs (sync, for test setup)
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(log.path()).unwrap();
        writeln!(file, "{{this is not valid json").unwrap();

        // Write another valid entry
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "bob".into(), "b1".into(), "hey".into(), "discord".into(), None,
        )).await.unwrap();

        let entries = log.read_all().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id(), "001");
        assert_eq!(entries[1].id(), "003");
    }

    #[tokio::test]
    async fn test_full_flow_incoming_cursor_read() {
        let dir = TempDir::new().unwrap();
        let log = ChannelLog::open(dir.path(), "discord", "general");

        // Simulate: 3 messages arrive, agent reads (cursor), 2 more arrive
        log.append_entry(&MessageEntry::incoming(
            "001".into(), "alice".into(), "a1".into(),
            "hello".into(), "discord".into(), Some("d001".into()),
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "002".into(), "bob".into(), "b1".into(),
            "hi there".into(), "discord".into(), Some("d002".into()),
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "003".into(), "carol".into(), "c1".into(),
            "hey all".into(), "discord".into(), Some("d003".into()),
        )).await.unwrap();

        // Agent reads — should get all 3 (no prior cursor)
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 3);

        // Agent speaks — implicit cursor
        log.append_entry(&MessageEntry::agent(
            "004".into(), "hello everyone!".into(),
            "discord".into(), Some("d004".into()),
        )).await.unwrap();

        // Two more messages arrive
        log.append_entry(&MessageEntry::incoming(
            "005".into(), "alice".into(), "a1".into(),
            "how are you?".into(), "discord".into(), Some("d005".into()),
        )).await.unwrap();
        log.append_entry(&MessageEntry::incoming(
            "006".into(), "bob".into(), "b1".into(),
            "doing great".into(), "discord".into(), Some("d006".into()),
        )).await.unwrap();

        // Agent reads again — should only get the 2 new messages
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id(), "005");
        assert_eq!(new[1].id(), "006");

        // Agent reads but doesn't speak — writes cursor
        log.append_entry(&CursorEntry::new("007".into())).await.unwrap();

        // One more message
        log.append_entry(&MessageEntry::incoming(
            "008".into(), "carol".into(), "c1".into(),
            "late message".into(), "discord".into(), Some("d008".into()),
        )).await.unwrap();

        // Agent reads — should get 1 new message (after cursor)
        let new = log.read_since_cursor(50).await.unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id(), "008");
    }

    #[tokio::test]
    async fn test_read_home_since() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("home.jsonl");
        let log = ChannelLog::from_path(path);

        use super::super::entry::{HomeChannelEntry, MessageEntry};

        for i in 0..5 {
            let entry = HomeChannelEntry::Message(MessageEntry::agent(
                format!("{:032x}", i), format!("msg {}", i), "home".into(), None,
            ));
            log.append_entry(&entry).await.unwrap();
        }

        // Read since entry 1 (should get entries 2, 3, 4)
        let after_id = format!("{:032x}", 1);
        let entries = log.read_home_since(&after_id).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id(), format!("{:032x}", 2));
        assert_eq!(entries[2].id(), format!("{:032x}", 4));
    }

    #[tokio::test]
    async fn test_read_home_since_opt_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("home.jsonl");
        let log = ChannelLog::from_path(path);

        use super::super::entry::{HomeChannelEntry, MessageEntry};

        for i in 0..3 {
            let entry = HomeChannelEntry::Message(MessageEntry::agent(
                format!("{:032x}", i), format!("msg {}", i), "home".into(), None,
            ));
            log.append_entry(&entry).await.unwrap();
        }

        // Read since None (should get all entries)
        let entries = log.read_home_since_opt(None).await.unwrap();
        assert_eq!(entries.len(), 3);
    }
}
