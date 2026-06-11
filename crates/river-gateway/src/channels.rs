//! The channel layer (wall ch. 05): one append-only JSONL log per
//! channel — the engine's communication ground truth — plus the
//! notification queue that wakes the agent. Write-then-notify is the
//! binding order: an entry is durably on disk before its pointer is
//! queued, so the agent never wakes to find missing data.
//!
//! All writes go through one `Channels` handle (lock-serialized per
//! the single-writer invariant, wall ch. 10); reads scan fresh.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

pub const NEVER_VISITED_TAIL: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryRole {
    Agent,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelEntry {
    pub id: String,
    pub role: EntryRole,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub msg_id: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub cursor: bool,
    /// Explicit cursors point at the last entry actually consumed.
    /// Without this, a cursor appended at settle would falsely cover
    /// entries that arrived (unread) during the turn's final model
    /// call.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub up_to: Option<String>,
}

/// A queued wake pointer. Pointers, never payloads.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub channel: String,
    pub ulid: String,
}

#[derive(Clone)]
pub struct Channels {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    files: Mutex<HashMap<String, File>>,
    notify: mpsc::Sender<Notification>,
}

impl Channels {
    pub fn open(workspace: &Path, notify: mpsc::Sender<Notification>) -> anyhow::Result<Self> {
        let dir = workspace.join("channels");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                dir,
                files: Mutex::new(HashMap::new()),
                notify,
            }),
        })
    }

    /// Inbound from an adapter: append to the log, then push the
    /// pointer. If the write fails the queue is never touched and the
    /// error goes back to the adapter.
    pub async fn inbound(
        &self,
        channel: &str,
        author: &str,
        author_id: Option<&str>,
        content: &str,
        adapter: &str,
        msg_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let entry = ChannelEntry {
            id: ulid::Ulid::new().to_string(),
            role: EntryRole::Other,
            author: Some(author.to_string()),
            author_id: author_id.map(str::to_string),
            content: Some(content.to_string()),
            adapter: Some(adapter.to_string()),
            msg_id: msg_id.map(str::to_string),
            cursor: false,
            up_to: None,
        };
        let ulid = entry.id.clone();
        self.append(channel, &entry)?; // ← must succeed first
        self.inner
            .notify
            .send(Notification {
                channel: channel.to_string(),
                ulid: ulid.clone(),
            })
            .await
            .map_err(|_| anyhow::anyhow!("notification queue closed"))?;
        Ok(ulid)
    }

    /// Outbound, logged after the platform accepted delivery. The
    /// entry doubles as the cursor.
    pub fn agent_spoke(
        &self,
        channel: &str,
        content: &str,
        adapter: &str,
        msg_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let entry = ChannelEntry {
            id: ulid::Ulid::new().to_string(),
            role: EntryRole::Agent,
            author: None,
            author_id: None,
            content: Some(content.to_string()),
            adapter: Some(adapter.to_string()),
            msg_id: msg_id.map(str::to_string),
            cursor: false,
            up_to: None,
        };
        let ulid = entry.id.clone();
        self.append(channel, &entry)?;
        Ok(ulid)
    }

    /// Explicit cursor: "I read to `up_to`" without speaking. Written
    /// at settle for every channel read this turn (wall ch. 01).
    /// Entries after `up_to` stay unread even though the cursor entry
    /// itself sits later in the log.
    pub fn mark_read(&self, channel: &str, up_to: &str) -> anyhow::Result<String> {
        let entry = ChannelEntry {
            id: ulid::Ulid::new().to_string(),
            role: EntryRole::Agent,
            author: None,
            author_id: None,
            content: None,
            adapter: None,
            msg_id: None,
            cursor: true,
            up_to: Some(up_to.to_string()),
        };
        let ulid = entry.id.clone();
        self.append(channel, &entry)?;
        Ok(ulid)
    }

    /// Everything after the agent's read position. The position is
    /// the last agent entry — or, for an explicit cursor, the entry
    /// it points at. Never visited → the last 50 entries.
    pub fn read_since_cursor(&self, channel: &str) -> anyhow::Result<Vec<ChannelEntry>> {
        let entries = self.scan(channel)?;
        let Some(agent_pos) = entries.iter().rposition(|e| e.role == EntryRole::Agent) else {
            let start = entries.len().saturating_sub(NEVER_VISITED_TAIL);
            return Ok(entries[start..].to_vec());
        };
        let position = match &entries[agent_pos].up_to {
            Some(target_id) => entries
                .iter()
                .position(|e| &e.id == target_id)
                .unwrap_or(agent_pos),
            None => agent_pos,
        };
        Ok(entries[position + 1..]
            .iter()
            .filter(|e| e.role == EntryRole::Other)
            .cloned()
            .collect())
    }

    /// The log position of the agent's read cursor vs. a given entry:
    /// true when the agent's last agent-entry already covers it.
    pub fn covered(&self, channel: &str, entry_id: &str) -> anyhow::Result<bool> {
        let entries = self.scan(channel)?;
        let Some(agent_pos) = entries.iter().rposition(|e| e.role == EntryRole::Agent) else {
            return Ok(false);
        };
        let position = match &entries[agent_pos].up_to {
            Some(target_id) => entries
                .iter()
                .position(|e| &e.id == target_id)
                .unwrap_or(agent_pos),
            None => agent_pos,
        };
        let Some(entry_pos) = entries.iter().position(|e| e.id == entry_id) else {
            return Ok(false);
        };
        Ok(position >= entry_pos)
    }

    /// Full log scan, torn lines skipped with a warning.
    pub fn scan(&self, channel: &str) -> anyhow::Result<Vec<ChannelEntry>> {
        let path = self.path(channel);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut entries = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ChannelEntry>(raw) {
                Ok(entry) => entries.push(entry),
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    line = line_no + 1,
                    error = %e,
                    "skipping malformed channel entry"
                ),
            }
        }
        Ok(entries)
    }

    pub fn path(&self, channel: &str) -> PathBuf {
        self.inner.dir.join(format!("{}.jsonl", sanitize(channel)))
    }

    fn append(&self, channel: &str, entry: &ChannelEntry) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(entry)?;
        json.push('\n');
        let mut files = self.inner.files.lock().expect("channel files lock");
        let file = match files.entry(channel.to_string()) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let path = self.inner.dir.join(format!("{}.jsonl", sanitize(channel)));
                let file = OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&path)
                    .with_context(|| format!("opening {}", path.display()))?;
                v.insert(file)
            }
        };
        file.write_all(json.as_bytes())
            .with_context(|| format!("appending to channel {channel}"))?;
        file.sync_data()
            .with_context(|| format!("fsyncing channel {channel}"))?;
        Ok(())
    }
}

