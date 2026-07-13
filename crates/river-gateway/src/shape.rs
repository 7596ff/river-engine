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

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context as _;
use sha2::Digest as _;
use tokio::sync::mpsc;

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

/// One unit of work for the shape worker: gloss an atomic note (or
/// re-gloss on drift, or gloss a fresh write). The reason is
/// receipt-log detail, not behavior — all three sources take the
/// same path through `process_one`.
#[derive(Debug, Clone)]
pub struct GlossJob {
    pub note_id: String,
    pub note_path: String,
    pub reason: JobReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobReason {
    /// Startup scan: no row for this atomic.
    Missing,
    /// Startup scan: row exists but `(model_id, prompt_hash)`
    /// disagrees with the current witness.
    Drift,
    /// Live from `write_atomic` or the sync service.
    Write,
}

impl JobReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobReason::Missing => "missing",
            JobReason::Drift => "drift",
            JobReason::Write => "write",
        }
    }
}

/// Walk `workspace/knowledge/` and enqueue a `Missing` job for every
/// atomic whose id has no `shape_vectors` row. Returns the count
/// enqueued. Ids are read from frontmatter (line `id: <ulid>`); files
/// without an id line are skipped (the shape worker's contract is
/// about atomics, and atomics carry ids).
pub async fn enqueue_missing(
    memory: &Memory,
    workspace: &Path,
    sender: &mpsc::Sender<GlossJob>,
) -> anyhow::Result<usize> {
    let dir = workspace.join("knowledge");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    let existing: HashSet<String> = memory
        .list_shape_rows()?
        .into_iter()
        .map(|r| r.note_id)
        .collect();
    let mut enqueued = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(id) = read_frontmatter_id(&text) else {
            continue;
        };
        if existing.contains(&id) {
            continue;
        }
        let note_path = format!(
            "knowledge/{}",
            path.file_name().and_then(|s| s.to_str()).unwrap_or("")
        );
        if sender
            .send(GlossJob {
                note_id: id,
                note_path,
                reason: JobReason::Missing,
            })
            .await
            .is_err()
        {
            break;
        }
        enqueued += 1;
    }
    Ok(enqueued)
}

/// Enqueue a `Drift` job for every witness-authored row whose
/// `(model_id, prompt_hash)` disagrees with the current witness.
/// Agent-authored rows are exempt (the agent's `shape:` frontmatter
/// is authoritative; never overwritten).
pub async fn enqueue_drift(
    memory: &Memory,
    sender: &mpsc::Sender<GlossJob>,
    current_model_id: &str,
    current_prompt_hash: &str,
) -> anyhow::Result<usize> {
    let mut enqueued = 0usize;
    for row in memory.list_shape_rows()? {
        if row.author == ShapeAuthor::Agent {
            continue;
        }
        if row.model_id == current_model_id && row.prompt_hash == current_prompt_hash {
            continue;
        }
        if sender
            .send(GlossJob {
                note_id: row.note_id,
                note_path: row.file_path,
                reason: JobReason::Drift,
            })
            .await
            .is_err()
        {
            break;
        }
        enqueued += 1;
    }
    Ok(enqueued)
}

/// Process one queued job. Reads the note file body, glosses it, and
/// upserts. Returns `Ok(None)` when the row is now agent-authored
/// (the write-time race between `write_atomic`'s enqueue and the
/// sync service's frontmatter upsert). Returns `Ok(Some(gloss))` on
/// success. The caller writes the receipt line.
pub async fn process_one<C: Chat + Sync>(
    client: &C,
    memory: &Memory,
    workspace: &Path,
    prompt: &LoadedPrompt,
    system: &str,
    model_id: &str,
    job: &GlossJob,
) -> anyhow::Result<Option<String>> {
    // Agent-authored rows win the race — never overwritten.
    if let Some(existing) = memory.read_shape(&job.note_id)? {
        if existing.author == ShapeAuthor::Agent {
            return Ok(None);
        }
    }
    let path = workspace.join(&job.note_path);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let body = strip_frontmatter(&text).to_string();
    let gloss = gloss_note(
        client,
        memory,
        prompt,
        system,
        model_id,
        &job.note_id,
        &job.note_path,
        &body,
    )
    .await?;
    Ok(Some(gloss))
}

