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

use crate::channels::{Channels, EntryRole, Notification};
use crate::config::ContextConfig;
use crate::context::PersistentContext;
use crate::identity;
use crate::model::Chat;
use crate::record::{RecordRole, TurnRecord, last_turn};

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
    Shutdown,
}

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
    heartbeat: Duration,
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
        heartbeat: Duration,
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
            heartbeat,
        })
    }

    /// Run until shutdown flips true. Each iteration is one turn;
    /// shutdown is only observed between turns, so a turn in progress
    /// always runs to settle.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        loop {
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
                _ = tokio::time::sleep(self.heartbeat) => Wake::Heartbeat,
            };

            match wake {
                Wake::Shutdown => {
                    tracing::info!("shutdown: no turn in flight, exiting cleanly");
                    return Ok(());
                }
                wake => self.turn(wake).await?,
            }
        }
    }

    async fn turn(&mut self, wake: Wake) -> anyhow::Result<()> {
        self.turn_number += 1;
        let n = self.turn_number;
        let mut read_channels: Vec<String> = Vec::new();
        let mut spoke_in: Option<String> = None;

        match wake {
            Wake::Notifications(batch) => {
                // Dedup channels, preserving arrival order.
                let mut notified: Vec<String> = Vec::new();
                for note in &batch {
                    if !notified.contains(&note.channel) {
                        notified.push(note.channel.clone());
                    }
                }

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
                    let entries = self.channels.read_since_cursor(&channel)?;
                    for entry in entries {
                        if entry.role == EntryRole::Other
                            && let Some(content) = &entry.content
                        {
                            let author = entry.author.as_deref().unwrap_or("unknown");
                            let formatted = format!("[{channel}] {author}: {content}");
                            self.append(n, RecordRole::User, &formatted)?;
                        }
                    }
                    read_channels.push(channel);
                }
            }
            Wake::Heartbeat => {
                self.append(n, RecordRole::User, HEARTBEAT_MARKER)?;
            }
            Wake::Shutdown => unreachable!("handled by run"),
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

        let (system, messages) = self.context.messages();
        match self.client.chat(&system, &messages).await {
            Ok(response) => {
                self.context.calibrate(response.prompt_tokens);
                let channel = self.context.channel().to_string();
                self.append(n, RecordRole::Assistant, &response.content)?;
                // Deliver, then log post-acceptance: the broadcast is
                // the local delivery; the agent entry doubles as the
                // cursor (wall ch. 05).
                let _ = self.outbound.send(OutboundMessage {
                    channel: channel.clone(),
                    content: response.content.clone(),
                });
                self.channels
                    .agent_spoke(&channel, &response.content, LOCAL_ADAPTER, None)?;
                spoke_in = Some(channel);
            }
            Err(e) => {
                // Every turn settles: the model failure ends the turn;
                // everything persisted before it is already safe.
                tracing::warn!(turn = n, error = %e, "model call failed; turn ends");
            }
        }

        // SETTLE: a cursor to every channel read this turn; speaking
        // was already an implicit cursor for its channel.
        for channel in &read_channels {
            if spoke_in.as_deref() != Some(channel) {
                self.channels.mark_read(channel)?;
            }
        }
        let _ = self.health.send(Health {
            turn_number: n,
            last_settle: Some(jiff::Timestamp::now().to_string()),
            context_messages: self.context.len(),
        });
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
    }

    impl FakeModel {
        fn replying(replies: Vec<anyhow::Result<ChatResponse>>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl Chat for Arc<FakeModel> {
        async fn chat(
            &self,
            system: &str,
            messages: &[ChatMessage],
        ) -> anyhow::Result<ChatResponse> {
            self.seen
                .lock()
                .unwrap()
                .push((system.to_string(), messages.to_vec()));
            self.replies.lock().unwrap().remove(0)
        }
    }

    fn ok(content: &str) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            content: content.into(),
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
        let turn_loop = TurnLoop::new(
            dir.to_path_buf(),
            jiff::tz::TimeZone::UTC,
            ContextConfig::default(),
            model,
            channels.clone(),
            notify_rx,
            outbound_tx,
            health_tx,
            Duration::from_secs(3600),
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
    async fn message_turn_reads_persists_replies_and_cursors() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![ok("good morning")]);
        let mut h = harness(dir.path(), model.clone());

        let note = say(&mut h, DEFAULT_CHANNEL, "hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![note]))
            .await
            .unwrap();

        // The reply went out and was logged post-acceptance.
        let out = h.outbound.try_recv().unwrap();
        assert_eq!(out.content, "good morning");

        // The record holds both lines under turn 1, channel-tagged.
        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.turn == 1));
        assert!(lines.iter().all(|l| l.channel == DEFAULT_CHANNEL));
        assert_eq!(
            lines[0].content.as_deref(),
            Some("[local_main] cass: hello")
        );

        // Cursor honest: speaking was the implicit cursor.
        assert!(h.channels.read_since_cursor(DEFAULT_CHANNEL).unwrap().is_empty());
        let entries = h.channels.scan(DEFAULT_CHANNEL).unwrap();
        assert_eq!(entries.len(), 2); // inbound + agent message, no extra cursor
        assert_eq!(entries[1].role, EntryRole::Agent);
        assert_eq!(entries[1].content.as_deref(), Some("good morning"));

        // The model saw identity in the system string.
        let seen = model.seen.lock().unwrap();
        assert!(seen[0].0.contains("i am a test agent"));
        assert_eq!(h.health.borrow().turn_number, 1);
    }

    #[tokio::test]
    async fn model_failure_settles_with_explicit_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![Err(anyhow::anyhow!("api down"))]);
        let mut h = harness(dir.path(), model);

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
        let model = FakeModel::replying(vec![ok("heard both")]);
        let mut h = harness(dir.path(), model);

        let n1 = say(&mut h, DEFAULT_CHANNEL, "from local").await;
        let n2 = say(&mut h, "discord_general", "from discord").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n1, n2]))
            .await
            .unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines.len(), 3); // two user + one assistant
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
        let model = FakeModel::replying(vec![ok("quiet hour")]);
        let mut h = harness(dir.path(), model);

        h.turn_loop.turn(Wake::Heartbeat).await.unwrap();

        let lines = scan(&dir.path().join("record/turns.jsonl")).unwrap();
        assert_eq!(lines[0].content.as_deref(), Some("Read HEARTBEAT.md."));
        assert_eq!(lines[0].role, RecordRole::User);
    }

    #[tokio::test]
    async fn restart_resumes_numbering_and_rebuilds_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        {
            let model = FakeModel::replying(vec![ok("first life")]);
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
        let model = FakeModel::replying(vec![ok("second life")]);
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
        let model = FakeModel::replying(vec![ok("hi local"), ok("hi discord")]);
        let mut h = harness(dir.path(), model.clone());

        let n1 = say(&mut h, DEFAULT_CHANNEL, "local hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n1]))
            .await
            .unwrap();

        let n2 = say(&mut h, "discord_general", "discord hello").await;
        h.turn_loop
            .turn(Wake::Notifications(vec![n2]))
            .await
            .unwrap();

        assert_eq!(h.turn_loop.context.channel(), "discord_general");
        // The reply was spoken (and cursored) on discord.
        let discord = h.channels.scan("discord_general").unwrap();
        assert_eq!(discord.last().unwrap().content.as_deref(), Some("hi discord"));
    }
}
