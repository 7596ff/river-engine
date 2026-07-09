//! The witness (wall ch. 04): the second voice. A role, not a
//! process — a concurrent task in the same binary, with its own model
//! assignment, whose behavior is markdown in `workspace/witness/`.
//! This module implements duty one, moves: per-turn compressions of
//! the record, appended to `record/moves.jsonl`.
//!
//! The witness never trusts a self-summary — it reads the turn's
//! lines from the record. The whole model output, trimmed, is the
//! move (no formats, no parsing — the prompt carries the discipline).
//! A model failure produces a mechanical fallback move: a turn is
//! never lost from the arc.
//!
//! The wake signal is the latest settled turn number; the witness
//! scans the record for every turn up to it that has no move line and
//! processes them in order. That makes it self-healing twice over:
//! missed signals, restarts, and downtime recover by catch-up, and a
//! hand-edited moves.jsonl (a deleted line) regenerates from the
//! record — the record is the truth, the moves are derived.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use tokio::sync::watch;

use crate::model::{Chat, ChatMessage};
use crate::record::{self, MovesFile, RecordLine, RecordRole};
use crate::turn::{DIGESTION_MARKER, HEARTBEAT_MARKER};

pub const NOTHING_TO_GLEAN: &str = "nothing to glean";
pub const NOTHING_TO_CONNECT: &str = "nothing to connect";
const GLEAN_WINDOW_TURNS: u64 = 6;
/// How many top hits the connect duty scans past when self-write
/// guards keep tripping. Small on purpose — this is a filter, not a
/// search widening.
const CONNECT_SCAN_K: usize = 5;

/// Read `glean-log.jsonl` and recover the tail entry's turn. Torn
/// lines and parse failures are skipped with a warning — the tail
/// before the torn line still wins.
fn recover_last_glean_through(path: &Path) -> Option<u64> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "reading glean-log.jsonl");
            return None;
        }
    };
    let mut last: Option<u64> = None;
    for (line_no, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<GleanLogEntry>(raw) {
            Ok(entry) => last = Some(entry.turn),
            Err(e) => tracing::warn!(
                path = %path.display(),
                line = line_no + 1,
                error = %e,
                "skipping malformed glean-log entry"
            ),
        }
    }
    last
}

/// Recover the tail `turn` of the connect-log. Same shape as
/// `recover_last_glean_through`.
fn recover_last_connect_through(path: &Path) -> Option<u64> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "reading connect-log.jsonl");
            return None;
        }
    };
    let mut last: Option<u64> = None;
    for (line_no, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ConnectLogEntry>(raw) {
            Ok(entry) => last = Some(entry.turn),
            Err(e) => tracing::warn!(
                path = %path.display(),
                line = line_no + 1,
                error = %e,
                "skipping malformed connect-log entry"
            ),
        }
    }
    last
}

/// Append one entry to `connect-log.jsonl`, creating the file if
/// absent. Same fsync-per-line discipline as the glean log.
fn append_connect_log(path: &Path, entry: &ConnectLogEntry) -> anyhow::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut json = serde_json::to_string(entry)?;
    json.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("fsyncing {}", path.display()))?;
    Ok(())
}

/// Append one entry to `glean-log.jsonl`, creating the file if absent.
fn append_glean_log(path: &Path, entry: &GleanLogEntry) -> anyhow::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut json = serde_json::to_string(entry)?;
    json.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("fsyncing {}", path.display()))?;
    Ok(())
}

pub struct Witness<C: Chat> {
    workspace: PathBuf,
    client: C,
    identity: String,
    on_turn: Option<String>,
    on_glean: Option<String>,
    moves: MovesFile,
    memory: Option<crate::memory::Memory>,
    glean_probability: f64,
    /// Refractory threshold: minimum turns of forward movement
    /// required between queued candidates. Zero disables the gate.
    glean_min_new_turns: u64,
    /// Hard ceiling on the extraction queue at enqueue time. Zero
    /// disables. Refractory state stays untouched on a drop.
    max_queue_depth: u64,
    /// How many recent rejections to render into `{recent_rejections}`.
    recent_rejections_window: usize,
    /// Top-K semantically similar past rejections to render into
    /// `{similar_rejections}`. Zero disables the read path — no
    /// query-side embed, no scan.
    similar_rejections_top_k: usize,
    /// Cosine floor for similar-rejection retrieval.
    similar_rejections_threshold: f32,
    /// The `up_to_turn` of the most recently queued candidate;
    /// recovered at load from the log tail, updated on each enqueue.
    last_glean_through: Option<u64>,
    /// `workspace/witness/glean-log.jsonl`.
    glean_log_path: PathBuf,
    /// `workspace/witness/rejections.jsonl`.
    rejections_path: PathBuf,
    /// The compose-why prompt loaded from `witness/on-connect.md`, or
    /// `None` when the file is absent (connect duty disabled).
    on_connect: Option<String>,
    /// Cosine floor for the connect duty's top-hit gate. Zero also
    /// disables the duty (no embed, no scan, no model call).
    connect_threshold: f32,
    /// Refractory: minimum turns of forward movement between fired
    /// connects. Zero disables.
    connect_min_new_turns: u64,
    /// Look-back window for the connect duty's self-connection guard.
    connect_self_write_window: u64,
    /// The turn of the most recently fired connect, recovered from
    /// `connect-log.jsonl` on load.
    last_connect_through: Option<u64>,
    /// `workspace/witness/connect-log.jsonl`.
    connect_log_path: PathBuf,
    /// mpsc sender for connect frames — the seam that preserves the
    /// turn record's single-writer invariant. None when no memory or
    /// no sender was attached (duty disabled).
    connect_sender: Option<tokio::sync::mpsc::Sender<crate::turn::ConnectFrame>>,
}

/// One entry in `glean-log.jsonl`: the receipt for a queued candidate.
#[derive(serde::Serialize, serde::Deserialize)]
struct GleanLogEntry {
    id: String,
    turn: u64,
    at: String,
}

/// One entry in `connect-log.jsonl`: the receipt for a fired connect
/// frame. Persisted so refractory state (and idempotency across
/// restart-mid-catch-up) survives a crash.
#[derive(serde::Serialize, serde::Deserialize)]
struct ConnectLogEntry {
    turn: u64,
    target_ref: String,
    at: String,
}

/// One entry in `rejections.jsonl`: written by the agent's
/// `reject_candidate` tool, read here so the witness can learn what
/// the agent already turned away.
#[derive(serde::Deserialize)]
struct RejectionEntry {
    candidate_id: String,
    candidate: String,
    #[serde(default)]
    reason: Option<String>,
    turn: u64,
    #[allow(dead_code)]
    at: String,
}

const REJECTION_PREVIEW_CHARS: usize = 80;

/// Tail-read N rejection entries. Missing file or empty → empty list;
/// torn lines skipped with a warning, same as channels.rs.
fn recent_rejections(path: &Path, window: usize) -> Vec<RejectionEntry> {
    if window == 0 {
        return Vec::new();
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "reading rejections.jsonl");
            return Vec::new();
        }
    };
    let mut all: Vec<RejectionEntry> = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RejectionEntry>(raw) {
            Ok(entry) => all.push(entry),
            Err(e) => tracing::warn!(
                path = %path.display(),
                line = line_no + 1,
                error = %e,
                "skipping malformed rejection entry"
            ),
        }
    }
    let drop = all.len().saturating_sub(window);
    all.into_iter().skip(drop).collect()
}

