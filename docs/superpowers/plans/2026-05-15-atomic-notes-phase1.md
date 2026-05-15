# Phase 1: Atomic Notes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the agent a `write_atomic` tool to produce single-claim knowledge notes with typed links, stored in `embeddings/atomic/` and auto-indexed by the Phase 0 sync service.

**Architecture:** Add `SnowflakeType::AtomicNote`, extend the note parser to handle atomic frontmatter (with typed links), create a `write_atomic` tool with strict validation (≤100 words, links required, tags required), register it in the gateway.

**Tech Stack:** Rust, serde_yaml (frontmatter), chrono (timestamps)

---

### Task 1: Add SnowflakeType::AtomicNote

**Files:**
- Modify: `crates/river-core/src/snowflake/types.rs`

- [ ] **Step 1: Add AtomicNote variant**

In `crates/river-core/src/snowflake/types.rs`, add to the enum after `Context = 0x06`:

```rust
    /// Atomic knowledge note
    AtomicNote = 0x07,
```

Update the doc comment at the top to include `- AtomicNote: 0x07`.

Add to `from_u8`:
```rust
0x07 => Some(SnowflakeType::AtomicNote),
```

Add to `all()`:
```rust
SnowflakeType::AtomicNote,
```

Add to `Display`:
```rust
SnowflakeType::AtomicNote => write!(f, "AtomicNote"),
```

- [ ] **Step 2: Update tests**

In the `test_snowflake_type_from_u8_invalid` test, change `0x07` to `0x08`:
```rust
assert_eq!(SnowflakeType::from_u8(0x08), None);
```

In `test_snowflake_type_values`, add:
```rust
assert_eq!(SnowflakeType::AtomicNote.as_u8(), 0x07);
```

In `test_snowflake_type_display`, add:
```rust
assert!(format!("{}", SnowflakeType::AtomicNote).contains("AtomicNote"));
```

In `test_snowflake_type_all`, update the count assertion (from 6 to 7, or whatever it currently is + 1).

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-core -- snowflake::types`

Expected: All pass.

- [ ] **Step 4: Run full test suite**

Run: `cargo test`

Expected: All pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): add SnowflakeType::AtomicNote = 0x07"
```

---

### Task 2: Extend note parser for atomic notes

**Files:**
- Modify: `crates/river-gateway/src/embeddings/note.rs`

- [ ] **Step 1: Add Atomic to NoteType enum**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum NoteType {
    Note,
    Move,
    Moment,
    RoomNote,
    Atomic,
}
```

- [ ] **Step 2: Add NoteLink struct**

```rust
/// A typed link to another atomic note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteLink {
    /// Link type (extends, complicates, contradicts, supports, etc.)
    #[serde(rename = "type")]
    pub link_type: String,
    /// Target note ID (snowflake hex)
    pub target: String,
}
```

- [ ] **Step 3: Add links field to NoteFrontmatter**

```rust
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
    #[serde(default)]
    pub links: Option<Vec<NoteLink>>,
}
```

- [ ] **Step 4: Write test for parsing atomic note frontmatter**

Add to the tests module in `note.rs`:

```rust
#[test]
fn test_parse_atomic_note() {
    let content = r#"---
id: "abc123"
created: 2026-05-15T22:00:00Z
author: viola
type: atomic
links:
  - type: extends
    target: "def456"
  - type: complicates
    target: "ghi789"
tags: [hobbes, reason]
---

Hobbes's theory of reason requires agreed-upon names."#;

    let note = Note::parse("test-z.md", content).unwrap();
    assert_eq!(note.frontmatter.note_type, NoteType::Atomic);
    assert_eq!(note.frontmatter.tags, vec!["hobbes", "reason"]);
    let links = note.frontmatter.links.unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].link_type, "extends");
    assert_eq!(links[0].target, "def456");
    assert_eq!(links[1].link_type, "complicates");
    assert!(note.content.contains("Hobbes"));
}