/// Read the `id:` line out of YAML frontmatter, tolerantly (matches
/// the wall's link-resolution style — quoted or bare values both
/// resolve to the bare ulid). Returns None when the file has no
/// frontmatter or no id line.
fn read_frontmatter_id(text: &str) -> Option<String> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    for line in rest[..end].lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("id:") {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Everything after the closing frontmatter delimiter (or the whole
/// text when no frontmatter is present).
fn strip_frontmatter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => rest[end..]
            .trim_start_matches("\n---")
            .trim_start_matches('\n'),
        None => text,
    }
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

    fn make_memory(dir: &Path) -> Memory {
        use std::sync::Arc as StdArc;
        let workspace = dir.join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        Memory::open(
            &dir.join("data"),
            &workspace,
            &[],
            StdArc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap()
    }

    fn write_atomic_file(workspace: &Path, filename: &str, id: &str, body: &str) {
        let text = format!(
            "---\nid: {id}\ncreated: 2026-07-13T00:00:00Z\nlinks:\n  - extends: 01OTHER\n---\n\n{body}\n"
        );
        std::fs::write(workspace.join("knowledge").join(filename), text).unwrap();
    }

    #[test]
    fn read_frontmatter_id_finds_bare_and_quoted() {
        let bare = "---\nid: 01ABC\ncreated: t\n---\nbody";
        assert_eq!(read_frontmatter_id(bare).as_deref(), Some("01ABC"));
        let quoted = "---\nid: \"01DEF\"\ncreated: t\n---\nbody";
        assert_eq!(read_frontmatter_id(quoted).as_deref(), Some("01DEF"));
        let no_id = "---\ncreated: t\n---\nbody";
        assert!(read_frontmatter_id(no_id).is_none());
        let no_fm = "just body\n";
        assert!(read_frontmatter_id(no_fm).is_none());
    }

    #[test]
    fn strip_frontmatter_returns_body_only() {
        let with = "---\nid: 01A\n---\n\nthe body\n";
        assert_eq!(strip_frontmatter(with), "the body\n");
        let without = "just body\n";
        assert_eq!(strip_frontmatter(without), "just body\n");
    }

    #[tokio::test]
    async fn enqueue_missing_enqueues_new_atomics_only() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01A.md", "01A", "claim a");
        write_atomic_file(&workspace, "01B.md", "01B", "claim b");
        // Pre-populate a row for 01B so it's not enqueued again.
        mem.upsert_shape("01B", "knowledge/01B.md", "existing", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let n = enqueue_missing(&mem, &workspace, &tx).await.unwrap();
        drop(tx);
        assert_eq!(n, 1);
        let job = rx.recv().await.unwrap();
        assert_eq!(job.note_id, "01A");
        assert_eq!(job.note_path, "knowledge/01A.md");
        assert_eq!(job.reason, JobReason::Missing);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn enqueue_missing_skips_files_without_id() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::write(
            workspace.join("knowledge/no-id.md"),
            "---\ncreated: t\n---\nbody",
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let n = enqueue_missing(&mem, &workspace, &tx).await.unwrap();
        drop(tx);
        assert_eq!(n, 0);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn enqueue_drift_targets_only_stale_witness_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        mem.upsert_shape("01FRESH", "knowledge/01FRESH.md", "g", ShapeAuthor::Witness, "cur", "curh")
            .await
            .unwrap();
        mem.upsert_shape("01STALE", "knowledge/01STALE.md", "g", ShapeAuthor::Witness, "old", "oldh")
            .await
            .unwrap();
        mem.upsert_shape("01AGENT", "knowledge/01AGENT.md", "g", ShapeAuthor::Agent, "agent", "")
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let n = enqueue_drift(&mem, &tx, "cur", "curh").await.unwrap();
        drop(tx);
        assert_eq!(n, 1, "only 01STALE drifts; 01FRESH matches; 01AGENT exempt");
        let job = rx.recv().await.unwrap();
        assert_eq!(job.note_id, "01STALE");
        assert_eq!(job.reason, JobReason::Drift);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn process_one_upserts_witness_gloss() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01X.md", "01X", "the atomic body");
        let client = FakeModel::replying(vec![ok("a skeleton for X")]);
        let prompt = LoadedPrompt {
            text: "Body: {note_body}".into(),
            hash: "h".into(),
        };
        let job = GlossJob {
            note_id: "01X".into(),
            note_path: "knowledge/01X.md".into(),
            reason: JobReason::Missing,
        };
        let result = process_one(&client, &mem, &workspace, &prompt, "sys", "m1", &job)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, "a skeleton for X");
        let row = mem.read_shape("01X").unwrap().unwrap();
        assert_eq!(row.author, ShapeAuthor::Witness);
        assert_eq!(row.gloss, "a skeleton for X");

        // Verify frontmatter stripped from the model call — the
        // captured user message should NOT contain "id:".
        let prompts = client.prompts.lock().unwrap();
        assert!(!prompts[0].1.contains("id: 01X"), "frontmatter stripped: {}", prompts[0].1);
        assert!(prompts[0].1.contains("the atomic body"));
    }

    #[tokio::test]
    async fn process_one_skips_agent_authored_row() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01Y.md", "01Y", "body");
        // Pre-existing agent-authored row wins the race.
        mem.upsert_shape(
            "01Y",
            "knowledge/01Y.md",
            "the agent's own skeleton",
            ShapeAuthor::Agent,
            "agent",
            "",
        )
        .await
        .unwrap();
        // No replies queued — if the model is called, this fails.
        let client = FakeModel::replying(vec![]);
        let prompt = LoadedPrompt {
            text: "{note_body}".into(),
            hash: "h".into(),
        };
        let job = GlossJob {
            note_id: "01Y".into(),
            note_path: "knowledge/01Y.md".into(),
            reason: JobReason::Write,
        };
        let result = process_one(&client, &mem, &workspace, &prompt, "sys", "m", &job)
            .await
            .unwrap();
        assert!(result.is_none(), "skipped");
        let row = mem.read_shape("01Y").unwrap().unwrap();
        assert_eq!(row.gloss, "the agent's own skeleton", "unchanged");
        assert_eq!(row.author, ShapeAuthor::Agent);
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
