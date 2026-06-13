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
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use rusqlite::Connection;
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

/// A pending flash: surfaced into the next context's memory slot.
#[derive(Debug, Clone)]
pub struct Flash {
    pub note_id: String,
    pub text: String,
    pub neighbors: Vec<(String, String)>, // (link type, neighbor text)
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
                .timeout(std::time::Duration::from_secs(config.request_timeout_seconds))
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
                        .map(|v| v.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect())
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
}

/// One indexed file's graph identity and links, parsed live from the
/// workspace — links are ground truth, never cached. Files with
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

impl Memory {
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
        let conn = Connection::open(data_dir.join("river.db"))
            .with_context(|| format!("opening {}", data_dir.join("river.db").display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS extraction_queue (
                 id          TEXT PRIMARY KEY,
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
             );",
        )?;

        // knowledge/ and loom/ are always watched (wall chs. 02, 08);
        // config adds more. Paths are normalized (`.` components
        // dropped) and deduplicated so `index_dirs: ["."]` cannot
        // index the same file twice under two spellings.
        let mut watched: Vec<PathBuf> = Vec::new();
        for dir in ["knowledge", "loom"]
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
        })
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
        for (path, hash, text) in &on_disk {
            let unchanged = known.iter().any(|(p, h)| p == path && h == hash);
            if unchanged {
                continue;
            }
            // One bad file must not pin the sweep: warn and move on;
            // it retries next sweep.
            match self.index_file(path, hash, text).await {
                Ok(()) => indexed += 1,
                Err(e) => tracing::warn!(path, error = %e, "indexing failed; skipping"),
            }
        }

        let mut removed = 0;
        for (path, _) in &known {
            if !on_disk.iter().any(|(p, _, _)| p == path) {
                let db = self.db.lock().expect("db lock");
                db.execute("DELETE FROM segments WHERE file_path = ?1", [path])?;
                db.execute("DELETE FROM file_hashes WHERE file_path = ?1", [path])?;
                removed += 1;
            }
        }
        if indexed + removed > 0 {
            tracing::info!(indexed, removed, "sync sweep");
        }
        Ok((indexed, removed))
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
        let db = self.db.lock().expect("db lock");
        db.execute("DELETE FROM segments WHERE file_path = ?1", [path])?;
        for (seq, (seg_text, vector)) in segments.iter().zip(vectors.iter()).enumerate() {
            let blob: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
            db.execute(
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
        db.execute(
            "INSERT INTO file_hashes (file_path, hash, indexed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_path) DO UPDATE SET hash = ?2, indexed_at = ?3",
            rusqlite::params![path, hash, now()],
        )?;
        Ok(())
    }

    /// Top-k cosine over the stored vectors. Every hit is an ambient
    /// access for the file it touches.
    pub async fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>> {
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
            let bump = self.knobs.cognitive_bump;
            self.bump_path(&path.display().to_string(), bump, Carrier::Cognitive)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn bump_path(&self, path: &str, amount: f64, carrier: Carrier) -> anyhow::Result<()> {
        let note_id =
            frontmatter_id(Path::new(path)).unwrap_or_else(|| path.to_string());
        self.bump(&note_id, amount, carrier)
    }

    /// Apply a bump and its single-pass wave (wall ch. 02): ×0.5 per
    /// hop, 3 hops deep, one wave outward — propagated bumps trigger
    /// no further waves. Energy ignores link direction and type.
    pub fn bump(&self, origin: &str, amount: f64, carrier: Carrier) -> anyhow::Result<()> {
        let notes = self.notes();
        let resolver = Resolver::build(&notes);
        let mut adjacency: std::collections::HashMap<String, Vec<String>> = Default::default();
        for note in &notes {
            for (_, target) in &note.links {
                let Some(target) = resolver.resolve(target) else {
                    continue; // dangling: no edge, no energy lost
                };
                adjacency.entry(note.id.clone()).or_default().push(target.clone());
                adjacency.entry(target).or_default().push(note.id.clone());
            }
        }

        let mut visited: std::collections::HashSet<String> = Default::default();
        let mut frontier = vec![origin.to_string()];
        visited.insert(origin.to_string());
        let mut wave_amount = amount;

        for hop in 0..=self.knobs.propagation_hops {
            let mut next: Vec<String> = Vec::new();
            for id in &frontier {
                let hop_carrier = if hop == 0 { carrier } else { Carrier::Propagated };
                self.apply_bump(id, wave_amount, hop_carrier, &notes)?;
                if let Some(neighbors) = adjacency.get(id.as_str()) {
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
        let origin_path = notes
            .iter()
            .find(|n| n.id == origin)
            .map(|n| n.path.display().to_string())
            .unwrap_or_else(|| origin.to_string());
        self.semantic_spread(&origin_path, amount, &notes, &visited)?;
        Ok(())
    }

    /// Mean stored vector per indexed file.
    fn file_vectors(&self) -> anyhow::Result<Vec<(String, Vec<f32>)>> {
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
            let entry = sums.entry(path).or_insert_with(|| (vec![0.0; vector.len()], 0));
            for (s, v) in entry.0.iter_mut().zip(&vector) {
                *s += v;
            }
            entry.1 += 1;
        }
        Ok(sums
            .into_iter()
            .map(|(path, (sum, n))| {
                (path, sum.into_iter().map(|x| x / n as f32).collect())
            })
            .collect())
    }

    /// Semantic propagation (wall ch. 02, implicit warmth): the bump
    /// origin's embedding neighbors warm at ×0.25, one hop, no chain.
    fn semantic_spread(
        &self,
        origin_path: &str,
        amount: f64,
        notes: &[NoteInfo],
        already: &std::collections::HashSet<String>,
    ) -> anyhow::Result<()> {
        let vectors = self.file_vectors()?;
        let Some((_, origin_vec)) = vectors.iter().find(|(p, _)| p == origin_path) else {
            return Ok(());
        };
        let mut scored: Vec<(&String, f32)> = vectors
            .iter()
            .filter(|(p, _)| p != origin_path)
            .map(|(p, v)| (p, cosine(origin_vec, v)))
            .filter(|(_, s)| *s >= self.knobs.semantic_threshold)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (path, _) in scored.into_iter().take(self.knobs.semantic_top_k) {
            let id = frontmatter_id(Path::new(path)).unwrap_or_else(|| path.clone());
            if already.contains(&id) {
                continue; // the typed-link wave already reached it
            }
            self.apply_bump(&id, amount * self.knobs.semantic_factor, Carrier::Propagated, notes)?;
        }
        Ok(())
    }

    /// Conversation resonance (wall ch. 02, implicit warmth): the
    /// turn's own text warms the nearest notes ambiently, no waves.
    pub async fn resonate(&self, turn_text: &str) -> anyhow::Result<()> {
        self.resonate_with(turn_text, self.knobs.resonance_factor).await
    }

    /// Tool resonance (wall ch. 02, implicit warmth): each tool
    /// result's text warms the nearest notes at 0.8 × similarity —
    /// what passes through the agent's hands warms what it resembles.
    pub async fn resonate_tool(&self, result_text: &str) -> anyhow::Result<()> {
        self.resonate_with(result_text, self.knobs.tool_resonance_factor).await
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
        let vectors = self.file_vectors()?;
        let notes = self.notes();
        let mut scored: Vec<(&String, f32)> = vectors
            .iter()
            .map(|(p, v)| (p, cosine(&query, v)))
            .filter(|(_, s)| *s >= self.knobs.resonance_threshold)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (path, similarity) in scored.into_iter().take(self.knobs.resonance_top_k) {
            let id = frontmatter_id(Path::new(path)).unwrap_or_else(|| path.clone());
            self.apply_bump(&id, factor * similarity as f64, Carrier::Ambient, &notes)?;
        }
        Ok(())
    }

    /// One note's bump, with the flash carrier rule: only ambient or
    /// propagated warmth crossing the threshold from below fires a
    /// flash (halve + pend); a cognitive crossing stands silently.
    fn apply_bump(
        &self,
        note_id: &str,
        amount: f64,
        carrier: Carrier,
        notes: &[NoteInfo],
    ) -> anyhow::Result<()> {
        let old: f64 = {
            let db = self.db.lock().expect("db lock");
            db.query_row(
                "SELECT score FROM activation WHERE note_id = ?1",
                [note_id],
                |row| row.get(0),
            )
            .unwrap_or(0.0)
        };
        let mut new = old + amount;
        let threshold = self.knobs.flash_threshold;
        let crossed = old < threshold && new >= threshold;

        if crossed && carrier != Carrier::Cognitive && self.may_flash(note_id, notes) {
            new /= 2.0;
            let resolver = Resolver::build(notes);
            let flash = match notes.iter().find(|n| n.id == note_id) {
                Some(note) => Flash {
                    note_id: note_id.to_string(),
                    text: cap_chars(&note.body, FLASH_TEXT_CAP),
                    neighbors: note
                        .links
                        .iter()
                        .filter_map(|(link_type, target)| {
                            let target = resolver.resolve(target)?;
                            notes
                                .iter()
                                .find(|n| n.id == target)
                                .map(|n| (link_type.clone(), cap_chars(&n.body, FLASH_TEXT_CAP)))
                        })
                        .take(FLASH_NEIGHBOR_CAP)
                        .collect(),
                },
                None => Flash {
                    note_id: note_id.to_string(),
                    text: std::fs::read_to_string(note_id)
                        .map(|t| cap_chars(&t, FLASH_TEXT_CAP))
                        .unwrap_or_default(),
                    neighbors: Vec::new(),
                },
            };
            tracing::info!(note = %flash.note_id, "flash: crossed the threshold");
            self.pending_flashes.lock().expect("flash lock").push(flash);
        }

        let db = self.db.lock().expect("db lock");
        db.execute(
            "INSERT INTO activation (note_id, score, bumped_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(note_id) DO UPDATE SET score = ?2, bumped_at = ?3",
            rusqlite::params![note_id, new, now()],
        )?;
        Ok(())
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

    /// Agent-side: take the front of the FIFO queue.
    pub fn pop_candidate(&self) -> anyhow::Result<Option<String>> {
        let db = self.db.lock().expect("db lock");
        let front: Option<(String, String)> = db
            .query_row(
                "SELECT id, candidate FROM extraction_queue ORDER BY id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((id, candidate)) = front {
            db.execute("DELETE FROM extraction_queue WHERE id = ?1", [&id])?;
            return Ok(Some(candidate));
        }
        Ok(None)
    }

    pub fn queue_depth(&self) -> anyhow::Result<u64> {
        let db = self.db.lock().expect("db lock");
        Ok(db.query_row("SELECT COUNT(*) FROM extraction_queue", [], |row| row.get(0))?)
    }

    pub fn activation(&self, note_id: &str) -> anyhow::Result<Option<f64>> {
        let db = self.db.lock().expect("db lock");
        let mut stmt = db.prepare("SELECT score FROM activation WHERE note_id = ?1")?;
        let mut rows = stmt.query([note_id])?;
        Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Run the periodic sweep and the hourly decay tick until
    /// shutdown.
    pub async fn run_sync(
        self,
        mut reindex: tokio::sync::mpsc::Receiver<()>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut last_decay = std::time::Instant::now();
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
                _ = reindex.recv() => {}
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
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// Paragraph-accumulating segmentation, ~1200 bytes per segment.
/// Paragraphs longer than the hard cap are split at char boundaries —
/// a single giant paragraph (voice transcripts) must never exceed the
/// embedder's context.
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
fn frontmatter_id(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? .trim() != "---" {
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
    async fn sweep_indexes_changes_and_removals() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let note = dir.path().join("ws/knowledge/heron.md");
        std::fs::write(&note, "the heron waits in shallow water").unwrap();

        assert_eq!(mem.sweep().await.unwrap(), (1, 0));
        assert_eq!(mem.sweep().await.unwrap(), (0, 0), "unchanged: skipped");

        std::fs::write(&note, "the heron strikes quickly").unwrap();
        assert_eq!(mem.sweep().await.unwrap(), (1, 0), "changed: re-indexed");

        std::fs::remove_file(&note).unwrap();
        assert_eq!(mem.sweep().await.unwrap(), (0, 1), "deleted: removed");
        assert!(mem.search("heron").await.unwrap().is_empty());
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
        assert_eq!(mem.activation(&b_id).unwrap(), Some(0.5), "default ambient bump");
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
        assert_eq!(mem.activation(&id).unwrap(), Some(1.0), "default cognitive bump");

        // Unwatched reads do not bump.
        let elsewhere = dir.path().join("ws/draft.md");
        std::fs::write(&elsewhere, "x").unwrap();
        mem.on_read(&elsewhere).unwrap();
        assert_eq!(mem.activation(&elsewhere.display().to_string()).unwrap(), None);
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

    #[tokio::test]
    async fn flash_carrier_rule_holds() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        let k = dir.path().join("ws/knowledge");
        write_note(&k, "h.md", "NH", &[("same-pattern-as", "NO")], "the heron waits");
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
        write_note(&k, "h.md", "NH", &[("same-pattern-as", "NO")], "the heron waits");
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
        write_note(&k, "a.md", "NA", &[], "the heron waits in shallow water for fish");
        write_note(&k, "b.md", "NB", &[], "the heron waited by the shallow water for a fish");
        write_note(
            &k,
            "z.md",
            "NZ",
            &[],
            &"zzzz qqqq xxxx jjjj ".repeat(30),
        );
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
        write_note(&k, "h.md", "NH", &[], "the heron waits in shallow water for fish");
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
        write_note(&k, "h.md", "NH", &[], "the heron waits in shallow water for fish");
        write_note(&k, "z.md", "NZ", &[], &"zzzz qqqq xxxx jjjj ".repeat(30));
        mem.sweep().await.unwrap();

        let text = "a tool result mentioning the heron in the water and what it waits for";
        mem.resonate_tool(text).await.unwrap();
        let tool_warmth = mem.activation("NH").unwrap().expect("tool resonance bumped");
        assert_eq!(mem.activation("NZ").unwrap(), None, "dissimilar: cold");

        // Same text through conversation resonance lands at 1/4 the
        // warmth: the factors are 0.8 vs 0.2 on the same similarity.
        let dir2 = tempfile::tempdir().unwrap();
        let mem2 = memory(dir2.path());
        let k2 = dir2.path().join("ws/knowledge");
        write_note(&k2, "h.md", "NH", &[], "the heron waits in shallow water for fish");
        mem2.sweep().await.unwrap();
        mem2.resonate(text).await.unwrap();
        let conv_warmth = mem2.activation("NH").unwrap().expect("resonance bumped");
        assert!(
            (tool_warmth - conv_warmth * 4.0).abs() < 1e-9,
            "{tool_warmth} vs {conv_warmth}"
        );

        // Ambient carrier: a tool-result crossing flashes.
        mem.bump("NH", 0.95 - tool_warmth, Carrier::Ambient).unwrap();
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
        assert_eq!(mem.pop_candidate().unwrap().as_deref(), Some("first"));
        assert_eq!(mem.pop_candidate().unwrap().as_deref(), Some("second"));
        assert_eq!(mem.pop_candidate().unwrap(), None);
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
        let text = format!("{}\n\n{}\n\n{}", "a".repeat(800), "b".repeat(800), "c".repeat(100));
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
        write_note(&k, "b.md", "NB", &[("extends", "loom/20260612012002756.md")], "b");
        write_note(&k, "c.md", "NC", &[("extends", "../loom/20260612012002756.md")], "c");

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
        write_note(&k, "new.md", "NEWID", &[("extends", "20260612231237474")], "the newer claim");

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
        assert_eq!(mem.activation("X1").unwrap(), None, "ambiguity conducts nothing");
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
        write_note(&k, "a.md", "NA", &[("extends", "NB")], "claim citing [[NC]] inline");
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
        assert!(flashes[0].text.contains("the current note's telling"), "whole-body flash");
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
        assert!(hits.iter().all(|h| !h.file_path.contains("/./")), "{hits:?}");
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
        write_note(&workspace.join("knowledge"), "k.md", "NK", &[], "an atomic claim");
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
    async fn empty_files_index_without_embedding() {
        let dir = tempfile::tempdir().unwrap();
        let mem = memory(dir.path());
        std::fs::write(dir.path().join("ws/knowledge/empty.md"), "").unwrap();
        let (indexed, _) = mem.sweep().await.unwrap();
        assert_eq!(indexed, 1, "hash recorded");
        assert_eq!(mem.sweep().await.unwrap(), (0, 0), "not retried");
    }
}
