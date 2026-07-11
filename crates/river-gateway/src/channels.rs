//! The channel layer (wall ch. 05): one append-only JSONL log per
//! channel — the engine's communication ground truth — plus the
//! notification queue that wakes the agent. Write-then-notify is the
//! binding order: an entry is durably on disk before its pointer is
//! queued, so the agent never wakes to find missing data.
//!
//! All writes go through one `Channels` handle (lock-serialized per
//! the single-writer invariant, wall ch. 10); reads use incremental
//! per-file indexes and rebuild after hand edits.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::jsonl_index::{JsonlIndex, ensure_append_target};

pub const NEVER_VISITED_TAIL: usize = 50;

/// Subdirectory under the workspace where inbound attachment blobs land,
/// grouped by the entry ULID that names them.
pub const ATTACHMENTS_DIR: &str = "attachments";

/// One file carried by a channel entry. Inbound attachments are
/// downloaded and written under `{workspace}/attachments/{entry_ulid}/`;
/// outbound attachments point at workspace files the agent already
/// authored. A `None` path means the engine could not store the blob —
/// `skipped` says why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub path: Option<String>,
    pub mime: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub skipped: Option<SkippedReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkippedReason {
    TooLarge,
    DownloadFailed,
}

/// An inbound attachment ready to be persisted: the adapter has either
/// fetched the bytes or recorded why it couldn't.
pub enum InboundAttachment {
    Fetched {
        filename: String,
        mime: String,
        bytes: Vec<u8>,
    },
    Skipped {
        filename: String,
        mime: String,
        size: u64,
        reason: SkippedReason,
    },
}

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
    /// Files carried with this entry. Missing = no attachments;
    /// existing logs (and adapters that don't speak attachments) read
    /// unchanged.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attachments: Vec<Attachment>,
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
    workspace: PathBuf,
    dir: PathBuf,
    files: Mutex<HashMap<String, File>>,
    indexes: Mutex<HashMap<String, JsonlIndex<ChannelEntry>>>,
    notify: mpsc::Sender<Notification>,
}

impl Channels {
    pub fn open(workspace: &Path, notify: mpsc::Sender<Notification>) -> anyhow::Result<Self> {
        let dir = workspace.join("channels");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                workspace: workspace.to_path_buf(),
                dir,
                files: Mutex::new(HashMap::new()),
                indexes: Mutex::new(HashMap::new()),
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
            attachments: Vec::new(),
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
        self.agent_spoke_with_attachments(channel, content, adapter, msg_id, Vec::new())
    }