/// Render rejections as the `{recent_rejections}` block. Empty list
/// substitutes to an empty string so the prompt reads naturally
/// on day-one.
fn format_rejections(entries: &[RejectionEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n[your prior gleans the agent rejected]\n");
    for entry in entries {
        let preview = preview_line(&entry.candidate);
        match &entry.reason {
            Some(reason) => out.push_str(&format!(
                "turn {}: \"{preview}\" — reason: {reason}\n",
                entry.turn
            )),
            None => out.push_str(&format!("turn {}: \"{preview}\"\n", entry.turn)),
        }
    }
    out
}

/// Render similar-rejection hits as the `{similar_rejections}` block.
/// Empty list substitutes to an empty string so the operator's
/// surrounding label can sit alone on day one without looking broken
/// (same convention as `{recent_rejections}`).
fn format_similar_rejections(hits: &[crate::memory::SimilarRejection]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n[your prior gleans, semantically similar to what you're looking at now]\n",
    );
    for hit in hits {
        let preview = preview_line(&hit.candidate);
        match &hit.reason {
            Some(reason) => out.push_str(&format!(
                "turn {} (sim {:.2}): \"{preview}\" — reason: {reason}\n",
                hit.turn, hit.score
            )),
            None => out.push_str(&format!(
                "turn {} (sim {:.2}): \"{preview}\"\n",
                hit.turn, hit.score
            )),
        }
    }
    out
}

fn preview_line(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= REJECTION_PREVIEW_CHARS {
        first_line.to_string()
    } else {
        let mut t: String = first_line.chars().take(REJECTION_PREVIEW_CHARS).collect();
        t.push('…');
        t
    }
}

impl<C: Chat> Witness<C> {
    /// Load prompts and open the moves file. A missing
    /// `witness/identity.md` fails startup — the gateway does not run
    /// without its witness (wall ch. 04). Missing duty prompts
    /// disable their duty, logged once.
    pub fn load(
        workspace: &Path,
        client: C,
        memory: Option<crate::memory::Memory>,
        glean_probability: f64,
        glean_min_new_turns: u64,
        max_queue_depth: u64,
        recent_rejections_window: usize,
    ) -> anyhow::Result<Self> {
        // σ-retrieval knobs default off; main.rs applies configured
        // values via `with_similar_rejections` after load.
        let similar_rejections_top_k: usize = 0;
        let similar_rejections_threshold: f32 = 0.0;
        let identity_path = workspace.join("witness").join("identity.md");
        let identity = match std::fs::read_to_string(&identity_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
                "missing {} — compaction can only drop what the witness has \
                 compressed, so a harness without its witness pins its context; \
                 the gateway does not start without one",
                identity_path.display()
            ),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", identity_path.display()));
            }
        };

        let on_turn_path = workspace.join("witness").join("on-turn.md");
        let on_turn = match std::fs::read_to_string(&on_turn_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    path = %on_turn_path.display(),
                    "witness on-turn prompt missing; move duty disabled"
                );
                None
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", on_turn_path.display()));
            }
        };

        let on_glean_path = workspace.join("witness").join("on-glean.md");
        let on_glean = match std::fs::read_to_string(&on_glean_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    path = %on_glean_path.display(),
                    "witness on-glean prompt missing; gleaning duty disabled"
                );
                None
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", on_glean_path.display()));
            }
        };

        let moves = MovesFile::open(workspace)?;
        let glean_log_path = workspace.join("witness").join("glean-log.jsonl");
        let last_glean_through = recover_last_glean_through(&glean_log_path);
        let rejections_path = workspace.join("witness").join("rejections.jsonl");

        let on_connect_path = workspace.join("witness").join("on-connect.md");
        let on_connect = match std::fs::read_to_string(&on_connect_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    path = %on_connect_path.display(),
                    "witness on-connect prompt missing; connect duty disabled"
                );
                None
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", on_connect_path.display()));
            }
        };
        let connect_log_path = workspace.join("witness").join("connect-log.jsonl");
        let last_connect_through = recover_last_connect_through(&connect_log_path);

        Ok(Self {
            workspace: workspace.to_path_buf(),
            client,
            identity,
            on_turn,
            on_glean,
            moves,
            memory,
            glean_probability,
            glean_min_new_turns,
            max_queue_depth,
            recent_rejections_window,
            similar_rejections_top_k,
            similar_rejections_threshold,
            last_glean_through,
            glean_log_path,
            rejections_path,
            on_connect,
            connect_threshold: 0.0,
            connect_min_new_turns: 0,
            connect_self_write_window: 0,
            last_connect_through,
            connect_log_path,
            connect_sender: None,
        })
    }

    /// Enable the connect duty: attach the mpsc sender the turn loop
    /// is listening on and set the threshold + refractory + self-write
    /// window knobs. If the sender is `None` the duty stays disabled
    /// (loaders that don't want connect can pass `None`).
    pub fn with_connect(
        mut self,
        sender: Option<tokio::sync::mpsc::Sender<crate::turn::ConnectFrame>>,
        threshold: f32,
        min_new_turns: u64,
        self_write_window: u64,
    ) -> Self {
        self.connect_sender = sender;
        self.connect_threshold = threshold;
        self.connect_min_new_turns = min_new_turns;
        self.connect_self_write_window = self_write_window;
        self
    }

    /// Enable σ retrieval: the on-glean prompt's `{similar_rejections}`
    /// slot renders top-K past rejections whose cosine similarity to
    /// the current window meets the threshold. `top_k == 0` disables
    /// (which is also the load-time default).
    pub fn with_similar_rejections(mut self, top_k: usize, threshold: f32) -> Self {
        self.similar_rejections_top_k = top_k;
        self.similar_rejections_threshold = threshold;
        self
    }

    /// Run until shutdown: on every settled-turn signal, catch up
    /// from cursor + 1 to the latest turn, in order.
    pub async fn run(
        mut self,
        mut latest_turn: watch::Receiver<u64>,
        mut shutdown: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        loop {
            if self.on_turn.is_some() {
                let target = *latest_turn.borrow();
                for turn in self.missing_moves(target)? {
                    self.move_for(turn).await?;
                    // Connect duty (spec 2026-07-07): threshold-gated,
                    // per settled turn. Runs before the glean dice so a
                    // fired connect and a fired glean can coexist on the
                    // same turn without either blocking the other.
                    self.connect_for(turn).await?;
                    // Flat-probability gleaning (wall ch. 04): the
                    // agent cannot predict which turns get gleaned.
                    if rand::random::<f64>() < self.glean_probability {
                        self.glean(turn).await?;
                    }
                }
            }
            let stopping = tokio::select! {
                biased;
                _ = shutdown.wait_for(|&stop| stop) => true,
                changed = latest_turn.changed() => changed.is_err(),
            };
            if stopping {
                // The guaranteed end-of-session pass.
                let turn = *latest_turn.borrow();
                if turn > 0 {
                    self.glean(turn).await?;
                }
                return Ok(());
            }
        }
    }

    /// The catch-up set: every turn present in the record up to
    /// `target` with no move line. Normally the contiguous run after
    /// the last move; after a hand edit of moves.jsonl it also holds
    /// the holes, which regenerate from the record.
    fn missing_moves(&self, target: u64) -> anyhow::Result<Vec<u64>> {
        let have: std::collections::HashSet<u64> = record::read_moves(self.moves.path())?
            .iter()
            .map(|m| m.turn)
            .collect();
        let mut missing: Vec<u64> =
            record::scan(&self.workspace.join("record").join("turns.jsonl"))?
                .iter()
                .map(|l| l.turn)
                .filter(|t| *t <= target && !have.contains(t))
                .collect();
        missing.sort_unstable();
        missing.dedup();
        Ok(missing)
    }

    /// Duty two (wall ch. 04): review the recent stretch and write
    /// extraction candidates into the queue. Identifies knowledge;
    /// never writes it.
    async fn glean(&mut self, up_to_turn: u64) -> anyhow::Result<()> {
        let (Some(template), Some(memory)) = (&self.on_glean, &self.memory) else {
            return Ok(());
        };

        let from_turn = up_to_turn.saturating_sub(GLEAN_WINDOW_TURNS) + 1;
        let all_lines: Vec<RecordLine> =
            record::scan(&self.workspace.join("record").join("turns.jsonl"))?
                .into_iter()
                .filter(|l| l.turn >= from_turn && l.turn <= up_to_turn)
                .collect();
        if all_lines.is_empty() {
            return Ok(());
        }

        // A digestion turn carries no world-information — its only
        // inbound is the witness's own prior gleaning, framed by the
        // engine. Gleaning over it produces candidates about the
        // machinery of digestion, which the next quiet trigger then
        // re-digests; the abstraction climbs without bound. Skip the
        // dice roll entirely when the wake turn is a digestion, and
        // strip digestion turns from the window otherwise.
        if is_digestion_turn(&all_lines, up_to_turn) {
            tracing::debug!(turn = up_to_turn, "glean: skipped (digestion turn)");
            return Ok(());
        }
        // A heartbeat wake is the agent's autonomy floor (wall ch. 01).
        // Firing a glean from one turns the quiet floor into more
        // inbound — and the agent finds it intrusive. Skip the dice
        // when the wake itself is a heartbeat, but keep heartbeat turns
        // *in* the glean window: the loom work the agent does during
        // them is prime material (wall ch. 04), it just gets harvested
        // by the next real conversation turn or the end-of-session pass.
        if is_heartbeat_turn(&all_lines, up_to_turn) {
            tracing::debug!(turn = up_to_turn, "glean: skipped (heartbeat turn)");
            return Ok(());
        }
        if let Some(last) = self.last_glean_through
            && self.glean_min_new_turns > 0
            && up_to_turn.saturating_sub(last) < self.glean_min_new_turns
        {
            tracing::debug!(
                turn = up_to_turn,
                last_glean_through = last,
                min_new = self.glean_min_new_turns,
                "glean: skipped (refractory)"
            );
            return Ok(());
        }
        let digestion_turns: std::collections::HashSet<u64> = (from_turn..=up_to_turn)
            .filter(|t| is_digestion_turn(&all_lines, *t))
            .collect();
        let lines: Vec<RecordLine> = all_lines
            .into_iter()
            .filter(|l| !digestion_turns.contains(&l.turn))
            .collect();
        if lines.is_empty() {
            return Ok(());
        }

        let mut recent = format_transcript(&lines);
        let moves = record::read_moves(self.moves.path())?;
        if !moves.is_empty() {
            recent.push_str("\n[your moves]\n");
            for m in moves.iter().rev().take(GLEAN_WINDOW_TURNS as usize).rev() {
                recent.push_str(&format!("turn {}: {}\n", m.turn, m.summary));
            }
        }

        let rejections = recent_rejections(&self.rejections_path, self.recent_rejections_window);
        let rejection_block = format_rejections(&rejections);

        // σ-only retrieval: surface semantically similar past rejections
        // regardless of recency. On any failure — no memory system, no
        // top_k, embed error — the slot renders empty and the glean
        // proceeds; the jsonl-backed `{recent_rejections}` still fires.
        let similar_block = if self.similar_rejections_top_k > 0 {
            match &self.memory {
                Some(mem) => match mem
                    .top_similar_rejections(
                        &recent,
                        self.similar_rejections_top_k,
                        self.similar_rejections_threshold,
                    )
                    .await
                {
                    Ok(hits) => {
                        // Dedup: a rejection already in the recent list is
                        // dropped from the similar list (recent carries
                        // "you saw this recently" context the similar
                        // slot doesn't).
                        let recent_ids: std::collections::HashSet<&str> = rejections
                            .iter()
                            .map(|r| r.candidate_id.as_str())
                            .collect();
                        let filtered: Vec<_> = hits
                            .into_iter()
                            .filter(|h| !recent_ids.contains(h.candidate_id.as_str()))
                            .collect();
                        format_similar_rejections(&filtered)
                    }
                    Err(e) => {
                        tracing::warn!(turn = up_to_turn, error = %e, "similar-rejection retrieval failed");
                        String::new()
                    }
                },
                None => String::new(),
            }
        } else {
            String::new()
        };

        let prompt = template
            .replace("{recent_record}", &recent)
            .replace("{recent_rejections}", &rejection_block)
            .replace("{similar_rejections}", &similar_block);
        let messages = [ChatMessage::user(prompt)];
        match self.client.chat(&self.identity, &messages, &[]).await {
            Ok(response) => {
                let candidate = response.content.trim();
                if candidate.is_empty()
                    || candidate.eq_ignore_ascii_case(NOTHING_TO_GLEAN)
                {
                    tracing::debug!(turn = up_to_turn, "glean: nothing");
                } else if self.max_queue_depth > 0
                    && memory.queue_depth().unwrap_or(0) >= self.max_queue_depth
                {
                    // Queue at cap: drop. Refractory stays where it was
                    // — the drop should not burn forward movement.
                    tracing::warn!(
                        turn = up_to_turn,
                        depth = memory.queue_depth().unwrap_or(0),
                        cap = self.max_queue_depth,
                        candidate = %candidate,
                        "glean: candidate dropped (queue at cap)"
                    );
                } else {
                    let id = memory.enqueue_candidate(candidate)?;
                    // Log only after the queue insert succeeds — a torn
                    // log line cannot describe a phantom queue row, and
                    // a failed log write leaves the gate untouched so
                    // the next call retries naturally.
                    let entry = GleanLogEntry {
                        id,
                        turn: up_to_turn,
                        at: jiff::Timestamp::now().to_string(),
                    };
                    if let Err(e) = append_glean_log(&self.glean_log_path, &entry) {
                        tracing::warn!(turn = up_to_turn, error = %e, "glean-log append failed");
                    } else {
                        self.last_glean_through = Some(up_to_turn);
                    }
                    tracing::info!(turn = up_to_turn, "glean: candidate queued");
                }
            }
            Err(e) => {
                // Gleaning is best-effort; the dice roll again.
                tracing::warn!(turn = up_to_turn, error = %e, "glean failed");
            }
        }
        Ok(())
    }

    /// Duty one for a single turn: read the record, compress, append.
    async fn move_for(&mut self, turn: u64) -> anyhow::Result<()> {
        let template = self.on_turn.as_ref().expect("move duty enabled");
        let lines: Vec<RecordLine> =
            record::scan(&self.workspace.join("record").join("turns.jsonl"))?
                .into_iter()
                .filter(|l| l.turn == turn)
                .collect();

        let summary = if lines.is_empty() {
            tracing::warn!(turn, "no record lines for settled turn; mechanical move");
            format!("Turn {turn} settled with nothing recorded.")
        } else {
            let transcript = format_transcript(&lines);
            let prompt = template
                .replace("{turn_number}", &turn.to_string())
                .replace("{transcript}", &transcript);
            let messages = [ChatMessage::user(prompt)];
            match self.client.chat(&self.identity, &messages, &[]).await {
                Ok(response) if !response.content.trim().is_empty() => {
                    response.content.trim().to_string()
                }
                Ok(_) => {
                    tracing::warn!(turn, "witness model returned empty move; fallback");
                    fallback_move(&lines)
                }
                Err(e) => {
                    tracing::warn!(turn, error = %e, "witness model failed; fallback move");
                    fallback_move(&lines)
                }
            }
        };

        self.moves.append(turn, &summary)?;
        tracing::debug!(turn, "move written");
        Ok(())
    }

    /// Duty three (the connect duty, spec 2026-07-07): after a turn
    /// settles, semantically search the workspace for a note that
    /// connects. If the top hit clears the threshold and the self-
    /// write guard, compose a one-sentence why and post a
    /// [`ConnectFrame`] to the turn loop. Best-effort throughout —
    /// any failure downgrades to "no connect this turn."
    async fn connect_for(&mut self, turn: u64) -> anyhow::Result<()> {
        let (Some(template), Some(memory), Some(sender)) =
            (&self.on_connect, &self.memory, &self.connect_sender)
        else {
            return Ok(());
        };
        if self.connect_threshold <= 0.0 {
            return Ok(());
        }

        // Refractory: same shape as glean's.
        if let Some(last) = self.last_connect_through
            && self.connect_min_new_turns > 0
            && turn.saturating_sub(last) < self.connect_min_new_turns
        {
            tracing::debug!(
                turn,
                last_connect_through = last,
                min_new = self.connect_min_new_turns,
                "connect: skipped (refractory)"
            );
            return Ok(());
        }

        let all_lines: Vec<RecordLine> =
            record::scan(&self.workspace.join("record").join("turns.jsonl"))?;
        let this_turn_lines: Vec<RecordLine> = all_lines
            .iter()
            .filter(|l| l.turn == turn)
            .cloned()
            .collect();
        if this_turn_lines.is_empty() {
            return Ok(());
        }
        // Same exclusions the glean duty uses: digestion and heartbeat
        // turns don't carry the kind of substance connect should search
        // against.
        if is_digestion_turn(&all_lines, turn) || is_heartbeat_turn(&all_lines, turn) {
            tracing::debug!(turn, "connect: skipped (digestion/heartbeat)");
            return Ok(());
        }

        let transcript = format_transcript(&this_turn_lines);
        // The channel of the settled turn — every RecordLine for one
        // turn shares it (persist-once, wall ch. 01).
        let channel = this_turn_lines[0].channel.clone();

        let hits = match memory.search_no_bump(&transcript, CONNECT_SCAN_K).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(turn, error = %e, "connect search failed");
                return Ok(());
            }
        };

        let recent_writes = collect_recent_agent_writes(
            &all_lines,
            turn,
            self.connect_self_write_window,
            &self.workspace,
        );
        let hit = hits.into_iter().find(|h| {
            if h.score < self.connect_threshold {
                return false;
            }
            if self.connect_self_write_window == 0 {
                return true;
            }
            !recent_writes.iter().any(|w| paths_match(w, &h.file_path))
        });
        let Some(hit) = hit else {
            return Ok(());
        };

        let target_path = std::path::PathBuf::from(&hit.file_path);
        let target_ref = crate::memory::target_ref_for_path(&target_path);

        let prompt = template
            .replace("{transcript}", &transcript)
            .replace("{target_path}", &hit.file_path)
            .replace("{target_excerpt}", &hit.text);
        let messages = [ChatMessage::user(prompt)];
        let why = match self.client.chat(&self.identity, &messages, &[]).await {
            Ok(response) => {
                let text = response.content.trim();
                if text.is_empty() || text.eq_ignore_ascii_case(NOTHING_TO_CONNECT) {
                    tracing::debug!(turn, target = %target_ref, "connect: nothing to connect");
                    return Ok(());
                }
                text.to_string()
            }
            Err(e) => {
                tracing::warn!(turn, error = %e, "connect model call failed");
                return Ok(());
            }
        };

        let frame = crate::turn::ConnectFrame {
            turn,
            channel,
            target_ref: target_ref.clone(),
            target_path,
            why,
        };
        if let Err(e) = sender.try_send(frame) {
            tracing::warn!(turn, error = %e, "connect frame send failed; dropping");
            return Ok(());
        }

        // Log receipt AFTER send succeeds — a torn log line must not
        // describe a phantom frame (same discipline as glean).
        let entry = ConnectLogEntry {
            turn,
            target_ref: target_ref.clone(),
            at: jiff::Timestamp::now().to_string(),
        };
        if let Err(e) = append_connect_log(&self.connect_log_path, &entry) {
            tracing::warn!(turn, error = %e, "connect-log append failed");
        } else {
            self.last_connect_through = Some(turn);
        }
        tracing::info!(turn, target = %target_ref, "connect: frame posted");
        Ok(())
    }
}

