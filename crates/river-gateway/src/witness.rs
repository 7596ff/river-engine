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
use crate::turn::DIGESTION_MARKER;

pub const NOTHING_TO_GLEAN: &str = "nothing to glean";
const GLEAN_WINDOW_TURNS: u64 = 6;

pub struct Witness<C: Chat> {
    workspace: PathBuf,
    client: C,
    identity: String,
    on_turn: Option<String>,
    on_glean: Option<String>,
    moves: MovesFile,
    memory: Option<crate::memory::Memory>,
    glean_probability: f64,
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
    ) -> anyhow::Result<Self> {
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
        Ok(Self {
            workspace: workspace.to_path_buf(),
            client,
            identity,
            on_turn,
            on_glean,
            moves,
            memory,
            glean_probability,
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
                for turn in self.missing_moves(target)? {
                    self.move_for(turn).await?;
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

        let prompt = template.replace("{recent_record}", &recent);
        let messages = [ChatMessage::user(prompt)];
        match self.client.chat(&self.identity, &messages, &[]).await {
            Ok(response) => {
                let candidate = response.content.trim();
                if candidate.is_empty()
                    || candidate.eq_ignore_ascii_case(NOTHING_TO_GLEAN)
                {
                    tracing::debug!(turn = up_to_turn, "glean: nothing");
                } else {
                    memory.enqueue_candidate(candidate)?;
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
        let err = match Witness::load(dir.path(), model, None, 0.0) {
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
        let witness = Witness::load(dir.path(), model, None, 0.0).unwrap();
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
        let mut witness = Witness::load(dir.path(), model.clone(), None, 0.0).unwrap();
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
        let mut witness = Witness::load(dir.path(), model, None, 0.0).unwrap();
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
        let mut witness = Witness::load(dir.path(), model, None, 0.0).unwrap();
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
            Witness::load(dir.path(), model, Some(memory.clone()), 1.0).unwrap();

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
        let candidate = memory.pop_candidate().unwrap().unwrap();
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
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0).unwrap();
        witness.glean(1).await.unwrap();

        assert_eq!(memory.queue_depth().unwrap(), 0);
        assert!(
            model.prompts.lock().unwrap().is_empty(),
            "model must not be called on a digestion turn",
        );
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
            Witness::load(dir.path(), model.clone(), Some(memory.clone()), 1.0).unwrap();
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
        let witness = Witness::load(dir.path(), model, None, 0.0).unwrap();
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
        let witness = Witness::load(dir.path(), model, None, 0.0).unwrap();
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
        let witness = Witness::load(dir.path(), model, None, 0.0).unwrap();
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