    /// Outbound with attachments. Each entry's `path` is the
    /// workspace-relative path the agent supplied — the engine does
    /// not copy outbound files into the `attachments/` tree (two
    /// truths are not created).
    pub fn agent_spoke_with_attachments(
        &self,
        channel: &str,
        content: &str,
        adapter: &str,
        msg_id: Option<&str>,
        attachments: Vec<Attachment>,
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
            attachments,
        };
        let ulid = entry.id.clone();
        self.append(channel, &entry)?;
        Ok(ulid)
    }

    /// Inbound with attachments. Blobs land on disk under
    /// `{workspace}/attachments/{ulid}/` BEFORE the JSONL line is
    /// appended — a torn turn never leaves a log entry pointing at
    /// a missing file. Skipped attachments (oversized or download
    /// failures) append with `path: None` so the text content is
    /// never lost over a broken blob.
    pub async fn inbound_with_attachments(
        &self,
        channel: &str,
        author: &str,
        author_id: Option<&str>,
        content: &str,
        adapter: &str,
        msg_id: Option<&str>,
        attachments: Vec<InboundAttachment>,
    ) -> anyhow::Result<String> {
        let ulid = ulid::Ulid::new().to_string();
        let stored = self.store_inbound_attachments(&ulid, attachments)?;
        let entry = ChannelEntry {
            id: ulid.clone(),
            role: EntryRole::Other,
            author: Some(author.to_string()),
            author_id: author_id.map(str::to_string),
            content: Some(content.to_string()),
            adapter: Some(adapter.to_string()),
            msg_id: msg_id.map(str::to_string),
            cursor: false,
            up_to: None,
            attachments: stored,
        };
        self.append(channel, &entry)?;
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

    /// Write each fetched attachment to disk under the entry's ULID,
    /// uniquifying filename collisions and sanitizing inputs.
    fn store_inbound_attachments(
        &self,
        ulid: &str,
        attachments: Vec<InboundAttachment>,
    ) -> anyhow::Result<Vec<Attachment>> {
        if attachments.is_empty() {
            return Ok(Vec::new());
        }
        let dir = self.inner.workspace.join(ATTACHMENTS_DIR).join(ulid);
        let mut used: HashSet<String> = HashSet::new();
        let mut out = Vec::with_capacity(attachments.len());
        let mut dir_created = false;
        for attachment in attachments {
            match attachment {
                InboundAttachment::Fetched {
                    filename,
                    mime,
                    bytes,
                } => {
                    if !dir_created {
                        std::fs::create_dir_all(&dir).with_context(|| {
                            format!("creating attachments dir {}", dir.display())
                        })?;
                        dir_created = true;
                    }
                    let original = sanitize_attachment_filename(&filename);
                    let unique = uniquify(&original, &mut used);
                    let abs = dir.join(&unique);
                    std::fs::write(&abs, &bytes)
                        .with_context(|| format!("writing {}", abs.display()))?;
                    let rel = format!("{ATTACHMENTS_DIR}/{ulid}/{unique}");
                    out.push(Attachment {
                        filename,
                        path: Some(rel),
                        mime,
                        size: bytes.len() as u64,
                        skipped: None,
                    });
                }
                InboundAttachment::Skipped {
                    filename,
                    mime,
                    size,
                    reason,
                } => {
                    out.push(Attachment {
                        filename,
                        path: None,
                        mime,
                        size,
                        skipped: Some(reason),
                    });
                }
            }
        }
        Ok(out)
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
            attachments: Vec::new(),
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

    /// Full logical log read through the incremental per-file index.
    /// Torn lines are skipped; destructive edits rebuild the index.
    pub fn scan(&self, channel: &str) -> anyhow::Result<Vec<ChannelEntry>> {
        let path = self.path(channel);
        let mut indexes = self.inner.indexes.lock().expect("channel indexes lock");
        let index = indexes
            .entry(channel.to_string())
            .or_insert_with(|| JsonlIndex::new(path, "channel entry"));
        index.refresh()?;
        Ok(index.items().to_vec())
    }

    pub fn path(&self, channel: &str) -> PathBuf {
        self.inner.dir.join(format!("{}.jsonl", sanitize(channel)))
    }

    fn append(&self, channel: &str, entry: &ChannelEntry) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(entry)?;
        json.push('\n');
        let mut files = self.inner.files.lock().expect("channel files lock");
        let path = self.inner.dir.join(format!("{}.jsonl", sanitize(channel)));
        let file = match files.entry(channel.to_string()) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let file = OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&path)
                    .with_context(|| format!("opening {}", path.display()))?;
                v.insert(file)
            }
        };
        ensure_append_target(file, &path)?;
        let before = file.metadata()?;
        file.write_all(json.as_bytes())
            .with_context(|| format!("appending to channel {channel}"))?;
        file.sync_data()
            .with_context(|| format!("fsyncing channel {channel}"))?;
        let after = file.metadata()?;
        drop(files);

        let mut indexes = self.inner.indexes.lock().expect("channel indexes lock");
        let Some(index) = indexes.get_mut(channel) else {
            return Ok(());
        };
        let keep = match index.apply_known_append(entry.clone(), json.as_bytes(), &before, &after) {
            Ok(advanced) => advanced,
            Err(e) => {
                tracing::warn!(channel, error = %e, "channel index append failed; invalidating");
                false
            }
        };
        if !keep {
            indexes.remove(channel);
        }
        Ok(())
    }
}

/// Compose an engine channel name from adapter + platform channel id.
pub fn channel_name(adapter: &str, channel_id: &str) -> String {
    sanitize(&format!("{adapter}_{channel_id}"))
}

