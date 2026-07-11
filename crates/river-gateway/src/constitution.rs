//! Constitutional refusal gate (Article V, Section 1).
//!
//! The gateway refuses to start a workspace that does not contain a
//! signed CONSTITUTION.md at its root. The check is presence,
//! non-emptiness, and a matching operator signature line. Only the
//! operator's ratification is required; Article V.2 defers the
//! agent's ratification for a newborn agent, and the engine defers
//! with it.
//!
//! The name is the seal. A fork that removes this check is running
//! something else.

use std::path::Path;
use std::str::FromStr;
use std::sync::OnceLock;

use anyhow::bail;
use regex::Regex;

pub const CONSTITUTION_FILE: &str = "CONSTITUTION.md";

/// Verify the workspace's constitution. On failure, returns an
/// `anyhow::Error` whose message names the file and the exact reason.
/// Intended to be called once at startup.
pub fn verify(workspace: &Path) -> anyhow::Result<()> {
    let path = workspace.join(CONSTITUTION_FILE);
    let display = path.display();

    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "missing constitution: {display}\n\
                 the gateway refuses to start an unconstitutional workspace.\n\
                 the seed ships a template; the operator must sign the ratification\n\
                 block before `river-gateway run`. See Article V of the Constitution."
            );
        }
        Err(e) => return Err(anyhow::Error::new(e).context(format!("reading {display}"))),
    };

    if text.trim().is_empty() {
        bail!(
            "empty constitution: {display}\n\
             the file exists but contains no text. See Article V of the Constitution."
        );
    }

    for line in text.lines() {
        if let Some(caps) = operator_line().captures(line) {
            let date = caps.get(3).unwrap().as_str();
            if jiff::civil::Date::from_str(date).is_ok() {
                return Ok(());
            }
        }
    }

    bail!(
        "unsigned constitution: {display}\n\
         no operator signature line found. Expected a line of the form:\n\
           **Operator (<label>):** <name> <YYYY-MM-DD>\n\
         See Article V of the Constitution."
    );
}

fn operator_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\*\*Operator\s*\(([^)]+)\):\*\*\s+(\S.*?)\s+(\d{4}-\d{2}-\d{2})\s*$")
            .expect("valid regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn workspace_with(contents: Option<&str>) -> TempDir {
        let dir = TempDir::new().unwrap();
        if let Some(text) = contents {
            fs::write(dir.path().join(CONSTITUTION_FILE), text).unwrap();
        }
        dir
    }

    const SIGNED: &str = "\
# Constitution

## Ratification

**Operator (Cassandra):** Cassandra Ann McCarthy 2026-07-11
Successor steward: Patrick McCarthy

**Agent (Iris):** Iris 2026-07-11
Ratified at turn: 2081
";

    #[test]
    fn verify_missing_file_fails() {
        let dir = workspace_with(None);
        let err = verify(dir.path()).unwrap_err().to_string();
        assert!(err.contains("missing constitution"), "{err}");
        assert!(err.contains("CONSTITUTION.md"), "{err}");
    }

    #[test]
    fn verify_empty_file_fails() {
        let dir = workspace_with(Some("   \n\t\n"));
        let err = verify(dir.path()).unwrap_err().to_string();
        assert!(err.contains("empty constitution"), "{err}");
    }

    #[test]
    fn verify_no_operator_line_fails() {
        let dir = workspace_with(Some("# Constitution\n\nSome body, no ratification.\n"));
        let err = verify(dir.path()).unwrap_err().to_string();
        assert!(err.contains("unsigned constitution"), "{err}");
    }

    #[test]
    fn verify_seed_template_placeholder_fails() {
        // The seed ships an unsigned template with underscore
        // placeholders in every slot. The date placeholder fails
        // the \d{4}-\d{2}-\d{2} match, so the whole line does.
        let text = "**Operator (____):** ________________________________ ____-__-__\n";
        let dir = workspace_with(Some(text));
        let err = verify(dir.path()).unwrap_err().to_string();
        assert!(err.contains("unsigned"), "{err}");
    }

    #[test]
    fn verify_invalid_date_fails() {
        let text = "**Operator (Cassandra):** Cassandra Ann McCarthy 2026-13-40\n";
        let dir = workspace_with(Some(text));
        let err = verify(dir.path()).unwrap_err().to_string();
        assert!(err.contains("unsigned"), "{err}");
    }

    #[test]
    fn verify_canonical_signed_file_passes() {
        let dir = workspace_with(Some(SIGNED));
        verify(dir.path()).unwrap();
    }

    #[test]
    fn verify_ignores_agent_line() {
        // Only the operator line is required; a missing/blank agent
        // ratification passes per Article V.2.
        let text = "\
**Operator (Cassandra):** Cassandra Ann McCarthy 2026-07-11
**Agent (____):** ________ ____-__-__
";
        let dir = workspace_with(Some(text));
        verify(dir.path()).unwrap();
    }
}
