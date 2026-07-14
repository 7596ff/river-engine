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
#[allow(dead_code)] // Bridge landing pad; pending flash-subsystem spec.
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

/// Long-running task: drain the shape queue, gloss each job,
/// append a receipt line. Stops when `shutdown` flips true (any
/// in-flight gloss finishes before returning). Reloads the prompt
/// on each iteration so an operator's mid-run edit picks up
/// automatically on the next job.
pub async fn run_worker<C: Chat + Sync + Send + 'static>(
    mut receiver: mpsc::Receiver<GlossJob>,
    client: C,
    memory: Memory,
    workspace: PathBuf,
    mut prompt: Prompt,
    system: String,
    model_id: String,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // Fast-path: bail if shutdown was already flipped before we
    // spawned.
    if *shutdown.borrow() {
        return Ok(());
    }
    loop {
        tokio::select! {
            biased;
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    tracing::info!("shape worker shutting down");
                    return Ok(());
                }
                continue;
            }
            job = receiver.recv() => {
                let Some(job) = job else {
                    return Ok(());
                };
                let Some(loaded) = prompt.load()? else {
                    // No prompt → duty disabled; drop this job.
                    tracing::debug!(
                        note_id = %job.note_id,
                        "on-shape.md missing; dropping gloss job"
                    );
                    continue;
                };
                let outcome = process_one(
                    &client,
                    &memory,
                    &workspace,
                    &loaded,
                    &system,
                    &model_id,
                    &job,
                )
                .await;
                match outcome {
                    Ok(Some(gloss)) => {
                        let entry = ShapeLogEntry {
                            note_id: job.note_id.clone(),
                            author: "witness".into(),
                            model_id: model_id.clone(),
                            prompt_hash: loaded.hash.clone(),
                            gloss,
                            reason: job.reason.as_str().into(),
                            at: jiff::Timestamp::now().to_string(),
                        };
                        if let Err(e) = append_shape_log(&workspace, &entry) {
                            tracing::warn!(
                                note_id = %job.note_id,
                                error = %e,
                                "shape-log append failed"
                            );
                        }
                    }
                    Ok(None) => {
                        // Skipped (agent-authored row). Non-event.
                    }
                    Err(e) => {
                        tracing::warn!(
                            note_id = %job.note_id,
                            error = %e,
                            "shape gloss failed; job dropped"
                        );
                    }
                }
            }
        }
    }
}

/// One receipt-log line for a completed gloss job. Append-only,
/// torn-line tolerant (same shape as glean-log.jsonl,
/// connect-log.jsonl).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShapeLogEntry {
    pub note_id: String,
    pub author: String,
    pub model_id: String,
    pub prompt_hash: String,
    pub gloss: String,
    pub reason: String,
    pub at: String,
}

