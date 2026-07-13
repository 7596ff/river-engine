//! Shape index (wall ch. 02 addendum; spec:
//! `docs/superpowers/specs/2026-07-12-shape-index-design.md`).
//!
//! One-line "logical skeletons" of atomic notes, indexed in a second
//! embedding namespace so Bridge — the flash type that fires on same
//! move, different vocabulary — has a signal to run against. The
//! witness's compose-shape prompt lives at
//! `workspace/witness/on-shape.md`; a missing file disables the
//! duty. Agent-authored `shape:` frontmatter values override the
//! witness's gloss in the derived table (see `Memory::upsert_shape`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context as _;
use sha2::Digest as _;

use crate::memory::{Memory, ShapeAuthor};
use crate::model::{Chat, ChatMessage};

const NOTE_BODY_SLOT: &str = "{note_body}";

/// The prompt file that gates the witness's shape duty. Cached by
/// mtime so a load-per-turn is a cheap `stat` on the common path.
#[derive(Debug)]
pub struct Prompt {
    path: PathBuf,
    cache: Option<Cached>,
}

#[derive(Debug, Clone)]
struct Cached {
    modified: Option<SystemTime>,
    text: String,
    hash: String,
}

/// Result of a successful prompt load: the text and its sha256 hash.
#[derive(Debug, Clone)]
pub struct LoadedPrompt {
    pub text: String,
    pub hash: String,
}

