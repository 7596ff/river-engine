//! Session resume (wall ch. 03): `workspace/session.json` carries the
//! ephemeral state that would otherwise be lost across restarts —
//! current channel, estimator calibration, the memory slot's active
//! flashes, and the quiet-gate timer as elapsed seconds since the
//! last significant event. Written at every settle, read once at
//! startup; missing or malformed fields fall back to derivation.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::memory::Flash;
use crate::record;

const SESSION_VERSION: u32 = 1;

/// One row in `active_flashes` at snapshot time. The countdown
/// continues from `remaining` on the next turn after resume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlashSnapshot {
    pub note_id: String,
    pub text: String,
    pub neighbors: Vec<(String, String)>,
    pub remaining: u8,
}

impl From<&(Flash, u8)> for FlashSnapshot {
    fn from(value: &(Flash, u8)) -> Self {
        Self {
            note_id: value.0.note_id.clone(),
            text: value.0.text.clone(),
            neighbors: value.0.neighbors.clone(),
            remaining: value.1,
        }
    }
}

impl FlashSnapshot {
    pub fn into_active(self) -> (Flash, u8) {
        (
            Flash {
                note_id: self.note_id,
                text: self.text,
                neighbors: self.neighbors,
            },
            self.remaining,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSnapshot {
    pub version: u32,
    pub channel: String,
    pub turn_number: u64,
    pub saved_at: String,
    pub estimator_ratio: f64,
    pub active_flashes: Vec<FlashSnapshot>,
    /// Wall-clock seconds between the last significant event and the
    /// snapshot. On resume, treated as if that much silence already
    /// passed — extended downtime is extended silence.
    pub quiet_seconds: u64,
}

impl SessionSnapshot {
    pub fn new(
        channel: String,
        turn_number: u64,
        estimator_ratio: f64,
        active_flashes: &[(Flash, u8)],
        quiet_seconds: u64,
    ) -> Self {
        Self {
            version: SESSION_VERSION,
            channel,
            turn_number,
            saved_at: jiff::Timestamp::now().to_string(),
            estimator_ratio,
            active_flashes: active_flashes.iter().map(FlashSnapshot::from).collect(),
            quiet_seconds,
        }
    }

    /// Atomic write: tmp + fsync + rename. A killed process mid-write
    /// leaves the old `session.json` intact.
    pub fn write_atomic(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = tmp_path(path);
        let json = serde_json::to_vec_pretty(self)?;
        {
            let mut file = std::fs::File::create(&tmp)
                .with_context(|| format!("creating {}", tmp.display()))?;
            file.write_all(&json)
                .with_context(|| format!("writing {}", tmp.display()))?;
            file.sync_all()
                .with_context(|| format!("fsyncing {}", tmp.display()))?;
        }
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut filename = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    filename.push(".tmp");
    path.with_file_name(filename)
}

/// Read `session.json` if present and valid. Missing, torn, or
/// version-mismatched files return `None` with a logged warning —
/// callers fall back to derivation.
pub fn load(path: &Path) -> Option<SessionSnapshot> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "reading session.json");
            return None;
        }
    };
    let parsed: SessionSnapshot = match serde_json::from_str(&text) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "parsing session.json");
            return None;
        }
    };
    if parsed.version != SESSION_VERSION {
        tracing::warn!(
            path = %path.display(),
            found = parsed.version,
            expected = SESSION_VERSION,
            "session.json version mismatch; treating as missing"
        );
        return None;
    }
    Some(parsed)
}

/// Derive "where iris was talking" from the record tail when no
/// snapshot is available. The channel field of every record line
/// reflects "where the agent was" at the time of the line — whether
/// it's a user message, an assistant reply, a heartbeat, or a
/// digestion frame. The last line's channel is the right answer.
/// Returns None when the record is empty.
pub fn channel_from_record_tail(workspace: &Path) -> Option<String> {
    let path = workspace.join("record").join("turns.jsonl");
    record::tail(&path).ok().flatten().map(|line| line.channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flash(note_id: &str, text: &str) -> Flash {
        Flash {
            note_id: note_id.into(),
            text: text.into(),
            neighbors: vec![("extends".into(), "other".into())],
        }
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let snap = SessionSnapshot::new(
            "discord_12345".into(),
            664,
            0.988,
            &[(flash("note-a", "first flash"), 2)],
            247,
        );
        snap.write_atomic(&path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.channel, snap.channel);
        assert_eq!(loaded.turn_number, snap.turn_number);
        assert_eq!(loaded.estimator_ratio, snap.estimator_ratio);
        assert_eq!(loaded.active_flashes, snap.active_flashes);
        assert_eq!(loaded.quiet_seconds, snap.quiet_seconds);
        assert_eq!(loaded.version, SESSION_VERSION);
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("session.json")).is_none());
    }

    #[test]
    fn malformed_returns_none_logged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, "this is not json").unwrap();
        assert!(load(&path).is_none());
    }

    #[test]
    fn version_mismatch_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(
            &path,
            r#"{"version":99,"channel":"c","turn_number":0,"saved_at":"x","estimator_ratio":1.0,"active_flashes":[],"quiet_seconds":0}"#,
        )
        .unwrap();
        assert!(load(&path).is_none());
    }

    #[test]
    fn atomic_write_does_not_leave_partial_files_on_subsequent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        for i in 0..3u64 {
            let snap = SessionSnapshot::new("c".into(), i, 1.0, &[], 0);
            snap.write_atomic(&path).unwrap();
        }
        // No leftover tmp file.
        let mut entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        entries.sort();
        assert_eq!(entries, vec![std::ffi::OsString::from("session.json")]);
    }

    #[test]
    fn channel_from_record_tail_returns_last_lines_channel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("record")).unwrap();
        let record_path = dir.path().join("record/turns.jsonl");
        let lines = [
            r#"{"id":"01A","turn":1,"channel":"local_main","role":"user","content":"hi"}"#,
            r#"{"id":"01B","turn":2,"channel":"discord_12345","role":"user","content":"hello"}"#,
            r#"{"id":"01C","turn":3,"channel":"discord_12345","role":"assistant","content":"hey"}"#,
        ];
        std::fs::write(&record_path, lines.join("\n") + "\n").unwrap();
        assert_eq!(
            channel_from_record_tail(dir.path()).as_deref(),
            Some("discord_12345")
        );
    }

    #[test]
    fn channel_from_empty_record_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(channel_from_record_tail(dir.path()).is_none());
    }

    #[test]
    fn flash_snapshot_round_trip() {
        let original = (flash("n", "t"), 3);
        let snap = FlashSnapshot::from(&original);
        let restored = snap.into_active();
        assert_eq!(restored.0.note_id, original.0.note_id);
        assert_eq!(restored.0.text, original.0.text);
        assert_eq!(restored.0.neighbors, original.0.neighbors);
        assert_eq!(restored.1, original.1);
    }
}
