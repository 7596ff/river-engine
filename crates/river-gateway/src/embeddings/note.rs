//! Note format with YAML frontmatter

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Note types in the zettelkasten
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum NoteType {
    Note,
    Move,
    Moment,
    RoomNote,
}

/// Frontmatter metadata for a note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteFrontmatter {
    pub id: String,
    pub created: DateTime<Utc>,
    pub author: String,
    #[serde(rename = "type")]
    pub note_type: NoteType,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub channel: Option<String>,
}

/// A parsed note (frontmatter + content)
#[derive(Debug, Clone)]
pub struct Note {
    pub frontmatter: NoteFrontmatter,
    pub content: String,
    pub source_path: String,
}

impl Note {
    /// Parse a note from file contents
    pub fn parse(source_path: &str, text: &str) -> Result<Self, String> {
        let text = text.trim();
        if !text.starts_with("---") {
            return Err("Note must start with YAML frontmatter (---)".into());
        }

        let end = text[3..]
            .find("---")
            .ok_or("Missing closing --- for frontmatter")?;

        let yaml = &text[3..end + 3];
        let content = text[end + 6..].trim().to_string();

        let frontmatter: NoteFrontmatter =
            serde_yaml::from_str(yaml).map_err(|e| format!("Invalid frontmatter: {}", e))?;

        Ok(Note {
            frontmatter,
            content,
            source_path: source_path.to_string(),
        })
    }

    /// Create a new note with frontmatter
    pub fn to_string(&self) -> String {
        let yaml = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        format!("---\n{}---\n\n{}", yaml, self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_note() {
        let text = r#"---
id: "0x01a2b3c4"
created: 2026-03-23T14:32:07Z
author: agent
type: note
tags: [css, z-index]
---

# z-index hierarchy
Modal: 50, Navbar: 40"#;

        let note = Note::parse("notes/z-index.md", text).unwrap();
        assert_eq!(note.frontmatter.author, "agent");
        assert_eq!(note.frontmatter.note_type, NoteType::Note);
        assert!(note.content.contains("z-index hierarchy"));
    }

    #[test]
    fn test_parse_note_no_frontmatter() {
        let result = Note::parse("test.md", "just content");
        assert!(result.is_err());
    }

    #[test]
    fn test_note_roundtrip() {
        let text = r#"---
id: "test-001"
created: 2026-03-23T12:00:00Z
author: agent
type: moment
tags: []
---

Content here"#;
        let note = Note::parse("test.md", text).unwrap();
        let output = note.to_string();
        assert!(output.contains("id:"));
        assert!(output.contains("Content here"));
    }
}