/// Append one receipt line to `workspace/witness/shape-log.jsonl`.
/// Creates the parent directory if absent. Uses fsync per line so
/// crash recovery has ground truth to work with.
pub fn append_shape_log(workspace: &Path, entry: &ShapeLogEntry) -> anyhow::Result<()> {
    let dir = workspace.join("witness");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("shape-log.jsonl");
    let mut json = serde_json::to_string(entry)?;
    json.push('\n');
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("fsyncing {}", path.display()))?;
    Ok(())
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
    // Empty frontmatter: `---\n---\n...` — closing delimiter is at
    // position 0 of `rest`, with no preceding newline, so the
    // find("\n---") below wouldn't see it.
    if let Some(after) = rest.strip_prefix("---") {
        return after.trim_start_matches('\n');
    }
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

    #[tokio::test]
    async fn worker_processes_job_and_writes_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01W.md", "01W", "the body");
        std::fs::create_dir_all(workspace.join("witness")).unwrap();
        std::fs::write(
            workspace.join("witness/on-shape.md"),
            "Body: {note_body}",
        )
        .unwrap();

        let (tx, rx) = mpsc::channel(4);
        tx.send(GlossJob {
            note_id: "01W".into(),
            note_path: "knowledge/01W.md".into(),
            reason: JobReason::Write,
        })
        .await
        .unwrap();
        drop(tx); // close so recv returns None after processing

        let client = FakeModel::replying(vec![ok("worker output")]);
        let prompt = Prompt::at_workspace(&workspace);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        run_worker(
            rx,
            client,
            mem.clone(),
            workspace.clone(),
            prompt,
            "sys".into(),
            "m1".into(),
            shutdown_rx,
        )
        .await
        .unwrap();

        let row = mem.read_shape("01W").unwrap().unwrap();
        assert_eq!(row.gloss, "worker output");
        assert_eq!(row.author, ShapeAuthor::Witness);

        let log = std::fs::read_to_string(workspace.join("witness/shape-log.jsonl")).unwrap();
        assert!(log.contains("01W"), "receipt written: {log}");
        assert!(log.contains("worker output"));
        let _ = shutdown_tx; // silence unused
    }

    #[tokio::test]
    async fn worker_stops_on_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");

        let (_tx, rx) = mpsc::channel::<GlossJob>(4);
        let client = FakeModel::replying(vec![]);
        let prompt = Prompt::at_workspace(&workspace);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let handle = tokio::spawn(async move {
            run_worker(
                rx,
                client,
                mem,
                workspace,
                prompt,
                "sys".into(),
                "m".into(),
                shutdown_rx,
            )
            .await
        });
        tokio::task::yield_now().await;
        shutdown_tx.send(true).unwrap();
        // Should return promptly.
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("shutdown timely")
            .unwrap()
            .unwrap();
    }

    #[test]
    fn append_shape_log_writes_json_line() {
        let dir = tempfile::tempdir().unwrap();
        let entry = ShapeLogEntry {
            note_id: "01ATOM".into(),
            author: "witness".into(),
            model_id: "haiku".into(),
            prompt_hash: "h".into(),
            gloss: "a skeleton".into(),
            reason: "missing".into(),
            at: "2026-07-13T00:00:00Z".into(),
        };
        append_shape_log(dir.path(), &entry).unwrap();
        append_shape_log(dir.path(), &entry).unwrap();
        let text = std::fs::read_to_string(dir.path().join("witness/shape-log.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("01ATOM"));
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

    // -------- hunt tests --------
    //
    // Each names the drift it hunts. Same discipline as flashes.rs:
    // if the assertion can't fail on a plausible regression, don't
    // add it.

    /// Hunts: someone changes the missing-prompt branch from `continue`
    /// to `return Err(_)`, killing the whole worker on any job that
    /// races the operator deleting `on-shape.md`. Spec calls this
    /// "duty disabled; drop this job."
    #[tokio::test]
    async fn worker_drops_job_when_prompt_missing_no_bail() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01Z.md", "01Z", "body");
        // Intentionally do NOT create witness/on-shape.md.

        let (tx, rx) = mpsc::channel(4);
        tx.send(GlossJob {
            note_id: "01Z".into(),
            note_path: "knowledge/01Z.md".into(),
            reason: JobReason::Write,
        })
        .await
        .unwrap();
        drop(tx);

        // If the worker tried to call the model, this replies-empty
        // client would panic — proving the job was dropped upstream.
        let client = FakeModel::replying(vec![]);
        let prompt = Prompt::at_workspace(&workspace);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        run_worker(
            rx,
            client,
            mem.clone(),
            workspace.clone(),
            prompt,
            "sys".into(),
            "m1".into(),
            shutdown_rx,
        )
        .await
        .expect("worker must not bail on missing prompt");

        assert!(mem.read_shape("01Z").unwrap().is_none(), "no row written");
        assert!(
            !workspace.join("witness/shape-log.jsonl").exists(),
            "no receipt when duty disabled"
        );
    }

    /// Hunts: worker appends a receipt even when process_one returned
    /// Ok(None) (the agent-authored skip). Regression would fill the
    /// shape-log with phantom "witness"-tagged entries for atomics
    /// the agent owns.
    #[tokio::test]
    async fn worker_skips_receipt_on_agent_authored_skip() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01AGENT.md", "01AGENT", "body");
        std::fs::create_dir_all(workspace.join("witness")).unwrap();
        std::fs::write(workspace.join("witness/on-shape.md"), "{note_body}").unwrap();

        // Pre-populate agent-authored row.
        mem.upsert_shape(
            "01AGENT",
            "knowledge/01AGENT.md",
            "the agent's own skeleton",
            ShapeAuthor::Agent,
            "agent",
            "",
        )
        .await
        .unwrap();

        let (tx, rx) = mpsc::channel(4);
        tx.send(GlossJob {
            note_id: "01AGENT".into(),
            note_path: "knowledge/01AGENT.md".into(),
            reason: JobReason::Write,
        })
        .await
        .unwrap();
        drop(tx);

        // Empty replies: if process_one *did* call the model, it panics.
        let client = FakeModel::replying(vec![]);
        let prompt = Prompt::at_workspace(&workspace);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        run_worker(
            rx,
            client,
            mem.clone(),
            workspace.clone(),
            prompt,
            "sys".into(),
            "m1".into(),
            shutdown_rx,
        )
        .await
        .unwrap();

        // Row unchanged.
        let row = mem.read_shape("01AGENT").unwrap().unwrap();
        assert_eq!(row.author, ShapeAuthor::Agent);
        assert_eq!(row.gloss, "the agent's own skeleton");
        // Critically: no receipt line for this note_id.
        let log_path = workspace.join("witness/shape-log.jsonl");
        if log_path.exists() {
            let text = std::fs::read_to_string(&log_path).unwrap();
            assert!(
                !text.contains("01AGENT"),
                "phantom receipt for agent-authored row: {text}"
            );
        }
    }

    /// Hunts: someone changes the per-job error branch from
    /// warn+continue to bail+return, freezing the whole shape
    /// backfill on one bad note. Send job A that fails, then B that
    /// succeeds; verify B lands and worker exits cleanly.
    #[tokio::test]
    async fn worker_continues_after_per_job_gloss_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01BAD.md", "01BAD", "body a");
        write_atomic_file(&workspace, "01OK.md", "01OK", "body b");
        std::fs::create_dir_all(workspace.join("witness")).unwrap();
        std::fs::write(workspace.join("witness/on-shape.md"), "{note_body}").unwrap();

        let (tx, rx) = mpsc::channel(4);
        tx.send(GlossJob {
            note_id: "01BAD".into(),
            note_path: "knowledge/01BAD.md".into(),
            reason: JobReason::Missing,
        })
        .await
        .unwrap();
        tx.send(GlossJob {
            note_id: "01OK".into(),
            note_path: "knowledge/01OK.md".into(),
            reason: JobReason::Missing,
        })
        .await
        .unwrap();
        drop(tx);

        // First reply errors, second succeeds.
        let client = FakeModel::replying(vec![
            Err(anyhow::anyhow!("model exploded")),
            ok("skeleton for OK"),
        ]);
        let prompt = Prompt::at_workspace(&workspace);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        run_worker(
            rx,
            client,
            mem.clone(),
            workspace.clone(),
            prompt,
            "sys".into(),
            "m1".into(),
            shutdown_rx,
        )
        .await
        .expect("worker must not bail on per-job failure");

        // 01BAD has no row (gloss failed).
        assert!(mem.read_shape("01BAD").unwrap().is_none());
        // 01OK does — proving the worker reached the second job.
        let row = mem.read_shape("01OK").unwrap().unwrap();
        assert_eq!(row.gloss, "skeleton for OK");
    }

    /// Hunts: someone caches the LoadedPrompt for the worker's lifetime
    /// (e.g., moves prompt.load() outside the loop), breaking the
    /// documented "reloads on each iteration so an operator's mid-run
    /// edit picks up." The prompt_hash on the receipt row is the
    /// tell — a mid-run edit must change it.
    #[tokio::test]
    async fn worker_reloads_prompt_between_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01A.md", "01A", "body a");
        write_atomic_file(&workspace, "01B.md", "01B", "body b");
        std::fs::create_dir_all(workspace.join("witness")).unwrap();
        let prompt_path = workspace.join("witness/on-shape.md");
        std::fs::write(&prompt_path, "v1: {note_body}").unwrap();

        // A channel we can drive step by step. Buffer 1 so send
        // blocks until the worker consumes the first job.
        let (tx, rx) = mpsc::channel::<GlossJob>(1);
        let client = FakeModel::replying(vec![ok("gloss a"), ok("gloss b")]);
        let prompt = Prompt::at_workspace(&workspace);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let mem2 = mem.clone();
        let workspace2 = workspace.clone();
        let handle = tokio::spawn(async move {
            run_worker(
                rx,
                client,
                mem2,
                workspace2,
                prompt,
                "sys".into(),
                "m1".into(),
                shutdown_rx,
            )
            .await
        });

        // Job A → worker glosses with v1.
        tx.send(GlossJob {
            note_id: "01A".into(),
            note_path: "knowledge/01A.md".into(),
            reason: JobReason::Write,
        })
        .await
        .unwrap();
        // Wait for the row to appear (worker finished A).
        for _ in 0..100 {
            if mem.read_shape("01A").unwrap().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let hash_a = mem.read_shape("01A").unwrap().unwrap().prompt_hash;

        // Rewrite prompt so hash changes. Sleep to advance mtime past
        // the mtime-cache tick.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        std::fs::write(&prompt_path, "v2: {note_body} — different content").unwrap();

        tx.send(GlossJob {
            note_id: "01B".into(),
            note_path: "knowledge/01B.md".into(),
            reason: JobReason::Write,
        })
        .await
        .unwrap();
        drop(tx);
        handle.await.unwrap().unwrap();

        let hash_b = mem.read_shape("01B").unwrap().unwrap().prompt_hash;
        assert_ne!(
            hash_a, hash_b,
            "worker should reload prompt between jobs; hashes must differ"
        );

        // The receipt log entries should carry the same drift.
        let log = std::fs::read_to_string(workspace.join("witness/shape-log.jsonl")).unwrap();
        assert!(log.contains(&hash_a), "receipt for A carries v1 hash");
        assert!(log.contains(&hash_b), "receipt for B carries v2 hash");
    }

    // -------- Tier B: enqueue edges --------

    /// Hunts: someone removes the `if extension != "md" continue`
    /// guard. The subsystem starts trying to read binary attachments
    /// as text, wasting cycles and cluttering logs with warnings.
    #[tokio::test]
    async fn enqueue_missing_skips_non_md_files() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let workspace = dir.path().join("ws");
        write_atomic_file(&workspace, "01A.md", "01A", "atomic body");
        std::fs::write(workspace.join("knowledge/note.txt"), "---\nid: 01B\n---\nbody").unwrap();
        std::fs::write(workspace.join("knowledge/photo.png"), &[0u8, 1, 2, 3]).unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let n = enqueue_missing(&mem, &workspace, &tx).await.unwrap();
        drop(tx);
        assert_eq!(n, 1, "only the .md file enqueued");
        let job = rx.recv().await.unwrap();
        assert_eq!(job.note_id, "01A");
        assert!(rx.recv().await.is_none());
    }

    /// Hunts: someone changes the NotFound branch on `read_dir` from
    /// `Ok(0)` to `Err(_)`, making the shape subsystem fail to start
    /// on a fresh workspace with no knowledge/ dir yet.
    #[tokio::test]
    async fn enqueue_missing_returns_zero_when_knowledge_dir_absent() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        // No knowledge/ subdir.
        let mem_dir = dir.path().join("data");
        let mem = Memory::open(
            &mem_dir,
            &workspace,
            &[],
            Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let n = enqueue_missing(&mem, &workspace, &tx).await.unwrap();
        drop(tx);
        assert_eq!(n, 0, "missing knowledge/ dir returns 0, not error");
        assert!(rx.recv().await.is_none());
    }

    // -------- Tier C: parser adversarials --------

    /// Hunts: someone tightens strip_frontmatter to bail or panic on
    /// malformed atomics instead of degrading to "whole text."
    /// Downstream (gloss_note) then sees corrupted body and either
    /// gloss-fails or embeds frontmatter garbage into the shape.
    #[test]
    fn strip_frontmatter_degrades_gracefully_on_malformed() {
        // Missing opening ---: return whole text.
        assert_eq!(strip_frontmatter("no fm here\nline"), "no fm here\nline");
        // Missing closing ---: return whole text (no truncation).
        assert_eq!(
            strip_frontmatter("---\nid: 01A\nnever closed"),
            "---\nid: 01A\nnever closed"
        );
        // Empty frontmatter with empty body.
        let out = strip_frontmatter("---\n---\n");
        assert!(out.is_empty() || out == "\n", "empty fm produces empty body, got {out:?}");
        // `---` sequence inside body doesn't confuse the parser once
        // the opening was properly closed.
        let with_dashes_in_body = "---\nid: 01A\n---\n\nline one\n---\nline two\n";
        let stripped = strip_frontmatter(with_dashes_in_body);
        assert!(stripped.contains("line one"));
        assert!(stripped.contains("line two"));
        assert!(!stripped.contains("id: 01A"));
    }

    /// Hunts: someone relaxes read_frontmatter_id and returns Some("")
    /// or Some("value:with:colon") when they shouldn't. Empty-id rows
    /// would silently poison the shape_vectors table with unfindable
    /// entries.
    #[test]
    fn read_frontmatter_id_rejects_empty_and_handles_adversarial_values() {
        // Empty value → None (line 464: `if !value.is_empty()`).
        assert!(read_frontmatter_id("---\nid: \n---\nbody").is_none());
        assert!(read_frontmatter_id("---\nid:\n---\nbody").is_none());
        // Trailing whitespace still resolves after trim.
        assert_eq!(
            read_frontmatter_id("---\nid: 01WS   \n---\nbody").as_deref(),
            Some("01WS")
        );
        // Colons in the value pass through as part of the raw string
        // (yaml-lite behavior; we don't split on further colons).
        assert_eq!(
            read_frontmatter_id("---\nid: has:colons:in:it\n---\nbody").as_deref(),
            Some("has:colons:in:it")
        );
        // `id:` appearing only in the body (after closing ---) is not
        // returned — the parser stops at the closing delimiter.
        let with_body_id = "---\ncreated: t\n---\nid: 01BODY\nreal body";
        assert!(read_frontmatter_id(with_body_id).is_none());
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
