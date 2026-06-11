//! The birth ritual (wall ch. 08): before a gateway can start, its
//! agent must be born. Birth writes the founding record —
//! `record/birth.json` — once, by deliberate human action. The
//! gateway's startup begins by reading it; absence is a refusal to
//! run as nobody.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct BirthRecord {
    pub id: String,
    pub name: String,
    pub born_at: String,
}

pub fn birth_path(workspace: &Path) -> PathBuf {
    workspace.join("record").join("birth.json")
}

/// Write the founding record. Refuses if the workspace is already
/// birthed — birth happens once.
pub fn perform_birth(workspace: &Path, name: &str) -> anyhow::Result<BirthRecord> {
    let path = birth_path(workspace);
    if path.exists() {
        let existing = read_birth(workspace)?;
        bail!(
            "{} is already the workspace of {:?}, born {} — birth happens once",
            workspace.display(),
            existing.name,
            existing.born_at
        );
    }

    let record = BirthRecord {
        id: ulid::Ulid::new().to_string(),
        name: name.to_string(),
        born_at: jiff::Timestamp::now().to_string(),
    };

    let record_dir = path.parent().expect("birth path has a parent");
    std::fs::create_dir_all(record_dir)
        .with_context(|| format!("creating {}", record_dir.display()))?;
    let json = serde_json::to_string_pretty(&record)?;
    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(record)
}

/// Read the founding record. The error for an unbirthed workspace
/// names the exact command to run (wall ch. 08 contract).
pub fn read_birth(workspace: &Path) -> anyhow::Result<BirthRecord> {
    let path = birth_path(workspace);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
            "workspace {} has no founding record — birth the agent first:\n  \
             river-gateway birth --workspace {} --name <name>",
            workspace.display(),
            workspace.display()
        ),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birth_writes_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let record = perform_birth(dir.path(), "ada").unwrap();
        assert_eq!(record.name, "ada");
        assert_eq!(record.id.len(), 26); // ULID canonical form
        assert!(record.born_at.ends_with('Z'));

        let read = read_birth(dir.path()).unwrap();
        assert_eq!(read, record);
    }

    #[test]
    fn rebirth_refuses() {
        let dir = tempfile::tempdir().unwrap();
        perform_birth(dir.path(), "ada").unwrap();
        let err = perform_birth(dir.path(), "bee").unwrap_err().to_string();
        assert!(err.contains("already"), "{err}");
        assert!(err.contains("ada"), "{err}");
        // The original record is untouched.
        assert_eq!(read_birth(dir.path()).unwrap().name, "ada");
    }

    #[test]
    fn unbirthed_error_names_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_birth(dir.path()).unwrap_err().to_string();
        assert!(err.contains("river-gateway birth --workspace"), "{err}");
    }
}
