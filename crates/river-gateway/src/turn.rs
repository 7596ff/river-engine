//! The minimal turn loop (wall ch. 01, barebones reduction): a
//! serialized event queue, one turn at a time. Wake on a notification
//! or the heartbeat timer; drain everything pending; persist each
//! message to the turn record at the moment it enters the context
//! (persist-once); call the model; reply; settle. No tools yet.
//!
//! Shutdown is cooperative: the loop only observes the shutdown
//! signal between turns, so a turn in progress always runs to settle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch};

use crate::context::RollingContext;
use crate::model::{Chat, Role};
use crate::record::{RecordRole, TurnRecord, last_turn};

pub const HEARTBEAT_MARKER: &str = ":heartbeat:";
pub const DEFAULT_CHANNEL: &str = "local_main";

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub channel: String,
    pub author: String,
    pub content: String,
}

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
    Messages(Vec<InboundMessage>),
    Heartbeat,
    Shutdown,
}

pub struct TurnLoop<C: Chat> {
    workspace: PathBuf,
    system_prompt: String,
    client: C,
    context: RollingContext,
    records: HashMap<String, TurnRecord>,
    turn_number: u64,
    current_channel: String,
    inbound: mpsc::Receiver<InboundMessage>,
    outbound: broadcast::Sender<OutboundMessage>,
    health: watch::Sender<Health>,
    heartbeat: Duration,
}

