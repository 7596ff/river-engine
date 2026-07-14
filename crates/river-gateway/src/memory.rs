//! The memory system's stores and index (wall chs. 02, 10): one
//! SQLite file per agent holding only derived state (vector segments,
//! sync hashes) and ephemeral state (activation, extraction queue).
//! Ground truth is workspace files; delete the database and the sync
//! service rebuilds everything — warmth and pending digestion are the
//! only losses.
//!
//! The sync sweep hashes every file under the watched directories,
//! re-embedding new or changed content in segments and removing
//! vectors for deleted files. Search is cosine similarity. Every
//! search hit is an ambient access (bump 0.5); reads through the
//! tool seam are cognitive accesses (bump 1.0) — propagation, decay,
//! and flash arrive with the activation dynamics (roadmap step 5).

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension as _};
use sha2::Digest as _;

use river_core::config::ModelConfig;

// Activation dynamics are knobs now (ActivationConfig, wall ch. 02):
// per-agent, optional, defaulting to the wall's constants. What stays
// constant here is mechanics, not dynamics.
const SEGMENT_TARGET_BYTES: usize = 1200;
const SEGMENT_HARD_CAP: usize = 4 * SEGMENT_TARGET_BYTES;
const SEGMENT_MIN_CAP: usize = 600;
const DECAY_INTERVAL_SECS: u64 = 3600;
// Flash bodies are capped: atomics never notice, but a path-keyed
// node (a transcript, a chapter) must not dump itself whole into the
// memory slot. Neighbors are capped in count, typed links first.
const FLASH_TEXT_CAP: usize = 1200;
const FLASH_NEIGHBOR_CAP: usize = 6;

/// What carried a bump (wall ch. 02): only ambient or propagated
/// warmth can flash a note — the flash carrier rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carrier {
    Cognitive,
    Ambient,
    Propagated,
}

#[derive(Debug)]
struct BumpOp {
    note_id: String,
    amount: f64,
    carrier: Carrier,
}

/// A pending flash: surfaced into the next context's memory slot.
#[derive(Debug, Clone)]
pub struct Flash {
    pub note_id: String,
    pub text: String,
    pub neighbors: Vec<(String, String)>, // (link type, neighbor text)
}

/// GET /graph payload (board card): the activation graph as JSON.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphPayload {
    pub flash_threshold: f64,
    pub flash_dirs: Vec<String>,
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphNode {
    pub id: String,
    /// Workspace-relative where possible.
    pub path: String,
    pub score: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub link_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
}

type EmbedFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<Vec<Vec<f32>>>> + Send + 'a>>;

/// The embedding seam: real client or test fake.
pub trait Embed: Send + Sync {
    fn embed<'a>(&'a self, texts: &'a [String]) -> EmbedFuture<'a>;
}

/// OpenAI-compatible /embeddings endpoint.
pub struct EmbeddingClient {
    http: reqwest::Client,
    endpoint: String,
    model_name: String,
    api_key_env: Option<String>,
}

impl EmbeddingClient {
    pub fn new(config: &ModelConfig) -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(
                    config.request_timeout_seconds,
                ))
                .build()?,
            endpoint: config.endpoint.trim_end_matches('/').to_string(),
            model_name: config.name.clone(),
            api_key_env: config.api_key_env.clone(),
        })
    }
}

impl Embed for EmbeddingClient {
    fn embed<'a>(&'a self, texts: &'a [String]) -> EmbedFuture<'a> {
        Box::pin(async move {
            let mut request = self
                .http
                .post(format!("{}/embeddings", self.endpoint))
                .json(&serde_json::json!({ "model": self.model_name, "input": texts }));
            if let Some(var) = &self.api_key_env {
                let key = std::env::var(var)
                    .map_err(|_| anyhow::anyhow!("api_key_env {var} is not set"))?;
                request = request.header("authorization", format!("Bearer {key}"));
            }
            let response = request.send().await?.error_for_status()?;
            let value: serde_json::Value = response.json().await?;
            let data = value["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("missing data array in embeddings response"))?;
            data.iter()
                .map(|item| {
                    item["embedding"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("missing embedding"))
                        .map(|v| {
                            v.iter()
                                .filter_map(|x| x.as_f64())
                                .map(|x| x as f32)
                                .collect()
                        })
                })
                .collect()
        })
    }
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub file_path: String,
    pub text: String,
    pub score: f32,
}

/// One retrieved past rejection: what the agent turned away and how
/// close it lands to the current glean window.
#[derive(Debug, Clone)]
pub struct SimilarRejection {
    pub candidate_id: String,
    pub turn: u64,
    pub candidate: String,
    pub reason: Option<String>,
    pub score: f32,
}

/// Who authored a shape gloss (wall ch. 04's divided-authorship
/// discipline). Agent-authored rows come from the atomic's `shape:`
/// frontmatter; witness-authored rows come from the gloss worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeAuthor {
    Witness,
    Agent,
}

impl ShapeAuthor {
    fn as_str(&self) -> &'static str {
        match self {
            ShapeAuthor::Witness => "witness",
            ShapeAuthor::Agent => "agent",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "witness" => Some(ShapeAuthor::Witness),
            "agent" => Some(ShapeAuthor::Agent),
            _ => None,
        }
    }
}

/// One row of `shape_vectors`. The embedding stays inside `Memory`;
/// callers work with the metadata + gloss text. `gloss` and `at`
/// are consumed by Bridge's frame body (deferred: pending flash
/// subsystem) and by the shape-log receipt in `shape::run_worker`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // `gloss` and `at` are Bridge-facing; kept for the flash-subsystem landing.
pub struct ShapeRow {
    pub note_id: String,
    pub file_path: String,
    pub gloss: String,
    pub author: ShapeAuthor,
    pub model_id: String,
    pub prompt_hash: String,
    pub at: String,
}

/// Mirror of one `rejections.jsonl` line, used only for the startup
/// rebuild path. The tool writes this shape; the witness reads it into
/// its own `RejectionEntry` for prompt rendering.
#[derive(serde::Deserialize)]
struct RejectionJsonl {
    candidate_id: String,
    candidate: String,
    #[serde(default)]
    reason: Option<String>,
    turn: u64,
    at: String,
}

/// The agent's memory store: database + embedder, shared by the sync
/// task, the search tool, and the capture seam.
#[derive(Clone)]
pub struct Memory {
    db: Arc<Mutex<Connection>>,
    embedder: Arc<dyn Embed>,
    workspace: PathBuf,
    watched: Vec<PathBuf>,
    knobs: Arc<river_core::config::ActivationConfig>,
    pending_flashes: Arc<Mutex<Vec<Flash>>>,
    queue_notify: Arc<tokio::sync::Notify>,
    graph_generation: Arc<AtomicU64>,
    graph_cache: Arc<Mutex<Option<CachedGraph>>>,
    /// Wired after construction by main.rs when the shape subsystem
    /// is configured. On sync events over `knowledge/`, sweep either
    /// enqueues a Missing job (no shape row and no `shape:`
    /// frontmatter) or upserts an agent-authored row directly (a
    /// `shape:` field is present). None disables the seam entirely,
    /// leaving Memory's behavior identical to a pre-shape build.
    shape_queue: Arc<Mutex<Option<tokio::sync::mpsc::Sender<crate::shape::GlossJob>>>>,
}

/// One indexed file's graph identity and links, derived from the
/// workspace into a disposable process-local snapshot. Files with
/// frontmatter are keyed by id; files without are keyed by path
/// (board card: wikilinks join the graph). Links carry frontmatter
/// typed links plus body wikilinks as type "wiki".
#[derive(Debug)]
struct NoteInfo {
    id: String,
    path: PathBuf,
    body: String,
    links: Vec<(String, String)>, // (type, target)
}

struct CachedGraph {
    generation: u64,
    snapshot: Arc<GraphSnapshot>,
}

/// One immutable view of the authored graph and its derived vectors.
/// A generation is published atomically: topology and semantic
/// identities therefore never come from different sync generations.
struct GraphSnapshot {
    notes: Vec<NoteInfo>,
    note_by_id: std::collections::HashMap<String, usize>,
    id_by_path: std::collections::HashMap<String, String>,
    resolver: Resolver,
    adjacency: std::collections::HashMap<String, Vec<String>>,
    resolved_links: Vec<ResolvedLink>,
    file_vectors: Vec<(String, Vec<f32>)>,
}

struct ResolvedLink {
    source: String,
    target: String,
    link_type: String,
}

/// The queue is ephemeral, but preserve pending candidates when
/// upgrading from the original ULID-ordered schema. A normal SQLite
/// rowid table assigns rowids in insertion order, so copying those
/// values into the explicit sequence recovers the intended FIFO even
/// when two ULIDs share a timestamp and their random suffixes invert.
fn migrate_extraction_queue(conn: &mut Connection) -> anyhow::Result<()> {
    let has_enqueue_seq = {
        let mut statement = conn.prepare("PRAGMA table_info(extraction_queue)")?;
        let mut rows = statement.query([])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == "enqueue_seq" {
                found = true;
                break;
            }
        }
        found
    };
    if has_enqueue_seq {
        return Ok(());
    }

    let transaction = conn.transaction()?;
    transaction.execute_batch(
        "ALTER TABLE extraction_queue RENAME TO extraction_queue_legacy;
         CREATE TABLE extraction_queue (
             enqueue_seq INTEGER PRIMARY KEY AUTOINCREMENT,
             id          TEXT NOT NULL UNIQUE,
             candidate   TEXT NOT NULL,
             created_at  INTEGER NOT NULL
         );
         INSERT INTO extraction_queue (enqueue_seq, id, candidate, created_at)
             SELECT rowid, id, candidate, created_at
             FROM extraction_queue_legacy
             ORDER BY rowid;
         DROP TABLE extraction_queue_legacy;",
    )?;
    transaction.commit()?;
    Ok(())
}

impl Memory {
    /// Test convenience — production uses `open_with` to thread the
    /// activation config through.
    #[cfg(test)]
    pub fn open(
        data_dir: &Path,
        workspace: &Path,
        index_dirs: &[String],
        embedder: Arc<dyn Embed>,
    ) -> anyhow::Result<Self> {
        Self::open_with(
            data_dir,
            workspace,
            index_dirs,
            embedder,
            river_core::config::ActivationConfig::default(),
        )
    }

