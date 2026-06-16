//! The turn cycle (wall ch. 01) over the channel layer (ch. 05) and
//! the persistent context (ch. 03). Wake on a notification pointer or
//! the heartbeat; drain everything pending; read each notified
//! channel from its cursor; persist each message at context-append
//! time (persist-once); compact if needed; call the model; reply;
//! settle with cursors to every channel read.
//!
//! Turns are serial; numbers are monotonic for life; every turn
//! settles; shutdown is observed only between turns.

use std::path::PathBuf;
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

pub const HEARTBEAT_MARKER: &str = "Read HEARTBEAT.md.";
pub const DEFAULT_CHANNEL: &str = "local_main";
pub const LOCAL_ADAPTER: &str = "local";

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

enum Wake {
    Notifications(Vec<Notification>),
    Heartbeat,
    /// A quiet-trigger digestion turn carrying one extraction
    /// candidate (wall ch. 02).
    Digestion(String),
    /// The queue changed while parked: recompute the select arms.
    Recheck,
    Shutdown,
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
    ) -> anyhow::Result<Self> {
        let record = TurnRecord::open(&workspace)?;
        // Monotonic for life: resume from the record (wall ch. 01).
        let turn_number = last_turn(record.path())?;
        let system_prompt = fresh_system_prompt(&workspace, &tz)?;
        let context =
            PersistentContext::build(&workspace, DEFAULT_CHANNEL, system_prompt, knobs.clone())?;

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
            last_significant_at: std::time::Instant::now(),
            positions: HashMap::new(),
            active_flashes: Vec::new(),
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
                            Some(candidate) => Wake::Digestion(candidate),
                            None => Wake::Recheck,
                        }
                    } else {
                        wake
                    }
                }
                _ = tokio::time::sleep(self.heartbeat) => Wake::Heartbeat,
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
        self.turn_number += 1;
        let n = self.turn_number;
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
            Wake::Heartbeat => {
                self.append(n, RecordRole::User, HEARTBEAT_MARKER)?;
            }
            Wake::Digestion(candidate) => {
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

        if self.context.needs_compaction() {
            let system_prompt = fresh_system_prompt(&self.workspace, &self.tz)?;
            let lag_warning = self
                .context
                .compact(&self.workspace, system_prompt, n)?;
            if let Some(warning) = lag_warning {
                self.append(n, RecordRole::System, &warning)?;
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
            scrub: self.scrub.clone(),
            memory: self.memory.clone(),
            reindex: self.reindex.clone(),
            discord: self.discord.clone(),
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

            if iteration + 1 == self.max_iterations {
                tracing::warn!(turn = n, "iteration ceiling hit; turn ends");
            }
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
        // Persist-before-announce: every append above fsynced inline,
        // so the record already holds the whole turn.
        let _ = self.settled.send(n);
        tracing::debug!(turn = n, "settled");
        Ok(())
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
            ContextConfig::default(),
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
    async fn heartbeat_appends_the_instruction() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![done("quiet hour")]);
        let mut h = harness(dir.path(), model);

        h.turn_loop.turn(Wake::Heartbeat).await.unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines[0].content.as_deref(), Some("Read HEARTBEAT.md."));
        assert_eq!(lines[0].role, RecordRole::User);
    }

    #[tokio::test]
    async fn digestion_turn_frames_the_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![done("rejected: too thin to keep")]);
        let mut h = harness(dir.path(), model);

        h.turn_loop
            .turn(Wake::Digestion("the agent kept circling teal".into()))
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
            .turn(Wake::Digestion("the agent kept circling teal".into()))
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
}