/// Walk `record/turns.jsonl` from `turn - window` to `turn` inclusive
/// and collect every file path the agent wrote or edited during that
/// span. Used by the connect duty's self-connection guard so a note
/// the agent just authored does not surface as "you have a note that
/// connects to this."
fn collect_recent_agent_writes(
    all_lines: &[RecordLine],
    turn: u64,
    window: u64,
    workspace: &Path,
) -> Vec<PathBuf> {
    if window == 0 {
        return Vec::new();
    }
    let from = turn.saturating_sub(window);
    let mut out: Vec<PathBuf> = Vec::new();
    for line in all_lines.iter().filter(|l| l.turn >= from && l.turn <= turn) {
        if line.role != RecordRole::Assistant {
            continue;
        }
        for call in line.tool_calls.iter().flatten() {
            let is_write = matches!(call.name.as_str(), "write" | "edit" | "create_moment");
            if !is_write {
                continue;
            }
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&call.arguments) else {
                continue;
            };
            let Some(path_str) = args.get("path").and_then(|v| v.as_str()) else {
                // create_moment writes under record/moments/{id}.md — but
                // record/ is not indexed (wall ch. 10), so it never
                // matches a hit anyway. Nothing to record here.
                continue;
            };
            let candidate = std::path::PathBuf::from(path_str);
            let abs = if candidate.is_absolute() {
                candidate
            } else {
                workspace.join(candidate)
            };
            if !out.contains(&abs) {
                out.push(abs);
            }
        }
    }
    out
}