#[test]
fn test_parse_regular_note_no_links() {
    let content = r#"---
id: "abc123"
created: 2026-05-15T22:00:00Z
author: agent
type: note
tags: []
---

Regular note content."#;

    let note = Note::parse("test.md", content).unwrap();
    assert_eq!(note.frontmatter.note_type, NoteType::Note);
    assert!(note.frontmatter.links.is_none());
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-gateway -- embeddings::note`

Expected: All pass.

- [ ] **Step 6: Run full test suite**

Run: `cargo test`

Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: add NoteType::Atomic and NoteLink to note parser"
```

---

### Task 3: write_atomic tool

**Files:**
- Create: `crates/river-gateway/src/tools/atomic.rs`
- Modify: `crates/river-gateway/src/tools/mod.rs`
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Create the tool**

Create `crates/river-gateway/src/tools/atomic.rs`:

```rust
//! write_atomic tool — create single-claim knowledge notes with typed links

use chrono::Utc;
use river_core::{RiverError, SnowflakeGenerator, SnowflakeType};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use super::registry::{Tool, ToolResult};

pub struct WriteAtomicTool {
    workspace: PathBuf,
    snowflake_gen: Arc<SnowflakeGenerator>,
    agent_name: String,
}

impl WriteAtomicTool {
    pub fn new(
        workspace: PathBuf,
        snowflake_gen: Arc<SnowflakeGenerator>,
        agent_name: String,
    ) -> Self {
        Self {
            workspace,
            snowflake_gen,
            agent_name,
        }
    }
}

impl Tool for WriteAtomicTool {
    fn name(&self) -> &str {
        "write_atomic"
    }

    fn description(&self) -> &str {
        "Create a single-claim knowledge note with typed links. Content must be ≤100 words. At least one link and one tag required."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Single claim, observation, or connection (max 100 words)"
                },
                "links": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "description": "Link type (extends, complicates, contradicts, supports, resonates-with, etc.)" },
                            "target": { "type": "string", "description": "Target note ID (snowflake hex)" }
                        },
                        "required": ["type", "target"]
                    },
                    "description": "Typed links to other atomic notes (at least one required)"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags for categorization (at least one required)"
                }
            },
            "required": ["content", "links", "tags"]
        })
    }

    fn execute(&self, args: Value) -> Result<ToolResult, RiverError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RiverError::tool("Missing required parameter: content"))?;

        let links = args
            .get("links")
            .and_then(|v| v.as_array())
            .ok_or_else(|| RiverError::tool("Missing required parameter: links"))?;

        let tags = args
            .get("tags")
            .and_then(|v| v.as_array())
            .ok_or_else(|| RiverError::tool("Missing required parameter: tags"))?;

        // Validate content length
        let word_count = content.split_whitespace().count();
        if word_count > 100 {
            return Err(RiverError::tool(format!(
                "Content exceeds 100 word limit ({} words). Atomic notes must state a single claim.",
                word_count
            )));
        }

        // Validate links
        if links.is_empty() {
            return Err(RiverError::tool(
                "At least one link is required. Every atomic note must connect to something.",
            ));
        }
        for (i, link) in links.iter().enumerate() {
            if link.get("type").and_then(|v| v.as_str()).is_none() {
                return Err(RiverError::tool(format!(
                    "Link {} missing 'type' field",
                    i
                )));
            }
            if link.get("target").and_then(|v| v.as_str()).is_none() {
                return Err(RiverError::tool(format!(
                    "Link {} missing 'target' field",
                    i
                )));
            }
        }

        // Validate tags
        if tags.is_empty() {
            return Err(RiverError::tool(
                "At least one tag is required.",
            ));
        }

        // Generate ID
        let id = self.snowflake_gen.next_id(SnowflakeType::AtomicNote);
        let id_hex = id.to_string();

        // Build links YAML
        let links_yaml: Vec<String> = links
            .iter()
            .map(|l| {
                format!(
                    "  - type: {}\n    target: \"{}\"",
                    l["type"].as_str().unwrap(),
                    l["target"].as_str().unwrap()
                )
            })
            .collect();

        // Build tags YAML
        let tags_str: Vec<&str> = tags
            .iter()
            .filter_map(|t| t.as_str())
            .collect();

        // Build frontmatter
        let now = Utc::now().to_rfc3339();
        let frontmatter = format!(
            "---\nid: \"{}\"\ntype: atomic\ncreated: {}\nauthor: {}\nlinks:\n{}\ntags: [{}]\n---",
            id_hex,
            now,
            self.agent_name,
            links_yaml.join("\n"),
            tags_str.join(", "),
        );

        // Build full file
        let file_content = format!("{}\n\n{}\n", frontmatter, content);

        // Write file
        let atomic_dir = self.workspace.join("embeddings").join("atomic");
        std::fs::create_dir_all(&atomic_dir).map_err(|e| {
            RiverError::tool(format!("Failed to create embeddings/atomic/: {}", e))
        })?;

        let filename = format!("{}-z.md", id_hex);
        let file_path = atomic_dir.join(&filename);
        std::fs::write(&file_path, &file_content).map_err(|e| {
            RiverError::tool(format!("Failed to write atomic note: {}", e))
        })?;

        // Build link summary for output
        let link_summary: Vec<String> = links
            .iter()
            .map(|l| {
                format!(
                    "{} → {}",
                    l["type"].as_str().unwrap(),
                    l["target"].as_str().unwrap()
                )
            })
            .collect();

        let output = format!(
            "Created atomic note: {}\nPath: embeddings/atomic/{}\nLinks: {}\nTags: {}",
            id_hex,
            filename,
            link_summary.join(", "),
            tags_str.join(", "),
        );

        Ok(ToolResult::with_file(output, file_path.to_string_lossy().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::{AgentBirth, SnowflakeGenerator};
    use tempfile::TempDir;

    fn test_tool() -> (WriteAtomicTool, TempDir) {
        let temp = TempDir::new().unwrap();
        let birth = AgentBirth::new(2026, 5, 15, 12, 0, 0).unwrap();
        let gen = Arc::new(SnowflakeGenerator::new(birth));
        let tool = WriteAtomicTool::new(
            temp.path().to_path_buf(),
            gen,
            "test-agent".to_string(),
        );
        (tool, temp)
    }

    #[test]
    fn test_schema() {
        let (tool, _temp) = test_tool();
        assert_eq!(tool.name(), "write_atomic");
        let params = tool.parameters();
        assert!(params["properties"]["content"].is_object());
        assert!(params["properties"]["links"].is_object());
        assert!(params["properties"]["tags"].is_object());
    }

    #[test]
    fn test_write_success() {
        let (tool, temp) = test_tool();
        let result = tool.execute(json!({
            "content": "Hobbes requires agreed-upon names for reason to work.",
            "links": [{"type": "extends", "target": "abc123"}],
            "tags": ["hobbes", "reason"]
        })).unwrap();

        assert!(result.output.contains("Created atomic note"));
        assert!(result.output.contains("extends → abc123"));
        assert!(result.output_file.is_some());

        // Verify file exists and parses
        let path = result.output_file.unwrap();
        assert!(std::path::Path::new(&path).exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("type: atomic"));
        assert!(content.contains("Hobbes requires"));
        assert!(content.contains("extends"));
    }

    #[test]
    fn test_reject_over_100_words() {
        let (tool, _temp) = test_tool();
        let long_content = "word ".repeat(101);
        let result = tool.execute(json!({
            "content": long_content.trim(),
            "links": [{"type": "extends", "target": "abc"}],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("100 word limit"));
    }

    #[test]
    fn test_reject_empty_links() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("link is required"));
    }

    #[test]
    fn test_reject_empty_tags() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [{"type": "extends", "target": "abc"}],
            "tags": []
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tag is required"));
    }

    #[test]
    fn test_reject_link_missing_type() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [{"target": "abc"}],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'type'"));
    }

    #[test]
    fn test_reject_link_missing_target() {
        let (tool, _temp) = test_tool();
        let result = tool.execute(json!({
            "content": "A claim.",
            "links": [{"type": "extends"}],
            "tags": ["test"]
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'target'"));
    }
}
```

- [ ] **Step 2: Add to tools/mod.rs**

Add:
```rust
pub mod atomic;
```

And in the re-exports:
```rust
pub use atomic::WriteAtomicTool;
```

- [ ] **Step 3: Register in server.rs**

After the search tool registration, add:

```rust
// Register write_atomic tool
registry.register(Box::new(crate::tools::WriteAtomicTool::new(
    config.workspace.clone(),
    snowflake_gen.clone(),
    agent_name.clone(),
)));
tracing::info!("Registered write_atomic tool");
```

This goes alongside other tool registrations, before the registry is locked.

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-gateway -- tools::atomic`

Expected: All 7 tests pass.

- [ ] **Step 5: Run full test suite**

Run: `cargo test`

Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: write_atomic tool with strict validation, auto-indexed by sync service"
```

---

### Task 4: Full build and push

- [ ] **Step 1: Full build**

Run: `cargo build`

Expected: Clean build.

- [ ] **Step 2: Full test suite**

Run: `cargo test`

Expected: All pass.

- [ ] **Step 3: Push**

```bash
git push
```
