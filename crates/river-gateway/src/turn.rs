//! The turn cycle (wall ch. 01) over the channel layer (ch. 05) and
//! the persistent context (ch. 03). Wake on a notification pointer or
//! the heartbeat; drain everything pending; read each notified
//! channel from its cursor; persist each message at context-append
//! time (persist-once); compact if needed; call the model; reply;
//! settle with cursors to every channel read.
//!
//! Turns are serial; numbers are monotonic for life; every turn
//! settles; shutdown is observed only between turns.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch};

use std::collections::HashMap;

use crate::channels::{ChannelEntry, Channels, Notification};
use river_core::config::ContextConfig;
use crate::context::PersistentContext;
use crate::identity;
use crate::model::{Chat, ToolCall};
use crate::record::{RecordRole, TurnRecord, last_turn};
use crate::tools::{Registry, ToolContext};

/// The retired marker remains recognizable in old records, but new heartbeat
/// turns carry a dynamic `[workspace]` landscape instead.
pub const LEGACY_HEARTBEAT_MARKER: &str = "Read HEARTBEAT.md.";
pub const DEFAULT_CHANNEL: &str = "local_main";
pub const LOCAL_ADAPTER: &str = "local";
/// Marker prefix for the system frame the connect duty appends when
/// it surfaces a workspace-note connection on a settled turn. Stable
/// so tests, transcript formatting, and any future arc-layer reader
/// can key on it (wall ch. 04 + the flash spec 2026-07-13). Legacy
/// `[connect]` lines predating the flash pass remain in the record
/// and are still recognized by tools that scan for it.
#[allow(dead_code)]
pub const CONNECT_MARKER: &str = "[connect]";
/// The prefix flash frames use on the record's system-role line
/// (per-type: `[flash: connection]`, `[flash: echo]`, etc). Kept as
/// a const so scanners can key on the common prefix.
#[allow(dead_code)]
pub const FLASH_MARKER_PREFIX: &str = "[flash:";
/// Re-exported so callers can pattern-match `FlashType` without a
/// second `use` line.
pub use crate::flashes::FlashFrame;

#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub channel: String,
    pub content: String,
}

/// Live-path state for /health (wall chs. 06, 09): written by the
/// turn loop itself at settle, read by the local surface.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Health {
    pub turn_number: u64,
    pub last_settle: Option<String>,
    pub context_messages: usize,
    pub context_percent: u64,
    /// Agent turn minus witness cursor (wall ch. 09).
    pub witness_lag: u64,
    pub queue_depth: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeCause {
    Heartbeat,
    ChannelMessage,
    DigestionEvent,
}

enum Wake {
    Notifications(Vec<Notification>),
    Heartbeat,
    /// A quiet-trigger digestion turn carrying one extraction
    /// candidate (wall ch. 02). The ULID is the queue row id;
    /// the reject tool needs both to write an attributable entry
    /// to workspace/witness/rejections.jsonl.
    Digestion { id: String, candidate: String },
    /// The queue changed while parked: recompute the select arms.
    Recheck,
    Shutdown,
}

impl Wake {
    fn cause(&self) -> Option<WakeCause> {
        match self {
            Self::Notifications(_) => Some(WakeCause::ChannelMessage),
            Self::Heartbeat => Some(WakeCause::Heartbeat),
            Self::Digestion { .. } => Some(WakeCause::DigestionEvent),
            Self::Recheck | Self::Shutdown => None,
        }
    }
}

const QUIET_TRIGGER: Duration = Duration::from_secs(300);
/// How many turns a flash stays visible in the memory slot.
const FLASH_VISIBLE_TURNS: u8 = 3;
/// Marker prefix for the system framing of a digestion turn. Stable
/// because the witness uses it to exclude digestion turns from its
/// glean window (wall ch. 04) — without that filter, the witness
/// gleans over its own gleanings and the abstraction climbs without
/// bound.
pub const DIGESTION_MARKER: &str = "[digestion]";

pub struct TurnLoop<C: Chat> {
    workspace: PathBuf,
    tz: jiff::tz::TimeZone,
    knobs: ContextConfig,
    client: C,
    channels: Channels,
    context: PersistentContext,
    record: TurnRecord,
    turn_number: u64,
    notifications: mpsc::Receiver<Notification>,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Sender<Health>,
    /// The live window for GET /context: published at settle, read by
    /// the local surface. A window, never a hand.
    snapshot: watch::Sender<crate::context::ContextSnapshot>,
    /// The latest settled turn — the witness's wake signal. Sent only
    /// after the turn's lines are durably in the record
    /// (persist-before-announce, wall ch. 01).
    settled: watch::Sender<u64>,
    heartbeat: Duration,
    registry: Registry,
    profile: Vec<String>,
    scrub: Vec<String>,
    max_iterations: u32,
    memory: Option<crate::memory::Memory>,
    reindex: Option<mpsc::Sender<()>>,
    discord: Option<mpsc::Sender<crate::discord::SpeakRequest>>,
    /// The channel a turn is currently working, for presence
    /// signals (discord typing); None between turns.
    working: watch::Sender<Option<String>>,
    /// When the loop last did something meaningful — an inbound
    /// notification *or* a completed digestion turn. The quiet gate
    /// measures from here so digestions cannot fire back-to-back over
    /// a queue the witness is itself filling. Heartbeats do not reset
    /// it; they are internal scaffolding.
    last_significant_at: std::time::Instant,
    /// In-memory read positions (channel → last consumed entry id).
    /// Authoritative within the process; the log cursor recovers the
    /// position across restarts. Without this, an agent entry written
    /// mid-turn (speak) would swallow arrivals that landed before it.
    positions: HashMap<String, String>,
    /// Flashes currently riding the memory slot, each with its
    /// remaining visible turns.
    active_flashes: Vec<(crate::memory::Flash, u8)>,
    /// Raised by the `compact` tool; honored at the next turn start as
    /// a force-compaction (in addition to the threshold-based trigger).
    compact_requested: Arc<AtomicBool>,
    /// Written by `SettleTool::execute`; drained at end-of-turn to
    /// recompute [`Self::next_heartbeat_at`]. When set, the current
    /// turn ends after the pending batch of tool calls resolves.
    settle_intent: Arc<std::sync::Mutex<Option<crate::tools::SettleIntent>>>,
    /// When the next heartbeat wake should fire. Deadline-based (wall
    /// ch. 01, settle-tool amendment): only an explicit `settle` call
    /// recomputes it; every other wake (channel, digestion, natural
    /// end-of-turn) leaves it alone.
    next_heartbeat_at: std::time::Instant,
    /// Raised by `create_moment`; checked after every tool round to
    /// refresh the arc so a just-written moment is visible in the
    /// next model call instead of waiting on compaction.
    arc_dirty: Arc<AtomicBool>,
    /// One-shot high-water warning state. True between the moment the
    /// estimate first crosses 0.9 × compaction_threshold and the moment
    /// it dips below again. Without the latch the warning would fire
    /// every turn while the witness is at all behind.
    warned_high: bool,
    /// Inbound queue of connect frames from the witness's connect duty.
    /// Drained at each wake; each frame appends a `[connect]` system
    /// line to the referenced turn's record and (if the turn is still
    /// in HOT) synthesises the same line into the live window.
    /// `None` when no witness/memory is configured.
    flash_frames: Option<mpsc::Receiver<FlashFrame>>,
    atomic_max_words: usize,
    shape_queue: Option<mpsc::Sender<crate::shape::GlossJob>>,
    /// Cause of the turn currently in flight. None between turns.
    current_wake_cause: Option<WakeCause>,
}

