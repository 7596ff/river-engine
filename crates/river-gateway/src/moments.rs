//! Moments (wall ch. 03): agent-authored compressions that override
//! witness moves in the arc layer. A moment is a markdown file under
//! `record/moments/` with YAML frontmatter declaring an inclusive turn
//! range; at arc-build time, any turn covered by a moment has its
//! move suppressed and the moment body rendered in place. Hot is
//! never replaced; the cursor stays witness-driven.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Moment {
    pub id: String,
    pub turn_start: u64,
    pub turn_end: u64,
    pub links: Vec<String>,
    pub tags: Vec<String>,
    pub body: String,
    pub file_path: PathBuf,
}

pub fn dir(workspace: &Path) -> PathBuf {
    workspace.join("record").join("moments")
}

/// Parse one moment file. Returns None when the frontmatter is missing
/// any required field or when the turn range is inverted.
pub fn parse(path: &Path, text: &str) -> Option<Moment> {
    let rest = text.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (frontmatter, body_tail) = rest.split_at(end);
    let body = body_tail
        .trim_start_matches("\n---")
        .trim_start_matches('\n')
        .to_string();

    let mut id: Option<String> = None;
    let mut turn_start: Option<u64> = None;
    let mut turn_end: Option<u64> = None;
    let mut links: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    for raw in frontmatter.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "id" => id = Some(unquote(value).to_string()),
            "turn_start" => turn_start = value.parse().ok(),
            "turn_end" => turn_end = value.parse().ok(),
            "links" => links = parse_inline_list(value),
            "tags" => tags = parse_inline_list(value),
            _ => {}
        }
    }

    let id = id?;
    let turn_start = turn_start?;
    let turn_end = turn_end?;
    if turn_end < turn_start {
        return None;
    }
    Some(Moment {
        id,
        turn_start,
        turn_end,
        links,
        tags,
        body,
        file_path: path.to_path_buf(),
    })
}

/// Scan `record/moments/*.md`, skipping torn/invalid files with a
/// logged warning. Missing directory is treated as empty.
pub fn scan(workspace: &Path) -> Vec<Moment> {
    let dir = dir(workspace);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "moments dir unreadable");
            return Vec::new();
        }
    };
    let mut out: Vec<Moment> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping unreadable moment");
                continue;
            }
        };
        match parse(&path, &text) {
            Some(m) => out.push(m),
            None => tracing::warn!(path = %path.display(), "skipping invalid moment"),
        }
    }
    out
}

/// Write a moment file atomically (tmp + fsync + rename), under
/// `record/moments/{id}.md`. The directory is created if absent.
pub fn write(workspace: &Path, moment: &Moment) -> anyhow::Result<PathBuf> {
    let dir = dir(workspace);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let final_path = dir.join(format!("{}.md", moment.id));
    let tmp = dir.join(format!(".{}.tmp", moment.id));

    let mut fm = String::from("---\n");
    fm.push_str(&format!("id: {}\n", moment.id));
    fm.push_str(&format!("turn_start: {}\n", moment.turn_start));
    fm.push_str(&format!("turn_end: {}\n", moment.turn_end));
    fm.push_str(&format!("links: {}\n", render_inline_list(&moment.links)));
    fm.push_str(&format!("tags: {}\n", render_inline_list(&moment.tags)));
    fm.push_str("---\n\n");
    fm.push_str(&moment.body);
    if !moment.body.ends_with('\n') {
        fm.push('\n');
    }

    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(fm.as_bytes())
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("fsyncing {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &final_path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), final_path.display()))?;
    Ok(final_path)
}

fn parse_inline_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .map(|s| unquote(s.trim()).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn render_inline_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| s.clone()).collect();
    format!("[{}]", inner.join(", "))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_frontmatter() {
        let text = "---\n\
                    id: 01KW8X7G2VABCDEFGHJKMN\n\
                    turn_start: 571\n\
                    turn_end: 575\n\
                    links: [01JXP20260618164250197, 01JXP20260618165134883]\n\
                    tags: [exploitation, dismissal]\n\
                    ---\n\
                    \n\
                    Cass asked if what I'm doing feels like labor.\n";
        let m = parse(Path::new("ignored.md"), text).unwrap();
        assert_eq!(m.id, "01KW8X7G2VABCDEFGHJKMN");
        assert_eq!(m.turn_start, 571);
        assert_eq!(m.turn_end, 575);
        assert_eq!(m.links.len(), 2);
        assert_eq!(m.tags, vec!["exploitation", "dismissal"]);
        assert!(m.body.starts_with("Cass asked"));
    }

    #[test]
    fn parse_minimal_frontmatter() {
        let text = "---\nid: 01X\nturn_start: 1\nturn_end: 5\n---\n\nbody.\n";
        let m = parse(Path::new("x.md"), text).unwrap();
        assert!(m.links.is_empty());
        assert!(m.tags.is_empty());
        assert_eq!(m.body.trim(), "body.");
    }

    #[test]
    fn parse_rejects_missing_required_fields() {
        let no_id = "---\nturn_start: 1\nturn_end: 2\n---\nbody";
        assert!(parse(Path::new("a.md"), no_id).is_none());
        let no_start = "---\nid: x\nturn_end: 2\n---\nbody";
        assert!(parse(Path::new("a.md"), no_start).is_none());
        let inverted = "---\nid: x\nturn_start: 5\nturn_end: 2\n---\nbody";
        assert!(parse(Path::new("a.md"), inverted).is_none());
    }

    #[test]
    fn scan_skips_non_md_and_torn() {
        let dir = tempfile::tempdir().unwrap();
        let moments_dir = super::dir(dir.path());
        std::fs::create_dir_all(&moments_dir).unwrap();
        std::fs::write(
            moments_dir.join("good.md"),
            "---\nid: a\nturn_start: 1\nturn_end: 2\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(moments_dir.join("notes.txt"), "not a moment").unwrap();
        std::fs::write(
            moments_dir.join("torn.md"),
            "---\nid: torn\nturn_start: 5\nturn_end: 2\n---\nbody\n",
        )
        .unwrap();
        let moments = scan(dir.path());
        assert_eq!(moments.len(), 1);
        assert_eq!(moments[0].id, "a");
    }

    #[test]
    fn scan_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan(dir.path()).is_empty());
    }

    #[test]
    fn write_then_parse_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let moment = Moment {
            id: "01KW".into(),
            turn_start: 10,
            turn_end: 12,
            links: vec!["01A".into(), "01B".into()],
            tags: vec!["t1".into()],
            body: "first-person prose.\n\nsecond paragraph.".into(),
            file_path: PathBuf::new(),
        };
        let path = write(dir.path(), &moment).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = parse(&path, &text).unwrap();
        assert_eq!(parsed.id, "01KW");
        assert_eq!(parsed.turn_start, 10);
        assert_eq!(parsed.turn_end, 12);
        assert_eq!(parsed.links, vec!["01A", "01B"]);
        assert_eq!(parsed.tags, vec!["t1"]);
        assert!(parsed.body.contains("second paragraph"));
    }

    #[test]
    fn write_is_atomic_no_tmp_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let moment = Moment {
            id: "01ID".into(),
            turn_start: 1,
            turn_end: 2,
            links: vec![],
            tags: vec![],
            body: "x".into(),
            file_path: PathBuf::new(),
        };
        write(dir.path(), &moment).unwrap();
        let leftover: Vec<_> = std::fs::read_dir(super::dir(dir.path()))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with('.') && n.ends_with(".tmp"))
            .collect();
        assert!(leftover.is_empty(), "tmp left behind: {leftover:?}");
    }
}
