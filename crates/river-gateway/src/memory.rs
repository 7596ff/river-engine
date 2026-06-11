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

use crate::config::ModelConfig;

pub const COGNITIVE_BUMP: f64 = 1.0;
pub const AMBIENT_BUMP: f64 = 0.5;
const SEGMENT_TARGET_BYTES: usize = 1200;
const SEARCH_TOP_K: usize = 8;

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
}

impl Memory {
    pub fn open(
        data_dir: &Path,
        workspace: &Path,
        index_dirs: &[String],
        embedder: Arc<dyn Embed>,
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

        let mut watched = vec![workspace.join("knowledge")];
        for dir in index_dirs {
            watched.push(workspace.join(dir));
        }
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            embedder,
            workspace: workspace.to_path_buf(),
            watched,
        })
    }

    /// One sweep: hash every watched file, (re)index changes, remove
    /// vectors for deleted files. Returns (indexed, removed).
    pub async fn sweep(&self) -> anyhow::Result<(usize, usize)> {
        let mut on_disk: Vec<(String, String, String)> = Vec::new(); // path, hash, text
        for dir in &self.watched {
            collect_files(dir, &mut on_disk)?;
        }

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
            self.index_file(path, hash, text).await?;
            indexed += 1;
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
        let segments = segment(text);
        let vectors = self.embedder.embed(&segments).await?;
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
        hits.truncate(SEARCH_TOP_K);

        for hit in &hits {
            self.bump_path(&hit.file_path, AMBIENT_BUMP)?;
        }
        Ok(hits)
    }

    /// Is this path under a watched directory (and so indexed)?
    pub fn is_watched(&self, path: &Path) -> bool {
        self.watched.iter().any(|dir| path.starts_with(dir))
    }

    /// A cognitive access through the file-tool seam (wall ch. 07).
    pub fn on_read(&self, path: &Path) -> anyhow::Result<()> {
        if self.is_watched(path) {
            self.bump_path(&path.display().to_string(), COGNITIVE_BUMP)?;
        }
        Ok(())
    }

    /// A watched write: bump now; the next sweep re-indexes.
    pub fn on_write(&self, path: &Path) -> anyhow::Result<bool> {
        if self.is_watched(path) {
            self.bump_path(&path.display().to_string(), COGNITIVE_BUMP)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Record a bump (note identity = frontmatter id when the file
    /// has one, else the path). Propagation and decay are step 5.
    fn bump_path(&self, path: &str, amount: f64) -> anyhow::Result<()> {
        let note_id = frontmatter_id(Path::new(path)).unwrap_or_else(|| path.to_string());
        let db = self.db.lock().expect("db lock");
        db.execute(
            "INSERT INTO activation (note_id, score, bumped_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(note_id) DO UPDATE SET score = score + ?2, bumped_at = ?3",
            rusqlite::params![note_id, amount, now()],
        )?;
        Ok(())
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

    /// Run the periodic sweep until shutdown.
    pub async fn run_sync(
        self,
        mut reindex: tokio::sync::mpsc::Receiver<()>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        loop {
            if let Err(e) = self.sweep().await {
                tracing::warn!(error = %e, "sync sweep failed");
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
fn segment(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    for para in text.split("\n\n") {
        if !current.is_empty() && current.len() + para.len() > SEGMENT_TARGET_BYTES {
            segments.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
}

fn collect_files(dir: &Path, out: &mut Vec<(String, String, String)>) -> anyhow::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(()); // a watched dir may not exist yet
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            let hash = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
            out.push((path.display().to_string(), hash, text));
        }
    }
    Ok(())
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
            return Some(id.trim().to_string());
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
        assert_eq!(mem.activation(&b_id).unwrap(), Some(AMBIENT_BUMP));
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
        assert_eq!(mem.activation(&id).unwrap(), Some(COGNITIVE_BUMP));

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
        assert_eq!(mem.activation("01JXXTESTULID").unwrap(), Some(COGNITIVE_BUMP));
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

    #[test]
    fn segmentation_accumulates_paragraphs() {
        let text = format!("{}\n\n{}\n\n{}", "a".repeat(800), "b".repeat(800), "c".repeat(100));
        let segments = segment(&text);
        assert_eq!(segments.len(), 2);
        assert!(segments[1].contains('c'));
    }
}