impl Prompt {
    /// The default path: `workspace/witness/on-shape.md`.
    pub fn at_workspace(workspace: &Path) -> Self {
        Self::at(workspace.join("witness").join("on-shape.md"))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path, cache: None }
    }

    /// Load the prompt file. Returns `None` when the file is missing
    /// (duty disabled). The cache reuses the last read when mtime is
    /// unchanged.
    pub fn load(&mut self) -> anyhow::Result<Option<LoadedPrompt>> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.cache = None;
                return Ok(None);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", self.path.display()));
            }
        };
        let modified = metadata.modified().ok();
        if let Some(cached) = &self.cache {
            if cached.modified == modified {
                return Ok(Some(LoadedPrompt {
                    text: cached.text.clone(),
                    hash: cached.hash.clone(),
                }));
            }
        }
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let hash = sha256_hex(text.as_bytes());
        self.cache = Some(Cached {
            modified,
            text: text.clone(),
            hash: hash.clone(),
        });
        Ok(Some(LoadedPrompt { text, hash }))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for byte in out {
        use std::fmt::Write as _;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// Call the witness model to compose a one-line shape gloss for a
/// piece of text (an atomic note body, or a turn transcript). Returns
/// the trimmed model output; caller decides what to do with it.
///
/// The single template variable `{note_body}` is substituted with the
/// supplied text; other slots pass through verbatim, so the caller's
/// prompt file can carry a fixed preamble.
pub async fn gloss_text<C: Chat + Sync>(
    client: &C,
    prompt: &LoadedPrompt,
    system: &str,
    body: &str,
) -> anyhow::Result<String> {
    let user = prompt.text.replace(NOTE_BODY_SLOT, body);
    let response = client.chat(system, &[ChatMessage::user(user)], &[]).await?;
    Ok(response.content.trim().to_string())
}

/// Compose a shape gloss for the note at `note_path` and upsert it
/// into `shape_vectors`. Returns the gloss text on success. Errors
/// propagate; the caller (worker or write_atomic) decides how loud
/// to be.
pub async fn gloss_note<C: Chat + Sync>(
    client: &C,
    memory: &Memory,
    prompt: &LoadedPrompt,
    system: &str,
    model_id: &str,
    note_id: &str,
    note_path: &str,
    body: &str,
) -> anyhow::Result<String> {
    let gloss = gloss_text(client, prompt, system, body).await?;
    if gloss.is_empty() {
        anyhow::bail!("witness returned empty gloss for {note_id}");
    }
    memory
        .upsert_shape(
            note_id,
            note_path,
            &gloss,
            ShapeAuthor::Witness,
            model_id,
            &prompt.hash,
        )
        .await?;
    Ok(gloss)
}

/// Compose a shape gloss for a turn transcript. No storage side
/// effect — Bridge will embed the returned string separately.
pub async fn gloss_turn<C: Chat + Sync>(
    client: &C,
    prompt: &LoadedPrompt,
    system: &str,
    transcript: &str,
) -> anyhow::Result<String> {
    gloss_text(client, prompt, system, transcript).await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::model::ChatResponse;
    use crate::model::ToolSchema;

    use super::*;

    struct FakeModel {
        replies: Mutex<Vec<anyhow::Result<ChatResponse>>>,
        prompts: Mutex<Vec<(String, String)>>,
    }
    impl FakeModel {
        fn replying(replies: Vec<anyhow::Result<ChatResponse>>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies),
                prompts: Mutex::new(Vec::new()),
            })
        }
    }
    impl Chat for Arc<FakeModel> {
        async fn chat(
            &self,
            system: &str,
            messages: &[ChatMessage],
            _tools: &[ToolSchema],
        ) -> anyhow::Result<ChatResponse> {
            self.prompts
                .lock()
                .unwrap()
                .push((system.to_string(), messages[0].content.clone()));
            self.replies.lock().unwrap().remove(0)
        }
    }
    fn ok(content: &str) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            content: content.into(),
            tool_calls: Vec::new(),
            prompt_tokens: None,
        })
    }

    #[test]
    fn prompt_load_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut prompt = Prompt::at_workspace(dir.path());
        assert!(prompt.load().unwrap().is_none());
    }

    #[test]
    fn prompt_load_returns_text_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("witness")).unwrap();
        std::fs::write(
            dir.path().join("witness/on-shape.md"),
            "State the skeleton. Note body:\n{note_body}\n",
        )
        .unwrap();
        let mut prompt = Prompt::at_workspace(dir.path());
        let loaded = prompt.load().unwrap().unwrap();
        assert!(loaded.text.contains("{note_body}"));
        assert_eq!(loaded.hash.len(), 64, "sha256 hex string");
    }

    #[test]
    fn prompt_load_hash_stable_across_reads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("witness")).unwrap();
        std::fs::write(dir.path().join("witness/on-shape.md"), "same content").unwrap();
        let mut prompt = Prompt::at_workspace(dir.path());
        let first = prompt.load().unwrap().unwrap();
        let second = prompt.load().unwrap().unwrap();
        assert_eq!(first.hash, second.hash);
    }

    #[test]
    fn prompt_hash_changes_when_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("witness/on-shape.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "first").unwrap();
        let mut prompt = Prompt::at_workspace(dir.path());
        let first = prompt.load().unwrap().unwrap();
        // Wait so mtime advances past the cache tick, then rewrite.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "second-different-length").unwrap();
        let second = prompt.load().unwrap().unwrap();
        assert_ne!(first.hash, second.hash);
        assert_ne!(first.text, second.text);
    }

    #[tokio::test]
    async fn gloss_text_substitutes_note_body_and_returns_trimmed() {
        let client = FakeModel::replying(vec![ok("  a proxy under pressure diverges  ")]);
        let prompt = LoadedPrompt {
            text: "State the skeleton. Note body:\n{note_body}\n".into(),
            hash: "h".into(),
        };
        let out = gloss_text(&client, &prompt, "You are witness.", "the actual body")
            .await
            .unwrap();
        assert_eq!(out, "a proxy under pressure diverges");
        let prompts = client.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].0, "You are witness.");
        assert!(prompts[0].1.contains("the actual body"));
        assert!(!prompts[0].1.contains("{note_body}"));
    }

    #[tokio::test]
    async fn gloss_note_upserts_witness_row_with_prompt_hash() {
        use std::sync::Arc as StdArc;
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        let mem = Memory::open(
            &dir.path().join("data"),
            &workspace,
            &[],
            StdArc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();
        let client = FakeModel::replying(vec![ok("skeleton one")]);
        let prompt = LoadedPrompt {
            text: "Q: {note_body}".into(),
            hash: "abcdef".into(),
        };
        gloss_note(
            &client,
            &mem,
            &prompt,
            "sys",
            "haiku-4.5",
            "01ATOM",
            "knowledge/01ATOM.md",
            "the body",
        )
        .await
        .unwrap();
        let row = mem.read_shape("01ATOM").unwrap().unwrap();
        assert_eq!(row.gloss, "skeleton one");
        assert_eq!(row.author, ShapeAuthor::Witness);
        assert_eq!(row.model_id, "haiku-4.5");
        assert_eq!(row.prompt_hash, "abcdef");
    }

    #[tokio::test]
    async fn gloss_note_rejects_empty_model_output() {
        use std::sync::Arc as StdArc;
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        let mem = Memory::open(
            &dir.path().join("data"),
            &workspace,
            &[],
            StdArc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();
        let client = FakeModel::replying(vec![ok("   ")]);
        let prompt = LoadedPrompt {
            text: "{note_body}".into(),
            hash: "h".into(),
        };
        let err = gloss_note(
            &client,
            &mem,
            &prompt,
            "sys",
            "m",
            "01ATOM",
            "knowledge/01ATOM.md",
            "body",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("empty gloss"), "{err}");
        assert!(mem.read_shape("01ATOM").unwrap().is_none());
    }
}