impl<C: Chat> TurnLoop<C> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace: PathBuf,
        system_prompt: String,
        client: C,
        inbound: mpsc::Receiver<InboundMessage>,
        outbound: broadcast::Sender<OutboundMessage>,
        health: watch::Sender<Health>,
        heartbeat: Duration,
    ) -> anyhow::Result<Self> {
        // Resume turn numbering from the record: the life is one
        // sequence, restarts included.
        let main_record = TurnRecord::open(&workspace, DEFAULT_CHANNEL)?;
        let turn_number = last_turn(main_record.path())?;
        let mut records = HashMap::new();
        records.insert(DEFAULT_CHANNEL.to_string(), main_record);

        Ok(Self {
            workspace,
            system_prompt,
            client,
            context: RollingContext::new(),
            records,
            turn_number,
            current_channel: DEFAULT_CHANNEL.to_string(),
            inbound,
            outbound,
            health,
            heartbeat,
        })
    }

    /// Run until shutdown flips true. Each iteration is one turn.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        loop {
            let wake = tokio::select! {
                biased;
                _ = shutdown.wait_for(|&stop| stop) => Wake::Shutdown,
                msg = self.inbound.recv() => match msg {
                    Some(first) => {
                        let mut batch = vec![first];
                        while let Ok(more) = self.inbound.try_recv() {
                            batch.push(more);
                        }
                        Wake::Messages(batch)
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

        match wake {
            Wake::Messages(batch) => {
                for msg in batch {
                    let formatted = format!("[{}] {}: {}", msg.channel, msg.author, msg.content);
                    self.append(n, &msg.channel.clone(), Role::User, &formatted)?;
                    self.current_channel = msg.channel;
                }
            }
            Wake::Heartbeat => {
                let channel = self.current_channel.clone();
                self.append(n, &channel, Role::User, HEARTBEAT_MARKER)?;
            }
            Wake::Shutdown => unreachable!("handled by run"),
        }

        match self
            .client
            .chat(&self.system_prompt, &self.context.messages())
            .await
        {
            Ok(response) => {
                let channel = self.current_channel.clone();
                self.append(n, &channel, Role::Assistant, &response.content)?;
                // A send error only means no client is listening.
                let _ = self.outbound.send(OutboundMessage {
                    channel,
                    content: response.content,
                });
            }
            Err(e) => {
                // A failed model call ends the turn; settle still
                // runs and everything said is already persisted.
                tracing::warn!(turn = n, error = %e, "model call failed; turn ends");
            }
        }

        self.settle();
        Ok(())
    }

    /// Persist-once: context append and record append are one act.
    fn append(&mut self, turn: u64, channel: &str, role: Role, content: &str) -> anyhow::Result<()> {
        self.context.push(turn, role, content);
        let record = match self.records.get_mut(channel) {
            Some(record) => record,
            None => {
                let record = TurnRecord::open(&self.workspace, channel)?;
                self.records.entry(channel.to_string()).or_insert(record)
            }
        };
        let record_role = match role {
            Role::User => RecordRole::User,
            Role::Assistant => RecordRole::Assistant,
        };
        record.append(turn, record_role, Some(content))?;
        Ok(())
    }

    fn settle(&mut self) {
        let _ = self.health.send(Health {
            turn_number: self.turn_number,
            last_settle: Some(jiff::Timestamp::now().to_string()),
            context_messages: self.context.len(),
        });
        tracing::debug!(turn = self.turn_number, "settled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatMessage, ChatResponse};
    use crate::record::scan;
    use std::sync::{Arc, Mutex};

    struct FakeModel {
        replies: Mutex<Vec<anyhow::Result<ChatResponse>>>,
        seen: Mutex<Vec<Vec<ChatMessage>>>,
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
            _system: &str,
            messages: &[ChatMessage],
        ) -> anyhow::Result<ChatResponse> {
            self.seen.lock().unwrap().push(messages.to_vec());
            self.replies.lock().unwrap().remove(0)
        }
    }

    fn harness(
        workspace: PathBuf,
        model: Arc<FakeModel>,
    ) -> (
        TurnLoop<Arc<FakeModel>>,
        mpsc::Sender<InboundMessage>,
        broadcast::Receiver<OutboundMessage>,
        watch::Receiver<Health>,
    ) {
        let (inbound_tx, inbound_rx) = mpsc::channel(64);
        let (outbound_tx, outbound_rx) = broadcast::channel(64);
        let (health_tx, health_rx) = watch::channel(Health::default());
        let turn_loop = TurnLoop::new(
            workspace,
            "system".into(),
            model,
            inbound_rx,
            outbound_tx,
            health_tx,
            Duration::from_secs(3600),
        )
        .unwrap();
        (turn_loop, inbound_tx, outbound_rx, health_rx)
    }

    #[tokio::test]
    async fn message_turn_persists_and_replies() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![Ok(ChatResponse {
            content: "good morning".into(),
            prompt_tokens: Some(10),
        })]);
        let (mut turn_loop, _tx, mut out, health) = harness(dir.path().to_path_buf(), model.clone());

        turn_loop
            .turn(Wake::Messages(vec![InboundMessage {
                channel: DEFAULT_CHANNEL.into(),
                author: "cass".into(),
                content: "hello".into(),
            }]))
            .await
            .unwrap();

        let reply = out.try_recv().unwrap();
        assert_eq!(reply.content, "good morning");
        assert_eq!(reply.channel, DEFAULT_CHANNEL);

        let lines = scan(&dir.path().join("record/local_main.jsonl")).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].turn, 1);
        assert_eq!(
            lines[0].content.as_deref(),
            Some("[local_main] cass: hello")
        );
        assert_eq!(lines[1].content.as_deref(), Some("good morning"));

        assert_eq!(health.borrow().turn_number, 1);
        assert_eq!(health.borrow().context_messages, 2);

        // The model saw the formatted user message.
        let seen = model.seen.lock().unwrap();
        assert_eq!(seen[0][0].content, "[local_main] cass: hello");
    }

    #[tokio::test]
    async fn model_failure_ends_turn_with_messages_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![Err(anyhow::anyhow!("api down"))]);
        let (mut turn_loop, _tx, mut out, health) = harness(dir.path().to_path_buf(), model.clone());

        turn_loop
            .turn(Wake::Messages(vec![InboundMessage {
                channel: DEFAULT_CHANNEL.into(),
                author: "cass".into(),
                content: "hello".into(),
            }]))
            .await
            .unwrap();

        assert!(out.try_recv().is_err(), "no reply on model failure");
        let lines = scan(&dir.path().join("record/local_main.jsonl")).unwrap();
        assert_eq!(lines.len(), 1, "the user message is already persisted");
        assert_eq!(health.borrow().turn_number, 1, "settle still ran");
    }

    #[tokio::test]
    async fn heartbeat_appends_marker() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![Ok(ChatResponse {
            content: "quiet morning".into(),
            prompt_tokens: None,
        })]);
        let (mut turn_loop, _tx, _out, _health) = harness(dir.path().to_path_buf(), model.clone());

        turn_loop.turn(Wake::Heartbeat).await.unwrap();

        let lines = scan(&dir.path().join("record/local_main.jsonl")).unwrap();
        assert_eq!(lines[0].content.as_deref(), Some(HEARTBEAT_MARKER));
        assert_eq!(lines[0].role, crate::record::RecordRole::User);
    }

    #[tokio::test]
    async fn turn_numbers_resume_from_the_record() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut record = TurnRecord::open(dir.path(), DEFAULT_CHANNEL).unwrap();
            record.append(41, RecordRole::User, Some("old")).unwrap();
        }
        let model = FakeModel::replying(vec![Ok(ChatResponse {
            content: "back".into(),
            prompt_tokens: None,
        })]);
        let (mut turn_loop, _tx, _out, health) = harness(dir.path().to_path_buf(), model.clone());

        turn_loop
            .turn(Wake::Messages(vec![InboundMessage {
                channel: DEFAULT_CHANNEL.into(),
                author: "cass".into(),
                content: "hello again".into(),
            }]))
            .await
            .unwrap();

        assert_eq!(health.borrow().turn_number, 42);
    }

    #[tokio::test]
    async fn drains_pending_messages_into_one_turn() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![Ok(ChatResponse {
            content: "got both".into(),
            prompt_tokens: None,
        })]);
        let (turn_loop, tx, mut out, health) = harness(dir.path().to_path_buf(), model.clone());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        tx.send(InboundMessage {
            channel: DEFAULT_CHANNEL.into(),
            author: "cass".into(),
            content: "one".into(),
        })
        .await
        .unwrap();
        tx.send(InboundMessage {
            channel: DEFAULT_CHANNEL.into(),
            author: "cass".into(),
            content: "two".into(),
        })
        .await
        .unwrap();

        let handle = tokio::spawn(turn_loop.run(shutdown_rx));
        // Wait for the reply, then stop the loop.
        let reply = out.recv().await.unwrap();
        assert_eq!(reply.content, "got both");
        shutdown_tx.send(true).unwrap();
        handle.await.unwrap().unwrap();

        assert_eq!(health.borrow().turn_number, 1, "both messages in one turn");
        let lines = scan(&dir.path().join("record/local_main.jsonl")).unwrap();
        assert_eq!(lines.len(), 3); // two user + one assistant
        assert!(lines.iter().all(|l| l.turn == 1));
    }
}
