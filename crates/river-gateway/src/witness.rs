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
const GLEAN_WINDOW_TURNS: u64 = 6;

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
    /// The compose-why prompt loaded from
    /// `witness/flashes/on-connection.md`, or `None` when the file is
    /// absent (Connection type disabled).
    on_connection: Option<String>,
    /// Flash-subsystem state: per-type refractory maps + config +
    /// log path. `None` disables the entire flash pass.
    flashes: Option<crate::flashes::State>,
    /// Bridge's shape-prompt loader. Shared with the shape worker's
    /// loader by convention (both look at
    /// `workspace/witness/on-shape.md`); each holds its own mtime
    /// cache.
    shape_prompt: crate::shape::Prompt,
    /// mpsc sender for flash frames — the seam that preserves the
    /// turn record's single-writer invariant. None when no memory or
    /// no sender was attached (flash pass disabled).
    flash_sender: Option<tokio::sync::mpsc::Sender<crate::flashes::FlashFrame>>,
}

/// One entry in `glean-log.jsonl`: the receipt for a queued candidate.
#[derive(serde::Serialize, serde::Deserialize)]
struct GleanLogEntry {
    id: String,
    turn: u64,
    at: String,
}

/// One entry in `connect-log.jsonl`: the receipt for a fired connect
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

impl<C: Chat + Sync> Witness<C> {
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