/// Compose an engine channel name from adapter + platform channel id.
pub fn channel_name(adapter: &str, channel_id: &str) -> String {
    sanitize(&format!("{adapter}_{channel_id}"))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn layer() -> (Channels, mpsc::Receiver<Notification>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        // Roomy: tests push many entries without draining, and a full
        // queue blocks inbound by design (backpressure on adapters).
        let (tx, rx) = mpsc::channel(256);
        let channels = Channels::open(dir.path(), tx).unwrap();
        (channels, rx, dir)
    }

    #[tokio::test]
    async fn inbound_writes_then_notifies() {
        let (channels, mut rx, _dir) = layer().await;
        let ulid = channels
            .inbound("local_main", "cass", Some("1"), "hello", "local", None)
            .await
            .unwrap();

        let note = rx.try_recv().unwrap();
        assert_eq!(note, Notification { channel: "local_main".into(), ulid: ulid.clone() });

        let entries = channels.scan("local_main").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, ulid);
        assert_eq!(entries[0].role, EntryRole::Other);
        assert_eq!(entries[0].author.as_deref(), Some("cass"));
        assert_eq!(entries[0].content.as_deref(), Some("hello"));
        assert!(!entries[0].cursor);
    }

    #[tokio::test]
    async fn read_since_cursor_after_speak() {
        let (channels, _rx, _dir) = layer().await;
        channels
            .inbound("c", "cass", None, "one", "local", None)
            .await
            .unwrap();
        channels.agent_spoke("c", "reply", "local", Some("m1")).unwrap();
        channels
            .inbound("c", "cass", None, "two", "local", None)
            .await
            .unwrap();
        channels
            .inbound("c", "cass", None, "three", "local", None)
            .await
            .unwrap();

        let unread = channels.read_since_cursor("c").unwrap();
        let contents: Vec<_> = unread.iter().filter_map(|e| e.content.as_deref()).collect();
        assert_eq!(contents, vec!["two", "three"]);
    }

    #[tokio::test]
    async fn explicit_cursor_marks_read_without_speaking() {
        let (channels, _rx, _dir) = layer().await;
        let id = channels
            .inbound("c", "cass", None, "one", "local", None)
            .await
            .unwrap();
        channels.mark_read("c", &id).unwrap();

        assert!(channels.read_since_cursor("c").unwrap().is_empty());
        assert!(channels.covered("c", &id).unwrap());

        let entries = channels.scan("c").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[1].cursor);
        assert_eq!(entries[1].role, EntryRole::Agent);
        assert!(entries[1].content.is_none());
        assert_eq!(entries[1].up_to.as_deref(), Some(id.as_str()));
    }

    #[tokio::test]
    async fn positional_cursor_keeps_late_arrivals_unread() {
        let (channels, _rx, _dir) = layer().await;
        let read_id = channels
            .inbound("c", "cass", None, "read by the agent", "local", None)
            .await
            .unwrap();
        let late_id = channels
            .inbound("c", "cass", None, "arrived during the model call", "local", None)
            .await
            .unwrap();
        // Settle writes the cursor pointing at what was actually
        // consumed — the cursor entry sits after the late arrival.
        channels.mark_read("c", &read_id).unwrap();

        let unread = channels.read_since_cursor("c").unwrap();
        assert_eq!(unread.len(), 1, "the late arrival is not lost");
        assert_eq!(unread[0].id, late_id);
        assert!(!channels.covered("c", &late_id).unwrap());
    }

    #[tokio::test]
    async fn never_visited_reads_last_fifty() {
        let (channels, _rx, _dir) = layer().await;
        for i in 0..60 {
            channels
                .inbound("c", "cass", None, &format!("msg {i}"), "local", None)
                .await
                .unwrap();
        }
        let unread = channels.read_since_cursor("c").unwrap();
        assert_eq!(unread.len(), NEVER_VISITED_TAIL);
        assert_eq!(unread[0].content.as_deref(), Some("msg 10"));
    }

    #[tokio::test]
    async fn torn_line_is_skipped() {
        let (channels, _rx, _dir) = layer().await;
        channels
            .inbound("c", "cass", None, "ok", "local", None)
            .await
            .unwrap();
        {
            use std::io::Write;
            let mut f = OpenOptions::new()
                .append(true)
                .open(channels.path("c"))
                .unwrap();
            f.write_all(b"{\"id\":\"torn\n").unwrap();
        }
        channels
            .inbound("c", "cass", None, "after", "local", None)
            .await
            .unwrap();
        let entries = channels.scan("c").unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn channel_names_compose_and_sanitize() {
        assert_eq!(channel_name("local", "main"), "local_main");
        assert_eq!(channel_name("discord", "general #1"), "discord_general__1");
    }

    #[tokio::test]
    async fn closed_queue_fails_inbound_after_write() {
        let (channels, rx, _dir) = layer().await;
        drop(rx);
        // Write succeeds, notify fails, caller learns: the adapter
        // reports the inbound event as failed (wall ch. 05).
        let result = channels
            .inbound("c", "cass", None, "hello", "local", None)
            .await;
        assert!(result.is_err());
    }
}
