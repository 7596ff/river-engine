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
use crate::config::ContextConfig;
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
    /// When the last inbound message arrived; the quiet trigger
    /// measures from here and any inbound resets it.
    last_inbound: std::time::Instant,
    /// In-memory read positions (channel → last consumed entry id).
    /// Authoritative within the process; the log cursor recovers the
    /// position across restarts. Without this, an agent entry written
    /// mid-turn (speak) would swallow arrivals that landed before it.
    positions: HashMap<String, String>,
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
        settled: watch::Sender<u64>,
        heartbeat: Duration,
        registry: Registry,
        profile: Vec<String>,
        scrub: Vec<String>,
        max_iterations: u32,
        memory: Option<crate::memory::Memory>,
        reindex: Option<mpsc::Sender<()>>,
        discord: Option<mpsc::Sender<crate::discord::SpeakRequest>>,
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
            settled,
            heartbeat,
            registry,
            profile,
            scrub,
            max_iterations,
            memory,
            reindex,
            discord,
            last_inbound: std::time::Instant::now(),
            positions: HashMap::new(),
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
                    Some(QUIET_TRIGGER.saturating_sub(self.last_inbound.elapsed()))
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

        // Flash delivery (wall ch. 02, amended): pending flashes ride
        // in the memory slot for exactly this turn.
        if let Some(memory) = &self.memory {
            let flashes = memory.take_flashes();
            let slot = if flashes.is_empty() {
                String::new()
            } else {
                let mut text = String::new();
                for flash in &flashes {
                    text.push_str(&format!("[flash] {}: {}\n", flash.note_id, flash.text));
                    for (link_type, neighbor) in &flash.neighbors {
                        text.push_str(&format!("  {link_type} → {neighbor}\n"));
                    }
                }
                text
            };
            self.context.set_memory_slot(slot);
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

                self.last_inbound = std::time::Instant::now();
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
                            let formatted = format!("[{channel}] {author}: {content}");
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
                let framing = format!(
                    "[digestion] Your witness gleaned this from your recent activity:\n\n\
                     {candidate}\n\n\
                     Re-engage it: re-read what it cites if you need to, then either \
                     write a fresh atomic note in knowledge/ with the write tool — one \
                     claim, at most ~100 words, your own words, typed links in the \
                     frontmatter (id, links) — or reject the candidate, saying briefly \
                     why. Never copy the witness's phrasing."
                );
                self.append(n, RecordRole::User, &framing)?;
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

        for iteration in 0..self.max_iterations {
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
                self.last_inbound = std::time::Instant::now();
                let mut notice = String::from("[arrived mid-turn]");
                let mut anything_new = false;
                for channel in &arrived {
                    let entries = self.consume(channel)?;
                    for entry in &entries {
                        if let Some(content) = &entry.content {
                            let author = entry.author.as_deref().unwrap_or("unknown");
                            notice.push_str(&format!("\n[{channel}] {author}: {content}"));
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

        let _ = self.health.send(Health {
            turn_number: n,
            last_settle: Some(jiff::Timestamp::now().to_string()),
            context_messages: self.context.len(),
        });
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
    }

    fn harness(dir: &Path, model: Arc<FakeModel>) -> Harness {
        write_identity(dir);
        let (notify_tx, notify_rx) = mpsc::channel(256);
        let channels = Channels::open(dir, notify_tx).unwrap();
        let (outbound_tx, outbound_rx) = broadcast::channel(64);
        let (health_tx, health_rx) = watch::channel(Health::default());
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
            settled_tx,
            Duration::from_secs(3600),
            Registry::core(),
            crate::config::DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect(),
            vec![],
            10,
            None,
            None,
            None,
        )
        .unwrap();
        Harness {
            turn_loop,
            channels,
            notify_rx_drained: mpsc::channel(1).1,
            outbound: outbound_rx,
            health: health_rx,
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
        assert_eq!(h.health.borrow().turn_number, 1, "a real turn, settled");
    }

    #[tokio::test]
    async fn pending_flash_rides_the_memory_slot_for_one_turn() {
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

        let model = FakeModel::replying(vec![done("noticed"), done("quiet")]);
        let mut h = harness(dir.path(), model.clone());
        h.turn_loop.memory = Some(memory);

        h.turn_loop.turn(Wake::Heartbeat).await.unwrap();
        let seen = model.seen.lock().unwrap();
        assert!(seen[0].0.contains("[flash] NOWL"), "flash in the slot");
        assert!(seen[0].0.contains("owl is silent"));
        drop(seen);

        // The flash rides exactly one turn.
        h.turn_loop.turn(Wake::Heartbeat).await.unwrap();
        let seen = model.seen.lock().unwrap();
        assert!(!seen[1].0.contains("[flash]"), "slot cleared next turn");
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