    /// Open with explicit activation knobs (wall ch. 02): the
    /// per-agent `activation` config block, defaults = the wall's
    /// constants.
    pub fn open_with(
        data_dir: &Path,
        workspace: &Path,
        index_dirs: &[String],
        embedder: Arc<dyn Embed>,
        knobs: river_core::config::ActivationConfig,
    ) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let mut conn = Connection::open(data_dir.join("river.db"))
            .with_context(|| format!("opening {}", data_dir.join("river.db").display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS extraction_queue (
                 enqueue_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 id          TEXT NOT NULL UNIQUE,
                 candidate   TEXT NOT NULL,
                 created_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS activation (
                 note_id     TEXT PRIMARY KEY,
                 score       REAL NOT NULL,
                 bumped_at   INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS segments (
                 id          TEXT PRIMARY KEY,
                 file_path   TEXT NOT NULL,
                 seq         INTEGER NOT NULL,
                 text        TEXT NOT NULL,
                 embedding   BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_segments_path ON segments (file_path);
             CREATE TABLE IF NOT EXISTS file_hashes (
                 file_path   TEXT PRIMARY KEY,
                 hash        TEXT NOT NULL,
                 indexed_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS rejection_vectors (
                 candidate_id TEXT PRIMARY KEY,
                 turn         INTEGER NOT NULL,
                 candidate    TEXT NOT NULL,
                 reason       TEXT,
                 at           TEXT NOT NULL,
                 embedding    BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_rejection_vectors_turn
               ON rejection_vectors (turn);
             CREATE TABLE IF NOT EXISTS shape_vectors (
                 note_id      TEXT PRIMARY KEY,
                 file_path    TEXT NOT NULL,
                 gloss        TEXT NOT NULL,
                 author       TEXT NOT NULL,
                 model_id     TEXT NOT NULL,
                 prompt_hash  TEXT NOT NULL,
                 embedding    BLOB NOT NULL,
                 at           TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_shape_vectors_file
               ON shape_vectors (file_path);",
        )?;
        migrate_extraction_queue(&mut conn)?;

        // knowledge/, loom/, and record/moments/ are always watched
        // (wall chs. 02, 03, 08); config adds more. Paths are
        // normalized (`.` components dropped) and deduplicated so
        // `index_dirs: ["."]` cannot index the same file twice under
        // two spellings.
        let mut watched: Vec<PathBuf> = Vec::new();
        for dir in ["knowledge", "loom", "record/moments"]
            .iter()
            .map(|d| workspace.join(d))
            .chain(index_dirs.iter().map(|d| workspace.join(d)))
        {
            let dir: PathBuf = dir.components().collect();
            if !watched.contains(&dir) {
                watched.push(dir);
            }
        }
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            embedder,
            workspace: workspace.to_path_buf(),
            watched,
            knobs: Arc::new(knobs),
            pending_flashes: Arc::new(Mutex::new(Vec::new())),
            queue_notify: Arc::new(tokio::sync::Notify::new()),
            graph_generation: Arc::new(AtomicU64::new(0)),
            graph_cache: Arc::new(Mutex::new(None)),
            shape_queue: Arc::new(Mutex::new(None)),
        })
    }

    /// Wire the shape worker's queue. Idempotent — the last sender
    /// wins. Passing `None` disables the seam.
    pub fn set_shape_queue(
        &self,
        sender: Option<tokio::sync::mpsc::Sender<crate::shape::GlossJob>>,
    ) {
        *self.shape_queue.lock().expect("shape queue lock") = sender;
    }

    fn shape_sender(&self) -> Option<tokio::sync::mpsc::Sender<crate::shape::GlossJob>> {
        self.shape_queue.lock().expect("shape queue lock").clone()
    }

    /// Every watched file, read once, deduplicated by path (watched
    /// dirs may nest, e.g. `index_dirs: ["."]` plus `knowledge/`).
    fn watched_files(&self) -> Vec<(String, String, String)> {
        let mut out: Vec<(String, String, String)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = Default::default();
        for dir in &self.watched {
            let mut files: Vec<(String, String, String)> = Vec::new();
            let _ = collect_files(dir, &mut files);
            for file in files {
                if seen.insert(file.0.clone()) {
                    out.push(file);
                }
            }
        }
        out
    }

    /// Every indexed file as a graph node, parsed live.
    fn notes(&self) -> Vec<NoteInfo> {
        self.watched_files()
            .into_iter()
            .map(|(path, _, text)| parse_note(Path::new(&path), &text))
            .collect()
    }

    fn graph_snapshot(&self) -> anyhow::Result<Arc<GraphSnapshot>> {
        loop {
            let generation = self.graph_generation.load(Ordering::Acquire);
            if let Some(snapshot) = self
                .graph_cache
                .lock()
                .expect("graph cache lock")
                .as_ref()
                .filter(|cached| cached.generation == generation)
                .map(|cached| Arc::clone(&cached.snapshot))
            {
                return Ok(snapshot);
            }

            let snapshot = Arc::new(self.build_graph_snapshot()?);
            if self.graph_generation.load(Ordering::Acquire) != generation {
                continue;
            }
            let mut cache = self.graph_cache.lock().expect("graph cache lock");
            if let Some(cached) = cache
                .as_ref()
                .filter(|cached| cached.generation == generation)
            {
                return Ok(Arc::clone(&cached.snapshot));
            }
            *cache = Some(CachedGraph {
                generation,
                snapshot: Arc::clone(&snapshot),
            });
            return Ok(snapshot);
        }
    }

    /// Retire the current disposable graph/vector snapshot. Existing
    /// callers may finish against their immutable `Arc`; subsequent
    /// callers rebuild from the new workspace/database generation.
    fn invalidate_graph(&self) {
        self.graph_generation.fetch_add(1, Ordering::AcqRel);
        *self.graph_cache.lock().expect("graph cache lock") = None;
    }

    fn build_graph_snapshot(&self) -> anyhow::Result<GraphSnapshot> {
        let notes = self.notes();
        let resolver = Resolver::build(&notes);
        let note_by_id = notes
            .iter()
            .enumerate()
            .map(|(index, note)| (note.id.clone(), index))
            .collect();
        let id_by_path = notes
            .iter()
            .map(|note| (note.path.display().to_string(), note.id.clone()))
            .collect();
        let mut adjacency: std::collections::HashMap<String, Vec<String>> = Default::default();
        let mut resolved_links = Vec::new();
        for note in &notes {
            for (link_type, target) in &note.links {
                let Some(target) = resolver.resolve(target) else {
                    continue;
                };
                adjacency
                    .entry(note.id.clone())
                    .or_default()
                    .push(target.clone());
                adjacency
                    .entry(target.clone())
                    .or_default()
                    .push(note.id.clone());
                resolved_links.push(ResolvedLink {
                    source: note.id.clone(),
                    target,
                    link_type: link_type.clone(),
                });
            }
        }
        Ok(GraphSnapshot {
            notes,
            note_by_id,
            id_by_path,
            resolver,
            adjacency,
            resolved_links,
            file_vectors: self.file_vectors_uncached()?,
        })
    }

    /// One sweep: hash every watched file, (re)index changes, remove
    /// vectors for deleted files. Returns (indexed, removed).
    pub async fn sweep(&self) -> anyhow::Result<(usize, usize)> {
        let on_disk = self.watched_files(); // path, hash, text

        let known: Vec<(String, String)> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT file_path, hash FROM file_hashes")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?
        };

        let mut indexed = 0;
        let mut changed_atomics: Vec<(String, String)> = Vec::new();
        for (path, hash, text) in &on_disk {
            let unchanged = known.iter().any(|(p, h)| p == path && h == hash);
            if unchanged {
                continue;
            }
            // One bad file must not pin the sweep: warn and move on;
            // it retries next sweep.
            match self.index_file(path, hash, text).await {
                Ok(()) => {
                    indexed += 1;
                    if self.is_atomic_path(path) {
                        changed_atomics.push((path.clone(), text.clone()));
                    }
                }
                Err(e) => tracing::warn!(path, error = %e, "indexing failed; skipping"),
            }
        }

        let mut removed = 0;
        {
            let removed_paths: Vec<&str> = known
                .iter()
                .filter(|(path, _)| !on_disk.iter().any(|(p, _, _)| p == path))
                .map(|(path, _)| path.as_str())
                .collect();
            if !removed_paths.is_empty() {
                let removal_result = (|| -> anyhow::Result<()> {
                    let mut db = self.db.lock().expect("db lock");
                    let transaction = db.transaction()?;
                    for path in &removed_paths {
                        transaction.execute("DELETE FROM segments WHERE file_path = ?1", [path])?;
                        transaction
                            .execute("DELETE FROM file_hashes WHERE file_path = ?1", [path])?;
                    }
                    transaction.commit()?;
                    Ok(())
                })();
                if let Err(error) = removal_result {
                    if indexed > 0 {
                        self.invalidate_graph();
                    }
                    return Err(error);
                }
                removed = removed_paths.len();
            }
        }
        if indexed + removed > 0 {
            self.invalidate_graph();
            tracing::info!(indexed, removed, "sync sweep");
        }

        // Shape hook: agent-authored `shape:` frontmatter upserts
        // directly (Author=Agent); otherwise enqueue a Write job so
        // the shape worker glosses it on the next idle window. No
        // shape queue configured (or shape disabled) → no-op.
        if let Some(sender) = self.shape_sender() {
            for (path, text) in &changed_atomics {
                let Some(id) = read_frontmatter_field(text, "id") else {
                    continue;
                };
                let relative = self.workspace_relative(path);
                if let Some(shape) = read_frontmatter_field(text, "shape") {
                    if let Err(e) = self
                        .upsert_shape(
                            &id,
                            &relative,
                            &shape,
                            ShapeAuthor::Agent,
                            "agent",
                            "",
                        )
                        .await
                    {
                        tracing::warn!(path, error = %e, "agent shape upsert failed");
                    }
                    continue;
                }
                if self.read_shape(&id)?.is_some() {
                    continue; // already glossed; no re-enqueue on unchanged shape
                }
                let job = crate::shape::GlossJob {
                    note_id: id,
                    note_path: relative,
                    reason: crate::shape::JobReason::Write,
                };
                let _ = sender.try_send(job);
            }
        }
        // On atomic deletion, drop the shape row alongside segments.
        let atomic_removed: Vec<String> = known
            .iter()
            .filter(|(path, _)| !on_disk.iter().any(|(p, _, _)| p == path))
            .filter(|(path, _)| self.is_atomic_path(path))
            .map(|(path, _)| self.workspace_relative(path))
            .collect();
        for path in atomic_removed {
            if let Err(e) = self.delete_shape_by_path(&path) {
                tracing::warn!(path, error = %e, "shape row cleanup failed");
            }
        }

        Ok((indexed, removed))
    }

    fn is_atomic_path(&self, path: &str) -> bool {
        let knowledge = self.workspace.join("knowledge");
        Path::new(path).starts_with(&knowledge)
    }

    fn workspace_relative(&self, path: &str) -> String {
        Path::new(path)
            .strip_prefix(&self.workspace)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.to_string())
    }

    async fn index_file(&self, path: &str, hash: &str, text: &str) -> anyhow::Result<()> {
        // Byte caps are guesses about tokenization; the embedder is
        // the oracle. Token-dense content (file paths, CJK, base64)
        // can overflow the model's context inside any fixed cap, so
        // on failure re-segment at half the cap and retry, down to a
        // floor.
        let mut cap = SEGMENT_HARD_CAP;
        let (segments, vectors) = loop {
            let segments = segment_with_cap(text, cap);
            if segments.is_empty() {
                break (segments, Vec::new()); // empty file: record the hash only
            }
            match self.embedder.embed(&segments).await {
                Ok(vectors) => break (segments, vectors),
                Err(e) if cap / 2 >= SEGMENT_MIN_CAP => {
                    cap /= 2;
                    tracing::debug!(path, error = %e, cap, "embed failed; re-segmenting smaller");
                }
                Err(e) => return Err(e),
            }
        };
        let mut db = self.db.lock().expect("db lock");
        let transaction = db.transaction()?;
        transaction.execute("DELETE FROM segments WHERE file_path = ?1", [path])?;
        for (seq, (seg_text, vector)) in segments.iter().zip(vectors.iter()).enumerate() {
            let blob: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
            transaction.execute(
                "INSERT INTO segments (id, file_path, seq, text, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    ulid::Ulid::new().to_string(),
                    path,
                    seq as i64,
                    seg_text,
                    blob
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO file_hashes (file_path, hash, indexed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_path) DO UPDATE SET hash = ?2, indexed_at = ?3",
            rusqlite::params![path, hash, now()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Embed a single query text with the same shrinking-cap retry
    /// discipline `index_file` uses for segments: start at
    /// `SEGMENT_HARD_CAP`, halve on failure down to `SEGMENT_MIN_CAP`.
    /// Truncation is by character count so multibyte glyphs stay
    /// intact. Used by witness-side retrieval (connect duty and σ)
    /// where the query is the current window's transcript and can
    /// legitimately exceed the embedding model's context.
    pub async fn embed_query(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut cap = SEGMENT_HARD_CAP;
        loop {
            let capped = cap_chars(text, cap);
            match self.embedder.embed(&[capped]).await {
                Ok(mut vs) => {
                    return vs
                        .drain(..)
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"));
                }
                Err(e) if cap / 2 >= SEGMENT_MIN_CAP => {
                    cap /= 2;
                    tracing::debug!(cap, error = %e, "query embed failed; halving");
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Same top-K cosine scan as [`Memory::search`] but without the
    /// ambient bumps. The witness's connect duty uses this at every
    /// settled turn; the agent's `search` tool uses `search` proper.
    /// Firing an ambient bump per settled turn would pump warmth into
    /// notes the agent never saw.
    /// Compat wrapper kept for tests and external consumers that
    /// don't already hold a query embedding. The flash pass calls
    /// `search_no_bump_with_vec` directly to avoid re-embedding.
    #[allow(dead_code)]
    pub async fn search_no_bump(
        &self,
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        let query_vec = self.embed_query(query).await?;
        self.search_no_bump_with_vec(&query_vec, top_k)
    }

    /// Cosine scan against `segments`, reusing an already-embedded
    /// query vector. The flash pass embeds the transcript once and
    /// threads the vec through this variant plus Bridge's per-
    /// candidate text-sim check.
    pub fn search_no_bump_with_vec(
        &self,
        query_vec: &[f32],
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, String, Vec<u8>)> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT file_path, text, embedding FROM segments")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?
        };
        let expected_bytes = query_vec.len() * 4;
        let mut hits: Vec<SearchHit> = rows
            .into_iter()
            .filter_map(|(file_path, text, blob)| {
                if blob.len() != expected_bytes {
                    return None;
                }
                let vector: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                Some(SearchHit {
                    file_path,
                    text,
                    score: cosine(query_vec, &vector),
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(top_k);
        Ok(hits)
    }

    /// Max cosine between `query_vec` and any segment of the file at
    /// `candidate_path`. Bridge uses this to check `text_sim ≤
    /// text_sim_max` for its shape-retrieved candidates without
    /// re-embedding the transcript. A path with no `segments` rows
    /// (unindexed, or recently added and not yet swept) returns
    /// 0.0.
    pub fn text_sim(&self, candidate_path: &str, query_vec: &[f32]) -> anyhow::Result<f32> {
        let rows: Vec<Vec<u8>> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt =
                db.prepare("SELECT embedding FROM segments WHERE file_path = ?1")?;
            stmt.query_map([candidate_path], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        if rows.is_empty() {
            return Ok(0.0);
        }
        let expected_bytes = query_vec.len() * 4;
        let mut best: f32 = 0.0;
        for blob in rows {
            if blob.len() != expected_bytes {
                continue;
            }
            let vector: Vec<f32> = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let sim = cosine(query_vec, &vector);
            if sim > best {
                best = sim;
            }
        }
        Ok(best)
    }

    /// Top-k cosine over the stored vectors. Every hit is an ambient
    /// access for the file it touches. Delegates to
    /// [`Memory::search_with_prefixes`] with an empty filter.
    pub async fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>> {
        self.search_with_prefixes(query, &[]).await
    }

    /// Same as [`Memory::search`] but with an optional allowed-prefix
    /// list. When the list is non-empty, only hits whose file path is
    /// under one of the workspace-relative prefixes survive the top-K
    /// truncation, and ambient bumps fire only on surviving hits.
    /// An empty list is equivalent to no filter (all indexed files
    /// eligible).
    pub async fn search_with_prefixes(
        &self,
        query: &str,
        allowed_prefixes: &[String],
    ) -> anyhow::Result<Vec<SearchHit>> {
        let query_vec = self
            .embedder
            .embed(&[query.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"))?;

        let rows: Vec<(String, String, Vec<u8>)> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT file_path, text, embedding FROM segments")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?
        };

        let mut hits: Vec<SearchHit> = rows
            .into_iter()
            .filter(|(file_path, _, _)| self.path_matches_prefixes(file_path, allowed_prefixes))
            .map(|(file_path, text, blob)| {
                let vector: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                SearchHit {
                    file_path,
                    text,
                    score: cosine(&query_vec, &vector),
                }
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(self.knobs.search_top_k);

        for hit in &hits {
            self.bump_path(&hit.file_path, self.knobs.ambient_bump, Carrier::Ambient)?;
        }
        Ok(hits)
    }

    /// True when `file_path` (as stored in `segments`, an absolute
    /// path from `path.display().to_string()`) starts with any of the
    /// workspace-relative `allowed_prefixes`. An empty list is
    /// unconditionally true. The prefix comparison strips the
    /// workspace root so callers write natural values like
    /// `"knowledge/"`, `"loom/"`, `"knowledge/philosophy/"`.
    fn path_matches_prefixes(&self, file_path: &str, allowed_prefixes: &[String]) -> bool {
        if allowed_prefixes.is_empty() {
            return true;
        }
        let path = Path::new(file_path);
        let relative = path.strip_prefix(&self.workspace).unwrap_or(path);
        let relative_str = relative.display().to_string();
        allowed_prefixes
            .iter()
            .any(|prefix| relative_str.starts_with(prefix.as_str()))
    }

    /// Is this path under a watched directory (and so indexed)?
    /// Hidden components and the engine-managed record/ and channels/
    /// are never indexed, judged relative to the workspace.
    pub fn is_watched(&self, path: &Path) -> bool {
        if !self.watched.iter().any(|dir| path.starts_with(dir)) {
            return false;
        }
        let Ok(relative) = path.strip_prefix(&self.workspace) else {
            return true; // watched dir outside the workspace: its call
        };
        !relative.components().any(|c| {
            let name = c.as_os_str().to_string_lossy();
            (name.starts_with('.') && name.len() > 1) || name == "record" || name == "channels"
        })
    }

    /// A cognitive access through the file-tool seam (wall ch. 07).
    pub fn on_read(&self, path: &Path) -> anyhow::Result<()> {
        if self.is_watched(path) && indexable(path) {
            let bump = self.knobs.cognitive_bump;
            self.bump_path(&path.display().to_string(), bump, Carrier::Cognitive)?;
        }
        Ok(())
    }

    /// A watched write: bump now; the next sweep re-indexes.
    pub fn on_write(&self, path: &Path) -> anyhow::Result<bool> {
        if self.is_watched(path) && indexable(path) {
            self.invalidate_graph();
            let bump = self.knobs.cognitive_bump;
            self.bump_path(&path.display().to_string(), bump, Carrier::Cognitive)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn bump_path(&self, path: &str, amount: f64, carrier: Carrier) -> anyhow::Result<()> {
        let graph = self.graph_snapshot()?;
        let note_id = graph
            .id_by_path
            .get(path)
            .cloned()
            .or_else(|| frontmatter_id(Path::new(path)))
            .unwrap_or_else(|| path.to_string());
        self.bump_with_graph(&note_id, amount, carrier, &graph)
    }

    /// Apply a bump and its single-pass wave (wall ch. 02): ×0.5 per
    /// hop, 3 hops deep, one wave outward — propagated bumps trigger
    /// no further waves. Energy ignores link direction and type.
    #[cfg(test)]
    pub fn bump(&self, origin: &str, amount: f64, carrier: Carrier) -> anyhow::Result<()> {
        let graph = self.graph_snapshot()?;
        self.bump_with_graph(origin, amount, carrier, &graph)
    }

    fn bump_with_graph(
        &self,
        origin: &str,
        amount: f64,
        carrier: Carrier,
        graph: &GraphSnapshot,
    ) -> anyhow::Result<()> {
        let mut operations = Vec::new();
        let mut visited: std::collections::HashSet<String> = Default::default();
        let mut frontier = vec![origin.to_string()];
        visited.insert(origin.to_string());
        let mut wave_amount = amount;

        for hop in 0..=self.knobs.propagation_hops {
            let mut next: Vec<String> = Vec::new();
            for id in &frontier {
                let hop_carrier = if hop == 0 {
                    carrier
                } else {
                    Carrier::Propagated
                };
                operations.push(BumpOp {
                    note_id: id.clone(),
                    amount: wave_amount,
                    carrier: hop_carrier,
                });
                if let Some(neighbors) = graph.adjacency.get(id.as_str()) {
                    for n in neighbors {
                        if visited.insert(n.to_string()) {
                            next.push(n.to_string());
                        }
                    }
                }
            }
            frontier = next;
            wave_amount *= self.knobs.propagation_factor;
            if frontier.is_empty() {
                break;
            }
        }

        // Implicit warmth: semantic neighbors of the origin, one hop,
        // skipping anything the typed-link wave already reached.
        let origin_path = graph
            .note_by_id
            .get(origin)
            .map(|&index| &graph.notes[index])
            .map(|n| n.path.display().to_string())
            .unwrap_or_else(|| origin.to_string());
        self.plan_semantic_spread(&origin_path, amount, graph, &visited, &mut operations);
        self.apply_wave(&operations, graph)
    }

    /// Mean stored vector per indexed file.
    fn file_vectors_uncached(&self) -> anyhow::Result<Vec<(String, Vec<f32>)>> {
        let rows: Vec<(String, Vec<u8>)> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT file_path, embedding FROM segments")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let mut sums: std::collections::HashMap<String, (Vec<f32>, usize)> = Default::default();
        for (path, blob) in rows {
            let vector: Vec<f32> = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let entry = sums
                .entry(path)
                .or_insert_with(|| (vec![0.0; vector.len()], 0));
            for (s, v) in entry.0.iter_mut().zip(&vector) {
                *s += v;
            }
            entry.1 += 1;
        }
        Ok(sums
            .into_iter()
            .map(|(path, (sum, n))| (path, sum.into_iter().map(|x| x / n as f32).collect()))
            .collect())
    }

    /// Semantic propagation (wall ch. 02, implicit warmth): the bump
    /// origin's embedding neighbors warm at ×0.25, one hop, no chain.
    fn plan_semantic_spread(
        &self,
        origin_path: &str,
        amount: f64,
        graph: &GraphSnapshot,
        already: &std::collections::HashSet<String>,
        operations: &mut Vec<BumpOp>,
    ) {
        let Some((_, origin_vec)) = graph.file_vectors.iter().find(|(p, _)| p == origin_path)
        else {
            return;
        };
        let mut scored: Vec<(&String, f32)> = graph
            .file_vectors
            .iter()
            .filter(|(p, _)| p != origin_path)
            .map(|(p, v)| (p, cosine(origin_vec, v)))
            .filter(|(_, s)| *s >= self.knobs.semantic_threshold)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (path, _) in scored.into_iter().take(self.knobs.semantic_top_k) {
            let id = graph
                .id_by_path
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.clone());
            if already.contains(&id) {
                continue; // the typed-link wave already reached it
            }
            operations.push(BumpOp {
                note_id: id,
                amount: amount * self.knobs.semantic_factor,
                carrier: Carrier::Propagated,
            });
        }
    }

    /// Conversation resonance (wall ch. 02, implicit warmth): the
    /// turn's own text warms the nearest notes ambiently, no waves.
    pub async fn resonate(&self, turn_text: &str) -> anyhow::Result<()> {
        self.resonate_with(turn_text, self.knobs.resonance_factor)
            .await
    }

    /// Tool resonance (wall ch. 02, implicit warmth): each tool
    /// result's text warms the nearest notes at 0.8 × similarity —
    /// what passes through the agent's hands warms what it resembles.
    pub async fn resonate_tool(&self, result_text: &str) -> anyhow::Result<()> {
        self.resonate_with(result_text, self.knobs.tool_resonance_factor)
            .await
    }

    async fn resonate_with(&self, text: &str, factor: f64) -> anyhow::Result<()> {
        if text.trim().is_empty() {
            return Ok(());
        }
        // Cap well under the embedder's context; tool results are
        // often token-dense (path listings), so shrink and retry on
        // failure rather than trusting the byte cap.
        let mut cut = text.len().min(4000);
        let query = loop {
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            match self.embedder.embed(&[text[..cut].to_string()]).await {
                Ok(vectors) => {
                    break vectors
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"))?;
                }
                Err(e) if cut / 2 >= SEGMENT_MIN_CAP => {
                    cut /= 2;
                    tracing::debug!(error = %e, cut, "resonance embed failed; shrinking");
                }
                Err(e) => return Err(e),
            }
        };
        let graph = self.graph_snapshot()?;
        let mut scored: Vec<(&String, f32)> = graph
            .file_vectors
            .iter()
            .map(|(p, v)| (p, cosine(&query, v)))
            .filter(|(_, s)| *s >= self.knobs.resonance_threshold)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let operations: Vec<BumpOp> = scored
            .into_iter()
            .take(self.knobs.resonance_top_k)
            .map(|(path, similarity)| {
                let id = graph
                    .id_by_path
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| path.clone());
                BumpOp {
                    note_id: id,
                    amount: factor * similarity as f64,
                    carrier: Carrier::Ambient,
                }
            })
            .collect();
        self.apply_wave(&operations, &graph)
    }

    /// Commit one complete logical activation wave atomically. Flash
    /// publication follows the commit so a rolled-back wave cannot
    /// become visible in the next context slot.
    fn apply_wave(&self, operations: &[BumpOp], graph: &GraphSnapshot) -> anyhow::Result<()> {
        if operations.is_empty() {
            return Ok(());
        }
        let mut db = self.db.lock().expect("db lock");
        let transaction = db.transaction()?;
        let mut flashes = Vec::new();
        let bumped_at = now();
        for operation in operations {
            if let Some(flash) = self.apply_bump_tx(&transaction, operation, graph, bumped_at)? {
                flashes.push(flash);
            }
        }
        transaction.commit()?;
        if !flashes.is_empty() {
            let mut pending = self.pending_flashes.lock().expect("flash lock");
            for flash in flashes {
                tracing::info!(note = %flash.note_id, "flash: crossed the threshold");
                pending.push(flash);
            }
        }
        Ok(())
    }

    /// Apply one operation inside its wave's transaction. Operations
    /// remain sequential: a later operation observes any score and
    /// flash halving produced by an earlier operation in the wave.
    fn apply_bump_tx(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        operation: &BumpOp,
        graph: &GraphSnapshot,
        bumped_at: i64,
    ) -> anyhow::Result<Option<Flash>> {
        let old: f64 = match transaction.query_row(
            "SELECT score FROM activation WHERE note_id = ?1",
            [&operation.note_id],
            |row| row.get(0),
        ) {
            Ok(score) => score,
            Err(rusqlite::Error::QueryReturnedNoRows) => 0.0,
            Err(error) => return Err(error.into()),
        };
        let mut new = old + operation.amount;
        let threshold = self.knobs.flash_threshold;
        let crossed = old < threshold && new >= threshold;

        let flash = if crossed
            && operation.carrier != Carrier::Cognitive
            && self.may_flash(&operation.note_id, &graph.notes)
        {
            new /= 2.0;
            let flash = match graph
                .note_by_id
                .get(&operation.note_id)
                .map(|&index| &graph.notes[index])
            {
                Some(note) => Flash {
                    note_id: operation.note_id.clone(),
                    text: cap_chars(&note.body, FLASH_TEXT_CAP),
                    neighbors: note
                        .links
                        .iter()
                        .filter_map(|(link_type, target)| {
                            let target = graph.resolver.resolve(target)?;
                            graph
                                .note_by_id
                                .get(&target)
                                .map(|&index| &graph.notes[index])
                                .map(|n| (link_type.clone(), cap_chars(&n.body, FLASH_TEXT_CAP)))
                        })
                        .take(FLASH_NEIGHBOR_CAP)
                        .collect(),
                },
                None => Flash {
                    note_id: operation.note_id.clone(),
                    text: std::fs::read_to_string(&operation.note_id)
                        .map(|t| cap_chars(&t, FLASH_TEXT_CAP))
                        .unwrap_or_default(),
                    neighbors: Vec::new(),
                },
            };
            Some(flash)
        } else {
            None
        };

        transaction.execute(
            "INSERT INTO activation (note_id, score, bumped_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(note_id) DO UPDATE SET score = ?2, bumped_at = ?3",
            rusqlite::params![operation.note_id, new, bumped_at],
        )?;
        Ok(flash)
    }

    /// The whole graph, made visible (board card: GET /graph): every
    /// indexed note (cold included, score 0), typed + wiki edges with
    /// dangling targets dropped, semantic edges above the configured
    /// threshold. Read-only — a window, never a hand.
    pub fn graph(&self) -> anyhow::Result<GraphPayload> {
        let graph = self.graph_snapshot()?;
        let scores: std::collections::HashMap<String, f64> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT note_id, score FROM activation")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?
        };

        let relative = |path: &Path| {
            path.strip_prefix(&self.workspace)
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let nodes: Vec<GraphNode> = graph
            .notes
            .iter()
            .map(|n| GraphNode {
                id: n.id.clone(),
                path: relative(&n.path),
                score: scores.get(&n.id).copied().unwrap_or(0.0),
            })
            .collect();

        let mut links: Vec<GraphLink> = Vec::new();
        for resolved in &graph.resolved_links {
            if resolved.target == resolved.source {
                continue;
            }
            let link = GraphLink {
                source: resolved.source.clone(),
                target: resolved.target.clone(),
                link_type: resolved.link_type.clone(),
                similarity: None,
            };
            if !links.iter().any(|l| {
                l.source == link.source && l.target == link.target && l.link_type == link.link_type
            }) {
                links.push(link);
            }
        }

        // Semantic edges: per node, its top-k embedding neighbors
        // above the threshold, deduped by unordered pair.
        let vectors = &graph.file_vectors;
        let id_of_path: std::collections::HashMap<String, &str> = graph
            .notes
            .iter()
            .map(|n| (n.path.display().to_string(), n.id.as_str()))
            .collect();
        for (path, vec) in vectors {
            let Some(&source) = id_of_path.get(path) else {
                continue;
            };
            let mut scored: Vec<(&str, f32)> = vectors
                .iter()
                .filter(|(p, _)| p != path)
                .filter_map(|(p, v)| id_of_path.get(p).map(|id| (*id, cosine(vec, v))))
                .filter(|(_, s)| *s >= self.knobs.semantic_threshold)
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (target, similarity) in scored.into_iter().take(self.knobs.semantic_top_k) {
                let (a, b) = if source <= target {
                    (source, target)
                } else {
                    (target, source)
                };
                if !links
                    .iter()
                    .any(|l| l.link_type == "semantic" && l.source == a && l.target == b)
                {
                    links.push(GraphLink {
                        source: a.to_string(),
                        target: b.to_string(),
                        link_type: "semantic".to_string(),
                        similarity: Some(similarity),
                    });
                }
            }
        }

        Ok(GraphPayload {
            flash_threshold: self.knobs.flash_threshold,
            flash_dirs: self.knobs.flash_dirs.clone(),
            nodes,
            links,
        })
    }

    /// The flash directory filter (board card): when `flash_dirs` is
    /// configured, only notes under those workspace-relative prefixes
    /// may surface. Everything else still warms, conducts, and
    /// propagates — a filtered crossing stands silently, exactly like
    /// a cognitive one.
    fn may_flash(&self, note_id: &str, notes: &[NoteInfo]) -> bool {
        if self.knobs.flash_dirs.is_empty() {
            return true;
        }
        let path = notes
            .iter()
            .find(|n| n.id == note_id)
            .map(|n| n.path.clone())
            .unwrap_or_else(|| PathBuf::from(note_id));
        self.knobs.flash_dirs.iter().any(|dir| {
            let prefix: PathBuf = self.workspace.join(dir).components().collect();
            path.starts_with(&prefix)
        })
    }

    /// Drain pending flashes for the memory slot.
    pub fn take_flashes(&self) -> Vec<Flash> {
        std::mem::take(&mut *self.pending_flashes.lock().expect("flash lock"))
    }

    /// The hourly tick: S(t) = S₀ · decay^t, stable between ticks.
    pub fn decay_tick(&self) -> anyhow::Result<()> {
        let db = self.db.lock().expect("db lock");
        db.execute(
            "UPDATE activation SET score = score * ?1",
            [self.knobs.decay_factor],
        )?;
        db.execute("DELETE FROM activation WHERE score < 0.01", [])?;
        Ok(())
    }

    /// Witness-side: append an extraction candidate (wall ch. 02).
    pub fn enqueue_candidate(&self, candidate: &str) -> anyhow::Result<String> {
        let id = ulid::Ulid::new().to_string();
        let db = self.db.lock().expect("db lock");
        db.execute(
            "INSERT INTO extraction_queue (id, candidate, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, candidate, now()],
        )?;
        drop(db);
        // Wake anyone waiting on the quiet trigger's re-evaluation.
        self.queue_notify.notify_one();
        Ok(id)
    }

    /// Resolves when a candidate is enqueued (level-triggered enough:
    /// callers re-check the depth after waking).
    pub async fn queue_wait(&self) {
        self.queue_notify.notified().await;
    }

    /// Agent-side: take the front of the FIFO queue. Returns the
    /// row's id alongside the candidate text so the digestion turn
    /// can record an attributable rejection (wall ch. 04).
    pub fn pop_candidate(&self) -> anyhow::Result<Option<(String, String)>> {
        let db = self.db.lock().expect("db lock");
        let front: Option<(String, String)> = db
            .query_row(
                "SELECT id, candidate FROM extraction_queue
                 ORDER BY enqueue_seq, id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((id, candidate)) = front {
            db.execute("DELETE FROM extraction_queue WHERE id = ?1", [&id])?;
            return Ok(Some((id, candidate)));
        }
        Ok(None)
    }

    pub fn queue_depth(&self) -> anyhow::Result<u64> {
        let db = self.db.lock().expect("db lock");
        Ok(
            db.query_row("SELECT COUNT(*) FROM extraction_queue", [], |row| {
                row.get(0)
            })?,
        )
    }

    /// Embed a rejection's candidate text and store its vector.
    /// Best-effort: called by the reject_candidate tool after the jsonl
    /// append succeeds, so failure here never blocks the rejection
    /// itself. Duplicate candidate_ids are ignored (the startup rebuild
    /// races with a live write in exactly this way).
    /// Upsert a `shape_vectors` row. Embeds `gloss` internally via
    /// the memory embedder. Author is `Witness` for gloss-worker
    /// writes, `Agent` for values pulled from a note's `shape:`
    /// frontmatter (in which case `model_id="agent"` and
    /// `prompt_hash=""` by convention).
    ///
    /// Returns `true` when a row was inserted or updated, `false`
    /// when the authorship guard vetoed the write (an agent-authored
    /// row must never be overwritten by a witness gloss — the guard
    /// is enforced in SQL so it holds even if callers forget to
    /// pre-check). All other combinations proceed as before.
    pub async fn upsert_shape(
        &self,
        note_id: &str,
        file_path: &str,
        gloss: &str,
        author: ShapeAuthor,
        model_id: &str,
        prompt_hash: &str,
    ) -> anyhow::Result<bool> {
        let vector = self
            .embedder
            .embed(&[gloss.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"))?;
        let blob: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        let at = jiff::Timestamp::now().to_string();
        let db = self.db.lock().expect("db lock");
        let changed = db.execute(
            "INSERT INTO shape_vectors
               (note_id, file_path, gloss, author, model_id, prompt_hash, embedding, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(note_id) DO UPDATE SET
               file_path = excluded.file_path,
               gloss = excluded.gloss,
               author = excluded.author,
               model_id = excluded.model_id,
               prompt_hash = excluded.prompt_hash,
               embedding = excluded.embedding,
               at = excluded.at
             WHERE NOT (shape_vectors.author = 'agent'
                        AND excluded.author = 'witness')",
            rusqlite::params![
                note_id,
                file_path,
                gloss,
                author.as_str(),
                model_id,
                prompt_hash,
                blob,
                at
            ],
        )?;
        Ok(changed > 0)
    }

    /// Read one `shape_vectors` row by `note_id`; `None` if absent.
    pub fn read_shape(&self, note_id: &str) -> anyhow::Result<Option<ShapeRow>> {
        let db = self.db.lock().expect("db lock");
        let row: Option<(String, String, String, String, String, String, String)> = db
            .query_row(
                "SELECT note_id, file_path, gloss, author, model_id, prompt_hash, at
                 FROM shape_vectors WHERE note_id = ?1",
                [note_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((note_id, file_path, gloss, author, model_id, prompt_hash, at)) = row else {
            return Ok(None);
        };
        let author = ShapeAuthor::parse(&author)
            .ok_or_else(|| anyhow::anyhow!("unknown shape author: {author:?}"))?;
        Ok(Some(ShapeRow {
            note_id,
            file_path,
            gloss,
            author,
            model_id,
            prompt_hash,
            at,
        }))
    }

    /// All `shape_vectors` rows — used by the drift-repair startup
    /// scan and by tests. Order unspecified.
    pub fn list_shape_rows(&self) -> anyhow::Result<Vec<ShapeRow>> {
        let db = self.db.lock().expect("db lock");
        let mut stmt = db.prepare(
            "SELECT note_id, file_path, gloss, author, model_id, prompt_hash, at
             FROM shape_vectors",
        )?;
        let rows: Vec<(String, String, String, String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        rows.into_iter()
            .map(
                |(note_id, file_path, gloss, author, model_id, prompt_hash, at)| {
                    let author = ShapeAuthor::parse(&author).ok_or_else(|| {
                        anyhow::anyhow!("unknown shape author: {author:?}")
                    })?;
                    Ok(ShapeRow {
                        note_id,
                        file_path,
                        gloss,
                        author,
                        model_id,
                        prompt_hash,
                        at,
                    })
                },
            )
            .collect()
    }

    /// Delete every `shape_vectors` row whose `file_path` matches.
    /// Called by the sync service when an atomic is removed.
    pub fn delete_shape_by_path(&self, file_path: &str) -> anyhow::Result<()> {
        let db = self.db.lock().expect("db lock");
        db.execute(
            "DELETE FROM shape_vectors WHERE file_path = ?1",
            [file_path],
        )?;
        Ok(())
    }

    /// Top-K shape neighbors by cosine over an already-embedded query
    /// vector. Bridge (`flashes.rs::types::bridge`) embeds the turn's
    /// shape gloss once and reuses the vector across candidates.
    /// Rows whose stored vector length disagrees with the query
    /// (embedding-model dim change) are silently skipped.
    #[allow(dead_code)] // Bridge landing pad; used only in tests until flash subsystem ships.
    pub fn search_shapes(
        &self,
        query_vec: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<(String, f32)>> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, Vec<u8>)> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT note_id, embedding FROM shape_vectors")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let expected_bytes = query_vec.len() * 4;
        let mut hits: Vec<(String, f32)> = rows
            .into_iter()
            .filter_map(|(note_id, blob)| {
                if blob.len() != expected_bytes {
                    return None;
                }
                let vector: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                Some((note_id, cosine(query_vec, &vector)))
            })
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits.truncate(k);
        Ok(hits)
    }

    pub async fn insert_rejection_vector(
        &self,
        candidate_id: &str,
        candidate: &str,
        reason: Option<&str>,
        turn: u64,
        at: &str,
    ) -> anyhow::Result<()> {
        let vector = self
            .embedder
            .embed(&[candidate.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"))?;
        let blob: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        let db = self.db.lock().expect("db lock");
        db.execute(
            "INSERT OR IGNORE INTO rejection_vectors
               (candidate_id, turn, candidate, reason, at, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![candidate_id, turn as i64, candidate, reason, at, blob],
        )?;
        Ok(())
    }

    /// Rows returned by [`Memory::top_similar_rejections`].
    /// `score` is cosine similarity in [-1, 1] but in practice ≥ 0 for
    /// the OpenAI-family embeddings we ship against.
    #[cfg(test)]
    pub fn rejection_vector_count(&self) -> anyhow::Result<u64> {
        let db = self.db.lock().expect("db lock");
        Ok(
            db.query_row("SELECT COUNT(*) FROM rejection_vectors", [], |row| {
                row.get(0)
            })?,
        )
    }

    /// Semantic retrieval over past rejections. Full-table cosine scan
    /// (rejection volume is bounded; revisit if evidence forces it).
    /// Rows whose stored vector length does not match the query vector
    /// (embedding-model dim change) are silently skipped — the startup
    /// rebuild pass rewrites them from the jsonl.
    pub async fn top_similar_rejections(
        &self,
        query: &str,
        top_k: usize,
        threshold: f32,
    ) -> anyhow::Result<Vec<SimilarRejection>> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        let query_vec = self.embed_query(query).await?;

        let rows: Vec<(String, i64, String, Option<String>, Vec<u8>)> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare(
                "SELECT candidate_id, turn, candidate, reason, embedding
                 FROM rejection_vectors",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<Result<_, _>>()?
        };

        let expected_bytes = query_vec.len() * 4;
        let mut hits: Vec<SimilarRejection> = rows
            .into_iter()
            .filter_map(|(candidate_id, turn, candidate, reason, blob)| {
                if blob.len() != expected_bytes {
                    return None;
                }
                let vector: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let score = cosine(&query_vec, &vector);
                if score < threshold {
                    return None;
                }
                Some(SimilarRejection {
                    candidate_id,
                    turn: turn as u64,
                    candidate,
                    reason,
                    score,
                })
            })
            .collect();
        // Score first, newer turn as tiebreaker.
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.turn.cmp(&a.turn)));
        hits.truncate(top_k);
        Ok(hits)
    }

    /// Startup: bring `rejection_vectors` back into sync with
    /// `witness/rejections.jsonl`. Missing entries are embedded and
    /// inserted; a stored vector whose length disagrees with a fresh
    /// probe (embedding-model dim change) wipes the table before the
    /// jsonl walk. Torn jsonl lines are skipped with a warning.
    pub async fn ensure_rejection_vectors_ready(
        &self,
        rejections_path: &Path,
    ) -> anyhow::Result<()> {
        let text = match std::fs::read_to_string(rejections_path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", rejections_path.display()));
            }
        };
        let mut entries: Vec<RejectionJsonl> = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<RejectionJsonl>(raw) {
                Ok(e) => entries.push(e),
                Err(e) => tracing::warn!(
                    path = %rejections_path.display(),
                    line = line_no + 1,
                    error = %e,
                    "skipping torn rejection line during rebuild"
                ),
            }
        }
        if entries.is_empty() {
            return Ok(());
        }

        // Check for embedding-dim drift against a fresh probe.
        let stored_len: Option<usize> = {
            let db = self.db.lock().expect("db lock");
            db.query_row(
                "SELECT LENGTH(embedding) FROM rejection_vectors LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(|n| n as usize)
        };
        if let Some(existing) = stored_len {
            let probe = self
                .embedder
                .embed(&[entries[0].candidate.clone()])
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"))?;
            if probe.len() * 4 != existing {
                tracing::warn!(
                    stored_bytes = existing,
                    probe_bytes = probe.len() * 4,
                    "rejection_vectors dim mismatch; wiping and rebuilding"
                );
                let db = self.db.lock().expect("db lock");
                db.execute("DELETE FROM rejection_vectors", [])?;
            }
        }

        let known: std::collections::HashSet<String> = {
            let db = self.db.lock().expect("db lock");
            let mut stmt = db.prepare("SELECT candidate_id FROM rejection_vectors")?;
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<_, _>>()?
        };

        for entry in entries {
            if known.contains(&entry.candidate_id) {
                continue;
            }
            if let Err(e) = self
                .insert_rejection_vector(
                    &entry.candidate_id,
                    &entry.candidate,
                    entry.reason.as_deref(),
                    entry.turn,
                    &entry.at,
                )
                .await
            {
                tracing::warn!(
                    candidate_id = %entry.candidate_id,
                    error = %e,
                    "rejection rebuild insert failed; will retry next startup"
                );
            }
        }
        Ok(())
    }

    pub fn activation(&self, note_id: &str) -> anyhow::Result<Option<f64>> {
        let db = self.db.lock().expect("db lock");
        let mut stmt = db.prepare("SELECT score FROM activation WHERE note_id = ?1")?;
        let mut rows = stmt.query([note_id])?;
        Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
    }

    /// Test-only: set an activation score for a note directly, without
    /// the bump/propagation machinery. Lets tests parameterize warmth
    /// gates precisely; production code always goes through the
    /// bump path so warmth reflects real access.
    #[cfg(test)]
    pub fn set_activation_for_test(&self, note_id: &str, score: f64) -> anyhow::Result<()> {
        let db = self.db.lock().expect("db lock");
        db.execute(
            "INSERT INTO activation (note_id, score, bumped_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(note_id) DO UPDATE SET score = ?2, bumped_at = ?3",
            rusqlite::params![note_id, score, jiff::Timestamp::now().to_string()],
        )?;
        Ok(())
    }

    /// The workspace root this Memory was opened for. Used by
    /// consumers that render paths back to their workspace-relative
    /// or absolute form.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace
    }

    /// Embed a single text via the configured embedder. Bridge uses
    /// this to embed the turn-shape gloss returned by
    /// `shape::gloss_turn` before scanning `shape_vectors`.
    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.embedder
            .embed(&[text.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedder returned nothing"))
    }


    /// Run the periodic sweep and the hourly decay tick until
    /// shutdown.
    pub async fn run_sync(
        self,
        mut reindex: tokio::sync::mpsc::Receiver<()>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut last_decay = std::time::Instant::now();
        let mut reindex_open = true;
        loop {
            if let Err(e) = self.sweep().await {
                tracing::warn!(error = %e, "sync sweep failed");
            }
            if last_decay.elapsed().as_secs() >= DECAY_INTERVAL_SECS {
                if let Err(e) = self.decay_tick() {
                    tracing::warn!(error = %e, "decay tick failed");
                }
                last_decay = std::time::Instant::now();
            }
            tokio::select! {
                biased;
                _ = shutdown.wait_for(|&s| s) => return,
                message = reindex.recv(), if reindex_open => {
                    if message.is_none() {
                        reindex_open = false;
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
            }
        }
    }
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn cap_chars(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(cap).collect();
        t.push('…');
        t
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Paragraph-accumulating segmentation. Paragraphs longer than the
/// hard cap are split at char boundaries — a single giant paragraph
/// (voice transcripts) must never exceed the embedder's context.
#[cfg(test)]
fn segment(text: &str) -> Vec<String> {
    segment_with_cap(text, SEGMENT_HARD_CAP)
}

fn segment_with_cap(text: &str, hard_cap: usize) -> Vec<String> {
    let target = SEGMENT_TARGET_BYTES.min(hard_cap);
    let mut segments = Vec::new();
    let mut current = String::new();
    for para in text.split("\n\n") {
        if !current.is_empty() && current.len() + para.len() > target {
            segments.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
        while current.len() > hard_cap {
            let mut cut = hard_cap;
            while !current.is_char_boundary(cut) {
                cut -= 1;
            }
            let rest = current.split_off(cut);
            segments.push(std::mem::take(&mut current));
            current = rest;
        }
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments.retain(|s| !s.trim().is_empty());
    segments
}

/// Indexable file type: markdown only.
fn indexable(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("md")
}

fn collect_files(dir: &Path, out: &mut Vec<(String, String, String)>) -> anyhow::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(()); // a watched dir may not exist yet
    };
    for entry in entries {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with('.') || name == "record" || name == "channels" {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if indexable(&path)
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            let hash = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
            out.push((path.display().to_string(), hash, text));
        }
    }
    Ok(())
}

/// Read a single scalar YAML frontmatter field by name, tolerating
/// bare and quoted values. Returns None when the file has no
/// frontmatter or the field is absent. Only handles simple `key:
/// value` shapes on one line — the fields we consume (`id`, `shape`)
/// fit that shape by contract.
fn read_frontmatter_field(text: &str, field: &str) -> Option<String> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let prefix = format!("{field}:");
    for line in rest[..end].lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = unquote(value.trim());
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// YAML scalars arrive bare or quoted; link targets and ids must
/// compare equal either way.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(s)
}

/// Body wikilinks: `[[target]]`, with Obsidian-style `|alias` and
/// `#heading` suffixes stripped.
fn wiki_links(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else { break };
        let target = rest[..end].split(['|', '#']).next().unwrap_or("").trim();
        if !target.is_empty() && !out.iter().any(|t| t == target) {
            out.push(target.to_string());
        }
        rest = &rest[end + 2..];
    }
    out
}

/// Parse any indexed file as a graph node (wall ch. 02 + the
/// wikilinks card): frontmatter id + typed links (`- type: target`)
/// when frontmatter exists, identity keyed by path when it does not;
/// body `[[wikilinks]]` join the link set as type "wiki" either way.
fn parse_note(path: &Path, text: &str) -> NoteInfo {
    let mut id = None;
    let mut links = Vec::new();
    let mut body = text.trim().to_string();

    if let Some(rest) = text.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        let (frontmatter, fm_body) = rest.split_at(end);
        body = fm_body.trim_start_matches("\n---").trim().to_string();
        let mut in_links = false;
        for line in frontmatter.lines() {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("id:") {
                id = Some(unquote(value).to_string());
                in_links = false;
            } else if trimmed == "links:" {
                in_links = true;
            } else if in_links && let Some(item) = trimmed.strip_prefix("- ") {
                if let Some((link_type, target)) = item.split_once(':') {
                    links.push((link_type.trim().to_string(), unquote(target).to_string()));
                }
            } else if !trimmed.starts_with('-') && trimmed.contains(':') {
                in_links = false;
            }
        }
    }

    for target in wiki_links(&body) {
        if !links.iter().any(|(_, t)| *t == target) {
            links.push(("wiki".to_string(), target));
        }
    }
    NoteInfo {
        id: id.unwrap_or_else(|| path.display().to_string()),
        path: path.to_path_buf(),
        body,
        links,
    }
}

/// The last path component with any `.md` stripped — the tolerant
/// half of link resolution: `../loom/20260612.md`, `loom/20260612.md`
/// and `20260612` all share the stem `20260612`.
fn link_stem(target: &str) -> &str {
    let last = target.rsplit('/').next().unwrap_or(target);
    last.strip_suffix(".md").unwrap_or(last)
}

/// Tolerant link resolution (board card): a target resolves by exact
/// frontmatter id first, then by filename stem — and only when the
/// stem names exactly one note. Ambiguity resolves to nothing rather
/// than to the wrong note.
struct Resolver {
    ids: std::collections::HashSet<String>,
    by_stem: std::collections::HashMap<String, Option<String>>, // None = ambiguous
}

impl Resolver {
    fn build(notes: &[NoteInfo]) -> Self {
        let mut ids = std::collections::HashSet::new();
        let mut by_stem: std::collections::HashMap<String, Option<String>> = Default::default();
        for note in notes {
            ids.insert(note.id.clone());
            let stem = note
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            by_stem
                .entry(stem)
                .and_modify(|existing| {
                    if existing.as_deref() != Some(note.id.as_str()) {
                        *existing = None;
                    }
                })
                .or_insert_with(|| Some(note.id.clone()));
        }
        Self { ids, by_stem }
    }

    fn resolve(&self, target: &str) -> Option<String> {
        if self.ids.contains(target) {
            return Some(target.to_string());
        }
        self.by_stem.get(link_stem(target)).cloned().flatten()
    }
}

/// The `id:` from a leading `---` frontmatter block, if any.
/// The wikilink target this note is addressable by: frontmatter id
/// when present, else the filename stem (`.md` stripped). Mirrors the
/// resolution the memory system does at link-graph build time.
pub fn target_ref_for_path(path: &Path) -> String {
    frontmatter_id(path).unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    })
}

fn frontmatter_id(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            return None;
        }
        if let Some(id) = line.strip_prefix("id:") {
            return Some(unquote(id).to_string());
        }
    }
    None
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Deterministic fake: embeds by letter histogram, so related
    /// texts land near each other.
    pub struct FakeEmbedder;
    impl Embed for FakeEmbedder {
        fn embed<'a>(&'a self, texts: &'a [String]) -> EmbedFuture<'a> {
            Box::pin(async move {
                Ok(texts
                    .iter()
                    .map(|t| {
                        let mut v = vec![0f32; 26];
                        for c in t.to_lowercase().chars() {
                            if c.is_ascii_lowercase() {
                                v[(c as u8 - b'a') as usize] += 1.0;
                            }
                        }
                        v
                    })
                    .collect())
            })
        }
    }

    fn memory(dir: &Path) -> Memory {
        let workspace = dir.join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        Memory::open(&dir.join("data"), &workspace, &[], Arc::new(FakeEmbedder)).unwrap()
    }

    /// Rejects any input over a byte budget — the shape of ollama's
    /// "input length exceeds the context length" 400.
    struct StrictEmbedder(usize);
    impl Embed for StrictEmbedder {
        fn embed<'a>(&'a self, texts: &'a [String]) -> EmbedFuture<'a> {
            let budget = self.0;
            Box::pin(async move {
                if texts.iter().any(|t| t.len() > budget) {
                    anyhow::bail!("400: the input length exceeds the context length");
                }
                FakeEmbedder.embed(texts).await
            })
        }
    }

    #[tokio::test]
    async fn sweep_enqueues_gloss_job_for_new_atomic_without_shape() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        mem.set_shape_queue(Some(tx));

        let k = dir.path().join("ws/knowledge");
        write_note(&k, "01ATOM.md", "01ATOM", &[("extends", "01OTHER")], "the body");

        mem.sweep().await.unwrap();
        let job = rx.try_recv().unwrap();
        assert_eq!(job.note_id, "01ATOM");
        assert_eq!(job.note_path, "knowledge/01ATOM.md");
        assert_eq!(job.reason, crate::shape::JobReason::Write);
    }

    #[tokio::test]
    async fn sweep_upserts_agent_shape_when_frontmatter_present() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        mem.set_shape_queue(Some(tx));

        let k = dir.path().join("ws/knowledge");
        std::fs::create_dir_all(&k).unwrap();
        std::fs::write(
            k.join("01ATOM.md"),
            "---\nid: 01ATOM\nlinks:\n  - extends: 01O\nshape: the agent's own skeleton\n---\n\nbody\n",
        )
        .unwrap();

        mem.sweep().await.unwrap();
        assert!(rx.try_recv().is_err(), "no queue job when shape frontmatter is present");
        let row = mem.read_shape("01ATOM").unwrap().unwrap();
        assert_eq!(row.author, ShapeAuthor::Agent);
        assert_eq!(row.gloss, "the agent's own skeleton");
        assert_eq!(row.model_id, "agent");
        assert_eq!(row.prompt_hash, "");
    }

    #[tokio::test]
    async fn sweep_skips_atomic_that_already_has_shape_row() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        mem.set_shape_queue(Some(tx));

        let k = dir.path().join("ws/knowledge");
        write_note(&k, "01ATOM.md", "01ATOM", &[("extends", "01O")], "body");
        mem.upsert_shape("01ATOM", "knowledge/01ATOM.md", "existing", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();

        mem.sweep().await.unwrap();
        assert!(rx.try_recv().is_err(), "no re-enqueue when row already present");
    }

    #[tokio::test]
    async fn sweep_deletes_shape_row_on_atomic_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        mem.set_shape_queue(Some(tx));

        let k = dir.path().join("ws/knowledge");
        write_note(&k, "01GONE.md", "01GONE", &[("extends", "01O")], "body");
        mem.sweep().await.unwrap();
        mem.upsert_shape("01GONE", "knowledge/01GONE.md", "g", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();
        assert!(mem.read_shape("01GONE").unwrap().is_some());

        std::fs::remove_file(k.join("01GONE.md")).unwrap();
        mem.sweep().await.unwrap();
        assert!(mem.read_shape("01GONE").unwrap().is_none(), "row cleaned up on removal");
    }

    #[tokio::test]
    async fn sweep_without_shape_queue_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // No shape queue configured — legacy behavior.
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "01ATOM.md", "01ATOM", &[("extends", "01O")], "body");
        let (indexed, removed) = mem.sweep().await.unwrap();
        assert_eq!((indexed, removed), (1, 0));
        assert!(mem.read_shape("01ATOM").unwrap().is_none(), "no shape row without queue");
    }

    #[tokio::test]
    async fn search_with_prefixes_filters_to_allowed_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let ws = dir.path().join("ws");
        write_note(&ws.join("knowledge"), "k.md", "K1", &[("extends", "X")], "the heron waits");
        std::fs::create_dir_all(ws.join("loom")).unwrap();
        std::fs::write(ws.join("loom/l.md"), "the heron in loom").unwrap();
        mem.sweep().await.unwrap();

        let only_knowledge = mem
            .search_with_prefixes("heron", &["knowledge/".to_string()])
            .await
            .unwrap();
        assert!(only_knowledge.iter().all(|h| h.file_path.contains("knowledge/")));
        assert!(only_knowledge.iter().any(|h| h.file_path.contains("k.md")));
        assert!(!only_knowledge.iter().any(|h| h.file_path.contains("l.md")));

        let both = mem
            .search_with_prefixes(
                "heron",
                &["knowledge/".to_string(), "loom/".to_string()],
            )
            .await
            .unwrap();
        assert!(both.iter().any(|h| h.file_path.contains("k.md")));
        assert!(both.iter().any(|h| h.file_path.contains("l.md")));

        // No match: nonexistent prefix returns empty.
        let none = mem
            .search_with_prefixes("heron", &["notreal/".to_string()])
            .await
            .unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn search_with_empty_prefixes_matches_search() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "a.md", "A", &[("extends", "X")], "the heron waits");
        mem.sweep().await.unwrap();
        let via_search = mem.search("heron").await.unwrap();
        let via_prefixes = mem.search_with_prefixes("heron", &[]).await.unwrap();
        assert_eq!(via_search.len(), via_prefixes.len());
        for (a, b) in via_search.iter().zip(via_prefixes.iter()) {
            assert_eq!(a.file_path, b.file_path);
        }
    }

    #[tokio::test]
    async fn search_with_prefixes_bumps_only_kept_hits() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let ws = dir.path().join("ws");
        write_note(&ws.join("knowledge"), "k.md", "K1", &[("extends", "X")], "the heron waits");
        std::fs::create_dir_all(ws.join("loom")).unwrap();
        std::fs::write(ws.join("loom/l.md"), "the heron in loom").unwrap();
        mem.sweep().await.unwrap();

        let _ = mem
            .search_with_prefixes("heron", &["knowledge/".to_string()])
            .await
            .unwrap();
        // K1 got its ambient bump; the loom file did not.
        assert!(mem.activation("K1").unwrap().unwrap_or(0.0) > 0.0);
        // Loom file is keyed by path (no frontmatter id). Fetch by path.
        let loom_ref = crate::memory::target_ref_for_path(&ws.join("loom/l.md"));
        assert_eq!(mem.activation(&loom_ref).unwrap().unwrap_or(0.0), 0.0);
    }

    #[tokio::test]
    async fn embed_query_is_public_and_returns_nonempty_vec() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let vec = mem.embed_query("hello world").await.unwrap();
        assert!(!vec.is_empty());
    }

    #[tokio::test]
    async fn text_sim_returns_max_cosine_over_candidate_segments() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        // Write a note whose text has known letter content.
        write_note(&k, "n.md", "01N", &[("extends", "01O")], "the heron waits in shallow water");
        mem.sweep().await.unwrap();
        let path = k.join("n.md").display().to_string();

        // Query vec close to the file's content.
        let close = mem.embed_query("heron heron water").await.unwrap();
        let close_sim = mem.text_sim(&path, &close).unwrap();
        assert!(close_sim > 0.0, "close query cosine: {close_sim}");

        // Query vec far from it.
        let far = mem.embed_query("xxxxxxxx yyyyyyyyy zzzzzzz").await.unwrap();
        let far_sim = mem.text_sim(&path, &far).unwrap();
        assert!(close_sim > far_sim, "close ({close_sim}) > far ({far_sim})");
    }

    #[tokio::test]
    async fn text_sim_unknown_path_returns_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let query = mem.embed_query("anything").await.unwrap();
        let sim = mem.text_sim("no/such/file.md", &query).unwrap();
        assert_eq!(sim, 0.0);
    }

    #[tokio::test]
    async fn search_no_bump_with_vec_matches_search_no_bump() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "a.md", "01A", &[("extends", "01O")], "the heron waits");
        write_note(&k, "b.md", "01B", &[("extends", "01O")], "the owl asks");
        mem.sweep().await.unwrap();

        let via_query = mem.search_no_bump("heron", 2).await.unwrap();
        let vec = mem.embed_query("heron").await.unwrap();
        let via_vec = mem.search_no_bump_with_vec(&vec, 2).unwrap();
        assert_eq!(via_query.len(), via_vec.len());
        for (a, b) in via_query.iter().zip(via_vec.iter()) {
            assert_eq!(a.file_path, b.file_path);
            assert!((a.score - b.score).abs() < 1e-6, "scores match: {} vs {}", a.score, b.score);
        }
    }

    #[tokio::test]
    async fn upsert_shape_and_read_shape_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.upsert_shape(
            "01ATOM",
            "knowledge/01ATOM.md",
            "a proxy under optimization pressure diverges",
            ShapeAuthor::Witness,
            "haiku-4.5",
            "abc123",
        )
        .await
        .unwrap();

        let row = mem.read_shape("01ATOM").unwrap().expect("row present");
        assert_eq!(row.note_id, "01ATOM");
        assert_eq!(row.file_path, "knowledge/01ATOM.md");
        assert_eq!(row.gloss, "a proxy under optimization pressure diverges");
        assert_eq!(row.author, ShapeAuthor::Witness);
        assert_eq!(row.model_id, "haiku-4.5");
        assert_eq!(row.prompt_hash, "abc123");
        assert!(!row.at.is_empty());

        // Upsert (same note_id) replaces.
        mem.upsert_shape(
            "01ATOM",
            "knowledge/01ATOM.md",
            "a different skeleton",
            ShapeAuthor::Agent,
            "agent",
            "",
        )
        .await
        .unwrap();
        let row = mem.read_shape("01ATOM").unwrap().unwrap();
        assert_eq!(row.gloss, "a different skeleton");
        assert_eq!(row.author, ShapeAuthor::Agent);
    }

    #[tokio::test]
    async fn read_shape_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        assert!(mem.read_shape("01NOSUCH").unwrap().is_none());
    }

    #[tokio::test]
    async fn search_shapes_ranks_by_cosine() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // FakeEmbedder is letter-histogram; craft glosses whose letter
        // content ranks predictably.
        mem.upsert_shape("A", "knowledge/A.md", "aaaa bbbb", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();
        mem.upsert_shape("B", "knowledge/B.md", "aaaa cccc", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();
        mem.upsert_shape("C", "knowledge/C.md", "xxxx yyyy", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();

        let query_vec = FakeEmbedder
            .embed(&["aaaa bbbb".into()])
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let hits = mem.search_shapes(&query_vec, 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "A", "closest wins");
        assert!(hits[0].1 > hits[1].1);
        assert_ne!(hits[1].0, "C", "unrelated shape excluded from top-2");
    }

    #[tokio::test]
    async fn list_shape_rows_returns_all() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.upsert_shape("A", "knowledge/A.md", "g1", ShapeAuthor::Witness, "m1", "h1")
            .await
            .unwrap();
        mem.upsert_shape("B", "knowledge/B.md", "g2", ShapeAuthor::Agent, "agent", "")
            .await
            .unwrap();
        let rows = mem.list_shape_rows().unwrap();
        assert_eq!(rows.len(), 2);
        let by_id: std::collections::HashMap<String, ShapeRow> =
            rows.into_iter().map(|r| (r.note_id.clone(), r)).collect();
        assert_eq!(by_id["A"].author, ShapeAuthor::Witness);
        assert_eq!(by_id["B"].author, ShapeAuthor::Agent);
    }

    #[tokio::test]
    async fn delete_shape_by_path_removes_row() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.upsert_shape("A", "knowledge/A.md", "g", ShapeAuthor::Witness, "m", "h")
            .await
            .unwrap();
        assert!(mem.read_shape("A").unwrap().is_some());
        mem.delete_shape_by_path("knowledge/A.md").unwrap();
        assert!(mem.read_shape("A").unwrap().is_none());
    }

    #[tokio::test]
    async fn sweep_indexes_changes_and_removals() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let note = dir.path().join("ws/knowledge/heron.md");
        std::fs::write(&note, "the heron waits in shallow water").unwrap();

        assert_eq!(mem.sweep().await.unwrap(), (1, 0));
        let first = mem.graph_snapshot().unwrap();
        assert_eq!(mem.sweep().await.unwrap(), (0, 0), "unchanged: skipped");
        let unchanged = mem.graph_snapshot().unwrap();
        assert!(
            Arc::ptr_eq(&first, &unchanged),
            "unchanged sweep reuses cache"
        );

        std::fs::write(&note, "the heron strikes quickly").unwrap();
        assert_eq!(mem.sweep().await.unwrap(), (1, 0), "changed: re-indexed");
        let changed = mem.graph_snapshot().unwrap();
        assert!(
            !Arc::ptr_eq(&first, &changed),
            "vector change retires cache"
        );

        std::fs::remove_file(&note).unwrap();
        assert_eq!(mem.sweep().await.unwrap(), (0, 1), "deleted: removed");
        let deleted = mem.graph_snapshot().unwrap();
        assert!(deleted.notes.is_empty());
        assert!(deleted.file_vectors.is_empty());
        assert!(mem.search("heron").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn watched_write_bump_uses_fresh_link_topology() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        let a = k.join("a.md");
        write_note(&k, "a.md", "NA", &[("extends", "NB")], "claim a");
        write_note(&k, "b.md", "NB", &[], "claim b");
        write_note(&k, "c.md", "NC", &[], "claim c");

        let stale = mem.graph_snapshot().unwrap();
        assert!(
            stale
                .adjacency
                .get("NA")
                .unwrap()
                .iter()
                .any(|id| id == "NB")
        );
        write_note(&k, "a.md", "NA", &[("extends", "NC")], "claim a revised");

        assert!(mem.on_write(&a).unwrap());
        assert_eq!(mem.activation("NA").unwrap(), Some(1.0));
        assert_eq!(mem.activation("NB").unwrap(), None, "old edge retired");
        assert_eq!(
            mem.activation("NC").unwrap(),
            Some(0.5),
            "new edge conducts immediately"
        );
    }

    #[tokio::test]
    async fn search_ranks_and_bumps_ambient() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        std::fs::write(k.join("a.md"), "zzz zzz zzz").unwrap();
        std::fs::write(k.join("b.md"), "heron heron heron").unwrap();
        mem.sweep().await.unwrap();

        let hits = mem.search("heron").await.unwrap();
        assert!(hits[0].file_path.ends_with("b.md"));
        assert!(hits[0].score > hits[1].score);

        // Every result is an ambient access.
        let b_id = k.join("b.md").display().to_string();
        assert_eq!(
            mem.activation(&b_id).unwrap(),
            Some(0.5),
            "default ambient bump"
        );
    }

    #[tokio::test]
    async fn capture_seam_bumps_cognitive_for_watched_reads() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let note = dir.path().join("ws/knowledge/owl.md");
        std::fs::write(&note, "the owl asks who").unwrap();
        mem.sweep().await.unwrap();

        mem.on_read(&note).unwrap();
        let id = note.display().to_string();
        assert_eq!(
            mem.activation(&id).unwrap(),
            Some(1.0),
            "default cognitive bump"
        );

        // Unwatched reads do not bump.
        let elsewhere = dir.path().join("ws/draft.md");
        std::fs::write(&elsewhere, "x").unwrap();
        mem.on_read(&elsewhere).unwrap();
        assert_eq!(
            mem.activation(&elsewhere.display().to_string()).unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn frontmatter_id_keys_the_bump() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let note = dir.path().join("ws/knowledge/note.md");
        std::fs::write(&note, "---\nid: 01JXXTESTULID\n---\n\na claim").unwrap();
        mem.on_read(&note).unwrap();
        assert_eq!(mem.activation("01JXXTESTULID").unwrap(), Some(1.0));
    }

    #[tokio::test]
    async fn database_is_disposable() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        std::fs::write(dir.path().join("ws/knowledge/n.md"), "the river remembers").unwrap();
        mem.sweep().await.unwrap();
        assert!(!mem.search("river").await.unwrap().is_empty());
        drop(mem);

        std::fs::remove_file(dir.path().join("data/river.db")).unwrap();
        let mem = memory(dir.path());
        assert!(mem.search("river").await.unwrap().is_empty(), "fresh db");
        mem.sweep().await.unwrap();
        assert!(
            !mem.search("river").await.unwrap().is_empty(),
            "rebuilt from the workspace"
        );
    }

    fn write_note(dir: &Path, name: &str, id: &str, links: &[(&str, &str)], body: &str) {
        let mut text = format!("---\nid: {id}\n");
        if !links.is_empty() {
            text.push_str("links:\n");
            for (t, target) in links {
                text.push_str(&format!("  - {t}: {target}\n"));
            }
        }
        text.push_str(&format!("tags: [test]\n---\n\n{body}\n"));
        std::fs::write(dir.join(name), text).unwrap();
    }

    #[tokio::test]
    async fn propagation_waves_three_hops_single_pass() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        // a — b — c — d — e (chain)
        write_note(&k, "a.md", "NA", &[("extends", "NB")], "claim a");
        write_note(&k, "b.md", "NB", &[("extends", "NC")], "claim b");
        write_note(&k, "c.md", "NC", &[("extends", "ND")], "claim c");
        write_note(&k, "d.md", "ND", &[("extends", "NE")], "claim d");
        write_note(&k, "e.md", "NE", &[], "claim e");

        mem.bump("NA", 1.0, Carrier::Cognitive).unwrap();
        assert_eq!(mem.activation("NA").unwrap(), Some(1.0));
        assert_eq!(mem.activation("NB").unwrap(), Some(0.5));
        assert_eq!(mem.activation("NC").unwrap(), Some(0.25));
        assert_eq!(mem.activation("ND").unwrap(), Some(0.125));
        assert_eq!(mem.activation("NE").unwrap(), None, "3 hops only");
    }

    #[test]
    fn activation_wave_operations_observe_prior_scores_and_halving() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "a.md", "NA", &[], "claim a");
        let graph = mem.graph_snapshot().unwrap();