impl<C: Chat> TurnLoop<C> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace: PathBuf,
        tz: jiff::tz::TimeZone,
        knobs: ContextConfig,
        client: C,
        channels: Channels,
        notifications: mpsc::Receiver<Notification>,
        outbound: broadcast::Sender<OutboundMessage>,
        health: watch::Sender<Health>,
        snapshot: watch::Sender<crate::context::ContextSnapshot>,
        settled: watch::Sender<u64>,
        heartbeat: Duration,
        registry: Registry,
        profile: Vec<String>,
        scrub: Vec<String>,
        max_iterations: u32,
        memory: Option<crate::memory::Memory>,
        reindex: Option<mpsc::Sender<()>>,
        discord: Option<mpsc::Sender<crate::discord::SpeakRequest>>,
        working: watch::Sender<Option<String>>,
        resume: Option<crate::session::SessionSnapshot>,
        flash_frames: Option<mpsc::Receiver<FlashFrame>>,
        atomic_max_words: usize,
        shape_queue: Option<mpsc::Sender<crate::shape::GlossJob>>,
    ) -> anyhow::Result<Self> {
        let mut record = TurnRecord::open(&workspace)?;
        // Monotonic for life: resume from the record (wall ch. 01).
        let mut turn_number = last_turn(record.path())?;
        let system_prompt = fresh_system_prompt(&workspace, &tz)?;

        // Pick the resume channel: session.json wins, then the
        // record's tail (where iris was actually talking), then
        // DEFAULT_CHANNEL for first-session cold starts.
        let channel = match &resume {
            Some(snap) => snap.channel.clone(),
            None => crate::session::channel_from_record_tail(&workspace)
                .unwrap_or_else(|| DEFAULT_CHANNEL.to_string()),
        };

        // Cross-session handoff (wall ch. 03): the previous session's
        // `compact` tool left a message in `workspace/handoff.md`.
        // Consume it once — append as a system-role record line under
        // the next turn number, then delete the file so it doesn't
        // surface again. The line becomes a permanent part of the
        // record; the next live turn will see it in hot.
        let handoff = crate::tools::handoff_path(&workspace);
        if handoff.is_file() {
            match std::fs::read_to_string(&handoff) {
                Ok(text) if !text.trim().is_empty() => {
                    let handoff_turn = turn_number.saturating_add(1);
                    let body = format!(
                        "[handoff from previous session]\n{}",
                        text.trim_end()
                    );
                    record.append(handoff_turn, &channel, RecordRole::System, Some(&body))?;
                    turn_number = handoff_turn;
                    if let Err(e) = std::fs::remove_file(&handoff) {
                        tracing::warn!(path = %handoff.display(), error = %e, "removing consumed handoff");
                    }
                    tracing::info!(turn = handoff_turn, channel = %channel, "handoff appended");
                }
                Ok(_) => {
                    tracing::warn!(path = %handoff.display(), "empty handoff; discarding");
                    let _ = std::fs::remove_file(&handoff);
                }
                Err(e) => {
                    tracing::warn!(path = %handoff.display(), error = %e, "reading handoff");
                }
            }
        }

        let mut context =
            PersistentContext::build(&workspace, &channel, system_prompt, knobs.clone())?;

        let (last_significant_at, active_flashes) = match resume {
            Some(snap) => {
                context.set_estimator_ratio(snap.estimator_ratio);
                let last_significant_at = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(snap.quiet_seconds))
                    .unwrap_or_else(std::time::Instant::now);
                let active = snap
                    .active_flashes
                    .into_iter()
                    .map(crate::session::FlashSnapshot::into_active)
                    .collect();
                (last_significant_at, active)
            }
            None => (std::time::Instant::now(), Vec::new()),
        };

        Ok(Self {
            workspace,
            tz,
            knobs,
            client,
            channels,
            context,
            record,
            turn_number,
            notifications,
            outbound,
            health,
            snapshot,
            settled,
            heartbeat,
            registry,
            profile,
            scrub,
            max_iterations,
            memory,
            reindex,
            discord,
            working,
            last_significant_at,
            positions: HashMap::new(),
            active_flashes,
            compact_requested: Arc::new(AtomicBool::new(false)),
            arc_dirty: Arc::new(AtomicBool::new(false)),
            settle_intent: Arc::new(std::sync::Mutex::new(None)),
            next_heartbeat_at: std::time::Instant::now() + heartbeat,
            warned_high: false,
            flash_frames,
            atomic_max_words,
            shape_queue,
            current_wake_cause: None,
        })
    }

    /// Read a channel forward from this process's position (falling
    /// back to the log cursor), advancing the position.
    fn consume(&mut self, channel: &str) -> anyhow::Result<Vec<ChannelEntry>> {
        let entries = match self.positions.get(channel) {
            Some(last_id) => {
                let all = self.channels.scan(channel)?;
                let position = all.iter().position(|e| &e.id == last_id);
                match position {
                    Some(pos) => all[pos + 1..]
                        .iter()
                        .filter(|e| e.role == crate::channels::EntryRole::Other)
                        .cloned()
                        .collect(),
                    None => self.channels.read_since_cursor(channel)?,
                }
            }
            None => self.channels.read_since_cursor(channel)?,
        };
        if let Some(last) = entries.last() {
            self.positions
                .insert(channel.to_string(), last.id.clone());
        }
        Ok(entries)
    }

    /// Run until shutdown flips true. Each iteration is one turn;
    /// shutdown is only observed between turns, so a turn in progress
    /// always runs to settle.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        loop {
            // The quiet trigger: when candidates wait, sleep out the
            // remaining silence; when none do, wait for the queue to
            // gain one (then recompute). Never polls.
            let quiet_in: Option<Duration> = match &self.memory {
                Some(memory) if memory.queue_depth().unwrap_or(0) > 0 => {
                    Some(QUIET_TRIGGER.saturating_sub(self.last_significant_at.elapsed()))
                }
                Some(_) => None, // armed on queue_wait below
                None => None,
            };
            let memory = self.memory.clone();

            let wake = tokio::select! {
                biased;
                _ = shutdown.wait_for(|&stop| stop) => Wake::Shutdown,
                note = self.notifications.recv() => match note {
                    Some(first) => {
                        let mut batch = vec![first];
                        while let Ok(more) = self.notifications.try_recv() {
                            batch.push(more);
                        }
                        Wake::Notifications(batch)
                    }
                    None => Wake::Shutdown,
                },
                wake = async {
                    match (&memory, quiet_in) {
                        (Some(_), Some(wait)) => {
                            tokio::time::sleep(wait).await;
                            Wake::Recheck // depth re-checked below
                        }
                        (Some(m), None) => {
                            m.queue_wait().await;
                            Wake::Recheck
                        }
                        (None, _) => std::future::pending().await,
                    }
                } => {
                    if matches!(quiet_in, Some(_)) {
                        match memory.as_ref().and_then(|m| m.pop_candidate().ok().flatten()) {
                            Some((id, candidate)) => Wake::Digestion { id, candidate },
                            None => Wake::Recheck,
                        }
                    } else {
                        wake
                    }
                }
                _ = tokio::time::sleep_until(self.next_heartbeat_at.into()) => Wake::Heartbeat,
            };

            match wake {
                Wake::Shutdown => {
                    tracing::info!("shutdown: no turn in flight, exiting cleanly");
                    return Ok(());
                }
                Wake::Recheck => continue,
                wake => self.turn(wake).await?,
            }
        }
    }

    async fn turn(&mut self, wake: Wake) -> anyhow::Result<()> {
        // Drain any pending connect frames from the witness before the
        // context assembly for this turn. Each frame appends a
        // [connect] system line to the referenced turn's record and
        // (if the turn is still in HOT) synthesises the same line into
        // the live window so the model sees it on the very next call.
        self.drain_flash_frames();

        self.turn_number += 1;
        let n = self.turn_number;
        let wake_cause = wake.cause().expect("turn called only for a real wake");
        self.current_wake_cause = Some(wake_cause);
        // Channels read this turn, with the last entry id consumed —
        // the settle cursor points at it (never past it).
        let mut read_channels: Vec<(String, String)> = Vec::new();

        // Flash delivery (wall ch. 02, amended): a flash rides the
        // memory slot for FLASH_VISIBLE_TURNS turns, then fades. A
        // re-flash while visible refreshes the countdown.
        if let Some(memory) = &self.memory {
            for flash in memory.take_flashes() {
                match self
                    .active_flashes
                    .iter_mut()
                    .find(|(f, _)| f.note_id == flash.note_id)
                {
                    Some((existing, remaining)) => {
                        *existing = flash;
                        *remaining = FLASH_VISIBLE_TURNS;
                    }
                    None => self.active_flashes.push((flash, FLASH_VISIBLE_TURNS)),
                }
            }
            let mut slot = String::new();
            for (flash, _) in &self.active_flashes {
                slot.push_str(&format!("[flash] {}: {}\n", flash.note_id, flash.text));
                for (link_type, neighbor) in &flash.neighbors {
                    slot.push_str(&format!("  {link_type} → {neighbor}\n"));
                }
            }
            self.context.set_memory_slot(slot);
            for (_, remaining) in &mut self.active_flashes {
                *remaining -= 1;
            }
            self.active_flashes.retain(|(_, remaining)| *remaining > 0);
        }

        let mut digestion: Option<crate::tools::DigestionInfo> = None;
        match wake {
            Wake::Notifications(batch) => {
                // Dedup channels, preserving arrival order.
                let mut notified: Vec<String> = Vec::new();
                for note in &batch {
                    if !notified.contains(&note.channel) {
                        notified.push(note.channel.clone());
                    }
                }

                self.last_significant_at = std::time::Instant::now();
                // Channel switch, deferred to turn start (wall ch. 03):
                // the first notified channel is where attention goes.
                if notified[0] != self.context.channel() {
                    let system_prompt = fresh_system_prompt(&self.workspace, &self.tz)?;
                    self.context = PersistentContext::build(
                        &self.workspace,
                        &notified[0],
                        system_prompt,
                        self.knobs.clone(),
                    )?;
                }

                for channel in notified {
                    let entries = self.consume(&channel)?;
                    for entry in &entries {
                        if let Some(content) = &entry.content {
                            let author = entry.author.as_deref().unwrap_or("unknown");
                            let formatted =
                                format_inbound(&channel, author, content, &entry.attachments);
                            self.append(n, RecordRole::User, &formatted)?;
                        }
                    }
                    if let Some(last) = entries.last() {
                        read_channels.push((channel, last.id.clone()));
                    }
                }
            }
            Wake::Heartbeat => {}
            Wake::Digestion { id, candidate } => {
                digestion = Some(crate::tools::DigestionInfo {
                    candidate_id: id,
                    candidate_text: candidate.clone(),
                    turn: n,
                });
                // A digestion turn is itself activity. Reset the
                // quiet gate so the next candidate waits a full
                // QUIET_TRIGGER before firing, regardless of queue
                // depth — the witness may be queueing in real time,
                // and without this reset every queued candidate would
                // fire back-to-back the moment `last_significant_at`
                // first crossed the threshold (river's bug report).
                self.last_significant_at = std::time::Instant::now();
                // System role, not user: the framing is the harness
                // speaking. A candidate is the agent's own past — as a
                // user message, conversational candidates read as
                // someone talking *now*, and the agent answers people
                // who are not there.
                let framing = format!(
                    "{DIGESTION_MARKER} A quiet moment. Your witness gleaned this from your \
                     recent activity — it is your own memory passing through \
                     digestion, not a message from anyone. No one has spoken; no one \
                     is waiting on a reply.\n\n\
                     {candidate}\n\n\
                     Re-engage it: re-read what it cites if you need to, then either \
                     write a fresh atomic note in knowledge/ with the write tool — one \
                     claim, at most ~100 words, your own words, typed links in the \
                     frontmatter (id, links) — or reject the candidate, saying briefly \
                     why. Never copy the witness's phrasing."
                );
                self.append(n, RecordRole::System, &framing)?;
            }
            Wake::Shutdown | Wake::Recheck => unreachable!("handled by run"),
        }

        let forced = self.compact_requested.swap(false, Ordering::Relaxed);
        if self.context.needs_compaction() || forced {
            let system_prompt = fresh_system_prompt(&self.workspace, &self.tz)?;
            let lag_warning = self
                .context
                .compact(&self.workspace, system_prompt, n)?;
            if let Some(warning) = lag_warning {
                self.append(n, RecordRole::System, &warning)?;
            }
            if forced {
                tracing::info!(turn = n, "compaction forced by compact tool");
            }
        }

        // High-water nudge: if the estimate is past 0.9 × the
        // compaction threshold without having tripped it (or
        // compaction couldn't free much because the witness is
        // behind), tell the agent once per crossing so it can wind
        // down on its own terms with `compact` instead of being
        // surprised by a forced compaction.
        let high_water = self.knobs.compaction_threshold * 0.9 * self.knobs.limit as f64;
        let total = self.context.estimate_total();
        if total < high_water {
            self.warned_high = false;
        } else if !self.warned_high {
            let pct = (total / self.knobs.limit as f64 * 100.0).round() as u64;
            let notice = format!(
                "[system] Context is at {pct}% of the limit and approaching compaction. \
                 If you want to wind down on your own terms, the `compact` tool takes a \
                 handoff summary; otherwise compaction will run automatically when the \
                 threshold trips."
            );
            self.append(n, RecordRole::System, &notice)?;
            self.warned_high = true;
        }

        // The heartbeat landscape is assembled last so it is the freshest
        // message at the first model call. It is strictly cause-gated: no
        // channel or digestion wake can manufacture this synthetic user.
        if wake_cause == WakeCause::Heartbeat {
            let state_path = self.workspace.join("state/landscape-generator.json");
            if let Some(prompt) = crate::wake_prompt::generate(&self.workspace, &state_path)? {
                let framed = format!("{}\n\n{prompt}", crate::wake_prompt::WORKSPACE_PREFIX);
                self.append(n, RecordRole::User, &framed)?;
            }
        }

        // Presence: the agent is doing something on this channel.
        let _ = self.working.send(Some(self.context.channel().to_string()));

        // THINK / ACT (wall ch. 01): bounded by max_iterations.
        let schemas = self.registry.schemas(&self.profile);
        let tool_ctx = ToolContext {
            workspace: self.workspace.clone(),
            channels: self.channels.clone(),
            outbound: self.outbound.clone(),
            current_channel: self.context.channel().to_string(),
            current_turn: n,
            scrub: self.scrub.clone(),
            memory: self.memory.clone(),
            reindex: self.reindex.clone(),
            discord: self.discord.clone(),
            digestion,
            compact_requested: self.compact_requested.clone(),
            arc_dirty: self.arc_dirty.clone(),
            atomic_max_words: self.atomic_max_words,
            shape_queue: self.shape_queue.clone(),
            heartbeat_default_minutes: self.heartbeat.as_secs() / 60,
            settle_intent: self.settle_intent.clone(),
        };

        // Budget warning fires in the last 20% of the turn's tool
        // rounds (integer ceil), so the agent can choose to wind down
        // — speak, summarize, or end — instead of piling on tools
        // whose results it will not see. The notice goes through
        // `append`: durable in the record and immediately visible to
        // the next model call via hot.
        let warn_window = (self.max_iterations + 4) / 5;
        for iteration in 0..self.max_iterations {
            let remaining = self.max_iterations - iteration;
            if remaining <= warn_window {
                let notice = format!(
                    "[{remaining}/{} tool calls remaining]",
                    self.max_iterations
                );
                self.append(n, RecordRole::System, &notice)?;
            }
            let (system, messages) = self.context.messages();
            // Debug: dump the exact prompt about to go up the wire.
            // Overwritten each call; the last live prompt is always at
            // `workspace/last_prompt.txt` for inspection.
            {
                let path = self.workspace.join("last_prompt.txt");
                let mut dump = String::new();
                dump.push_str("=== SYSTEM ===\n");
                dump.push_str(&system);
                dump.push_str("\n\n=== MESSAGES ===\n");
                for m in &messages {
                    dump.push_str(&format!("[{:?}] {}\n", m.role, m.content));
                }
                if let Err(e) = std::fs::write(&path, dump) {
                    tracing::debug!(error = %e, "last_prompt dump failed");
                }
            }
            let response = match self.client.chat(&system, &messages, &schemas).await {
                Ok(response) => response,
                Err(e) => {
                    // Every turn settles: a failed model call ends the
                    // turn; everything persisted before it is safe.
                    tracing::warn!(turn = n, iteration, error = %e, "model call failed; turn ends");
                    break;
                }
            };
            self.context.calibrate(response.prompt_tokens);
            self.append_assistant(n, &response.content, response.tool_calls.clone())?;

            if response.tool_calls.is_empty() {
                break; // the turn is over
            }
            for call in &response.tool_calls {
                let result = self
                    .registry
                    .execute(call, &self.profile, &tool_ctx)
                    .await;
                self.append_tool_result(n, &call.id, &result)?;
                // Tool resonance (wall ch. 02): what passes through
                // the agent's hands warms what it resembles;
                // fire-and-forget, never blocks the act loop.
                if let Some(memory) = &self.memory {
                    let m = memory.clone();
                    tokio::spawn(async move {
                        if let Err(e) = m.resonate_tool(&result).await {
                            tracing::debug!(error = %e, "tool resonance failed");
                        }
                    });
                }
            }

            // A `create_moment` call raises arc_dirty; refresh the arc
            // before the next model call so the just-written moment is
            // visible without waiting on compaction.
            if self.arc_dirty.swap(false, Ordering::Relaxed) {
                if let Err(e) = self.context.refresh_arc(&self.workspace) {
                    tracing::warn!(error = %e, "arc refresh after moment write failed");
                }
            }

            // Mid-turn arrivals fold into the current turn as one
            // system notice; their channels get cursors at settle.
            let mut arrived: Vec<String> = Vec::new();
            while let Ok(note) = self.notifications.try_recv() {
                if !arrived.contains(&note.channel) {
                    arrived.push(note.channel.clone());
                }
            }
            if !arrived.is_empty() {
                // Inbound resets the quiet timer from zero (wall ch. 01).
                self.last_significant_at = std::time::Instant::now();
                let mut notice = String::from("[arrived mid-turn]");
                let mut anything_new = false;
                for channel in &arrived {
                    let entries = self.consume(channel)?;
                    for entry in &entries {
                        if let Some(content) = &entry.content {
                            let author = entry.author.as_deref().unwrap_or("unknown");
                            notice.push('\n');
                            notice.push_str(&format_inbound(
                                channel,
                                author,
                                content,
                                &entry.attachments,
                            ));
                            anything_new = true;
                        }
                    }
                    if let Some(last) = entries.last() {
                        match read_channels.iter_mut().find(|(c, _)| c == channel) {
                            Some((_, last_id)) => *last_id = last.id.clone(),
                            None => read_channels.push((channel.clone(), last.id.clone())),
                        }
                    }
                }
                if anything_new {
                    self.append(n, RecordRole::System, &notice)?;
                }
            }

            // The agent called `settle` in this batch; the turn ends
            // after the batch resolves (wall ch. 01, settle amendment).
            // The intent is drained and applied to `next_heartbeat_at`
            // below, in the settle path.
            if self.settle_intent.lock().unwrap().is_some() {
                break;
            }

            if iteration + 1 == self.max_iterations {
                tracing::warn!(turn = n, "iteration ceiling hit; turn ends");
            }
        }

        // Apply any pending settle intent to the heartbeat deadline.
        // Bare settle recomputes to now + config default; NextHeartbeat
        // uses the (already-clamped) minutes the agent chose. Natural
        // end-of-turn leaves the deadline alone.
        if let Some(intent) = self.settle_intent.lock().unwrap().take() {
            let minutes = match intent {
                crate::tools::SettleIntent::Bare => self.heartbeat.as_secs() / 60,
                crate::tools::SettleIntent::NextHeartbeat(m) => m,
            };
            self.next_heartbeat_at =
                std::time::Instant::now() + Duration::from_secs(minutes * 60);
        }

        // SETTLE: a cursor to every channel read this turn, pointing
        // at the last entry consumed. Speaking already covers a
        // channel (the agent entry is the implicit cursor); entries
        // that arrived unread stay unread.
        for (channel, last_id) in &read_channels {
            if !self.channels.covered(channel, last_id)? {
                self.channels.mark_read(channel, last_id)?;
            }
        }
        // Conversation resonance (wall ch. 02): the turn's own text
        // warms the nearest notes; fire-and-forget, never blocks.
        if let Some(memory) = &self.memory {
            let m = memory.clone();
            let text = self.context.turn_text(n);
            tokio::spawn(async move {
                if let Err(e) = m.resonate(&text).await {
                    tracing::debug!(error = %e, "resonance failed");
                }
            });
        }

        let witness_cursor =
            crate::record::witness_cursor(&crate::record::moves_path(&self.workspace))
                .unwrap_or(0);
        let _ = self.health.send(Health {
            turn_number: n,
            last_settle: Some(jiff::Timestamp::now().to_string()),
            context_messages: self.context.len(),
            context_percent: (self.context.estimate_total() / self.knobs.limit as f64 * 100.0)
                as u64,
            witness_lag: n.saturating_sub(witness_cursor),
            queue_depth: self
                .memory
                .as_ref()
                .and_then(|m| m.queue_depth().ok())
                .unwrap_or(0),
        });
        let _ = self.snapshot.send(self.context.snapshot(n));
        let _ = self.working.send(None);
        if let Err(e) = self.write_session() {
            tracing::warn!(turn = n, error = %e, "session.json write failed");
        }
        // Persist-before-announce: every append above fsynced inline,
        // so the record already holds the whole turn.
        let _ = self.settled.send(n);
        self.current_wake_cause = None;
        tracing::debug!(turn = n, "settled");
        Ok(())
    }

    /// Snapshot the ephemeral context state for the next session
    /// (wall ch. 03 — session resume). Written atomically each settle;
    /// missing or torn on next startup falls back to derivation.
    fn write_session(&self) -> anyhow::Result<()> {
        let snap = crate::session::SessionSnapshot::new(
            self.context.channel().to_string(),
            self.turn_number,
            self.context.estimator_ratio(),
            &self.active_flashes,
            self.last_significant_at.elapsed().as_secs(),
        );
        snap.write_atomic(&self.workspace.join("session.json"))
    }

    /// Persist-once: context append and record append are one act,
    /// under the turn number and the channel the turn is facing.
    fn append(&mut self, turn: u64, role: RecordRole, content: &str) -> anyhow::Result<()> {
        let channel = self.context.channel().to_string();
        self.context.append(turn, role, content);
        self.record.append(turn, &channel, role, Some(content))?;
        Ok(())
    }

    fn append_assistant(
        &mut self,
        turn: u64,
        content: &str,
        tool_calls: Vec<ToolCall>,
    ) -> anyhow::Result<()> {
        let channel = self.context.channel().to_string();
        self.context.append_full(
            turn,
            RecordRole::Assistant,
            content,
            tool_calls.clone(),
            None,
        );
        self.record.append_full(
            turn,
            &channel,
            RecordRole::Assistant,
            Some(content),
            (!tool_calls.is_empty()).then_some(tool_calls),
            None,
        )?;
        Ok(())
    }

    fn append_tool_result(&mut self, turn: u64, call_id: &str, result: &str) -> anyhow::Result<()> {
        let channel = self.context.channel().to_string();
        self.context.append_full(
            turn,
            RecordRole::Tool,
            result,
            Vec::new(),
            Some(call_id.to_string()),
        );
        self.record.append_full(
            turn,
            &channel,
            RecordRole::Tool,
            Some(result),
            None,
            Some(call_id.to_string()),
        )?;
        Ok(())
    }

    /// Non-blocking drain of the flash-frame receiver. Each frame's
    /// body is pre-formatted by the per-type module in flashes.rs
    /// (target head appended, capped per wall ch. 02); this just
    /// appends it as a system-role line on the referenced turn's
    /// record and (when the referenced turn is still in HOT)
    /// synthesises the same line into the live window.
    fn drain_flash_frames(&mut self) {
        let Some(rx) = self.flash_frames.as_mut() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(frame) => {
                    let content = frame.body.clone();
                    if let Err(e) = self.record.append_full(
                        frame.turn,
                        &frame.channel,
                        RecordRole::System,
                        Some(&content),
                        None,
                        None,
                    ) {
                        tracing::warn!(
                            turn = frame.turn,
                            error = %e,
                            "flash frame record-append failed"
                        );
                        continue;
                    }
                    if self.context.contains_turn(frame.turn) {
                        self.context.append(frame.turn, RecordRole::System, content);
                    }
                    tracing::info!(
                        turn = frame.turn,
                        target = %frame.target_ref,
                        flash_type = frame.flash_type.as_str(),
                        "flash frame landed"
                    );
                }
                Err(mpsc::error::TryRecvError::Empty) => return,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.flash_frames = None;
                    return;
                }
            }
        }
    }
}

