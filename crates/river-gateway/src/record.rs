//! The turn record (wall ch. 10): `record/{channel}.jsonl`,
//! append-only, one JSON object per line, written by exactly one
//! writer, fsynced on append. Every context message is persisted at
//! the moment it enters the context, exactly once, under its turn
//! number (persist-once, wall ch. 01). Readers skip torn lines with a
//! warning — a crash mid-append never poisons the file.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RecordLine {
    pub id: String,
    pub turn: u64,
    pub role: RecordRole,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
}

/// The single writer for one channel's turn record.
pub struct TurnRecord {
    path: PathBuf,
    file: File,
}

impl TurnRecord {
    pub fn open(workspace: &Path, channel: &str) -> anyhow::Result<Self> {
        let dir = workspace.join("record");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(format!("{}.jsonl", sanitize(channel)));
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self { path, file })
    }

    /// Append one line and fsync. Returns the line's ULID.
    pub fn append(
        &mut self,
        turn: u64,
        role: RecordRole,
        content: Option<&str>,
    ) -> anyhow::Result<String> {
        let line = RecordLine {
            id: ulid::Ulid::new().to_string(),
            turn,
            role,
            content: content.map(str::to_string),
            tool_calls: None,
            tool_call_id: None,
        };
        let mut json = serde_json::to_string(&line)?;
        json.push('\n');
        self.file
            .write_all(json.as_bytes())
            .with_context(|| format!("appending to {}", self.path.display()))?;
        self.file
            .sync_data()
            .with_context(|| format!("fsyncing {}", self.path.display()))?;
        Ok(line.id)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Read a record file, skipping malformed lines with a logged
/// warning (torn-line tolerance).
pub fn scan(path: &Path) -> anyhow::Result<Vec<RecordLine>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut lines = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RecordLine>(raw) {
            Ok(line) => lines.push(line),
            Err(e) => tracing::warn!(
                path = %path.display(),
                line = line_no + 1,
                error = %e,
                "skipping malformed record line"
            ),
        }
    }
    Ok(lines)
}

/// The last `n` whole turns of a record file, in order. A tail-scan:
/// collect from the end, never split a turn.
pub fn tail_turns(path: &Path, n: usize) -> anyhow::Result<Vec<RecordLine>> {
    let lines = scan(path)?;
    let mut turns: Vec<u64> = lines.iter().map(|l| l.turn).collect();
    turns.dedup();
    let keep: std::collections::BTreeSet<u64> =
        turns.into_iter().rev().take(n).collect();
    Ok(lines
        .into_iter()
        .filter(|l| keep.contains(&l.turn))
        .collect())
}

/// The highest turn number in a record file, or 0 for none.
pub fn last_turn(path: &Path) -> anyhow::Result<u64> {
    Ok(scan(path)?.last().map(|l| l.turn).unwrap_or(0))
}

fn sanitize(channel: &str) -> String {
    channel
        .chars()
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

    #[test]
    fn appends_and_scans_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path(), "local_main").unwrap();
        record
            .append(1, RecordRole::User, Some("[local_main] cass: hello"))
            .unwrap();
        record.append(1, RecordRole::Assistant, Some("hi")).unwrap();

        let lines = scan(record.path()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].turn, 1);
        assert_eq!(lines[0].role, RecordRole::User);
        assert_eq!(lines[1].role, RecordRole::Assistant);
        assert!(lines[0].id < lines[1].id, "ULIDs order");
    }

    #[test]
    fn torn_line_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path(), "local_main").unwrap();
        record.append(1, RecordRole::User, Some("a")).unwrap();
        // Simulate a crash mid-append.
        {
            use std::io::Write;
            let mut f = OpenOptions::new()
                .append(true)
                .open(record.path())
                .unwrap();
            f.write_all(b"{\"id\":\"torn").unwrap();
            f.write_all(b"\n").unwrap();
        }
        let mut record = TurnRecord::open(dir.path(), "local_main").unwrap();
        record.append(2, RecordRole::User, Some("b")).unwrap();

        let lines = scan(record.path()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].turn, 2);
    }

    #[test]
    fn tail_turns_keeps_whole_turns() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path(), "c").unwrap();
        for turn in 1..=3 {
            record.append(turn, RecordRole::User, Some("q")).unwrap();
            record.append(turn, RecordRole::Assistant, Some("a")).unwrap();
        }
        let tail = tail_turns(record.path(), 2).unwrap();
        assert_eq!(tail.len(), 4);
        assert!(tail.iter().all(|l| l.turn >= 2));
    }

    #[test]
    fn last_turn_reads_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path(), "c").unwrap();
        assert_eq!(last_turn(record.path()).unwrap(), 0);
        record.append(41, RecordRole::User, Some("x")).unwrap();
        assert_eq!(last_turn(record.path()).unwrap(), 41);
    }

    #[test]
    fn channel_names_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let record = TurnRecord::open(dir.path(), "discord/general #1").unwrap();
        assert!(
            record
                .path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .eq("discord_general__1.jsonl")
        );
    }

    #[test]
    fn missing_file_scans_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lines = scan(&dir.path().join("nope.jsonl")).unwrap();
        assert!(lines.is_empty());
    }
}