        // The flash pass's Connection type composes its "why" via
        // this prompt. Missing file disables Connection specifically;
        // Echo/Return/Bridge use fixed templates and don't need it.
        let on_connection_path = workspace
            .join("witness")
            .join("flashes")
            .join("on-connection.md");
        let on_connection = match std::fs::read_to_string(&on_connection_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    path = %on_connection_path.display(),
                    "witness on-connection prompt missing; Connection flash type disabled"
                );
                None
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", on_connection_path.display()));
            }
        };

        // Bridge glosses the turn transcript via the shape prompt.
        // The shape worker uses its own Prompt instance; this one is
        // held by the witness for the flash pass.
        let shape_prompt = crate::shape::Prompt::at_workspace(workspace);

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
            on_connection,
            flashes: None,
            shape_prompt,
            flash_sender: None,
        })
    }

    /// Enable the flash subsystem: attach the mpsc sender the turn
    /// loop is listening on and initialize per-type refractory state
    /// from `witness/flashes.jsonl`. A `None` sender or `None` config
    /// disables the pass.
    pub fn with_flashes(
        mut self,
        sender: Option<tokio::sync::mpsc::Sender<crate::flashes::FlashFrame>>,
        config: Option<river_core::config::FlashConfig>,
    ) -> anyhow::Result<Self> {
        if let (Some(sender), Some(config)) = (sender, config) {
            self.flash_sender = Some(sender);
            self.flashes = Some(crate::flashes::State::new(config, &self.workspace)?);
        }
        Ok(self)
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

    /// Run until shutdown. Move repair scans the record independently
    /// of the live settled-turn frontier; connect and probabilistic
    /// glean duties run once for each newly settled turn.
    pub async fn run(
        mut self,
        mut latest_turn: watch::Receiver<u64>,
        mut shutdown: watch::Receiver<bool>,
        mut duties_through: u64,
    ) -> anyhow::Result<()> {
        // Startup repairs missing moves, including holes left by hand
        // edits, without replaying probabilistic or connective duties
        // for historical turns.
        if self.on_turn.is_some() {
            let target = *latest_turn.borrow();
            for turn in self.missing_moves(target)? {
                self.move_for(turn).await?;
            }
        }

        loop {
            let stopping = tokio::select! {
                biased;
                _ = shutdown.wait_for(|&stop| stop) => true,
                changed = latest_turn.changed() => changed.is_err(),
            };

            let target = *latest_turn.borrow();
            if self.on_turn.is_some() {
                for turn in self.missing_moves(target)? {
                    self.move_for(turn).await?;
                }
            }
            if target > duties_through {
                for turn in duties_through + 1..=target {
                    // Connect is threshold-gated per settled turn. It
                    // runs before glean so both may fire independently.
                    self.flash_pass_for(turn).await?;
                    // Flat-probability gleaning: the agent cannot
                    // predict which turns get gleaned.
                    if rand::random::<f64>() < self.glean_probability {
                        self.glean(turn).await?;
                    }
                }
                duties_through = target;
            }

            if stopping {
                // The guaranteed end-of-session pass.
                if target > 0 {
                    self.glean(target).await?;
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
        let have: std::collections::HashSet<u64> =
            record::move_turns(self.moves.path())?.into_iter().collect();
        let mut missing = record::turn_numbers_through(
            &self.workspace.join("record").join("turns.jsonl"),
            target,
        )?;
        missing.retain(|turn| !have.contains(turn));
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
        let all_lines = record::scan_turn_range(
            &self.workspace.join("record").join("turns.jsonl"),
            from_turn,
            up_to_turn,
        )?;
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
        let moves = record::read_moves_range(self.moves.path(), from_turn, up_to_turn)?;
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
        let lines = record::scan_turn(
            &self.workspace.join("record").join("turns.jsonl"),
            turn,
        )?;

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

    /// Duty three (the flash pass, spec 2026-07-13): after a turn
    /// settles, run every enabled flash type against the turn
    /// transcript. Frames are sent to the turn loop via
    /// `flash_sender`. Best-effort throughout — any failure logs and
    /// contributes zero frames.
    async fn flash_pass_for(&mut self, turn: u64) -> anyhow::Result<()> {
        let (Some(memory), Some(sender), Some(state)) =
            (self.memory.as_ref(), self.flash_sender.as_ref(), self.flashes.as_mut())
        else {
            return Ok(());
        };

        // Widest lookback the flash pass might need — Connection's
        // self-write window. Read once and reused for the pass.
        let self_write_window = state.config.types.connection.self_write_window;
        let all_lines = record::scan_turn_range(
            &self.workspace.join("record").join("turns.jsonl"),
            turn.saturating_sub(self_write_window),
            turn,
        )?;
        let this_turn_lines: Vec<RecordLine> = all_lines
            .iter()
            .filter(|l| l.turn == turn)
            .cloned()
            .collect();
        if this_turn_lines.is_empty() {
            return Ok(());
        }
        // v2 removes the connect era's digestion + heartbeat
        // exclusions; both turn types carry substance the flash pass
        // should see.

        let transcript = format_transcript(&this_turn_lines);
        let channel = this_turn_lines[0].channel.clone();
        let recent_writes =
            collect_recent_agent_writes(&all_lines, turn, self_write_window, &self.workspace);

        // Bridge glosses through this Prompt; missing on-shape.md
        // returns None, disabling Bridge silently.
        let shape_prompt_loaded = match self.shape_prompt.load() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(turn, error = %e, "shape prompt load failed; Bridge disabled this pass");
                None
            }
        };

        let ctx = crate::flashes::FlashPassCtx {
            turn,
            channel,
            transcript,
            memory,
            workspace: &self.workspace,
            client: &self.client,
            identity: &self.identity,
            state,
            on_connection: self.on_connection.as_deref(),
            shape_prompt: shape_prompt_loaded.as_ref(),
            recent_agent_writes: recent_writes,
            sender,
        };
        crate::flashes::flash_pass(ctx).await
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
    // Build a tool-result index once: id → content (JSON string).
    // Needed for `write_atomic`, which generates its path inside the
    // tool and returns it in the result JSON rather than accepting it
    // as an argument.
    let tool_results: std::collections::HashMap<&str, &str> = all_lines
        .iter()
        .filter(|l| l.role == RecordRole::Tool && l.turn >= from && l.turn <= turn)
        .filter_map(|l| {
            let id = l.tool_call_id.as_deref()?;
            let content = l.content.as_deref()?;
            Some((id, content))
        })
        .collect();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_relative = |path_str: &str| {
        let candidate = std::path::PathBuf::from(path_str);
        let abs = if candidate.is_absolute() {
            candidate
        } else {
            workspace.join(candidate)
        };
        if !out.contains(&abs) {
            out.push(abs);
        }
    };
    for line in all_lines.iter().filter(|l| l.turn >= from && l.turn <= turn) {
        if line.role != RecordRole::Assistant {
            continue;
        }
        for call in line.tool_calls.iter().flatten() {
            match call.name.as_str() {
                "write" | "edit" | "create_moment" => {
                    // Path is in the args for these three.
                    let Ok(args) = serde_json::from_str::<serde_json::Value>(&call.arguments)
                    else {
                        continue;
                    };
                    if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                        push_relative(p);
                    }
                    // create_moment writes under record/moments/{id}.md
                    // — but record/ is not indexed (wall ch. 10), so
                    // it never matches a hit anyway.
                }
                "write_atomic" => {
                    // Path is in the tool RESULT, not args. Look up
                    // by call id.
                    let Some(result) = tool_results.get(call.id.as_str()) else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(result) else {
                        continue;
                    };
                    if let Some(p) = value.get("path").and_then(|v| v.as_str()) {
                        push_relative(p);
                    }
                }
                _ => continue,
            }
        }
    }
    out
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
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx, 0));
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
    async fn run_schedules_glean_without_move_prompt() {
        // Post-flash-v2 shape: gleaning fires on newly settled turns
        // even when on-turn.md is missing. The old test also
        // asserted a connect frame arrived; the flash pass now
        // shipping in its own module handles that responsibility.
        let dir = tempfile::tempdir().unwrap();
        let witness_dir = dir.path().join("witness");
        std::fs::create_dir_all(&witness_dir).unwrap();
        std::fs::write(witness_dir.join("identity.md"), "You are the witness.").unwrap();
        std::fs::write(
            witness_dir.join("on-glean.md"),
            "Recent:\n{recent_record}\nGlean.",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        record_turn(dir.path(), 1, "teal", Some("blue-green"));

        let memory = fresh_memory(dir.path());
        memory.sweep().await.unwrap();
        let model = FakeModel::replying(vec![ok("Teal matters to Cass — worth remembering.")]);
        let witness = Witness::load(
            dir.path(),
            model.clone(),
            Some(memory.clone()),
            1.0,
            12,
            5,
            0,
        )
        .unwrap();

        let (latest_tx, latest_rx) = watch::channel(0u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx, 0));
        latest_tx.send(1).unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if memory.queue_depth().unwrap() == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("glean must run without on-turn.md");
        shutdown_tx.send(true).unwrap();
        handle.await.unwrap().unwrap();

        assert!(
            read_moves(&record::moves_path(dir.path()))
                .unwrap()
                .is_empty()
        );
        assert_eq!(model.prompts.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn startup_move_repair_does_not_replay_settled_duties() {
        let dir = tempfile::tempdir().unwrap();
        seed_glean(dir.path());
        record_turn(dir.path(), 1, "teal", Some("noted"));
        let memory = fresh_memory(dir.path());
        let model = FakeModel::replying(vec![ok("move one"), ok("nothing to glean")]);
        let witness = Witness::load(dir.path(), model.clone(), Some(memory), 1.0, 0, 0, 0).unwrap();
        let moves_path = witness.moves.path().to_path_buf();

        let (latest_tx, latest_rx) = watch::channel(1u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx, 1));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while read_moves(&moves_path).unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("startup move repair completes");
        shutdown_tx.send(true).unwrap();
        drop(latest_tx);
        handle.await.unwrap().unwrap();

        let prompts = model.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2, "one repaired move plus final glean only");
        assert!(prompts[0].1.contains("Write the move."));
        assert!(prompts[1].1.contains("Glean."));
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
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx, 3));
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
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx, 4));
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
        let handle = tokio::spawn(witness.run(latest_rx, shutdown_rx, 3));

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

    // -------- hunt tests --------
    //
    // Each names the drift it catches. Per CLAUDE.md, no test that
    // cannot fail on a plausible regression.

    // --- collect_recent_agent_writes ---

    fn assistant_line(
        turn: u64,
        call_name: &str,
        call_id: &str,
        args: serde_json::Value,
    ) -> RecordLine {
        RecordLine {
            id: format!("assist-{turn}-{call_id}"),
            turn,
            channel: "local_main".into(),
            role: RecordRole::Assistant,
            content: None,
            tool_calls: Some(vec![crate::model::ToolCall {
                id: call_id.into(),
                name: call_name.into(),
                arguments: args.to_string(),
            }]),
            tool_call_id: None,
        }
    }

    fn tool_line(turn: u64, call_id: &str, content: &str) -> RecordLine {
        RecordLine {
            id: format!("tool-{turn}-{call_id}"),
            turn,
            channel: "local_main".into(),
            role: RecordRole::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }

    /// Hunts: someone removes the `if window == 0` early return,
    /// causing a wasted scan that would also emit spurious writes
    /// from turn 0 (saturating_sub against unsigned).
    #[test]
    fn collect_recent_agent_writes_window_zero_returns_empty() {
        let ws = tempfile::tempdir().unwrap();
        let lines = vec![assistant_line(
            5,
            "write",
            "c1",
            serde_json::json!({"path": "foo.md"}),
        )];
        let out = collect_recent_agent_writes(&lines, 5, 0, ws.path());
        assert!(out.is_empty(), "window=0 must short-circuit");
    }

    /// Hunts: someone drops the dedupe check and Connection's guard
    /// gets an inflated recent_writes list — mostly harmless but a
    /// signal the list is being built naively.
    #[test]
    fn collect_recent_agent_writes_dedupes_duplicate_paths() {
        let ws = tempfile::tempdir().unwrap();
        let lines = vec![
            assistant_line(5, "write", "c1", serde_json::json!({"path": "foo.md"})),
            assistant_line(5, "edit", "c2", serde_json::json!({"path": "foo.md"})),
            assistant_line(6, "write", "c3", serde_json::json!({"path": "foo.md"})),
        ];
        let out = collect_recent_agent_writes(&lines, 6, 5, ws.path());
        assert_eq!(out.len(), 1, "same path across three calls counted once: {out:?}");
        assert_eq!(out[0], ws.path().join("foo.md"));
    }

    /// Hunts: someone widens the tool-name match set and starts
    /// treating `read`/`bash` as writes, or narrows it and drops one
    /// of the three legitimate write tools.
    #[test]
    fn collect_recent_agent_writes_ignores_non_write_tools() {
        let ws = tempfile::tempdir().unwrap();
        let lines = vec![
            assistant_line(5, "read", "c1", serde_json::json!({"path": "a.md"})),
            assistant_line(5, "bash", "c2", serde_json::json!({"command": "ls"})),
            assistant_line(5, "search", "c3", serde_json::json!({"query": "x"})),
            assistant_line(5, "write", "c4", serde_json::json!({"path": "kept.md"})),
        ];
        let out = collect_recent_agent_writes(&lines, 5, 5, ws.path());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], ws.path().join("kept.md"));
    }

    /// Hunts: the fix for write_atomic's self-write gap regressing.
    /// write_atomic generates its path inside the tool and returns it
    /// in the tool RESULT (not args), so the guard has to consult the
    /// matching Tool-role line by call id.
    #[test]
    fn collect_recent_agent_writes_reads_write_atomic_path_from_result() {
        let ws = tempfile::tempdir().unwrap();
        let lines = vec![
            assistant_line(
                5,
                "write_atomic",
                "call_A",
                serde_json::json!({"body": "a claim", "links": []}),
            ),
            tool_line(
                5,
                "call_A",
                &serde_json::json!({
                    "id": "01ATOM",
                    "path": "knowledge/01ATOM.md",
                    "warnings": []
                })
                .to_string(),
            ),
        ];
        let out = collect_recent_agent_writes(&lines, 5, 5, ws.path());
        assert_eq!(
            out.len(),
            1,
            "write_atomic path must be extracted from tool result"
        );
        assert_eq!(out[0], ws.path().join("knowledge/01ATOM.md"));
    }

    /// Hunts: a write_atomic call whose tool line is missing (torn
    /// mid-write, or the pair straddles the window boundary and got
    /// filtered). The guard should silently drop the case, not panic.
    #[test]
    fn collect_recent_agent_writes_write_atomic_without_result_skips() {
        let ws = tempfile::tempdir().unwrap();
        let lines = vec![assistant_line(
            5,
            "write_atomic",
            "call_A",
            serde_json::json!({"body": "a"}),
        )];
        let out = collect_recent_agent_writes(&lines, 5, 5, ws.path());
        assert!(out.is_empty(), "no result line → no path");
    }

    // --- is_digestion_turn / is_heartbeat_turn ---

    fn user_line(turn: u64, content: &str) -> RecordLine {
        RecordLine {
            id: format!("u-{turn}"),
            turn,
            channel: "local_main".into(),
            role: RecordRole::User,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn system_line(turn: u64, content: &str) -> RecordLine {
        RecordLine {
            id: format!("s-{turn}"),
            turn,
            channel: "local_main".into(),
            role: RecordRole::System,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Hunts: someone loosens is_digestion_turn to treat any turn
    /// with a `[digestion]` marker as digestion-driven, missing the
    /// "conservative" contract: a real user message on the same turn
    /// disqualifies (hybrid turns must not be skipped).
    #[test]
    fn is_digestion_turn_user_line_disqualifies_even_with_marker() {
        let lines = vec![
            system_line(5, &format!("{DIGESTION_MARKER} candidate x")),
            user_line(5, "[local_main] cass: interrupt"),
        ];
        assert!(!is_digestion_turn(&lines, 5));
    }

    /// Hunts: someone drops the "any other system frame disqualifies"
    /// branch. A hybrid turn (digestion + budget warning, or
    /// digestion + mid-turn arrival notice) must be treated as real
    /// activity, not skipped from glean.
    #[test]
    fn is_digestion_turn_other_system_frame_disqualifies() {
        let lines = vec![
            system_line(5, &format!("{DIGESTION_MARKER} candidate x")),
            system_line(5, "[3/10 tool calls remaining]"),
        ];
        assert!(!is_digestion_turn(&lines, 5));
    }

    /// Hunts: symmetry with is_digestion_turn — a heartbeat turn is
    /// only heartbeat-driven if the sole user line is the exact
    /// HEARTBEAT_MARKER content. Any real user message disqualifies.
    #[test]
    fn is_heartbeat_turn_non_marker_user_line_disqualifies() {
        let lines = vec![
            user_line(5, HEARTBEAT_MARKER),
            user_line(5, "[local_main] cass: hi"),
        ];
        assert!(!is_heartbeat_turn(&lines, 5));
    }

    /// Hunts: someone lets system frames slip past the heartbeat
    /// classifier. Any system frame on a turn means "not a bare
    /// heartbeat" — the glean should not skip.
    #[test]
    fn is_heartbeat_turn_system_frame_disqualifies() {
        let lines = vec![
            user_line(5, HEARTBEAT_MARKER),
            system_line(5, "compaction happened"),
        ];
        assert!(!is_heartbeat_turn(&lines, 5));
    }

    // --- preview_line ---

    /// Hunts: someone switches `chars().count()` to `.len()` (bytes)
    /// and multi-byte previews truncate on wrong boundaries or count
    /// wrong. Also hunts the 80-char cap flipping to a smaller value
    /// silently.
    #[test]
    fn preview_line_truncation_boundary_and_utf8() {
        // Under cap: whole text, no ellipsis.
        let short = "a".repeat(80);
        assert_eq!(preview_line(&short), short);
        // At cap boundary (81 chars): truncates + ellipsis.
        let long = "a".repeat(81);
        let out = preview_line(&long);
        assert_eq!(out.chars().count(), 81, "80 chars + ellipsis");
        assert!(out.ends_with('…'));
        // Multi-byte: 100 chars of a 3-byte codepoint.
        let wide: String = "字".repeat(100);
        let out = preview_line(&wide);
        assert_eq!(out.chars().count(), 81, "80 chars + ellipsis");
        assert!(out.chars().take(80).all(|c| c == '字'));
        // Multi-line: takes only the first line, trimmed.
        let multi = "  first line  \nsecond line\nthird";
        assert_eq!(preview_line(multi), "first line");
    }

    // --- recent_rejections ---

    /// Hunts: someone changes the tail-take from `skip(len - window)`
    /// to `take(window)` (head-take) — silently returning the OLDEST
    /// rejections instead of the most recent, so the glean prompt
    /// warns about ancient turned-down candidates.
    #[test]
    fn recent_rejections_returns_last_window_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("witness")).unwrap();
        let path = dir.path().join("witness/rejections.jsonl");
        // Write 5 rejections; request window=3 → should get turns 3, 4, 5.
        let mut body = String::new();
        for t in 1..=5 {
            body.push_str(&format!(
                "{{\"candidate_id\":\"01T{t}\",\"candidate\":\"c{t}\",\"turn\":{t},\"at\":\"t\"}}\n",
            ));
        }
        std::fs::write(&path, body).unwrap();
        let entries = recent_rejections(&path, 3);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].turn, 3);
        assert_eq!(entries[1].turn, 4);
        assert_eq!(entries[2].turn, 5);
    }

    /// Hunts: someone changes the malformed-line handler from
    /// warn+skip to bail, breaking every glean after a torn write to
    /// rejections.jsonl.
    #[test]
    fn recent_rejections_skips_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("witness")).unwrap();
        let path = dir.path().join("witness/rejections.jsonl");
        let mut body = String::new();
        body.push_str(
            "{\"candidate_id\":\"01A\",\"candidate\":\"a\",\"turn\":1,\"at\":\"t\"}\n",
        );
        body.push_str("{ this is not valid json\n");
        body.push_str(
            "{\"candidate_id\":\"01B\",\"candidate\":\"b\",\"turn\":2,\"at\":\"t\"}\n",
        );
        body.push_str("\n"); // blank line (already handled)
        body.push_str("garbage\n");
        body.push_str(
            "{\"candidate_id\":\"01C\",\"candidate\":\"c\",\"turn\":3,\"at\":\"t\"}\n",
        );
        std::fs::write(&path, body).unwrap();
        let entries = recent_rejections(&path, 10);
        assert_eq!(entries.len(), 3, "three valid entries survive junk");
        let turns: Vec<_> = entries.iter().map(|e| e.turn).collect();
        assert_eq!(turns, vec![1, 2, 3]);
    }

    // --- format_rejections ---

    /// Hunts: someone changes the format so the "reason:" suffix is
    /// dropped, or emits it even when None. Prompt shape matters —
    /// the witness's on-glean template reads this block.
    #[test]
    fn format_rejections_renders_reason_when_present_and_omits_when_absent() {
        let with_reason = RejectionEntry {
            candidate_id: "01A".into(),
            candidate: "a claim".into(),
            reason: Some("too vague".into()),
            turn: 3,
            at: "t".into(),
        };
        let without = RejectionEntry {
            candidate_id: "01B".into(),
            candidate: "another".into(),
            reason: None,
            turn: 7,
            at: "t".into(),
        };
        let out = format_rejections(&[with_reason, without]);
        assert!(out.contains("turn 3:"));
        assert!(out.contains("a claim"));
        assert!(out.contains("reason: too vague"), "reason renders: {out}");
        assert!(out.contains("turn 7:"));
        assert!(out.contains("another"));
        // The no-reason line must NOT carry the em-dash suffix.
        let line7 = out.lines().find(|l| l.contains("turn 7")).unwrap();
        assert!(
            !line7.contains(" — "),
            "no-reason line has no reason suffix: {line7}"
        );
    }
}