// A note is *atomic* iff it lives under `knowledge/` in the workspace
// **and** its file starts with YAML frontmatter carrying an `id`.
// Kept as a private helper only used by the (now-removed) tests below;
// left here in case a future scanner wants the same check.
#[allow(dead_code)]
fn is_atomic_note(path: &Path, workspace: &Path, text: &str) -> bool {
    let Ok(rel) = path.strip_prefix(workspace) else {
        return false;
    };
    if rel.components().next().and_then(|c| c.as_os_str().to_str()) != Some("knowledge") {
        return false;
    }
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            return false;
        }
        if line.starts_with("id:") {
            return true;
        }
    }
    false
}

/// Render an inbound channel entry for the model: the text, then a
/// metadata line per attachment so the agent can choose to open it
/// with the file tools (or knows it existed when it can't).
pub(crate) fn format_inbound(
    channel: &str,
    author: &str,
    content: &str,
    attachments: &[crate::channels::Attachment],
) -> String {
    let mut out = format!("[{channel}] {author}: {content}");
    for att in attachments {
        out.push('\n');
        out.push_str(&format_attachment(att));
    }
    out
}

pub(crate) fn format_attachment(att: &crate::channels::Attachment) -> String {
    use crate::channels::SkippedReason;
    let where_ = match (&att.path, att.skipped) {
        (Some(path), _) => format!("path={path}"),
        (None, Some(SkippedReason::TooLarge)) => "skipped=too_large".to_string(),
        (None, Some(SkippedReason::DownloadFailed)) => "skipped=download_failed".to_string(),
        (None, None) => "skipped=unknown".to_string(),
    };
    format!(
        "  [attachment: {} ({}, {} bytes) {where_}]",
        att.filename, att.mime, att.size,
    )
}