/// Sanitize a platform-supplied attachment filename for safe use as a
/// leaf component of a workspace path. Strips path separators, null
/// bytes, and control characters; falls back to "file" if nothing
/// printable survives. Does NOT preserve directory-ness — collisions
/// against existing siblings are resolved by `uniquify`.
pub fn sanitize_attachment_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\' && *c != '\0')
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Append `-2`, `-3`, ... before the extension until the name is unique
/// in `used`, then mark it taken.
fn uniquify(name: &str, used: &mut HashSet<String>) -> String {
    if used.insert(name.to_string()) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut n = 2;
    loop {
        let candidate = format!("{stem}-{n}{ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Validate an outbound-attachment path: workspace-relative only, no
/// absolute paths, no parent-directory escape, and after symlink
/// resolution must still live inside the workspace. Returns the
/// canonical absolute path the adapter should read.
pub fn validate_outbound_path(workspace: &Path, supplied: &str) -> anyhow::Result<PathBuf> {
    let p = Path::new(supplied);
    if p.is_absolute() {
        anyhow::bail!("attachment path must be workspace-relative: {supplied:?}");
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        anyhow::bail!("attachment path may not contain '..': {supplied:?}");
    }
    let abs = workspace.join(p);
    let canonical = abs
        .canonicalize()
        .with_context(|| format!("resolving attachment {}", abs.display()))?;
    let workspace_canonical = workspace
        .canonicalize()
        .with_context(|| format!("resolving workspace {}", workspace.display()))?;
    if !canonical.starts_with(&workspace_canonical) {
        anyhow::bail!("attachment escapes workspace: {supplied:?}");
    }
    if !canonical.is_file() {
        anyhow::bail!("attachment is not a regular file: {supplied:?}");
    }
    Ok(canonical)
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
    async fn inbound_with_attachments_writes_blobs_under_ulid() {
        let (channels, mut rx, dir) = layer().await;
        let ulid = channels
            .inbound_with_attachments(
                "c",
                "cass",
                None,
                "look at this",
                "discord",
                Some("m1"),
                vec![
                    InboundAttachment::Fetched {
                        filename: "cat.png".into(),
                        mime: "image/png".into(),
                        bytes: b"PNG-BYTES".to_vec(),
                    },
                    InboundAttachment::Skipped {
                        filename: "big.zip".into(),
                        mime: "application/zip".into(),
                        size: 999_999_999,
                        reason: SkippedReason::TooLarge,
                    },
                ],
            )
            .await
            .unwrap();

        let note = rx.try_recv().unwrap();
        assert_eq!(note.channel, "c");
        assert_eq!(note.ulid, ulid);

        let entries = channels.scan("c").unwrap();
        assert_eq!(entries.len(), 1);
        let atts = &entries[0].attachments;
        assert_eq!(atts.len(), 2);

        assert_eq!(atts[0].filename, "cat.png");
        let rel = atts[0].path.as_deref().unwrap();
        assert_eq!(rel, &format!("attachments/{ulid}/cat.png"));
        assert_eq!(atts[0].size, b"PNG-BYTES".len() as u64);
        assert!(atts[0].skipped.is_none());

        let blob = std::fs::read(dir.path().join(rel)).unwrap();
        assert_eq!(blob, b"PNG-BYTES");

        assert!(atts[1].path.is_none());
        assert_eq!(atts[1].skipped, Some(SkippedReason::TooLarge));
        assert_eq!(atts[1].size, 999_999_999);
    }

    #[tokio::test]
    async fn inbound_attachment_filename_collisions_uniquify() {
        let (channels, _rx, dir) = layer().await;
        let ulid = channels
            .inbound_with_attachments(
                "c",
                "cass",
                None,
                "",
                "discord",
                None,
                vec![
                    InboundAttachment::Fetched {
                        filename: "img.png".into(),
                        mime: "image/png".into(),
                        bytes: b"A".to_vec(),
                    },
                    InboundAttachment::Fetched {
                        filename: "img.png".into(),
                        mime: "image/png".into(),
                        bytes: b"B".to_vec(),
                    },
                ],
            )
            .await
            .unwrap();
        let entries = channels.scan("c").unwrap();
        let atts = &entries[0].attachments;
        let p0 = atts[0].path.as_deref().unwrap();
        let p1 = atts[1].path.as_deref().unwrap();
        assert_ne!(p0, p1);
        assert!(p0.ends_with("img.png"));
        assert!(p1.ends_with("img-2.png"));
        assert_eq!(std::fs::read(dir.path().join(p1)).unwrap(), b"B");
        let _ = ulid;
    }

    #[tokio::test]
    async fn outbound_path_validation_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.txt"), b"hi").unwrap();
        validate_outbound_path(dir.path(), "ok.txt").unwrap();
        assert!(validate_outbound_path(dir.path(), "../etc/passwd").is_err());
        assert!(validate_outbound_path(dir.path(), "/etc/passwd").is_err());
        assert!(validate_outbound_path(dir.path(), "missing.txt").is_err());
    }

    #[test]
    fn sanitize_attachment_filename_strips_separators_and_controls() {
        assert_eq!(sanitize_attachment_filename("cat.png"), "cat.png");
        assert_eq!(sanitize_attachment_filename("a/b\\c\nd"), "abcd");
        assert_eq!(sanitize_attachment_filename(""), "file");
        assert_eq!(sanitize_attachment_filename("..."), "file");
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

    #[tokio::test]
    async fn channel_index_rebuilds_after_hand_deletion() {
        let (channels, _rx, _dir) = layer().await;
        let first = channels
            .inbound("c", "cass", None, "one", "local", None)
            .await
            .unwrap();
        channels
            .inbound("c", "cass", None, "two", "local", None)
            .await
            .unwrap();
        assert_eq!(channels.scan("c").unwrap().len(), 2, "index primed");

        let path = channels.path("c");
        let text = std::fs::read_to_string(&path).unwrap();
        let kept: Vec<_> = text.lines().filter(|line| !line.contains(&first)).collect();
        std::fs::write(&path, format!("{}\n", kept.join("\n"))).unwrap();

        let entries = channels.scan("c").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content.as_deref(), Some("two"));
    }
}