        mem.apply_wave(
            &[
                BumpOp {
                    note_id: "NA".to_string(),
                    amount: 0.6,
                    carrier: Carrier::Ambient,
                },
                BumpOp {
                    note_id: "NA".to_string(),
                    amount: 0.6,
                    carrier: Carrier::Ambient,
                },
            ],
            &graph,
        )
        .unwrap();

        assert_eq!(mem.activation("NA").unwrap(), Some(0.6));
        assert_eq!(mem.take_flashes().len(), 1);
    }

    #[test]
    fn failed_activation_wave_rolls_back_scores_and_publishes_no_flash() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "a.md", "NA", &[], "claim a");
        write_note(&k, "b.md", "NB", &[], "claim b");
        let graph = mem.graph_snapshot().unwrap();
        {
            let db = mem.db.lock().expect("db lock");
            db.execute_batch(
                "CREATE TRIGGER fail_second_activation
                 BEFORE INSERT ON activation
                 WHEN NEW.note_id = 'NB'
                 BEGIN
                   SELECT RAISE(ABORT, 'injected activation failure');
                 END;",
            )
            .unwrap();
        }

        let result = mem.apply_wave(
            &[
                BumpOp {
                    note_id: "NA".to_string(),
                    amount: 0.4,
                    carrier: Carrier::Ambient,
                },
                BumpOp {
                    note_id: "NB".to_string(),
                    amount: 1.2,
                    carrier: Carrier::Ambient,
                },
            ],
            &graph,
        );

        assert!(result.is_err());
        assert_eq!(
            mem.activation("NA").unwrap(),
            None,
            "first write rolled back"
        );
        assert_eq!(mem.activation("NB").unwrap(), None);
        assert!(
            mem.take_flashes().is_empty(),
            "rolled-back flash stayed private"
        );
    }

    #[tokio::test]
    async fn flash_carrier_rule_holds() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(
            &k,
            "h.md",
            "NH",
            &[("same-pattern-as", "NO")],
            "the heron waits",
        );
        write_note(&k, "o.md", "NO", &[], "the owl is silent");

        // Two direct reads: NH crosses 1.0 cognitively — never
        // flashes, never halves. But its wave warms NO by propagation
        // (0.5 + 0.5 = 1.0): NO crosses on a propagated carrier and
        // flashes — the edge of attention, not the center.
        mem.bump("NH", 1.0, Carrier::Cognitive).unwrap();
        mem.bump("NH", 1.0, Carrier::Cognitive).unwrap();
        let flashes = mem.take_flashes();
        assert!(
            flashes.iter().all(|f| f.note_id != "NH"),
            "cognitive crossings never flash the touched note"
        );
        assert_eq!(mem.activation("NH").unwrap(), Some(2.0), "no halving");
        assert_eq!(flashes.len(), 1, "the propagated neighbor flashed");
        assert_eq!(flashes[0].note_id, "NO");
        assert_eq!(mem.activation("NO").unwrap(), Some(0.5), "halved on flash");

        // Ambient crossing flashes too.
        mem.bump("NO", 0.6, Carrier::Ambient).unwrap();
        let flashes = mem.take_flashes();
        assert_eq!(flashes.len(), 1);
        assert_eq!(flashes[0].note_id, "NO");
        assert!(flashes[0].text.contains("owl"));
    }

    #[tokio::test]
    async fn flash_carries_neighbors() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(
            &k,
            "h.md",
            "NH",
            &[("same-pattern-as", "NO")],
            "the heron waits",
        );
        write_note(&k, "o.md", "NO", &[], "the owl is silent");

        mem.bump("NH", 1.2, Carrier::Ambient).unwrap();
        let flashes = mem.take_flashes();
        assert_eq!(flashes.len(), 1);
        assert_eq!(flashes[0].note_id, "NH");
        assert_eq!(flashes[0].neighbors.len(), 1);
        assert_eq!(flashes[0].neighbors[0].0, "same-pattern-as");
        assert!(flashes[0].neighbors[0].1.contains("owl"));
    }

    #[tokio::test]
    async fn semantic_propagation_warms_unlinked_neighbors() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        // Similar bodies, NO typed links between them; one outlier
        // with an orthogonal letter histogram.
        write_note(
            &k,
            "a.md",
            "NA",
            &[],
            "the heron waits in shallow water for fish",
        );
        write_note(
            &k,
            "b.md",
            "NB",
            &[],
            "the heron waited by the shallow water for a fish",
        );
        write_note(&k, "z.md", "NZ", &[], &"zzzz qqqq xxxx jjjj ".repeat(30));
        mem.sweep().await.unwrap();

        mem.bump("NA", 1.0, Carrier::Cognitive).unwrap();
        assert_eq!(mem.activation("NA").unwrap(), Some(1.0));
        assert_eq!(
            mem.activation("NB").unwrap(),
            Some(0.25),
            "unlinked but near: warmed semantically"
        );
        assert_eq!(mem.activation("NZ").unwrap(), None, "dissimilar: cold");
    }

    #[tokio::test]
    async fn resonance_warms_topical_notes_and_can_flash() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(
            &k,
            "h.md",
            "NH",
            &[],
            "the heron waits in shallow water for fish",
        );
        write_note(&k, "z.md", "NZ", &[], &"zzzz qqqq xxxx jjjj ".repeat(30));
        mem.sweep().await.unwrap();

        mem.resonate("we were talking about the heron in the water and what it waits for")
            .await
            .unwrap();
        let warmth = mem.activation("NH").unwrap().expect("resonated");
        assert!(warmth > 0.1 && warmth <= 0.2, "{warmth}");
        assert_eq!(mem.activation("NZ").unwrap(), None);

        // Sustained drift can flash: pre-warm just under threshold,
        // resonate over it — ambient carrier, so it fires.
        mem.bump("NH", 0.95 - warmth, Carrier::Ambient).unwrap();
        let _ = mem.take_flashes();
        mem.resonate("still on the subject of herons waiting in water")
            .await
            .unwrap();
        let flashes = mem.take_flashes();
        assert_eq!(flashes.len(), 1, "topical drift alone flashed it");
        assert_eq!(flashes[0].note_id, "NH");
    }

    #[tokio::test]
    async fn tool_resonance_warms_at_higher_factor() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(
            &k,
            "h.md",
            "NH",
            &[],
            "the heron waits in shallow water for fish",
        );
        write_note(&k, "z.md", "NZ", &[], &"zzzz qqqq xxxx jjjj ".repeat(30));
        mem.sweep().await.unwrap();

        let text = "a tool result mentioning the heron in the water and what it waits for";
        mem.resonate_tool(text).await.unwrap();
        let tool_warmth = mem
            .activation("NH")
            .unwrap()
            .expect("tool resonance bumped");
        assert_eq!(mem.activation("NZ").unwrap(), None, "dissimilar: cold");

        // Same text through conversation resonance lands at 1/4 the
        // warmth: the factors are 0.8 vs 0.2 on the same similarity.
        let dir2 = tempfile::tempdir().unwrap();
        let mem2 = memory(dir2.path());
        let k2 = dir2.path().join("ws/knowledge");
        write_note(
            &k2,
            "h.md",
            "NH",
            &[],
            "the heron waits in shallow water for fish",
        );
        mem2.sweep().await.unwrap();
        mem2.resonate(text).await.unwrap();
        let conv_warmth = mem2.activation("NH").unwrap().expect("resonance bumped");
        assert!(
            (tool_warmth - conv_warmth * 4.0).abs() < 1e-9,
            "{tool_warmth} vs {conv_warmth}"
        );

        // Ambient carrier: a tool-result crossing flashes.
        mem.bump("NH", 0.95 - tool_warmth, Carrier::Ambient)
            .unwrap();
        let _ = mem.take_flashes();
        mem.resonate_tool(text).await.unwrap();
        let flashes = mem.take_flashes();
        assert_eq!(flashes.len(), 1, "tool resonance alone flashed it");
        assert_eq!(flashes[0].note_id, "NH");
    }

    #[tokio::test]
    async fn decay_tick_multiplies_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "a.md", "NA", &[], "claim a");
        mem.bump("NA", 0.5, Carrier::Cognitive).unwrap();
        mem.decay_tick().unwrap();
        assert_eq!(mem.activation("NA").unwrap(), Some(0.4));
        for _ in 0..20 {
            mem.decay_tick().unwrap();
        }
        assert_eq!(mem.activation("NA").unwrap(), None, "pruned below 0.01");
    }

    #[tokio::test]
    async fn extraction_queue_is_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.enqueue_candidate("first").unwrap();
        mem.enqueue_candidate("second").unwrap();
        assert_eq!(mem.queue_depth().unwrap(), 2);
        assert_eq!(
            mem.pop_candidate()
                .unwrap()
                .map(|(_, text)| text)
                .as_deref(),
            Some("first")
        );
        assert_eq!(
            mem.pop_candidate()
                .unwrap()
                .map(|(_, text)| text)
                .as_deref(),
            Some("second")
        );
        assert!(mem.pop_candidate().unwrap().is_none());
    }

    #[test]
    fn extraction_queue_fifo_does_not_depend_on_ulid_randomness() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let first_id = "01J0000000ZZZZZZZZZZZZZZZZ";
        let second_id = "01J00000000000000000000000";
        {
            let db = mem.db.lock().unwrap();
            db.execute(
                "INSERT INTO extraction_queue (id, candidate, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![first_id, "first", 1],
            )
            .unwrap();
            db.execute(
                "INSERT INTO extraction_queue (id, candidate, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![second_id, "second", 1],
            )
            .unwrap();
        }

        assert!(second_id < first_id, "the later ULID sorts first");
        assert_eq!(mem.pop_candidate().unwrap().unwrap().0, first_id);
        assert_eq!(mem.pop_candidate().unwrap().unwrap().0, second_id);
    }

    #[test]
    fn legacy_extraction_queue_migrates_in_insertion_order() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        let first_id = "01J0000000ZZZZZZZZZZZZZZZZ";
        let second_id = "01J00000000000000000000000";
        {
            let legacy = Connection::open(data_dir.join("river.db")).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE extraction_queue (
                         id TEXT PRIMARY KEY,
                         candidate TEXT NOT NULL,
                         created_at INTEGER NOT NULL
                     );",
                )
                .unwrap();
            legacy
                .execute(
                    "INSERT INTO extraction_queue (id, candidate, created_at)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![first_id, "first", 1],
                )
                .unwrap();
            legacy
                .execute(
                    "INSERT INTO extraction_queue (id, candidate, created_at)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![second_id, "second", 1],
                )
                .unwrap();
        }

        let mem = Memory::open(&data_dir, &workspace, &[], Arc::new(FakeEmbedder)).unwrap();
        let third_id = mem.enqueue_candidate("third").unwrap();
        assert_eq!(mem.pop_candidate().unwrap().unwrap().0, first_id);
        assert_eq!(mem.pop_candidate().unwrap().unwrap().0, second_id);
        assert_eq!(mem.pop_candidate().unwrap().unwrap().0, third_id);
    }

    #[tokio::test]
    async fn whole_workspace_indexes_markdown_only_and_skips_managed_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        for sub in ["knowledge", "record", "channels", "loom", ".git"] {
            std::fs::create_dir_all(ws.join(sub)).unwrap();
        }
        std::fs::write(ws.join("loom/note.md"), "the heron waits by the water").unwrap();
        std::fs::write(ws.join("top-level.md"), "a workspace-root markdown file").unwrap();
        std::fs::write(ws.join("BSKY"), "app-password-secret").unwrap();
        std::fs::write(ws.join("record/turns.jsonl"), "{\"x\":1}").unwrap();
        std::fs::write(ws.join("channels/c.jsonl"), "{\"x\":1}").unwrap();
        std::fs::write(ws.join(".git/config.md"), "hidden markdown").unwrap();
        std::fs::write(ws.join("data.jsonl"), "not markdown").unwrap();

        let mem = Memory::open(
            &dir.path().join("data"),
            &ws,
            &[".".to_string()],
            Arc::new(FakeEmbedder),
        )
        .unwrap();
        let (indexed, _) = mem.sweep().await.unwrap();
        assert_eq!(indexed, 2, "only the two markdown files");

        let hits = mem.search("heron water").await.unwrap();
        assert!(hits.iter().all(|h| h.file_path.ends_with(".md")));
        assert!(!hits.iter().any(|h| h.file_path.contains("BSKY")));

        // Capture seam honors the same rule.
        mem.on_read(&ws.join("record/turns.jsonl")).unwrap();
        assert_eq!(
            mem.activation(&ws.join("record/turns.jsonl").display().to_string())
                .unwrap(),
            None,
            "engine-managed files never bump"
        );
        mem.on_read(&ws.join("loom/note.md")).unwrap();
        assert!(
            mem.activation(&ws.join("loom/note.md").display().to_string())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn segmentation_accumulates_paragraphs() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(800),
            "b".repeat(800),
            "c".repeat(100)
        );
        let segments = segment(&text);
        assert_eq!(segments.len(), 2);
        assert!(segments[1].contains('c'));
    }

    #[test]
    fn segmentation_splits_giant_paragraphs_and_skips_empty() {
        // One 20KB paragraph — a voice transcript's shape.
        let segments = segment(&"x".repeat(20_000));
        assert!(segments.len() >= 4, "split despite no paragraph breaks");
        assert!(segments.iter().all(|s| s.len() <= 4 * SEGMENT_TARGET_BYTES));

        assert!(segment("").is_empty());
        assert!(segment("\n\n  \n\n").is_empty());
    }

    #[tokio::test]
    async fn token_dense_files_index_by_shrinking_segments() {
        // An embedder whose real window is smaller than the byte cap
        // assumes — the dense-path-transcript failure shape.
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        let mem = Memory::open(
            &dir.path().join("data"),
            &workspace,
            &[],
            Arc::new(StrictEmbedder(2000)),
        )
        .unwrap();

        // One giant paragraph: segments at the 4800 cap all exceed
        // the embedder's 2000-byte window; shrinking to 2400 → 1200
        // gets under it.
        let dense = "/home/cassie/river/engine/core/src/main.rs ".repeat(400);
        std::fs::write(workspace.join("knowledge/paths.md"), &dense).unwrap();
        let (indexed, _) = mem.sweep().await.unwrap();
        assert_eq!(indexed, 1, "indexed despite the tight window");
        assert!(!mem.search("river engine main").await.unwrap().is_empty());

        // Resonance shrinks the same way instead of failing.
        mem.resonate_tool(&dense).await.unwrap();

        // A window below the shrink floor still fails per-file — and
        // the sweep survives it (warn + skip).
        let mem_floor = Memory::open(
            &dir.path().join("data2"),
            &workspace,
            &[],
            Arc::new(StrictEmbedder(100)),
        )
        .unwrap();
        let (indexed, _) = mem_floor.sweep().await.unwrap();
        assert_eq!(indexed, 0, "skipped, not fatal");
    }

    #[tokio::test]
    async fn link_targets_resolve_by_id_then_filename_stem() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        std::fs::create_dir_all(dir.path().join("ws/loom")).unwrap();

        // Three target shapes for the same loom note: bare stem,
        // workspace-relative path, ../-relative path.
        write_note(
            &dir.path().join("ws/loom"),
            "20260612012002756.md",
            "LOOMID",
            &[],
            "the loom note",
        );
        write_note(&k, "a.md", "NA", &[("extends", "20260612012002756")], "a");
        write_note(
            &k,
            "b.md",
            "NB",
            &[("extends", "loom/20260612012002756.md")],
            "b",
        );
        write_note(
            &k,
            "c.md",
            "NC",
            &[("extends", "../loom/20260612012002756.md")],
            "c",
        );

        // Small bumps so nothing crosses the flash threshold: each
        // origin sends 0.25 to the loom note via its resolved link.
        for origin in ["NA", "NB", "NC"] {
            mem.bump(origin, 0.5, Carrier::Cognitive).unwrap();
        }
        assert_eq!(
            mem.activation("LOOMID").unwrap(),
            Some(0.75),
            "all three target shapes propagated (3 × 0.25)"
        );
    }

    #[tokio::test]
    async fn exact_id_resolution_still_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "a.md", "NA", &[], "a");
        write_note(&k, "d.md", "ND", &[("extends", "NA")], "d");
        mem.bump("ND", 1.0, Carrier::Cognitive).unwrap();
        assert_eq!(mem.activation("NA").unwrap(), Some(0.5));
    }

    #[tokio::test]
    async fn quoted_frontmatter_ids_resolve_against_bare_targets() {
        // The shape of iris's real atomics: id: "2026..." (quoted)
        // linked as `- extends: 2026...` (bare).
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        std::fs::write(
            k.join("old.md"),
            "---\nid: \"20260612231237474\"\ntags: [t]\n---\n\nthe older claim\n",
        )
        .unwrap();
        write_note(
            &k,
            "new.md",
            "NEWID",
            &[("extends", "20260612231237474")],
            "the newer claim",
        );

        mem.bump("NEWID", 1.0, Carrier::Cognitive).unwrap();
        assert_eq!(
            mem.activation("20260612231237474").unwrap(),
            Some(0.5),
            "quotes stripped; the link conducts"
        );
        // The capture seam keys quoted ids bare, too.
        mem.on_read(&k.join("old.md")).unwrap();
        assert_eq!(mem.activation("20260612231237474").unwrap(), Some(1.5));
    }

    #[tokio::test]
    async fn ambiguous_stems_resolve_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        std::fs::create_dir_all(k.join("x")).unwrap();
        std::fs::create_dir_all(k.join("y")).unwrap();
        write_note(&k.join("x"), "dup.md", "X1", &[], "first dup");
        write_note(&k.join("y"), "dup.md", "X2", &[], "second dup");
        write_note(&k, "src.md", "SRC", &[("extends", "dup")], "the linker");

        mem.bump("SRC", 1.0, Carrier::Cognitive).unwrap();
        assert_eq!(
            mem.activation("X1").unwrap(),
            None,
            "ambiguity conducts nothing"
        );
        assert_eq!(mem.activation("X2").unwrap(), None);
    }

    #[tokio::test]
    async fn wikilinks_conduct_warmth_between_frontmatterless_files() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let loom = dir.path().join("ws/loom");
        std::fs::create_dir_all(&loom).unwrap();
        // A loom chain: b links back to a, iris-style first line.
        std::fs::write(loom.join("20260601000000000.md"), "first note, no links").unwrap();
        std::fs::write(
            loom.join("20260602000000000.md"),
            "[[20260601000000000]]\n\nsecond note in the chain",
        )
        .unwrap();

        let second = loom.join("20260602000000000.md").display().to_string();
        let first = loom.join("20260601000000000.md").display().to_string();
        mem.bump(&second, 1.0, Carrier::Cognitive).unwrap();
        assert_eq!(mem.activation(&second).unwrap(), Some(1.0));
        assert_eq!(
            mem.activation(&first).unwrap(),
            Some(0.5),
            "the loom conducts warmth"
        );
    }

    #[tokio::test]
    async fn wikilinks_in_atomic_bodies_join_the_typed_links() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(
            &k,
            "a.md",
            "NA",
            &[("extends", "NB")],
            "claim citing [[NC]] inline",
        );
        write_note(&k, "b.md", "NB", &[], "b");
        write_note(&k, "c.md", "NC", &[], "c");

        mem.bump("NA", 0.5, Carrier::Cognitive).unwrap();
        assert_eq!(mem.activation("NB").unwrap(), Some(0.25), "typed link");
        assert_eq!(mem.activation("NC").unwrap(), Some(0.25), "wiki link");
    }

    #[tokio::test]
    async fn path_keyed_flashes_carry_capped_body_and_neighbors() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let loom = dir.path().join("ws/loom");
        std::fs::create_dir_all(&loom).unwrap();
        std::fs::write(loom.join("prev.md"), "the previous note's telling").unwrap();
        let huge_tail = "x".repeat(5_000);
        std::fs::write(
            loom.join("cur.md"),
            format!("[[prev]]\n\nthe current note's telling\n{huge_tail}"),
        )
        .unwrap();

        let cur = loom.join("cur.md").display().to_string();
        mem.bump(&cur, 1.2, Carrier::Ambient).unwrap();
        let flashes = mem.take_flashes();
        assert_eq!(flashes.len(), 1);
        assert!(
            flashes[0].text.contains("the current note's telling"),
            "whole-body flash"
        );
        assert!(
            flashes[0].text.chars().count() <= FLASH_TEXT_CAP + 1,
            "capped: {}",
            flashes[0].text.len()
        );
        assert_eq!(flashes[0].neighbors.len(), 1, "wiki neighbor rides along");
        assert_eq!(flashes[0].neighbors[0].0, "wiki");
        assert!(flashes[0].neighbors[0].1.contains("previous note"));
    }

    #[test]
    fn wiki_link_parsing_handles_aliases_headings_and_dupes() {
        let body = "see [[a|alias]] and [[b#section]] and [[a]] and [[ ]]";
        assert_eq!(wiki_links(body), vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn loom_is_always_watched_and_nested_dirs_do_not_double_index() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(ws.join("knowledge")).unwrap();
        std::fs::create_dir_all(ws.join("loom")).unwrap();
        std::fs::write(ws.join("loom/telling.md"), "the loom holds the telling").unwrap();
        std::fs::write(ws.join("knowledge/k.md"), "a claim about herons").unwrap();

        // No index_dirs at all: loom/ is watched by default.
        let mem = Memory::open(&dir.path().join("data"), &ws, &[], Arc::new(FakeEmbedder)).unwrap();
        let (indexed, _) = mem.sweep().await.unwrap();
        assert_eq!(indexed, 2, "loom note indexed without config");
        mem.on_read(&ws.join("loom/telling.md")).unwrap();
        assert!(
            mem.activation(&ws.join("loom/telling.md").display().to_string())
                .unwrap()
                .is_some(),
            "loom reads bump"
        );

        // index_dirs ["."]: nested watch never indexes a file twice,
        // and no path carries a `.` component.
        let mem2 = Memory::open(
            &dir.path().join("data2"),
            &ws,
            &[".".to_string()],
            Arc::new(FakeEmbedder),
        )
        .unwrap();
        let (indexed, _) = mem2.sweep().await.unwrap();
        assert_eq!(indexed, 2, "each file indexed exactly once");
        let hits = mem2.search("heron claim").await.unwrap();
        assert!(
            hits.iter().all(|h| !h.file_path.contains("/./")),
            "{hits:?}"
        );
    }

    #[tokio::test]
    async fn activation_knobs_change_the_dynamics() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        let knobs = river_core::config::ActivationConfig {
            cognitive_bump: 2.0,
            propagation_factor: 0.1,
            propagation_hops: 1,
            decay_factor: 0.5,
            ..Default::default()
        };
        let mem = Memory::open_with(
            &dir.path().join("data"),
            &workspace,
            &[],
            Arc::new(FakeEmbedder),
            knobs,
        )
        .unwrap();
        let k = workspace.join("knowledge");
        write_note(&k, "a.md", "NA", &[("extends", "NB")], "a");
        write_note(&k, "b.md", "NB", &[("extends", "NC")], "b");
        write_note(&k, "c.md", "NC", &[], "c");

        mem.on_read(&k.join("a.md")).unwrap();
        assert_eq!(mem.activation("NA").unwrap(), Some(2.0), "knob bump");
        assert_eq!(mem.activation("NB").unwrap(), Some(0.2), "knob factor");
        assert_eq!(mem.activation("NC").unwrap(), None, "knob hops");

        mem.decay_tick().unwrap();
        assert_eq!(mem.activation("NA").unwrap(), Some(1.0), "knob decay");
    }

    #[tokio::test]
    async fn flash_dirs_filter_who_may_surface() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        std::fs::create_dir_all(workspace.join("loom")).unwrap();
        let knobs = river_core::config::ActivationConfig {
            flash_dirs: vec!["knowledge".to_string()],
            ..Default::default()
        };
        let mem = Memory::open_with(
            &dir.path().join("data"),
            &workspace,
            &[],
            Arc::new(FakeEmbedder),
            knobs,
        )
        .unwrap();
        write_note(
            &workspace.join("knowledge"),
            "k.md",
            "NK",
            &[],
            "an atomic claim",
        );
        std::fs::write(workspace.join("loom/long.md"), "a loom telling").unwrap();

        // The atomic may flash.
        mem.bump("NK", 1.2, Carrier::Ambient).unwrap();
        let flashes = mem.take_flashes();
        assert_eq!(flashes.len(), 1);
        assert_eq!(mem.activation("NK").unwrap(), Some(0.6), "halved");

        // The loom note crosses but stands silently: no flash, no
        // halving — it still holds its warmth and conducts.
        let loom_id = workspace.join("loom/long.md").display().to_string();
        mem.bump(&loom_id, 1.2, Carrier::Ambient).unwrap();
        assert!(mem.take_flashes().is_empty(), "filtered: cannot surface");
        assert_eq!(mem.activation(&loom_id).unwrap(), Some(1.2), "not halved");
    }

    #[tokio::test]
    async fn graph_payload_has_cold_nodes_typed_wiki_and_semantic_edges() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(
            &k,
            "a.md",
            "NA",
            &[("extends", "NB")],
            "the heron waits in shallow water",
        );
        write_note(&k, "b.md", "NB", &[], "claim citing [[NC]] in passing");
        write_note(&k, "c.md", "NC", &[], "the heron waited by shallow water");
        mem.sweep().await.unwrap();
        mem.bump("NA", 0.5, Carrier::Cognitive).unwrap();

        let graph = mem.graph().unwrap();
        assert_eq!(graph.flash_threshold, 1.0);
        assert_eq!(graph.nodes.len(), 3, "cold nodes included");
        let nc = graph.nodes.iter().find(|n| n.id == "NC").unwrap();
        assert_eq!(nc.path, "knowledge/c.md", "workspace-relative");
        let na = graph.nodes.iter().find(|n| n.id == "NA").unwrap();
        assert!(na.score > 0.0);

        assert!(
            graph
                .links
                .iter()
                .any(|l| l.link_type == "extends" && l.source == "NA" && l.target == "NB")
        );
        assert!(
            graph
                .links
                .iter()
                .any(|l| l.link_type == "wiki" && l.source == "NB" && l.target == "NC")
        );
        let semantic: Vec<_> = graph
            .links
            .iter()
            .filter(|l| l.link_type == "semantic")
            .collect();
        assert!(
            semantic.iter().any(|l| {
                let pair = (l.source.as_str(), l.target.as_str());
                pair == ("NA", "NC") || pair == ("NC", "NA")
            }),
            "near-identical bodies share a semantic edge: {semantic:?}"
        );
        for l in &semantic {
            assert!(l.similarity.unwrap() >= 0.65);
        }
        // Deduped by unordered pair.
        let mut pairs: Vec<(String, String)> = semantic
            .iter()
            .map(|l| (l.source.clone(), l.target.clone()))
            .collect();
        pairs.sort();
        let before = pairs.len();
        pairs.dedup();
        assert_eq!(pairs.len(), before, "no duplicate semantic edges");
    }

    #[tokio::test]
    async fn empty_files_index_without_embedding() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        std::fs::write(dir.path().join("ws/knowledge/empty.md"), "").unwrap();
        let (indexed, _) = mem.sweep().await.unwrap();
        assert_eq!(indexed, 1, "hash recorded");
        assert_eq!(mem.sweep().await.unwrap(), (0, 0), "not retried");
    }

    #[tokio::test]
    async fn rejection_vector_insert_and_top_k() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.insert_rejection_vector("01A", "warm goodnight", Some("not a claim"), 5, "t1")
            .await
            .unwrap();
        mem.insert_rejection_vector(
            "01B",
            "the pattern of enqueue-before-log",
            Some("meta-mining"),
            8,
            "t2",
        )
        .await
        .unwrap();
        mem.insert_rejection_vector("01C", "eliot's boat", None, 12, "t3")
            .await
            .unwrap();
        assert_eq!(mem.rejection_vector_count().unwrap(), 3);

        // FakeEmbedder is a letter histogram; goodnight ↔ goodbye
        // share most letters so cosine is high.
        let hits = mem
            .top_similar_rejections("warm goodbye", 2, 0.0)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2, "top_k respected");
        assert_eq!(hits[0].candidate_id, "01A", "closest by letter overlap");
        assert!(hits[0].score > hits[1].score, "sorted by score desc");
    }

    #[tokio::test]
    async fn top_k_zero_returns_empty_without_embedding() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.insert_rejection_vector("01A", "anything", None, 1, "t")
            .await
            .unwrap();
        let hits = mem
            .top_similar_rejections("anything", 0, 0.0)
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn threshold_filters_hits_below_floor() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // Two rejections with disjoint letters — low cosine to each other.
        mem.insert_rejection_vector("01A", "aaa", None, 1, "t")
            .await
            .unwrap();
        mem.insert_rejection_vector("01B", "zzz", None, 2, "t")
            .await
            .unwrap();
        // Query is exactly "aaa" (score 1.0 for A, 0.0 for B). Threshold
        // 0.5 keeps A, drops B.
        let hits = mem.top_similar_rejections("aaa", 10, 0.5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].candidate_id, "01A");
    }

    #[tokio::test]
    async fn duplicate_candidate_id_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.insert_rejection_vector("01A", "first text", None, 1, "t")
            .await
            .unwrap();
        // Second call with the same id — the INSERT OR IGNORE holds.
        mem.insert_rejection_vector("01A", "second text ignored", None, 2, "t2")
            .await
            .unwrap();
        assert_eq!(mem.rejection_vector_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn dim_mismatch_rows_are_skipped_by_retrieval() {
        // Simulate a row from a different embedding model by writing a
        // wrong-length blob directly. Retrieval must skip it, not panic.
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        mem.insert_rejection_vector("01A", "aaa", None, 1, "t")
            .await
            .unwrap();
        {
            let db = mem.db.lock().unwrap();
            db.execute(
                "INSERT INTO rejection_vectors
                   (candidate_id, turn, candidate, reason, at, embedding)
                 VALUES ('01B', 2, 'stale', NULL, 't', ?1)",
                [vec![0u8; 3]], // wrong length
            )
            .unwrap();
        }
        let hits = mem.top_similar_rejections("aaa", 10, 0.0).await.unwrap();
        assert_eq!(hits.len(), 1, "stale-dim row skipped");
        assert_eq!(hits[0].candidate_id, "01A");
    }

    #[tokio::test]
    async fn startup_rebuild_populates_missing_from_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let rejections = dir.path().join("ws/witness/rejections.jsonl");
        std::fs::create_dir_all(rejections.parent().unwrap()).unwrap();
        std::fs::write(
            &rejections,
            "{\"candidate_id\":\"01A\",\"candidate\":\"aaa\",\"turn\":1,\"at\":\"t\"}\n\
             {\"candidate_id\":\"01B\",\"candi\n\
             {\"candidate_id\":\"01C\",\"candidate\":\"ccc\",\"reason\":\"why\",\"turn\":3,\"at\":\"t\"}\n",
        )
        .unwrap();
        mem.ensure_rejection_vectors_ready(&rejections)
            .await
            .unwrap();
        // Torn line skipped; two rows inserted.
        assert_eq!(mem.rejection_vector_count().unwrap(), 2);
        // Idempotent: second call is a no-op.
        mem.ensure_rejection_vectors_ready(&rejections)
            .await
            .unwrap();
        assert_eq!(mem.rejection_vector_count().unwrap(), 2);
    }

    #[tokio::test]
    async fn startup_rebuild_wipes_on_dim_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // Plant a stale-dim row.
        {
            let db = mem.db.lock().unwrap();
            db.execute(
                "INSERT INTO rejection_vectors
                   (candidate_id, turn, candidate, reason, at, embedding)
                 VALUES ('stale', 1, 'stale', NULL, 't', ?1)",
                [vec![0u8; 3]],
            )
            .unwrap();
        }
        let rejections = dir.path().join("ws/witness/rejections.jsonl");
        std::fs::create_dir_all(rejections.parent().unwrap()).unwrap();
        std::fs::write(
            &rejections,
            "{\"candidate_id\":\"01A\",\"candidate\":\"aaa\",\"turn\":1,\"at\":\"t\"}\n",
        )
        .unwrap();
        mem.ensure_rejection_vectors_ready(&rejections)
            .await
            .unwrap();
        assert_eq!(mem.rejection_vector_count().unwrap(), 1);
        // The stale row is gone; the fresh one from jsonl is in.
        let hits = mem.top_similar_rejections("aaa", 10, 0.0).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].candidate_id, "01A");
    }

    #[tokio::test]
    async fn oversize_query_shrinks_and_succeeds_via_embed_query() {
        // Reproduces the ollama 400 iris hit: `{recent_record}` grew
        // past the embedding model's context. embed_query must halve
        // the input until the strict embedder accepts it, so retrieval
        // degrades to a truncated query rather than failing the glean.
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        let mem = Memory::open(
            &dir.path().join("data"),
            &workspace,
            &[],
            Arc::new(StrictEmbedder(700)),
        )
        .unwrap();
        mem.insert_rejection_vector("01A", "warm goodnight", None, 1, "t")
            .await
            .unwrap();
        let long = "warm ".repeat(2000); // ~10 KB, far past 700
        let hits = mem
            .top_similar_rejections(&long, 3, 0.0)
            .await
            .expect("shrinking-cap retry succeeds instead of 400ing");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].candidate_id, "01A");
    }

    // -------- hunt tests --------
    //
    // Each names the drift it catches. Per CLAUDE.md, no test that
    // cannot fail on a plausible regression.

    // --- cosine primitive ---

    /// Hunts: someone drops the `na == 0.0 || nb == 0.0` guard and
    /// zero vectors produce NaN, which then poisons every downstream
    /// comparator (`NaN < x` = false, `NaN > x` = false) — silent
    /// misfires or silent silence across the whole retrieval layer.
    /// Also hunts drift of the identity/orthogonal/anti properties.
    #[test]
    fn cosine_zero_and_dim_and_identity_properties() {
        // Zero vector: 0.0, not NaN.
        let z = vec![0.0f32; 3];
        assert_eq!(cosine(&[1.0, 2.0, 3.0], &z), 0.0);
        assert_eq!(cosine(&z, &[1.0, 2.0, 3.0]), 0.0);
        assert!(!cosine(&z, &z).is_nan(), "zero/zero must not be NaN");
        // Dim mismatch: 0.0.
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
        // Empty: 0.0.
        assert_eq!(cosine(&[], &[]), 0.0);
        // Identity: 1.0.
        let v = [3.0f32, 4.0, 0.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5);
        // Orthogonal: 0.0.
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-5);
        // Anti-parallel: -1.0.
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-5);
    }

    // --- cap_chars ---

    /// Hunts: someone converts cap_chars from chars() to len() (bytes)
    /// and multi-byte truncation panics on non-ASCII boundaries. Also
    /// hunts changes that omit the ellipsis suffix.
    #[test]
    fn cap_chars_utf8_and_boundary() {
        assert_eq!(cap_chars("hello", 100), "hello", "under cap: unchanged");
        assert_eq!(cap_chars("hello", 5), "hello", "at cap: unchanged");
        let out = cap_chars("hello", 4);
        assert_eq!(out, "hell…", "over cap: 4 chars + ellipsis");
        // 3-byte codepoints — no panic, correct char count.
        let wide: String = "字".repeat(1000);
        let out = cap_chars(&wide, 200);
        assert_eq!(out.chars().count(), 201, "200 chars + ellipsis");
        assert!(out.chars().take(200).all(|c| c == '字'));
    }

    // --- search_no_bump_with_vec ---

    /// Hunts: someone removes the top_k == 0 short-circuit and does a
    /// full segments scan for nothing (perf regression on any Bridge
    /// call that gates top_k on config).
    #[tokio::test]
    async fn search_no_bump_with_vec_top_k_zero_short_circuits() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // Give it a segment to prove k=0 wouldn't accidentally return.
        let ws = dir.path().join("ws");
        std::fs::write(ws.join("knowledge/01A.md"), "some words here\n").unwrap();
        mem.sweep().await.unwrap();
        let v = mem.embed_query("anything").await.unwrap();
        let hits = mem.search_no_bump_with_vec(&v, 0).unwrap();
        assert!(hits.is_empty(), "k=0 must return empty");
    }

    /// Hunts: someone drops the `blob.len() == expected_bytes` guard,
    /// and a dim-mismatched row (stale after embedder swap) panics on
    /// chunks_exact or produces a nonsense cosine.
    #[tokio::test]
    async fn search_no_bump_with_vec_skips_dim_mismatched_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let ws = dir.path().join("ws");
        std::fs::write(ws.join("knowledge/01A.md"), "some words\n").unwrap();
        mem.sweep().await.unwrap();
        // Corrupt one segment's embedding to the wrong byte length.
        {
            let db = mem.db.lock().unwrap();
            db.execute(
                "UPDATE segments SET embedding = ?1",
                [&vec![0u8; 4] as &dyn rusqlite::ToSql],
            )
            .unwrap();
        }
        let v = mem.embed_query("query").await.unwrap();
        // FakeEmbedder produces 26-dim vectors → 104 bytes. Our
        // corrupted row is 4 bytes. Must be skipped, not panic.
        let hits = mem.search_no_bump_with_vec(&v, 10).unwrap();
        assert!(hits.is_empty(), "dim-mismatched row skipped, no panic");
    }

    // --- text_sim ---

    /// Hunts: someone changes text_sim to return the FIRST segment's
    /// similarity instead of the MAX. Bridge's text_sim_max gate then
    /// misclassifies files whose first segment is neutral but a later
    /// segment is a near-duplicate — spurious Bridge frames.
    #[tokio::test]
    async fn text_sim_returns_max_across_segments_not_first() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let ws = dir.path().join("ws");
        // Two paragraphs, split by blank line → two segments. First is
        // all consonants, second is a near-duplicate of the query.
        let path = ws.join("knowledge/01A.md");
        std::fs::write(
            &path,
            "bcdfg hjklm npqrs tvwxz\n\n\
             query words that should match\n",
        )
        .unwrap();
        mem.sweep().await.unwrap();
        let v = mem.embed_query("query words that should match").await.unwrap();
        let stored_path = path.display().to_string();
        let sim = mem.text_sim(&stored_path, &v).unwrap();
        // Should be very close to 1.0 (max hits the second segment,
        // which is nearly identical to the query).
        assert!(sim > 0.9, "text_sim must return MAX segment sim, got {sim}");
    }

    // --- decay_tick ---

    /// Hunts: someone changes the prune predicate from `< 0.01` to
    /// `<= 0.01` (or drops it entirely). The 0.01 floor is what keeps
    /// activation from silting up with sub-noise rows over time.
    #[tokio::test]
    async fn decay_tick_prunes_below_threshold_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // Set three activations: just above the floor, exactly at,
        // and just below.
        mem.set_activation_for_test("above", 0.011).unwrap();
        mem.set_activation_for_test("at", 0.010).unwrap();
        mem.set_activation_for_test("below", 0.009).unwrap();
        // Decay factor from default knobs (0.8 per wall) — but tick
        // multiplies BEFORE pruning, so post-tick scores are:
        //   above: 0.011 * decay
        //   at:    0.010 * decay
        //   below: 0.009 * decay
        // All three shrink; the prune keeps only rows with score >= 0.01.
        // Since even 0.011 * 0.8 = 0.0088 < 0.01, ALL three get pruned.
        // Use a decay of 1.0 (no shrink) by setting activation slightly
        // higher on the "above" case to verify the boundary logic.
        mem.set_activation_for_test("above", 100.0).unwrap();
        mem.set_activation_for_test("at", 0.010).unwrap();
        mem.set_activation_for_test("below", 0.009).unwrap();
        mem.decay_tick().unwrap();
        // "below" (< 0.01 after decay) is gone.
        assert!(mem.activation("below").unwrap().is_none(), "below-floor pruned");
        // "at" — 0.010 * 0.8 = 0.008 < 0.01, also pruned.
        assert!(mem.activation("at").unwrap().is_none(), "at-floor pruned after decay");
        // "above" survives — its score after decay is 80.0.
        let score_above = mem.activation("above").unwrap().unwrap();
        assert!(
            (score_above - 80.0).abs() < 1e-6,
            "above-floor survives with expected decayed score, got {score_above}"
        );
    }

    // --- read_frontmatter_field ---

    /// Hunts: someone tightens the parser to reject colons in the
    /// value, empty values passing through as Some(""), or missing
    /// closing --- treated as a valid block.
    #[test]
    fn read_frontmatter_field_handles_adversarial_shapes() {
        // Simple bare value.
        assert_eq!(
            read_frontmatter_field("---\nid: 01A\n---\nbody", "id").as_deref(),
            Some("01A")
        );
        // Quoted value.
        assert_eq!(
            read_frontmatter_field("---\nid: \"01B\"\n---\nbody", "id").as_deref(),
            Some("01B")
        );
        // Value contains a colon.
        assert_eq!(
            read_frontmatter_field("---\nshape: has:colons\n---\nbody", "shape").as_deref(),
            Some("has:colons")
        );
        // Empty value → None.
        assert!(read_frontmatter_field("---\nid: \n---\nbody", "id").is_none());
        // Field absent → None.
        assert!(read_frontmatter_field("---\nother: v\n---\nbody", "id").is_none());
        // No frontmatter at all → None.
        assert!(read_frontmatter_field("just body", "id").is_none());
        // Missing closing --- → None.
        assert!(read_frontmatter_field("---\nid: 01A\nnever ended", "id").is_none());
    }

    // --- unquote ---

    /// Hunts: someone breaks the paired-only unquoting (e.g., strips a
    /// single leading quote when the trailing is absent, mangling a
    /// value that happens to start with `"`).
    #[test]
    fn unquote_only_strips_paired_delimiters() {
        assert_eq!(unquote("bare"), "bare");
        assert_eq!(unquote("\"double\""), "double");
        assert_eq!(unquote("'single'"), "single");
        // Unpaired: return the raw value untouched.
        assert_eq!(unquote("\"unpaired"), "\"unpaired");
        assert_eq!(unquote("unpaired\""), "unpaired\"");
        // Whitespace is trimmed before quote handling.
        assert_eq!(unquote("  \"padded\"  "), "padded");
    }

    // --- link_stem ---

    /// Hunts: someone changes the stem extractor to always require the
    /// `.md` suffix (breaking bare-ulid targets) or to drop everything
    /// after the first `.` (breaking filenames like `20260612.md.bak`
    /// or partially-typed writes).
    #[test]
    fn link_stem_handles_bare_id_paths_and_no_extension() {
        assert_eq!(link_stem("knowledge/01A.md"), "01A");
        assert_eq!(link_stem("01A"), "01A");
        assert_eq!(link_stem("a/b/c/01A.md"), "01A");
        // No .md: return the last component as-is (bare wikilink).
        assert_eq!(link_stem("plain-target"), "plain-target");
        // Trailing slash: return empty (nothing after final /).
        assert_eq!(link_stem("dir/"), "");
    }

    // --- path_matches_prefixes ---

    /// Hunts: someone changes empty-list semantics from "match all" to
    /// "match none" (which would silently break every default search).
    /// Also hunts starts_with pitfalls where "knowledge/" would
    /// mistakenly match "knowledgeextra/foo.md" (it must not — the
    /// caller supplies a trailing slash by convention).
    #[tokio::test]
    async fn path_matches_prefixes_empty_and_starts_with_pitfall() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let ws = mem.workspace_root();

        // Empty prefixes: match anything.
        let arbitrary = ws.join("anything/foo.md").display().to_string();
        assert!(mem.path_matches_prefixes(&arbitrary, &[]));

        // Real prefix on workspace-relative path.
        let atomic = ws.join("knowledge/01A.md").display().to_string();
        assert!(mem.path_matches_prefixes(&atomic, &["knowledge/".to_string()]));

        // Same prefix WITHOUT trailing slash — matches a sibling dir
        // that shares the letter run. This documents current behavior:
        // the caller is expected to supply the trailing slash.
        let sibling = ws.join("knowledgex/foo.md").display().to_string();
        assert!(
            mem.path_matches_prefixes(&sibling, &["knowledge".to_string()]),
            "starts_with is literal — caller responsibility to trail-slash"
        );
        assert!(
            !mem.path_matches_prefixes(&sibling, &["knowledge/".to_string()]),
            "trailing slash disambiguates"
        );
    }

    // --- is_atomic_path / workspace_relative ---

    /// Hunts: someone widens is_atomic_path to include record/ or
    /// channels/, which would let the shape subsystem try to gloss
    /// engine-managed files.
    #[tokio::test]
    async fn is_atomic_path_scoped_to_knowledge_and_workspace_relative_strips_root() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let ws = mem.workspace_root();
        let atomic = ws.join("knowledge/01A.md").display().to_string();
        let loom = ws.join("loom/2026.md").display().to_string();
        assert!(mem.is_atomic_path(&atomic));
        assert!(!mem.is_atomic_path(&loom), "loom is not an atomic path");
        assert!(!mem.is_atomic_path("random/knowledge/01A.md"), "not workspace-anchored");

        // workspace_relative strips the workspace prefix cleanly.
        assert_eq!(mem.workspace_relative(&atomic), "knowledge/01A.md");
        // Already-relative or unrelated: passed through.
        assert_eq!(mem.workspace_relative("outside/other.md"), "outside/other.md");
    }

    // --- extraction queue ---

    /// Hunts: someone changes pop_candidate to return Err on empty or
    /// to panic. On a healthy quiet cycle the queue is empty most of
    /// the time; a bail would break the digestion wake loop.
    #[tokio::test]
    async fn pop_candidate_on_empty_queue_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        assert!(mem.pop_candidate().unwrap().is_none());
        assert_eq!(mem.queue_depth().unwrap(), 0);
    }

    // --- segmentation ---

    /// Hunts: someone changes segment_with_cap to panic or return
    /// Vec![""] on empty input. Empty files pass through index_file
    /// via this function; a panic would kill the sweep.
    #[test]
    fn segment_with_cap_on_empty_and_whitespace_returns_empty() {
        assert!(segment_with_cap("", 100).is_empty());
        assert!(segment_with_cap("   \n  \n\n   ", 100).is_empty());
    }

    /// Hunts: the shape-authorship TOCTOU race. If a Witness write
    /// lands on a note that already has an Agent row (because the
    /// sync service raced ahead), the storage layer must reject it —
    /// this is defense-in-depth against a caller who forgot the
    /// pre-check, and it closes the tiny window between the pre-
    /// check and the write in shape::process_one.
    #[tokio::test]
    async fn upsert_shape_rejects_witness_overwriting_agent_row() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        // Agent claims the row first.
        let first_wrote = mem
            .upsert_shape(
                "01A",
                "knowledge/01A.md",
                "the agent's own skeleton",
                ShapeAuthor::Agent,
                "agent",
                "",
            )
            .await
            .unwrap();
        assert!(first_wrote, "initial insert must land");

        // Witness tries to overwrite: guard must reject.
        let second_wrote = mem
            .upsert_shape(
                "01A",
                "knowledge/01A.md",
                "witness's skeleton",
                ShapeAuthor::Witness,
                "haiku-4.5",
                "hash-x",
            )
            .await
            .unwrap();
        assert!(!second_wrote, "witness must not overwrite agent");
        // Row unchanged.
        let row = mem.read_shape("01A").unwrap().unwrap();
        assert_eq!(row.author, ShapeAuthor::Agent);
        assert_eq!(row.gloss, "the agent's own skeleton");
        assert_eq!(row.model_id, "agent");

        // Agent overwriting Agent is fine (agent updates their own).
        let third_wrote = mem
            .upsert_shape(
                "01A",
                "knowledge/01A.md",
                "agent's revised skeleton",
                ShapeAuthor::Agent,
                "agent",
                "",
            )
            .await
            .unwrap();
        assert!(third_wrote, "agent may overwrite their own row");
        let row = mem.read_shape("01A").unwrap().unwrap();
        assert_eq!(row.gloss, "agent's revised skeleton");

        // Witness overwriting witness is fine (drift-repair).
        let _ = mem
            .upsert_shape(
                "01B",
                "knowledge/01B.md",
                "v1",
                ShapeAuthor::Witness,
                "m1",
                "h1",
            )
            .await
            .unwrap();
        let updated = mem
            .upsert_shape(
                "01B",
                "knowledge/01B.md",
                "v2",
                ShapeAuthor::Witness,
                "m2",
                "h2",
            )
            .await
            .unwrap();
        assert!(updated, "witness→witness updates freely");
        let row = mem.read_shape("01B").unwrap().unwrap();
        assert_eq!(row.gloss, "v2");
        assert_eq!(row.model_id, "m2");
    }
}