pub fn fresh_system_prompt(
    workspace: &PathBuf,
    tz: &jiff::tz::TimeZone,
) -> anyhow::Result<String> {
    let identity = identity::load(workspace)?;
    Ok(identity.system_prompt(&jiff::Zoned::now().with_time_zone(tz.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatMessage, ChatResponse};
    use crate::record::scan;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    struct FakeModel {
        replies: Mutex<Vec<anyhow::Result<ChatResponse>>>,
        seen: Mutex<Vec<(String, Vec<ChatMessage>)>>,
        /// Injected into the channel on the first chat call —
        /// simulates a message landing mid-turn.
        inject: Mutex<Option<(Channels, String)>>,
    }

    impl FakeModel {
        fn replying(replies: Vec<anyhow::Result<ChatResponse>>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies),
                seen: Mutex::new(Vec::new()),
                inject: Mutex::new(None),
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
            self.seen
                .lock()
                .unwrap()
                .push((system.to_string(), messages.to_vec()));
            let inject = self.inject.lock().unwrap().take();
            if let Some((channels, content)) = inject {
                channels
                    .inbound(DEFAULT_CHANNEL, "cass", None, &content, LOCAL_ADAPTER, None)
                    .await
                    .unwrap();
            }
            self.replies.lock().unwrap().remove(0)
        }
    }

    fn done(content: &str) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            content: content.into(),
            tool_calls: Vec::new(),
            prompt_tokens: Some(50),
        })
    }

    fn create_moment(
        turn_start: u64,
        turn_end: u64,
        body: &str,
    ) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "call_moment".into(),
                name: "create_moment".into(),
                arguments: serde_json::json!({
                    "turn_start": turn_start,
                    "turn_end": turn_end,
                    "body": body
                })
                .to_string(),
            }],
            prompt_tokens: Some(50),
        })
    }

    fn speak(content: &str) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "call_speak".into(),
                name: "speak".into(),
                arguments: serde_json::json!({ "content": content }).to_string(),
            }],
            prompt_tokens: Some(50),
        })
    }

    fn write_identity(dir: &Path) {
        std::fs::write(dir.join("AGENTS.md"), "operate honestly").unwrap();
        std::fs::write(dir.join("IDENTITY.md"), "i am a test agent").unwrap();
        std::fs::write(dir.join("RULES.md"), "be brief").unwrap();
    }

    struct Harness {
        turn_loop: TurnLoop<Arc<FakeModel>>,
        channels: Channels,
        notify_rx_drained: mpsc::Receiver<Notification>,
        outbound: broadcast::Receiver<OutboundMessage>,
        health: watch::Receiver<Health>,
        snapshot: watch::Receiver<crate::context::ContextSnapshot>,
    }

    fn harness(dir: &Path, model: Arc<FakeModel>) -> Harness {
        harness_with(dir, model, None, ContextConfig::default())
    }

    fn harness_with_resume(
        dir: &Path,
        model: Arc<FakeModel>,
        resume: Option<crate::session::SessionSnapshot>,
    ) -> Harness {
        harness_with(dir, model, resume, ContextConfig::default())
    }

    fn harness_with(
        dir: &Path,
        model: Arc<FakeModel>,
        resume: Option<crate::session::SessionSnapshot>,
        knobs: ContextConfig,
    ) -> Harness {
        write_identity(dir);
        let (notify_tx, notify_rx) = mpsc::channel(256);
        let channels = Channels::open(dir, notify_tx).unwrap();
        let (outbound_tx, outbound_rx) = broadcast::channel(64);
        let (health_tx, health_rx) = watch::channel(Health::default());
        let (snapshot_tx, snapshot_rx) =
            watch::channel(crate::context::ContextSnapshot::default());
        let (settled_tx, _settled_rx) = watch::channel(0u64);
        let turn_loop = TurnLoop::new(
            dir.to_path_buf(),
            jiff::tz::TimeZone::UTC,
            knobs,
            model,
            channels.clone(),
            notify_rx,
            outbound_tx,
            health_tx,
            snapshot_tx,
            settled_tx,
            Duration::from_secs(3600),
            Registry::core(),
            river_core::config::DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect(),
            vec![],
            10,
            None,
            None,
            None,
            watch::channel(None).0,
            resume,
            None,
            100,
            None,
        )
        .unwrap();
        Harness {
            turn_loop,
            channels,
            notify_rx_drained: mpsc::channel(1).1,
            outbound: outbound_rx,
            health: health_rx,
            snapshot: snapshot_rx,
        }
    }

    async fn say(h: &mut Harness, channel: &str, content: &str) -> Notification {
        let ulid = h
            .channels
            .inbound(channel, "cass", None, content, LOCAL_ADAPTER, None)
            .await
            .unwrap();
        // Drain the queue the loop would have drained.
        let note = h.turn_loop.notifications.try_recv().unwrap();
        assert_eq!(note.ulid, ulid);
        note
    }

    #[tokio::test]
    async fn message_turn_speaks_persists_and_cursors() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![speak("good morning"), done("")]);
        let mut h = harness(dir.path(), model.clone());

        let note = say(&mut h, DEFAULT_CHANNEL, "hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        // The speak tool delivered and logged post-acceptance.
        let out = h.outbound.try_recv().unwrap();
        assert_eq!(out.content, "good morning");

        // Record: user, assistant(tool call), tool result, final
        // assistant — all turn 1, channel-tagged.
        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l.turn == 1));
        assert!(lines.iter().all(|l| l.channel == DEFAULT_CHANNEL));
        assert_eq!(lines[0].content.as_deref(), Some("[local_main] cass: hello"));
        assert_eq!(
            lines[1].tool_calls.as_ref().unwrap()[0].name,
            "speak"
        );
        assert_eq!(lines[2].role, RecordRole::Tool);
        assert_eq!(lines[2].tool_call_id.as_deref(), Some("call_speak"));

        // Cursor honest: speaking was the implicit cursor.
        assert!(h.channels.read_since_cursor(DEFAULT_CHANNEL).unwrap().is_empty());
        let entries = h.channels.scan(DEFAULT_CHANNEL).unwrap();
        assert_eq!(entries.len(), 2); // inbound + agent message, no extra cursor
        assert_eq!(entries[1].content.as_deref(), Some("good morning"));

        // The model saw identity in the system string and the speak
        // schema among its tools.
        let seen = model.seen.lock().unwrap();
        assert!(seen[0].0.contains("i am a test agent"));
        assert_eq!(seen.len(), 2, "tool iteration then final");
        assert_eq!(h.health.borrow().turn_number, 1);

        // The context snapshot published at settle (GET /context).
        let snap = h.snapshot.borrow().clone();
        assert_eq!(snap.turn_number, 1);
        assert_eq!(snap.channel, DEFAULT_CHANNEL);
        assert!(snap.hot_messages >= 4);
        assert_eq!(snap.hot_first_turn, Some(1));
        assert!(snap.system_tokens > 0.0);
        assert!(snap.estimate_total > 0.0);
    }

    #[tokio::test]
    async fn mid_turn_arrival_folds_into_the_current_turn() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![speak("one moment"), done("")]);
        let mut h = harness(dir.path(), model.clone());
        // The injected message lands during the first model call.
        *model.inject.lock().unwrap() =
            Some((h.channels.clone(), "wait, also this!".to_string()));

        let note = say(&mut h, DEFAULT_CHANNEL, "hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert!(lines.iter().all(|l| l.turn == 1), "one turn holds it all");
        let notice = lines
            .iter()
            .find(|l| l.role == RecordRole::System)
            .expect("system notice");
        assert!(notice.content.as_ref().unwrap().contains("[arrived mid-turn]"));
        assert!(notice.content.as_ref().unwrap().contains("wait, also this!"));

        // The second model call saw the notice.
        let seen = model.seen.lock().unwrap();
        assert!(seen[1].1.iter().any(|m| m.content.contains("wait, also this!")));

        // Settle cursored the fold: nothing unread.
        assert!(h.channels.read_since_cursor(DEFAULT_CHANNEL).unwrap().is_empty());
    }

    #[tokio::test]
    async fn model_failure_settles_with_explicit_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![Err(anyhow::anyhow!("api down"))]);
        let mut h = harness(dir.path(), model);
        let _ = &h.notify_rx_drained;

        let note = say(&mut h, DEFAULT_CHANNEL, "hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        // No reply, but the user line is persisted and the channel got
        // an explicit read-cursor at settle.
        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.len(), 1);
        let entries = h.channels.scan(DEFAULT_CHANNEL).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[1].cursor, "read-without-speak writes a cursor");
        assert!(h.channels.read_since_cursor(DEFAULT_CHANNEL).unwrap().is_empty());
        assert_eq!(h.health.borrow().turn_number, 1, "settle ran");
    }

    #[tokio::test]
    async fn two_channels_drain_into_one_turn_with_honest_cursors() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![speak("heard both"), done("")]);
        let mut h = harness(dir.path(), model);

        let n1 = say(&mut h, DEFAULT_CHANNEL, "from local").await;
        let n2 = say(&mut h, "discord_general", "from discord").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n1, n2]))
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.len(), 5); // two user + assistant + tool + final
        assert!(lines.iter().all(|l| l.turn == 1), "one wake, one turn");

        // Spoke in local (implicit cursor); discord read-only (explicit).
        assert!(h.channels.read_since_cursor(DEFAULT_CHANNEL).unwrap().is_empty());
        assert!(h.channels.read_since_cursor("discord_general").unwrap().is_empty());
        let discord = h.channels.scan("discord_general").unwrap();
        assert!(discord.last().unwrap().cursor);
    }

    #[tokio::test]
    async fn heartbeat_injects_workspace_landscape_last() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![done("quiet hour")]);
        let seen = model.clone();
        let mut h = harness(dir.path(), model);

        h.turn_loop.turn(Wake::Heartbeat).await.unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        let wake = lines[0].content.as_deref().unwrap();
        assert!(wake.starts_with("[workspace]\n\nYou last settled"), "{wake}");
        assert!(wake.ends_with(crate::wake_prompt::CLOSING), "{wake}");
        assert_eq!(lines[0].role, RecordRole::User);
        let calls = seen.seen.lock().unwrap();
        let messages = &calls[0].1;
        assert_eq!(messages.last().unwrap().role, crate::model::Role::User);
        assert!(messages.last().unwrap().content.starts_with("[workspace]"));
        assert_eq!(h.turn_loop.current_wake_cause, None, "cause clears at settle");
    }

    #[tokio::test]
    async fn channel_wake_never_injects_workspace_landscape() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![done("hello")]);
        let seen = model.clone();
        let mut h = harness(dir.path(), model);
        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;

        h.turn_loop.turn(Wake::Notifications(vec![note])).await.unwrap();

        let calls = seen.seen.lock().unwrap();
        assert!(calls[0].1.iter().all(|message| !message.content.starts_with("[workspace]")));
        assert!(!dir.path().join("state/landscape-generator.json").exists());
    }

    #[tokio::test]
    async fn digestion_turn_frames_the_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![done("rejected: too thin to keep")]);
        let mut h = harness(dir.path(), model);

        h.turn_loop
            .turn(Wake::Digestion { id: "01JTEST".into(), candidate: "the agent kept circling teal".into() })
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        let framing = lines[0].content.as_ref().unwrap();
        assert!(framing.contains("[digestion]"), "{framing}");
        assert!(framing.contains("circling teal"));
        assert!(framing.contains("or reject"), "rejection right named");
        assert!(
            framing.contains("not a message from anyone"),
            "candidates must not read as live conversation"
        );
        assert_eq!(
            lines[0].role,
            RecordRole::System,
            "the framing is the harness speaking, not a person"
        );
        assert_eq!(h.health.borrow().turn_number, 1, "a real turn, settled");
    }

    #[tokio::test]
    async fn budget_warning_fires_in_last_twenty_percent() {
        // max_iterations defaults to 10 in the harness; warn_window =
        // ceil(10 * 0.20) = 2. The agent should see `[2/10]` then
        // `[1/10]` as System frames in the last two rounds, so it
        // can choose to wind down instead of piling on tools whose
        // results it will not see.
        let dir = tempfile::tempdir().unwrap();
        // 10 speaks chained — enough to drive the loop to its ceiling.
        let mut replies = Vec::with_capacity(10);
        for i in 0..10 {
            replies.push(speak(&format!("round {i}")));
        }
        let model = FakeModel::replying(replies);
        let mut h = harness(dir.path(), model);
        let _outbound_keeper = h.outbound.resubscribe();

        let note = say(&mut h, DEFAULT_CHANNEL, "go").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        let budget: Vec<String> = lines
            .iter()
            .filter(|l| l.role == RecordRole::System)
            .filter_map(|l| l.content.as_deref())
            .filter(|c| c.contains("tool calls remaining"))
            .map(|c| c.to_string())
            .collect();
        assert_eq!(
            budget,
            vec![
                "[2/10 tool calls remaining]".to_string(),
                "[1/10 tool calls remaining]".to_string(),
            ],
            "budget warning fires at the last 20% — twice for max=10",
        );
    }

    #[tokio::test]
    async fn budget_warning_silent_when_turn_ends_early() {
        // A normal turn — one speak, then done — never crosses the
        // threshold; no budget frames should appear.
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![speak("brief reply"), done("")]);
        let mut h = harness(dir.path(), model);
        let _outbound_keeper = h.outbound.resubscribe();

        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        let any_budget = lines.iter().any(|l| {
            l.content
                .as_deref()
                .is_some_and(|c| c.contains("tool calls remaining"))
        });
        assert!(!any_budget, "no budget frames on a short turn");
    }

    #[tokio::test]
    async fn digestion_resets_the_quiet_gate() {
        // River's bug: once `last_significant_at` first crossed
        // QUIET_TRIGGER, every queued candidate fired back-to-back —
        // because the gate only reset on inbound, not on digestion.
        // After a digestion turn, the gate must read fresh.
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![done("rejected")]);
        let mut h = harness(dir.path(), model);

        // Pretend a long silence has elapsed: the gate is fully open.
        h.turn_loop.last_significant_at =
            std::time::Instant::now() - std::time::Duration::from_secs(1000);
        h.turn_loop
            .turn(Wake::Digestion { id: "01JTEST".into(), candidate: "the agent kept circling teal".into() })
            .await
            .unwrap();

        assert!(
            h.turn_loop.last_significant_at.elapsed() < std::time::Duration::from_secs(5),
            "digestion must reset the quiet gate so the next candidate \
             waits a full QUIET_TRIGGER",
        );
    }

    #[tokio::test]
    async fn pending_flash_rides_the_memory_slot_for_three_turns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        std::fs::write(
            dir.path().join("knowledge/owl.md"),
            "---\nid: NOWL\n---\n\nthe owl is silent",
        )
        .unwrap();
        let memory = crate::memory::Memory::open(
            &dir.path().join("data"),
            dir.path(),
            &[],
            std::sync::Arc::new(crate::memory::tests::FakeEmbedder),
        )
        .unwrap();
        memory.bump("NOWL", 1.2, crate::memory::Carrier::Ambient).unwrap();

        let model = FakeModel::replying(vec![
            done("noticed"),
            done("still here"),
            done("fading"),
            done("quiet"),
        ]);
        let mut h = harness(dir.path(), model.clone());
        h.turn_loop.memory = Some(memory);

        // Visible for exactly FLASH_VISIBLE_TURNS turns, then gone.
        for _ in 0..4 {
            h.turn_loop.turn(Wake::Heartbeat).await.unwrap();
        }
        let seen = model.seen.lock().unwrap();
        for turn in 0..3 {
            assert!(
                seen[turn].0.contains("[flash] NOWL"),
                "flash visible on turn {turn}"
            );
            assert!(seen[turn].0.contains("owl is silent"));
        }
        assert!(!seen[3].0.contains("[flash]"), "slot cleared on turn 4");
    }

    #[tokio::test]
    async fn restart_resumes_numbering_and_rebuilds_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        {
            let model = FakeModel::replying(vec![done("first life")]);
            let mut h = harness(dir.path(), model);
            let note = say(&mut h, DEFAULT_CHANNEL, "hello").await;
            h.turn_loop
                .turn(Wake::Notifications(vec![note]))
                .await
                .unwrap();
        }
        let lines_before = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines_before.len(), 2);

        // Second life: same workspace, fresh process.
        let model = FakeModel::replying(vec![done("second life")]);
        let mut h = harness(dir.path(), model.clone());

        // Rebuild duplicated nothing into the record.
        let lines_after_build = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines_after_build.len(), 2, "nothing duplicated by rebuild");

        let note = say(&mut h, DEFAULT_CHANNEL, "are you still there?").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.last().unwrap().turn, 2, "numbering resumed");

        // The rebuilt context carried the first life's exchange in.
        let seen = model.seen.lock().unwrap();
        let hot = &seen[0].1;
        assert!(hot.iter().any(|m| m.content.contains("hello")));
        assert!(hot.iter().any(|m| m.content == "first life"));
        assert!(
            hot.iter().any(|m| m.content.contains("still there")),
            "and the new message"
        );
    }

    #[tokio::test]
    async fn create_moment_refreshes_arc_before_next_model_call() {
        let dir = tempfile::tempdir().unwrap();
        // Seed moves for turns 1-3 so the witness has compressed them.
        // Cursor will be 3; turns 1-3 are out of hot and a moment over
        // [1, 2] is eligible to appear in arc.
        let moves_dir = dir.path().join("record");
        std::fs::create_dir_all(&moves_dir).unwrap();
        let moves: String = (1..=3u64)
            .map(|t| {
                serde_json::json!({
                    "id": ulid::Ulid::new().to_string(),
                    "turn": t,
                    "summary": format!("witness move {t}")
                })
                .to_string()
                    + "\n"
            })
            .collect();
        std::fs::write(moves_dir.join("moves.jsonl"), moves).unwrap();
        // Seed user turns for those numbers so turn numbering picks up
        // at 4 next.
        {
            let mut rec = crate::record::TurnRecord::open(dir.path()).unwrap();
            for t in 1..=3u64 {
                rec.append(t, DEFAULT_CHANNEL, RecordRole::User, Some("seed"))
                    .unwrap();
            }
        }

        // Two model rounds: first call returns create_moment, second
        // ends the turn. After the moment is written the turn loop
        // should refresh the arc, so the second model call sees a
        // system string that contains the moment block.
        let model = FakeModel::replying(vec![
            create_moment(1, 2, "what I made of the first stretch"),
            done(""),
        ]);
        let mut h = harness(dir.path(), model.clone());

        let note = say(&mut h, DEFAULT_CHANNEL, "another one").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        let seen = model.seen.lock().unwrap();
        let second_system = &seen[1].0;
        assert!(
            second_system.contains("what I made of the first stretch"),
            "arc should include the just-written moment by the next \
             model call, not wait on compaction. Got system:\n{second_system}"
        );
        assert!(
            second_system.contains("[turns 1–2,"),
            "moment header present: {second_system}"
        );
    }

    #[tokio::test]
    async fn high_water_warning_fires_once_per_crossing() {
        let dir = tempfile::tempdir().unwrap();
        // Small limit + heavy message so the first turn's estimate
        // lands above 0.9 × threshold (0.9 × 0.8 = 0.72 of limit).
        let knobs = ContextConfig {
            limit: 400,
            compaction_threshold: 0.80,
            fill_target: 0.40,
            min_messages: 1,
        };
        let model = FakeModel::replying(vec![done("first"), done("second")]);
        let mut h = harness_with(dir.path(), model, None, knobs);

        // ~1000 chars × ratio 1.0 → ~250+ tokens, comfortably above
        // 0.72 × 400 = 288 once the system prompt rides too.
        let heavy = "x".repeat(1100);
        let note = say(&mut h, DEFAULT_CHANNEL, &heavy).await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        let count_warnings = |path: &Path| -> usize {
            scan(path)
                .unwrap()
                .iter()
                .filter(|l| {
                    l.role == RecordRole::System
                        && l.content
                            .as_deref()
                            .map(|c| c.contains("approaching compaction"))
                            .unwrap_or(false)
                })
                .count()
        };
        let record_path = dir.path().join("record/turns.jsonl");
        assert_eq!(
            count_warnings(&record_path),
            1,
            "warning fires once on first crossing"
        );

        // Second turn while still above the line: latched, silent.
        let note = say(&mut h, DEFAULT_CHANNEL, "more").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        assert_eq!(
            count_warnings(&record_path),
            1,
            "latched: no repeat while still high"
        );
    }

    #[tokio::test]
    async fn handoff_file_lands_as_first_system_line_then_is_consumed() {
        let dir = tempfile::tempdir().unwrap();
        // Seed a prior turn so turn numbering has a real tail.
        {
            let model = FakeModel::replying(vec![done("earlier life")]);
            let mut h = harness(dir.path(), model);
            let note = say(&mut h, DEFAULT_CHANNEL, "earlier").await;
            h.turn_loop
                .turn(Wake::Notifications(vec![note]))
                .await
                .unwrap();
        }

        // The previous session's `compact` tool wrote a handoff.
        std::fs::write(
            dir.path().join("handoff.md"),
            "left off mid-thread on the labor question",
        )
        .unwrap();

        // Next session boots; the handoff appears as a system record
        // line under the next turn number, and the file is consumed.
        let model = FakeModel::replying(vec![done("morning")]);
        let mut h = harness(dir.path(), model.clone());
        assert!(
            !dir.path().join("handoff.md").exists(),
            "handoff should be deleted on consumption"
        );
        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        let handoff = lines
            .iter()
            .find(|l| {
                l.role == RecordRole::System
                    && l.content
                        .as_deref()
                        .map(|c| c.contains("handoff from previous session"))
                        .unwrap_or(false)
            })
            .expect("handoff record line present");
        assert_eq!(handoff.turn, 2, "handoff sits at last_turn + 1");
        assert!(handoff.content.as_deref().unwrap().contains("labor question"));

        // The next live turn picks up *after* the handoff (turn 3) and
        // the handoff rode into hot for the new session.
        let note = say(&mut h, DEFAULT_CHANNEL, "morning").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.last().unwrap().turn, 3, "live turn is handoff + 1");
        let seen = model.seen.lock().unwrap();
        let hot = &seen[0].1;
        assert!(
            hot.iter().any(|m| m
                .content
                .contains("left off mid-thread on the labor question")),
            "handoff visible in next session's hot"
        );
    }

    #[tokio::test]
    async fn channel_switch_rebuilds_at_turn_start() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![
            speak("hi local"),
            done(""),
            speak("hi front porch"),
            done(""),
        ]);
        let mut h = harness(dir.path(), model.clone());

        let n1 = say(&mut h, DEFAULT_CHANNEL, "local hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n1]))
            .await
            .unwrap();

        let n2 = say(&mut h, "porch_main", "porch hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n2]))
            .await
            .unwrap();

        assert_eq!(h.turn_loop.context.channel(), "porch_main");
        // The reply was spoken (and cursored) on the new channel.
        let porch = h.channels.scan("porch_main").unwrap();
        assert_eq!(porch.last().unwrap().content.as_deref(), Some("hi front porch"));
    }

    #[tokio::test]
    async fn settle_writes_session_json_with_current_channel_and_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![speak("hi"), done("")]);
        let mut h = harness(dir.path(), model.clone());
        let n1 = say(&mut h, "porch_main", "ping").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n1]))
            .await
            .unwrap();

        let snap = crate::session::load(&dir.path().join("session.json"))
            .expect("session.json should be written at settle");
        assert_eq!(snap.channel, "porch_main");
        assert_eq!(snap.version, 1);
        assert!(snap.turn_number >= 1);
    }

    #[tokio::test]
    async fn resume_picks_channel_from_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        // Seed a record so build() has something to scan.
        crate::record::TurnRecord::open(dir.path())
            .unwrap()
            .append(
                1,
                "porch_main",
                crate::record::RecordRole::User,
                Some("[porch_main] cass: hi"),
            )
            .unwrap();
        let snap = crate::session::SessionSnapshot::new(
            "porch_main".into(),
            1,
            0.88,
            &[],
            10,
        );
        snap.write_atomic(&dir.path().join("session.json")).unwrap();

        let model = FakeModel::replying(vec![]);
        let h = harness_with_resume(dir.path(), model, Some(snap));
        assert_eq!(h.turn_loop.context.channel(), "porch_main");
        assert!(
            (h.turn_loop.context.estimator_ratio() - 0.88).abs() < 1e-9,
            "estimator ratio restored"
        );
    }

    #[tokio::test]
    async fn resume_derives_channel_from_record_tail_when_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        crate::record::TurnRecord::open(dir.path())
            .unwrap()
            .append(
                1,
                "porch_main",
                crate::record::RecordRole::User,
                Some("[porch_main] cass: hi"),
            )
            .unwrap();
        let model = FakeModel::replying(vec![]);
        let h = harness_with_resume(dir.path(), model, None);
        assert_eq!(h.turn_loop.context.channel(), "porch_main");
    }

    #[tokio::test]
    async fn cold_start_with_no_record_uses_default_channel() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![]);
        let h = harness_with_resume(dir.path(), model, None);
        assert_eq!(h.turn_loop.context.channel(), DEFAULT_CHANNEL);
    }

    #[tokio::test]
    async fn resume_round_trip_active_flashes() {
        let dir = tempfile::tempdir().unwrap();
        let snap = crate::session::SessionSnapshot::new(
            DEFAULT_CHANNEL.into(),
            0,
            1.0,
            &[(
                crate::memory::Flash {
                    note_id: "n1".into(),
                    text: "first flash".into(),
                    neighbors: vec![],
                },
                2,
            )],
            0,
        );
        snap.write_atomic(&dir.path().join("session.json")).unwrap();

        let model = FakeModel::replying(vec![]);
        let h = harness_with_resume(dir.path(), model, Some(snap));
        assert_eq!(h.turn_loop.active_flashes.len(), 1);
        assert_eq!(h.turn_loop.active_flashes[0].0.note_id, "n1");
        assert_eq!(h.turn_loop.active_flashes[0].1, 2);
    }

    #[tokio::test]
    async fn resume_quiet_seconds_shifts_last_significant_at_into_the_past() {
        let dir = tempfile::tempdir().unwrap();
        let snap = crate::session::SessionSnapshot::new(
            DEFAULT_CHANNEL.into(),
            0,
            1.0,
            &[],
            200,
        );
        snap.write_atomic(&dir.path().join("session.json")).unwrap();
        let model = FakeModel::replying(vec![]);
        let h = harness_with_resume(dir.path(), model, Some(snap));
        let elapsed = h.turn_loop.last_significant_at.elapsed().as_secs();
        assert!(
            elapsed >= 200 && elapsed < 210,
            "elapsed = {elapsed}, expected ~200"
        );
    }

    // Flash-frame body composition tests moved into flashes.rs
    // per-type modules — the turn loop no longer builds bodies;
    // it just appends frame.body as-is.

    // -------- settle tool ↔ turn loop --------
    //
    // Every test names the specific drift it hunts. See CLAUDE.md:
    // "we don't write tests that pass, we write tests that HUNT."

    fn settle_call(id: &str, next_heartbeat: Option<u64>) -> anyhow::Result<ChatResponse> {
        let args = match next_heartbeat {
            Some(n) => serde_json::json!({ "next_heartbeat": n }),
            None => serde_json::json!({}),
        };
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: "settle".into(),
                arguments: args.to_string(),
            }],
            prompt_tokens: Some(50),
        })
    }

    fn speak_and_settle(next_heartbeat: Option<u64>) -> anyhow::Result<ChatResponse> {
        let settle_args = match next_heartbeat {
            Some(n) => serde_json::json!({ "next_heartbeat": n }),
            None => serde_json::json!({}),
        };
        Ok(ChatResponse {
            content: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "call_speak".into(),
                    name: "speak".into(),
                    arguments: serde_json::json!({ "content": "batched" }).to_string(),
                },
                ToolCall {
                    id: "call_settle".into(),
                    name: "settle".into(),
                    arguments: settle_args.to_string(),
                },
            ],
            prompt_tokens: Some(50),
        })
    }

    /// Hunts: someone deletes the `next_heartbeat_at: now + heartbeat`
    /// line in `TurnLoop::new` and the wake loop falls back to `Instant::now()`
    /// (immediate re-fire) or an uninitialized value.
    #[tokio::test]
    async fn cold_start_deadline_is_now_plus_config_default() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![]);
        let h = harness(dir.path(), model);
        let expected = std::time::Instant::now() + Duration::from_secs(3600);
        let deadline = h.turn_loop.next_heartbeat_at;
        let delta = if deadline > expected {
            (deadline - expected).as_secs()
        } else {
            (expected - deadline).as_secs()
        };
        assert!(delta < 5, "cold-start deadline off by {delta}s from expected");
    }

    /// Hunts: bare settle failing to recompute the deadline (turn just
    /// runs to natural end and the old deadline stays).
    #[tokio::test]
    async fn settle_bare_recomputes_deadline_to_config_default() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![settle_call("c1", None)]);
        let mut h = harness(dir.path(), model);
        // Force an obviously-wrong starting deadline so any drift-preserving
        // bug leaves the deadline near this value.
        h.turn_loop.next_heartbeat_at =
            std::time::Instant::now() + Duration::from_secs(9999);
        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        let expected = std::time::Instant::now() + Duration::from_secs(3600);
        let deadline = h.turn_loop.next_heartbeat_at;
        let delta = if deadline > expected {
            (deadline - expected).as_secs() as i64
        } else {
            -((expected - deadline).as_secs() as i64)
        };
        assert!(
            delta.abs() < 5,
            "bare settle should recompute to now + 3600s; delta {delta}s"
        );
        assert!(
            h.turn_loop.settle_intent.lock().unwrap().is_none(),
            "intent cleared after apply"
        );
    }

    /// Hunts: settle(N) arg ignored, unit confusion (minutes vs seconds),
    /// or intent applied at wrong point.
    #[tokio::test]
    async fn settle_with_arg_sets_deadline_at_now_plus_n_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![settle_call("c1", Some(120))]);
        let mut h = harness(dir.path(), model);
        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        let expected = std::time::Instant::now() + Duration::from_secs(120 * 60);
        let deadline = h.turn_loop.next_heartbeat_at;
        let delta = if deadline > expected {
            (deadline - expected).as_secs() as i64
        } else {
            -((expected - deadline).as_secs() as i64)
        };
        assert!(delta.abs() < 5, "deadline off by {delta}s");
    }

    /// Hunts: natural end-of-turn resetting the deadline (the deadline
    /// model's whole point). Also hunts intent leaking across turns.
    #[tokio::test]
    async fn natural_settle_preserves_deadline() {
        let dir = tempfile::tempdir().unwrap();
        // Turn 1: settle(240). Turn 2: natural end (speak + done).
        let model = FakeModel::replying(vec![
            settle_call("c1", Some(240)),
            speak("hi"),
            done(""),
        ]);
        let mut h = harness(dir.path(), model);
        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        let deadline_after_settle = h.turn_loop.next_heartbeat_at;
        // Simulate a channel wake for turn 2.
        let note2 = say(&mut h, DEFAULT_CHANNEL, "and more").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note2]))
            .await
            .unwrap();
        let deadline_after_natural = h.turn_loop.next_heartbeat_at;
        assert_eq!(
            deadline_after_natural, deadline_after_settle,
            "natural settle must not touch next_heartbeat_at"
        );
    }

    /// Hunts: settle short-circuiting a batch and dropping peer tool
    /// calls that were meant to run first.
    #[tokio::test]
    async fn settle_runs_after_batch_peers() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![speak_and_settle(Some(60))]);
        let mut h = harness(dir.path(), model);
        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        // The speak in the batch delivered.
        let out = h.outbound.try_recv().expect("speak fired");
        assert_eq!(out.content, "batched");
        // And settle set the deadline.
        let expected = std::time::Instant::now() + Duration::from_secs(60 * 60);
        let delta = h.turn_loop.next_heartbeat_at.saturating_duration_since(expected).as_secs()
            + expected.saturating_duration_since(h.turn_loop.next_heartbeat_at).as_secs();
        assert!(delta < 5, "deadline off by {delta}s");
    }

    /// Hunts: settle intent being ignored when it appears in a non-final
    /// iteration, causing extra model calls after the agent chose to end.
    #[tokio::test]
    async fn settle_ends_turn_after_current_batch() {
        let dir = tempfile::tempdir().unwrap();
        // Only ONE model response queued. If the loop iterates past the
        // settle-bearing batch, it'll try to call chat() a second time
        // and panic on the empty replies vector.
        let model = FakeModel::replying(vec![speak_and_settle(Some(30))]);
        let mut h = harness(dir.path(), model.clone());
        let note = say(&mut h, DEFAULT_CHANNEL, "hi").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();
        // Exactly one chat() call happened — the second iteration was
        // skipped because settle set the intent.
        assert_eq!(model.seen.lock().unwrap().len(), 1);
    }
}
