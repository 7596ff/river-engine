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
//! processes every turn from its cursor + 1 up to it, in order. That
//! makes it self-healing: missed signals, restarts, and downtime all
//! recover by catch-up.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use tokio::sync::watch;

use crate::model::{Chat, ChatMessage, Role};
use crate::record::{self, MovesFile, RecordLine, RecordRole};

pub struct Witness<C: Chat> {
    workspace: PathBuf,
    client: C,
    identity: String,
    on_turn: Option<String>,
    moves: MovesFile,
}

impl<C: Chat> Witness<C> {
    /// Load prompts and open the moves file. A missing
    /// `witness/identity.md` fails startup — the gateway does not run
    /// without its witness (wall ch. 04). Missing duty prompts
    /// disable their duty, logged once.
    pub fn load(workspace: &Path, client: C) -> anyhow::Result<Self> {
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

        let moves = MovesFile::open(workspace)?;
        Ok(Self {
            workspace: workspace.to_path_buf(),
            client,
            identity,
            on_turn,
            moves,
        })
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
                let cursor = record::witness_cursor(self.moves.path())?;
                for turn in (cursor + 1)..=target {
                    self.move_for(turn).await?;
                }
            }
            tokio::select! {
                biased;
                _ = shutdown.wait_for(|&stop| stop) => return Ok(()),
                changed = latest_turn.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                }
            }
        }
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
            let messages = [ChatMessage {
                role: Role::User,
                content: prompt,
            }];
            match self.client.chat(&self.identity, &messages).await {
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
}

/// The agent's own words are marked "you:" — the transcript carries
/// the deixis so the prompt doesn't have to.
pub fn format_transcript(lines: &[RecordLine]) -> String {
    let mut transcript = String::new();
    for line in lines {
        let Some(content) = &line.content else {
            continue;
        };
        match line.role {
            RecordRole::User => transcript.push_str(content),
            RecordRole::Assistant => {
                transcript.push_str("you: ");
                transcript.push_str(content);
            }
            RecordRole::System => {
                transcript.push_str("[system] ");
                transcript.push_str(content);
            }
            RecordRole::Tool => {
                transcript.push_str("[tool result] ");
                transcript.push_str(content);
            }
        }
        transcript.push('\n');
    }
    transcript
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

    #[test]
    fn missing_identity_fails_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeModel::replying(vec![]);
        let err = match Witness::load(dir.path(), model) {
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
        let witness = Witness::load(dir.path(), model).unwrap();
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
        let mut witness = Witness::load(dir.path(), model.clone()).unwrap();
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
        let mut witness = Witness::load(dir.path(), model).unwrap();
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
        let mut witness = Witness::load(dir.path(), model).unwrap();
        witness.move_for(1).await.unwrap();

        let moves = read_moves(witness.moves.path()).unwrap();
        assert!(moves[0].summary.contains("you replied"));
    }

    #[tokio::test]
    async fn catch_up_processes_every_turn_from_cursor_in_order() {
        let dir = tempfile::tempdir().unwrap();
        seed_witness(dir.path());
        for turn in 1..=3 {
            record_turn(dir.path(), turn, &format!("q{turn}"), Some(&format!("a{turn}")));
        }

        let model = FakeModel::replying(vec![ok("move one"), ok("move two"), ok("move three")]);
        let witness = Witness::load(dir.path(), model).unwrap();
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
}
