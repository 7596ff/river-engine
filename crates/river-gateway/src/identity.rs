//! Identity files (wall ch. 08): AGENTS.md, IDENTITY.md, RULES.md at
//! the workspace root, all required, joined into the system prompt
//! with the current time. Missing files fail startup naming every
//! absent file — no silent fallback to a generic prompt, ever.

use std::path::Path;

use anyhow::{Context as _, bail};

pub const REQUIRED_FILES: [&str; 3] = ["AGENTS.md", "IDENTITY.md", "RULES.md"];

#[derive(Debug)]
pub struct IdentityFiles {
    /// Contents in REQUIRED_FILES order: agents, identity, rules.
    contents: [String; 3],
}

/// Read the three identity files. All missing files are reported
/// together.
pub fn load(workspace: &Path) -> anyhow::Result<IdentityFiles> {
    let mut contents: [String; 3] = Default::default();
    let mut missing = Vec::new();

    for (i, name) in REQUIRED_FILES.iter().enumerate() {
        let path = workspace.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => contents[i] = text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(path.display().to_string());
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    if !missing.is_empty() {
        bail!(
            "missing identity file(s): {} — the gateway does not run as nobody",
            missing.join(", ")
        );
    }
    Ok(IdentityFiles { contents })
}

impl IdentityFiles {
    /// Assemble the system prompt: the three files joined with
    /// separators, plus the current time. Pure given a timestamp;
    /// callers re-invoke at session start, channel switch, and
    /// compaction (wall ch. 03).
    pub fn system_prompt(&self, now: &jiff::Zoned) -> String {
        let mut prompt = String::new();
        for content in &self.contents {
            prompt.push_str(content.trim_end());
            prompt.push_str("\n\n---\n\n");
        }
        prompt.push_str(&format!("Current time: {now}"));
        prompt
    }
}

/// Resolve the agent's timezone: a configured IANA name, or the
/// system timezone when unconfigured.
pub fn timezone(configured: Option<&str>) -> anyhow::Result<jiff::tz::TimeZone> {
    match configured {
        Some(name) => jiff::tz::TimeZone::get(name)
            .with_context(|| format!("unknown timezone {name:?} in config")),
        None => Ok(jiff::tz::TimeZone::system()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_identity(dir: &Path) {
        std::fs::write(dir.join("AGENTS.md"), "# How to operate\n").unwrap();
        std::fs::write(dir.join("IDENTITY.md"), "# Who I am\n").unwrap();
        std::fs::write(dir.join("RULES.md"), "# The floor\n").unwrap();
    }

    #[test]
    fn loads_and_assembles_in_order() {
        let dir = tempfile::tempdir().unwrap();
        write_identity(dir.path());
        let identity = load(dir.path()).unwrap();

        let tz = jiff::tz::TimeZone::UTC;
        let now = jiff::Timestamp::UNIX_EPOCH.to_zoned(tz);
        let prompt = identity.system_prompt(&now);

        let agents_pos = prompt.find("# How to operate").unwrap();
        let identity_pos = prompt.find("# Who I am").unwrap();
        let rules_pos = prompt.find("# The floor").unwrap();
        assert!(agents_pos < identity_pos && identity_pos < rules_pos);
        assert!(prompt.contains("Current time: 1970-01-01T00:00:00+00:00[UTC]"));
    }

    #[test]
    fn missing_files_reported_together() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("IDENTITY.md"), "x").unwrap();
        let err = load(dir.path()).unwrap_err().to_string();
        assert!(err.contains("AGENTS.md"), "{err}");
        assert!(err.contains("RULES.md"), "{err}");
        assert!(!err.contains("IDENTITY.md,"), "{err}");
    }

    #[test]
    fn timezone_resolution() {
        assert!(timezone(Some("America/New_York")).is_ok());
        assert!(timezone(Some("Mars/Olympus_Mons")).is_err());
        assert!(timezone(None).is_ok());
    }
}