/// Rough path equality: canonicalise both when possible (they exist),
/// otherwise fall back to component-wise comparison. Handles the
/// common shapes — workspace-relative vs. absolute, `./` prefixes.
fn paths_match(a: &Path, b: &str) -> bool {
    let b = Path::new(b);
    if let (Ok(ca), Ok(cb)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return ca == cb;
    }
    let na: PathBuf = a.components().collect();
    let nb: PathBuf = b.components().collect();
    na == nb
}

/// A turn is digestion-driven iff its inbound (non-Assistant,
/// non-Tool) lines are all System frames marked with
/// [`DIGESTION_MARKER`]. Conservative: any non-marked inbound line
/// disqualifies the turn, so a hybrid turn (digestion candidate plus
/// a mid-turn arrival or compaction warning) is not skipped.
fn is_digestion_turn(lines: &[RecordLine], turn: u64) -> bool {
    let mut saw_marker = false;
    for line in lines.iter().filter(|l| l.turn == turn) {
        match line.role {
            RecordRole::User => return false,
            RecordRole::System => {
                let Some(content) = &line.content else { continue };
                if content.starts_with(DIGESTION_MARKER) {
                    saw_marker = true;
                } else {
                    // Other system frames (compaction warnings,
                    // mid-turn arrival notices) mean real activity.
                    return false;
                }
            }
            RecordRole::Assistant | RecordRole::Tool => {}
        }
    }
    saw_marker
}

/// A heartbeat-driven turn: its only inbound is the
/// [`HEARTBEAT_MARKER`] user line the turn loop appends on the timer
/// fire (wall ch. 01). Conservative, matching `is_digestion_turn`: any
/// other inbound (a real user message, a non-marker system frame)
/// disqualifies, so a heartbeat turn that also caught a mid-turn
/// arrival is not skipped.
fn is_heartbeat_turn(lines: &[RecordLine], turn: u64) -> bool {
    let mut saw_marker = false;
    for line in lines.iter().filter(|l| l.turn == turn) {
        match line.role {
            RecordRole::User => {
                let Some(content) = &line.content else { continue };
                if content.trim() == HEARTBEAT_MARKER {
                    saw_marker = true;
                } else {
                    return false;
                }
            }
            RecordRole::System => return false,
            RecordRole::Assistant | RecordRole::Tool => {}
        }
    }
    saw_marker
}

/// The agent's own words are marked "you:" — the transcript carries
/// the deixis so the prompt doesn't have to. Speech is a tool in this
/// body, so what the agent said aloud lives in the speak call's
/// arguments: the transcript surfaces it as first-class speech
/// ("you spoke: ..."), and other tool calls carry a truncated
/// argument peek — the witness cannot compress what it cannot see.
pub fn format_transcript(lines: &[RecordLine]) -> String {
    let mut transcript = String::new();
    for line in lines {
        let Some(content) = &line.content else {
            continue;
        };
        match line.role {
            RecordRole::User => {
                transcript.push_str(content);
                transcript.push('\n');
            }
            RecordRole::Assistant => {
                if !content.trim().is_empty() {
                    transcript.push_str("you: ");
                    transcript.push_str(content);
                    transcript.push('\n');
                }
                for call in line.tool_calls.iter().flatten() {
                    if call.name == "speak" {
                        let args: serde_json::Value =
                            serde_json::from_str(&call.arguments).unwrap_or_default();
                        let spoken = args["content"].as_str().unwrap_or("");
                        match args["channel"].as_str() {
                            Some(channel) => transcript
                                .push_str(&format!("you spoke on {channel}: {spoken}\n")),
                            None => transcript.push_str(&format!("you spoke: {spoken}\n")),
                        }
                    } else {
                        transcript.push_str(&format!(
                            "[you called {}: {}]\n",
                            call.name,
                            peek(&call.arguments, ARG_PEEK_CHARS)
                        ));
                    }
                }
            }
            RecordRole::System => {
                transcript.push_str("[system] ");
                transcript.push_str(content);
                transcript.push('\n');
            }
            RecordRole::Tool => {
                transcript.push_str("[tool result] ");
                transcript.push_str(content);
                transcript.push('\n');
            }
        }
    }
    transcript
}

const ARG_PEEK_CHARS: usize = 200;

fn peek(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(cap).collect();
        t.push('…');
        t
    }
}

/// Mechanical, from the roles involved: never a gap in the arc.
fn fallback_move(lines: &[RecordLine]) -> String {
    let inbound = lines
        .iter()
        .filter(|l| l.role == RecordRole::User)
        .count();
    let replied = lines.iter().any(|l| l.role == RecordRole::Assistant);
    let channels: std::collections::BTreeSet<&str> =
        lines.iter().map(|l| l.channel.as_str()).collect();
    let channel_list = channels.into_iter().collect::<Vec<_>>().join(", ");
    if replied {
        format!(
            "{inbound} message(s) arrived on {channel_list}; you replied. \
             (Mechanical move: your witness's model was unavailable.)"
        )
    } else {
        format!(
            "{inbound} message(s) arrived on {channel_list}; you did not reply. \
             (Mechanical move: your witness's model was unavailable.)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChatResponse;
    use crate::record::{TurnRecord, read_moves};
    use std::sync::{Arc, Mutex};

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
            _tools: &[crate::model::ToolSchema],
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

    fn seed_witness(workspace: &Path) {
        let dir = workspace.join("witness");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("identity.md"), "You are the witness.").unwrap();
        std::fs::write(
            dir.join("on-turn.md"),
            "Turn {turn_number}:\n{transcript}\nWrite the move.",
        )
        .unwrap();
    }

    fn record_turn(workspace: &Path, turn: u64, question: &str, answer: Option<&str>) {
        let mut rec = TurnRecord::open(workspace).unwrap();
        rec.append(
            turn,
            "local_main",
            RecordRole::User,
            Some(&format!("[local_main] cass: {question}")),
        )
        .unwrap();
        if let Some(answer) = answer {
            rec.append(turn, "local_main", RecordRole::Assistant, Some(answer))
                .unwrap();
        }
    }

    /// Shape of a digestion turn as written by `TurnLoop`: a System
    /// frame with the [DIGESTION_MARKER] prefix carrying the
    /// candidate, plus the agent's response.
    fn record_digestion_turn(workspace: &Path, turn: u64, candidate: &str, reply: &str) {
        let mut rec = TurnRecord::open(workspace).unwrap();
        rec.append(
            turn,
            "local_main",
            RecordRole::System,
            Some(&format!("{DIGESTION_MARKER} ...framing...\n\n{candidate}")),
        )
        .unwrap();
        rec.append(turn, "local_main", RecordRole::Assistant, Some(reply))
            .unwrap();
    }

    #[test]
    fn missing_identity_fails_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![]);
        let err = match Witness::load(dir.path(), model, None, 0.0, 0, 0, 0) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("load should fail without witness identity"),
        };
        assert!(err.contains("witness/identity.md"), "{err}");
    }

    #[test]
    fn missing_on_turn_disables_the_duty() {
        let dir = tempfile::tempdir().unwrap();
        let witness_dir = dir.path().join("witness");
        std::fs::create_dir_all(&witness_dir).unwrap();
        std::fs::write(witness_dir.join("identity.md"), "You are the witness.").unwrap();

        let model = FakeModel::replying(vec![]);
        let witness = Witness::load(dir.path(), model, None, 0.0, 0, 0, 0).unwrap();
        assert!(witness.on_turn.is_none());
    }

    #[tokio::test]
    async fn move_is_the_verbatim_trimmed_output() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        record_turn(dir.path(), 1, "what is teal?", Some("A blue-green color."));

        let model = FakeModel::replying(vec![ok(
            "  Cass asked what teal is; you defined it as blue-green.  \n",
        )]);
        let mut witness = Witness::load(dir.path(), model.clone(), None, 0.0, 0, 0, 0).unwrap();
        witness.move_for(1).await.unwrap();

        let moves = read_moves(witness.moves.path()).unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].turn, 1);
        assert_eq!(
            moves[0].summary,
            "Cass asked what teal is; you defined it as blue-green."
        );

        // The prompt substituted both variables, and the transcript
        // marks the agent's line as "you:".
        let prompts = model.prompts.lock().unwrap();
        assert_eq!(prompts[0].0, "You are the witness.");
        assert!(prompts[0].1.contains("Turn 1:"));
        assert!(prompts[0].1.contains("[local_main] cass: what is teal?"));
        assert!(prompts[0].1.contains("you: A blue-green color."));
    }

    #[tokio::test]
    async fn model_failure_writes_the_mechanical_fallback() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        record_turn(dir.path(), 1, "hello?", None);

        let model = FakeModel::replying(vec![Err(anyhow::anyhow!("witness model down"))]);
        let mut witness = Witness::load(dir.path(), model, None, 0.0, 0, 0, 0).unwrap();
        witness.move_for(1).await.unwrap();

        let moves = read_moves(witness.moves.path()).unwrap();
        assert_eq!(moves.len(), 1, "a turn is never lost");
        assert!(moves[0].summary.contains("1 message(s)"), "{}", moves[0].summary);
        assert!(moves[0].summary.contains("did not reply"));
        assert!(moves[0].summary.contains("local_main"));
    }

    #[tokio::test]
    async fn empty_output_also_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        record_turn(dir.path(), 1, "hi", Some("hello"));

        let model = FakeModel::replying(vec![ok("   \n ")]);
        let mut witness = Witness::load(dir.path(), model, None, 0.0, 0, 0, 0).unwrap();
        witness.move_for(1).await.unwrap();

        let moves = read_moves(witness.moves.path()).unwrap();
        assert!(moves[0].summary.contains("you replied"));
    }

    #[tokio::test]
    async fn gleaning_queues_candidates_and_respects_the_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nGlean.",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        record_turn(dir.path(), 1, "teal is my favorite", Some("noted"));

        let memory = crate::memory::Memory::open(
            &dir.path().join("data"),
            dir.path(),
            &[],
            std::sync::Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();

        // Replies: the move, the per-turn glean (a real candidate),
        // then the shutdown-pass glean (the sentinel).
        let model = FakeModel::replying(vec![
            ok("a move"),
            ok("Cass's favorite color is teal — worth a note. suggested link: extends color-notes"),
            ok("nothing to glean"),
        ]);
        let witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 0, 0, 0).unwrap();

        let (latest_tx, latest_rx) = watch::channel(1u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx));
        for _ in 0..100 {
            if memory.queue_depth().unwrap() >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        shutdown_tx.send(true).unwrap();
        drop(latest_tx);
        handle.await.unwrap().unwrap();

        // One real candidate; the sentinel enqueued nothing.
        assert_eq!(memory.queue_depth().unwrap(), 1);
        let (_, candidate) = memory.pop_candidate().unwrap().unwrap();
        assert!(candidate.contains("teal"), "{candidate}");
    }

    #[tokio::test]
    async fn glean_skips_digestion_turn_without_calling_the_model() {
        // The bug river reported: the witness gleans turn N, the
        // candidate fires as digestion turn N+1, the witness then
        // gleans turn N+1 — extracting knowledge about its own
        // gleanings, with each pass climbing a rung of abstraction.
        // Fix: digestion turns never see the dice. Verified by giving
        // FakeModel zero replies — any call would panic.
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nGlean.",
        )
        .unwrap();

        let memory = crate::memory::Memory::open(
            &dir.path().join("data"),
            dir.path(),
            &[],
            std::sync::Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();

        record_digestion_turn(
            dir.path(),
            1,
            "candidate text",
            "I reject this; it is machinery, not knowledge.",
        );

        let model = FakeModel::replying(vec![]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 0).unwrap();
        witness.glean(1).await.unwrap();

        assert_eq!(memory.queue_depth().unwrap(), 0);
        assert!(
            model.prompts.lock().unwrap().is_empty(),
            "model must not be called on a digestion turn",
        );
    }

    /// Shape of a heartbeat turn as written by `TurnLoop`: a single
    /// User-role line with the [HEARTBEAT_MARKER] content, plus
    /// whatever the agent did in response (often loom work).
    fn record_heartbeat_turn(workspace: &Path, turn: u64, reply: &str) {
        let mut rec = TurnRecord::open(workspace).unwrap();
        rec.append(turn, "local_main", RecordRole::User, Some(HEARTBEAT_MARKER))
            .unwrap();
        rec.append(turn, "local_main", RecordRole::Assistant, Some(reply))
            .unwrap();
    }

    #[tokio::test]
    async fn glean_skips_heartbeat_turn_without_calling_the_model() {
        // Heartbeats are the agent's autonomy floor; firing a glean
        // from them turns the floor into more inbound. Skip the dice
        // when the wake itself is a heartbeat. Verified by giving
        // FakeModel zero replies — any call would panic.
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        let memory = fresh_memory(dir.path());

        record_heartbeat_turn(dir.path(), 1, "wrote a loom note about teal.");

        let model = FakeModel::replying(vec![]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 0).unwrap();
        witness.glean(1).await.unwrap();

        assert_eq!(memory.queue_depth().unwrap(), 0);
        assert!(
            model.prompts.lock().unwrap().is_empty(),
            "model must not be called on a heartbeat turn",
        );
    }

    #[tokio::test]
    async fn glean_keeps_heartbeat_turns_in_the_window() {
        // Heartbeats are skipped as a *trigger*, not stripped from the
        // window — the loom work the agent did during a heartbeat is
        // still prime glean material when a later real turn rolls the
        // dice (wall ch. 04).
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        let memory = fresh_memory(dir.path());

        record_heartbeat_turn(dir.path(), 1, "loom note: cass loves teal.");
        record_turn(dir.path(), 2, "anything new?", Some("nothing big."));

        let model = FakeModel::replying(vec![ok("cass loves teal — worth a note")]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 0).unwrap();
        witness.glean(2).await.unwrap();

        let prompts = model.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1, "real turn triggers; heartbeat in window");
        assert!(
            prompts[0].1.contains("loom note: cass loves teal."),
            "heartbeat-turn content should still feed the glean window:\n{}",
            prompts[0].1
        );
        assert_eq!(memory.queue_depth().unwrap(), 1);
    }

    #[tokio::test]
    async fn glean_filters_digestion_turns_from_the_window() {
        // A real turn surrounded by a digestion turn in the same
        // window: the model's prompt must show the real turn and
        // must not contain the digestion frame's content.
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nGlean.",
        )
        .unwrap();

        let memory = crate::memory::Memory::open(
            &dir.path().join("data"),
            dir.path(),
            &[],
            std::sync::Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();

        record_turn(dir.path(), 1, "teal is my favorite", Some("noted"));
        record_digestion_turn(
            dir.path(),
            2,
            "POISONED_CANDIDATE_should_not_appear",
            "rejecting; this is the digestion machinery",
        );
        record_turn(dir.path(), 3, "also: rosemary", Some("rosemary noted"));

        let model = FakeModel::replying(vec![ok("rosemary and teal — cass's signals")]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 0).unwrap();
        witness.glean(3).await.unwrap();

        let prompts = model.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1, "model should be called once");
        let user = &prompts[0].1;
        assert!(user.contains("teal is my favorite"), "{user}");
        assert!(user.contains("rosemary"), "{user}");
        assert!(
            !user.contains("POISONED_CANDIDATE_should_not_appear"),
            "digestion content leaked into the glean window:\n{user}"
        );
        assert!(
            !user.contains(DIGESTION_MARKER),
            "digestion marker leaked into the glean window:\n{user}"
        );
        assert_eq!(memory.queue_depth().unwrap(), 1);
    }

    #[test]
    fn transcript_surfaces_spoken_words_and_tool_arguments() {
        // The shape iris reported: empty assistant content, the
        // actual words buried in the speak call's arguments.
        let lines = vec![
            RecordLine {
                id: "1".into(),
                turn: 7,
                channel: "discord_dm".into(),
                role: RecordRole::User,
                content: Some("[discord_dm] cass: how was the night?".into()),
                tool_calls: None,
                tool_call_id: None,
            },
            RecordLine {
                id: "2".into(),
                turn: 7,
                channel: "discord_dm".into(),
                role: RecordRole::Assistant,
                content: Some("".into()),
                tool_calls: Some(vec![
                    crate::model::ToolCall {
                        id: "c1".into(),
                        name: "speak".into(),
                        arguments: r#"{"content":"quiet and settled. the gleans ran twice."}"#
                            .into(),
                    },
                    crate::model::ToolCall {
                        id: "c2".into(),
                        name: "write".into(),
                        arguments: format!(
                            r#"{{"path":"loom/note.md","content":"{}"}}"#,
                            "x".repeat(500)
                        ),
                    },
                ]),
                tool_call_id: None,
            },
            RecordLine {
                id: "3".into(),
                turn: 7,
                channel: "discord_dm".into(),
                role: RecordRole::Tool,
                content: Some("spoken on discord_dm (msg 9)".into()),
                tool_calls: None,
                tool_call_id: Some("c1".into()),
            },
        ];
        let transcript = format_transcript(&lines);
        assert!(
            transcript.contains("you spoke: quiet and settled. the gleans ran twice."),
            "{transcript}"
        );
        assert!(transcript.contains("[you called write: "), "{transcript}");
        assert!(
            transcript.contains('…') && transcript.len() < 700,
            "write arguments truncated to a peek: {transcript}"
        );
        assert!(
            !transcript.contains("you: \n"),
            "empty assistant content renders nothing: {transcript}"
        );
    }

    #[test]
    fn transcript_marks_channel_override_speech() {
        let lines = vec![RecordLine {
            id: "1".into(),
            turn: 1,
            channel: "local_main".into(),
            role: RecordRole::Assistant,
            content: Some("".into()),
            tool_calls: Some(vec![crate::model::ToolCall {
                id: "c1".into(),
                name: "speak".into(),
                arguments: r#"{"content":"over here","channel":"discord_dm"}"#.into(),
            }]),
            tool_call_id: None,
        }];
        let transcript = format_transcript(&lines);
        assert!(transcript.contains("you spoke on discord_dm: over here"), "{transcript}");
    }

    #[tokio::test]
    async fn hand_deleted_moves_regenerate_from_the_record() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        for turn in 1..=3 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }

        // First pass: moves for all three turns.
        let model = FakeModel::replying(vec![ok("move one"), ok("move two"), ok("move three")]);
        let witness = Witness::load(dir.path(), model, None, 0.0, 0, 0, 0).unwrap();
        let moves_path = witness.moves.path().to_path_buf();
        let (latest_tx, latest_rx) = watch::channel(3u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx));
        for _ in 0..100 {
            if read_moves(&moves_path).unwrap().len() == 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        shutdown_tx.send(true).unwrap();
        drop(latest_tx);
        handle.await.unwrap().unwrap();

        // Ground's hand edit: delete the middle move's line.
        let text = std::fs::read_to_string(&moves_path).unwrap();
        let kept: Vec<&str> = text.lines().filter(|l| !l.contains("move two")).collect();
        std::fs::write(&moves_path, format!("{}\n", kept.join("\n"))).unwrap();
        assert_eq!(
            record::witness_cursor(&moves_path).unwrap(),
            1,
            "the cursor falls back to before the gap — turn 2 is undroppable again"
        );

        // Next wake: the gap regenerates from the record; new turns
        // still process.
        record_turn(dir.path(), 4, "q4", Some("a4"));
        let model = FakeModel::replying(vec![ok("move two, retold"), ok("move four")]);
        let witness = Witness::load(dir.path(), model, None, 0.0, 0, 0, 0).unwrap();
        let (latest_tx, latest_rx) = watch::channel(4u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx));
        for _ in 0..100 {
            if read_moves(&moves_path).unwrap().len() == 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        shutdown_tx.send(true).unwrap();
        drop(latest_tx);
        handle.await.unwrap().unwrap();

        let moves = read_moves(&moves_path).unwrap();
        let mut turns: Vec<u64> = moves.iter().map(|m| m.turn).collect();
        turns.sort_unstable();
        assert_eq!(turns, vec![1, 2, 3, 4], "no gaps, no duplicates");
        let two = moves.iter().find(|m| m.turn == 2).unwrap();
        assert_eq!(two.summary, "move two, retold");
        assert_eq!(record::witness_cursor(&moves_path).unwrap(), 4, "frontier recovered");
    }

    #[tokio::test]
    async fn catch_up_processes_every_turn_from_cursor_in_order() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        for turn in 1..=3 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }

        let model = FakeModel::replying(vec![ok("move one"), ok("move two"), ok("move three")]);
        let witness = Witness::load(dir.path(), model, None, 0.0, 0, 0, 0).unwrap();
        let moves_path = witness.moves.path().to_path_buf();

        let (latest_tx, latest_rx) = watch::channel(3u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx));

        // Wait for catch-up, then stop.
        for _ in 0..100 {
            if read_moves(&moves_path).unwrap().len() == 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        shutdown_tx.send(true).unwrap();
        drop(latest_tx);
        handle.await.unwrap().unwrap();

        let moves = read_moves(&moves_path).unwrap();
        assert_eq!(
            moves.iter().map(|m| m.turn).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "in order, no gaps"
        );
        assert_eq!(moves[2].summary, "move three");
        assert_eq!(record::witness_cursor(&moves_path).unwrap(), 3);
    }

    fn seed_glean(workspace: &Path) {
        seed_witness(workspace);
        std::fs::write(
            workspace.join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nGlean.",
        )
        .unwrap();
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
    }

    fn fresh_memory(workspace: &Path) -> crate::memory::Memory {
        crate::memory::Memory::open(
            &workspace.join("data"),
            workspace,
            &[],
            std::sync::Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn refractory_blocks_glean_within_threshold_without_calling_the_model() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=10u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());
        // One reply for the first glean only. If the second (refractory-
        // blocked) glean called the model, FakeModel would panic on the
        // empty queue.
        let model = FakeModel::replying(vec![ok("candidate one — worth a note")]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 12, 0, 0).unwrap();

        witness.glean(6).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1);
        // Within refractory (6 → 11 is only 5 turns of forward movement,
        // threshold is 12): no model call, no enqueue, no panic.
        witness.glean(11).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1);
    }

    #[tokio::test]
    async fn refractory_releases_after_threshold_turns() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=30u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![
            ok("candidate one"),
            ok("candidate two"),
        ]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 12, 0, 0).unwrap();

        witness.glean(6).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1);
        // 6 → 18 is exactly 12 turns: refractory releases.
        witness.glean(18).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 2);
    }

    #[tokio::test]
    async fn first_glean_always_fires() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=3u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("first one")]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 1000, 0, 0).unwrap();

        witness.glean(3).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1, "no prior gleans, gate is open");
    }

    #[tokio::test]
    async fn refractory_state_persists_across_witness_load() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=30u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());

        // First witness queues at turn 6 and writes the log.
        {
            let model = FakeModel::replying(vec![ok("candidate one")]);
            let mut witness =
                Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 12, 0, 0).unwrap();
            witness.glean(6).await.unwrap();
            assert_eq!(memory.queue_depth().unwrap(), 1);
        }

        // Second witness loads with the same workspace; the log tail
        // recovers last_glean_through, so a glean at turn 11 (only 5
        // forward) stays gated even though the queue could be empty
        // (we already popped to verify recovery is from the log, not
        // the queue).
        memory.pop_candidate().unwrap().unwrap();
        {
            // One reply — if the gate doesn't recover, the model is
            // called and the reply is consumed; we then check the
            // depth went up. If the gate recovers, no call, depth stays
            // at zero.
            let model = FakeModel::replying(vec![ok("would-be candidate two")]);
            let mut witness = Witness::load(
                dir.path(),
                model.clone(),
                Some(memory.clone()),
                1.0,
                12,
                0,
                0,
            )
            .unwrap();
            witness.glean(11).await.unwrap();
            assert_eq!(
                memory.queue_depth().unwrap(),
                0,
                "refractory recovered from the log; no enqueue"
            );
            // And explicitly: model was not called.
            assert_eq!(model.prompts.lock().unwrap().len(), 0);
        }
    }

    #[tokio::test]
    async fn missing_log_means_open_gate() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        record_turn(dir.path(), 1, "q1", Some("a1"));
        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("the very first one")]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 1000, 0, 0).unwrap();
        // No log file exists; the first glean fires.
        witness.glean(1).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1);
    }

    #[tokio::test]
    async fn torn_log_line_does_not_poison_recovery() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=30u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        // Write a log by hand: one good entry at turn 6, then a torn
        // line. Recovery should land on turn 6 — the torn line is
        // skipped, not fatal.
        let log_path = dir.path().join("witness/glean-log.jsonl");
        std::fs::write(
            &log_path,
            "{\"id\":\"01J\",\"turn\":6,\"at\":\"2026-06-16T00:00:00Z\"}\n{\"id\":\"01K\",\"turn\":\n",
        )
        .unwrap();

        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("after torn")]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 12, 0, 0).unwrap();
        // Within refractory of turn 6 → suppressed.
        witness.glean(11).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 0);
        // 6 → 18 releases the gate.
        witness.glean(18).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1);
    }

    #[tokio::test]
    async fn refractory_zero_disables_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=5u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("one"), ok("two"), ok("three")]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 0, 0, 0).unwrap();

        witness.glean(1).await.unwrap();
        witness.glean(2).await.unwrap();
        witness.glean(3).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 3, "gate disabled, all fire");
    }

    fn write_rejection_line(workspace: &Path, turn: u64, candidate: &str, reason: Option<&str>) {
        let path = workspace.join("witness/rejections.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut entry = serde_json::json!({
            "candidate_id": format!("01TEST{turn}"),
            "candidate": candidate,
            "turn": turn,
            "at": "2026-06-17T00:00:00Z",
        });
        if let Some(reason) = reason {
            entry["reason"] = serde_json::Value::String(reason.into());
        }
        let line = format!("{entry}\n");
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .unwrap();
        f.write_all(line.as_bytes()).unwrap();
    }

    #[tokio::test]
    async fn glean_prompt_substitutes_recent_rejections_when_present() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nRejections:{recent_rejections}\nGlean.",
        )
        .unwrap();
        record_turn(dir.path(), 1, "hi", Some("hello"));
        write_rejection_line(dir.path(), 5, "warm goodnight", Some("not a claim"));
        write_rejection_line(
            dir.path(),
            6,
            "the pattern of enqueue-before-log",
            Some("meta-mining"),
        );

        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("nothing to glean")]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 5)
                .unwrap();
        witness.glean(1).await.unwrap();

        let prompts = model.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        let body = &prompts[0].1;
        assert!(
            body.contains("[your prior gleans the agent rejected]"),
            "{body}"
        );
        assert!(body.contains("warm goodnight"), "{body}");
        assert!(body.contains("reason: not a claim"), "{body}");
        assert!(body.contains("turn 5"), "{body}");
        assert!(body.contains("turn 6"), "{body}");
    }

    #[tokio::test]
    async fn glean_prompt_substitutes_empty_when_no_rejections_file() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nRejections:{recent_rejections}\nGlean.",
        )
        .unwrap();
        record_turn(dir.path(), 1, "hi", Some("hello"));

        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("nothing to glean")]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 5)
                .unwrap();
        witness.glean(1).await.unwrap();

        let prompts = model.prompts.lock().unwrap();
        let body = &prompts[0].1;
        // {recent_rejections} substitutes to empty; the literal label
        // ("Rejections:") remains alone with nothing under it.
        assert!(!body.contains("[your prior gleans"), "{body}");
        assert!(body.contains("Rejections:\nGlean."), "{body}");
    }

    #[tokio::test]
    async fn queue_cap_drops_enqueue_and_preserves_refractory_state() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=30u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());
        // Pre-fill the queue past the cap (=1) so the next glean drops.
        memory.enqueue_candidate("already there").unwrap();

        let model = FakeModel::replying(vec![
            ok("would-have-been-a-candidate"),
            ok("the second one fires once cap clears"),
        ]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 1, 0)
                .unwrap();

        // First glean: model returns text, but cap is at 1 already.
        // Drop should not consume refractory state.
        witness.glean(6).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1, "drop, queue unchanged");

        // Pop the pre-existing entry — queue is now empty.
        let _ = memory.pop_candidate().unwrap().unwrap();

        // Second glean fires because cap is no longer reached and
        // last_glean_through stayed None (the previous drop preserved it).
        witness.glean(7).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 1, "second one queued");
    }

    #[tokio::test]
    async fn queue_cap_zero_disables() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        for turn in 1..=10u64 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }
        let memory = fresh_memory(dir.path());
        // Stuff the queue ahead of time.
        for i in 0..5 {
            memory.enqueue_candidate(&format!("pre-{i}")).unwrap();
        }

        let model = FakeModel::replying(vec![ok("candidate")]);
        let mut witness =
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0, 0, 0, 0).unwrap();
        witness.glean(6).await.unwrap();
        assert_eq!(memory.queue_depth().unwrap(), 6, "no cap, no drop");
    }

    #[tokio::test]
    async fn rejection_window_zero_returns_no_entries() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        write_rejection_line(dir.path(), 1, "anything", Some("any reason"));
        let path = dir.path().join("witness/rejections.jsonl");
        let entries = recent_rejections(&path, 0);
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn rejection_torn_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        let path = dir.path().join("witness/rejections.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\"candidate_id\":\"01A\",\"candidate\":\"good one\",\"turn\":5,\"at\":\"x\"}\n\
             {\"candidate_id\":\"01B\",\"candi\n\
             {\"candidate_id\":\"01C\",\"candidate\":\"after\",\"turn\":6,\"at\":\"x\"}\n",
        )
        .unwrap();
        let entries = recent_rejections(&path, 10);
        assert_eq!(entries.len(), 2, "torn line skipped");
        assert_eq!(entries[1].candidate, "after");
    }

    #[tokio::test]
    async fn glean_prompt_substitutes_similar_rejections_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nSimilar:{similar_rejections}\nGlean.",
        )
        .unwrap();
        record_turn(dir.path(), 1, "warm goodbye", Some("noted"));

        let memory = fresh_memory(dir.path());
        // Seed a past rejection whose text overlaps the query letters
        // (goodnight ↔ goodbye ↔ warm — heavy shared letters in the
        // FakeEmbedder's histogram, so cosine is high).
        memory
            .insert_rejection_vector("01OLD", "warm goodnight", Some("not a claim"), 42, "t")
            .await
            .unwrap();

        let model = FakeModel::replying(vec![ok("nothing to glean")]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 0)
                .unwrap()
                .with_similar_rejections(3, 0.0);
        witness.glean(1).await.unwrap();

        let prompts = model.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        let body = &prompts[0].1;
        assert!(
            body.contains("[your prior gleans, semantically similar"),
            "similar block present:\n{body}"
        );
        assert!(body.contains("warm goodnight"), "hit text rendered:\n{body}");
        assert!(body.contains("turn 42"), "hit turn rendered:\n{body}");
        assert!(body.contains("reason: not a claim"), "reason rendered:\n{body}");
    }

    #[tokio::test]
    async fn glean_prompt_similar_slot_empty_when_top_k_zero() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:\n{recent_record}\nSimilar:{similar_rejections}\nGlean.",
        )
        .unwrap();
        record_turn(dir.path(), 1, "warm goodbye", Some("noted"));

        let memory = fresh_memory(dir.path());
        memory
            .insert_rejection_vector("01OLD", "warm goodnight", None, 42, "t")
            .await
            .unwrap();

        let model = FakeModel::replying(vec![ok("nothing to glean")]);
        // Default: top_k = 0 disables the read path.
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 0).unwrap();
        witness.glean(1).await.unwrap();

        let body = &model.prompts.lock().unwrap()[0].1;
        assert!(!body.contains("prior gleans, semantically similar"), "{body}");
        assert!(body.contains("Similar:\nGlean."), "empty slot renders blank:\n{body}");
    }

    #[tokio::test]
    async fn similar_dedups_against_recent_rejections() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        std::fs::write(
            dir.path().join("witness/on-glean.md"),
            "Recent:{recent_rejections}\nSimilar:{similar_rejections}\nR:{recent_record}",
        )
        .unwrap();
        record_turn(dir.path(), 1, "warm goodbye", Some("noted"));

        // Same candidate_id in both places: recent-window rendering
        // AND vector table. The similar slot must drop it since recent
        // already shows it.
        write_rejection_line(dir.path(), 42, "warm goodnight", Some("not a claim"));
        let memory = fresh_memory(dir.path());
        memory
            .insert_rejection_vector("01TEST42", "warm goodnight", Some("not a claim"), 42, "t")
            .await
            .unwrap();

        let model = FakeModel::replying(vec![ok("nothing to glean")]);
        let mut witness =
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0, 0, 0, 5)
                .unwrap()
                .with_similar_rejections(3, 0.0);
        witness.glean(1).await.unwrap();

        let body = &model.prompts.lock().unwrap()[0].1;
        assert!(
            body.contains("[your prior gleans the agent rejected]"),
            "recent block rendered:\n{body}"
        );
        assert!(
            !body.contains("[your prior gleans, semantically similar"),
            "similar block dropped (only hit was already in recent):\n{body}"
        );
    }

    // ----- connect duty (spec 2026-07-07) -----

    fn seed_connect(workspace: &Path) {
        seed_witness(workspace);
        std::fs::write(
            workspace.join("witness/on-connect.md"),
            "Transcript:\n{transcript}\nTarget: {target_path}\n\
             Excerpt: {target_excerpt}\nCompose the connection.",
        )
        .unwrap();
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
    }

    async fn connect_witness(
        workspace: &Path,
        model: std::sync::Arc<FakeModel>,
        memory: crate::memory::Memory,
        threshold: f32,
        min_new_turns: u64,
        self_write_window: u64,
    ) -> (Witness<std::sync::Arc<FakeModel>>, tokio::sync::mpsc::Receiver<crate::turn::ConnectFrame>) {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let witness =
            Witness::load(workspace, model, Some(memory), 0.0, 0, 0, 0)
                .unwrap()
                .with_connect(Some(tx), threshold, min_new_turns, self_write_window);
        (witness, rx)
    }

    #[tokio::test]
    async fn connect_fires_on_high_similarity_hit() {
        let dir = tempfile::tempdir().unwrap();
        seed_connect(dir.path());
        std::fs::write(
            dir.path().join("knowledge/teal.md"),
            "---\nid: NTEAL\n---\n\nteal is a shade between blue and green.\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();

        // Turn transcript uses letters overlapping with the note.
        record_turn(dir.path(), 1, "teal", Some("blue-green"));

        let model = FakeModel::replying(vec![ok("both dwell on the same colour.")]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.001, 0, 0).await;
        witness.connect_for(1).await.unwrap();

        let frame = rx.try_recv().expect("connect frame posted");
        assert_eq!(frame.turn, 1);
        assert_eq!(frame.target_ref, "NTEAL", "atomic frontmatter id used");
        assert!(frame.why.contains("dwell"), "why passed through: {}", frame.why);

        // Receipt landed.
        let log = std::fs::read_to_string(dir.path().join("witness/connect-log.jsonl")).unwrap();
        assert!(log.contains("\"turn\":1"), "connect-log entry:\n{log}");
        assert!(log.contains("NTEAL"), "connect-log target_ref:\n{log}");
    }

    #[tokio::test]
    async fn connect_skips_on_sentinel_without_logging() {
        let dir = tempfile::tempdir().unwrap();
        seed_connect(dir.path());
        std::fs::write(
            dir.path().join("knowledge/teal.md"),
            "---\nid: NTEAL\n---\n\nteal.\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();
        record_turn(dir.path(), 1, "teal", Some("noted"));

        let model = FakeModel::replying(vec![ok("nothing to connect")]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.001, 0, 0).await;
        witness.connect_for(1).await.unwrap();

        assert!(rx.try_recv().is_err(), "sentinel yields no frame");
        assert!(
            !dir.path().join("witness/connect-log.jsonl").exists(),
            "no receipt on sentinel — the log must not describe a phantom frame"
        );
    }

    #[tokio::test]
    async fn connect_below_threshold_never_calls_model() {
        let dir = tempfile::tempdir().unwrap();
        seed_connect(dir.path());
        // Disjoint alphabets — cosine near zero in FakeEmbedder.
        std::fs::write(
            dir.path().join("knowledge/x.md"),
            "---\nid: NX\n---\n\nxxx yyy zzz\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();
        record_turn(dir.path(), 1, "aaa bbb ccc", Some("ddd"));

        // Zero replies queued — a spurious model call would panic.
        let model = FakeModel::replying(vec![]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.5, 0, 0).await;
        witness.connect_for(1).await.unwrap();

        assert!(rx.try_recv().is_err());
        assert!(model.prompts.lock().unwrap().is_empty(), "model must not be called");
    }

    #[tokio::test]
    async fn connect_refractory_blocks_second_fire_without_model_call() {
        let dir = tempfile::tempdir().unwrap();
        seed_connect(dir.path());
        std::fs::write(
            dir.path().join("knowledge/teal.md"),
            "---\nid: NTEAL\n---\n\nteal shade.\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();
        for turn in 1..=8u64 {
            record_turn(dir.path(), turn, &format!("teal {turn}"), Some("noted"));
        }

        // One reply for the first connect. If refractory fails, the
        // second call consumes another reply and panics on the empty
        // queue.
        let model = FakeModel::replying(vec![ok("first connect.")]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.001, 6, 0).await;

        witness.connect_for(1).await.unwrap();
        assert!(rx.try_recv().is_ok(), "first fire");
        // Within refractory (1 → 5 is only 4 turns forward, min is 6).
        witness.connect_for(5).await.unwrap();
        assert!(rx.try_recv().is_err(), "refractory blocks the second fire");
    }

    #[tokio::test]
    async fn connect_self_write_guard_skips_recent_target() {
        let dir = tempfile::tempdir().unwrap();
        seed_connect(dir.path());
        let target_abs = dir.path().join("knowledge/teal.md");
        std::fs::write(
            &target_abs,
            "---\nid: NTEAL\n---\n\nteal shade.\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();

        // The agent wrote the target on turn 1 (recorded as a `write`
        // tool call). Turn 2 is the settled turn we run connect on.
        let mut rec = crate::record::TurnRecord::open(dir.path()).unwrap();
        rec.append_full(
            1,
            "local_main",
            RecordRole::Assistant,
            Some(""),
            Some(vec![crate::model::ToolCall {
                id: "c1".into(),
                name: "write".into(),
                arguments: format!(
                    r#"{{"path":"{}","content":"teal"}}"#,
                    target_abs.display()
                ),
            }]),
            None,
        )
        .unwrap();
        record_turn(dir.path(), 2, "teal", Some("noted"));

        // Zero model replies — a fire would panic. Guard must skip.
        let model = FakeModel::replying(vec![]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.001, 0, 5).await;
        witness.connect_for(2).await.unwrap();
        assert!(rx.try_recv().is_err(), "self-write guard skipped the hit");
    }

    #[tokio::test]
    async fn connect_duty_disabled_without_prompt() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path()); // note: no on-connect.md
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        std::fs::write(
            dir.path().join("knowledge/teal.md"),
            "---\nid: NTEAL\n---\n\nteal.\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();
        record_turn(dir.path(), 1, "teal", Some("noted"));

        let model = FakeModel::replying(vec![]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.001, 0, 0).await;
        witness.connect_for(1).await.unwrap();
        assert!(rx.try_recv().is_err());
        assert!(model.prompts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn connect_skipped_on_digestion_turn_without_model_call() {
        let dir = tempfile::tempdir().unwrap();
        seed_connect(dir.path());
        std::fs::write(
            dir.path().join("knowledge/teal.md"),
            "---\nid: NTEAL\n---\n\nteal.\n",
        )
        .unwrap();
        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();
        record_digestion_turn(dir.path(), 1, "candidate", "rejected");

        let model = FakeModel::replying(vec![]);
        let (mut witness, mut rx) =
            connect_witness(dir.path(), model.clone(), memory, 0.001, 0, 0).await;
        witness.connect_for(1).await.unwrap();
        assert!(rx.try_recv().is_err());
        assert!(model.prompts.lock().unwrap().is_empty());
    }
}
