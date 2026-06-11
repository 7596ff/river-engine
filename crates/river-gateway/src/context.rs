//! The persistent context (wall ch. 03): built once, appended in
//! place, rebuilt only by compaction, session start, or channel
//! switch. Assembly reads *who I am → what has happened → what memory
//! offers → what is happening now*. Compaction only ever drops turns
//! the witness has compressed — the lossless guarantee — and with the
//! witness cursor at 0 (no witness yet), nothing is droppable.
//!
//! Both model protocols receive the same shape: SYSTEM + ARC + MEMORY
//! SLOT fold into the system string in assembly order (Anthropic
//! requires system content top-level), HOT becomes the message list.

use std::path::Path;

use crate::config::ContextConfig;
use crate::model::{ChatMessage, Role, ToolCall};
use crate::record::{self, MoveLine, RecordRole};

/// Heuristic, self-correcting token estimator (wall ch. 03).
#[derive(Debug)]
pub struct Estimator {
    ratio: f64,
}

impl Estimator {
    pub fn new() -> Self {
        Self { ratio: 1.0 }
    }

    fn base(text: &str) -> f64 {
        text.len() as f64 / 4.0 + 4.0
    }

    pub fn estimate(&self, text: &str) -> f64 {
        Self::base(text) * self.ratio
    }

    /// ratio ← 0.7·ratio + 0.3·(reported / estimated); skip when
    /// either side of the division is zero.
    pub fn calibrate(&mut self, reported: u64, estimated: f64) {
        if reported == 0 || estimated <= 0.0 {
            return;
        }
        self.ratio = 0.7 * self.ratio + 0.3 * (reported as f64 / estimated);
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }
}

#[derive(Debug, Clone)]
pub struct HotEntry {
    pub turn: u64,
    pub role: RecordRole,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug)]
pub struct PersistentContext {
    knobs: ContextConfig,
    channel: String,
    system_prompt: String,
    arc: Vec<MoveLine>,
    memory_slot: String,
    hot: Vec<HotEntry>,
    estimator: Estimator,
    last_estimate: f64,
}

impl PersistentContext {
    /// Session start (and channel switch): rebuild from the record —
    /// whole turns touching the channel above the witness cursor,
    /// best-effort floor backfill below it, arc from the moves file.
    pub fn build(
        workspace: &Path,
        channel: &str,
        system_prompt: String,
        knobs: ContextConfig,
    ) -> anyhow::Result<Self> {
        let mut ctx = Self {
            knobs,
            channel: channel.to_string(),
            system_prompt,
            arc: Vec::new(),
            memory_slot: String::new(),
            hot: Vec::new(),
            estimator: Estimator::new(),
            last_estimate: 0.0,
        };

        let cursor = record::witness_cursor(&record::moves_path(workspace))?;
        let lines = record::scan(&workspace.join("record").join("turns.jsonl"))?;

        // Turns touching the channel, in order, split at the cursor.
        let mut above: Vec<HotEntry> = Vec::new();
        let mut below_turns: Vec<Vec<HotEntry>> = Vec::new();
        let touching: std::collections::BTreeSet<u64> = lines
            .iter()
            .filter(|l| l.channel == channel)
            .map(|l| l.turn)
            .collect();
        for turn in &touching {
            let entries: Vec<HotEntry> = lines
                .iter()
                .filter(|l| l.turn == *turn)
                .map(|l| HotEntry {
                    turn: l.turn,
                    role: l.role,
                    content: l.content.clone().unwrap_or_default(),
                    tool_calls: l.tool_calls.clone().unwrap_or_default(),
                    tool_call_id: l.tool_call_id.clone(),
                })
                .collect();
            if *turn > cursor {
                above.extend(entries);
            } else {
                below_turns.push(entries);
            }
        }

        ctx.hot = above;
        ctx.backfill_floor(below_turns);
        ctx.reload_arc(workspace)?;
        Ok(ctx)
    }

    pub fn channel(&self) -> &str {
        &self.channel
    }

    pub fn append(&mut self, turn: u64, role: RecordRole, content: impl Into<String>) {
        self.append_full(turn, role, content, Vec::new(), None);
    }

    pub fn append_full(
        &mut self,
        turn: u64,
        role: RecordRole,
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
        tool_call_id: Option<String>,
    ) {
        self.hot.push(HotEntry {
            turn,
            role,
            content: content.into(),
            tool_calls,
            tool_call_id,
        });
    }

    /// The memory system fills the slot; assembly only places it
    /// (slot discipline, wall ch. 03).
    pub fn set_memory_slot(&mut self, text: String) {
        self.memory_slot = text;
    }

    pub fn needs_compaction(&self) -> bool {
        self.estimate_total() >= self.knobs.compaction_threshold * self.knobs.limit as f64
    }

    /// The compaction algorithm (wall ch. 03). Runs at most once per
    /// turn — the caller's discipline — and its result is accepted
    /// even if still over threshold (never re-trigger). Returns the
    /// lag warning to append through the normal message path, if the
    /// witness is far behind.
    pub fn compact(
        &mut self,
        workspace: &Path,
        fresh_system_prompt: String,
        current_turn: u64,
    ) -> anyhow::Result<Option<String>> {
        // 1. Identity edits take effect here.
        self.system_prompt = fresh_system_prompt;

        // 2. The witness cursor: the tail of the moves file.
        let cursor = record::witness_cursor(&record::moves_path(workspace))?;

        // 3. Drop whole turns at or below the cursor — they are
        //    represented in the arc. Never a partial turn.
        let (dropped, kept): (Vec<HotEntry>, Vec<HotEntry>) =
            self.hot.drain(..).partition(|e| e.turn <= cursor);
        self.hot = kept;

        // 4. Best-effort floor: backfill whole turns, newest first.
        let mut below_turns: Vec<Vec<HotEntry>> = Vec::new();
        let mut turns: Vec<u64> = dropped.iter().map(|e| e.turn).collect();
        turns.dedup();
        for turn in turns {
            below_turns.push(dropped.iter().filter(|e| e.turn == turn).cloned().collect());
        }
        self.backfill_floor(below_turns);

        // 5. Reload the arc within its budget.
        self.reload_arc(workspace)?;

        // 6. Refresh the memory slot (the memory system fills it; for
        //    now it is legitimately empty — assembly never blocks).
        self.memory_slot.clear();

        tracing::info!(
            cursor,
            kept_messages = self.hot.len(),
            estimate = self.estimate_total() as u64,
            "compacted: dropped whole turns at or below the witness cursor"
        );

        // 7. Never re-trigger: accept the result as-is.
        // 8. Lag warning.
        let midpoint =
            (self.knobs.fill_target + self.knobs.compaction_threshold) / 2.0 * self.knobs.limit as f64;
        let turns_behind = current_turn.saturating_sub(cursor);
        if self.estimate_total() > midpoint && turns_behind >= 10 {
            return Ok(Some(format!(
                "[system] Your compression is behind: the witness has compressed \
                 through turn {cursor}, and you are on turn {current_turn} — {turns_behind} \
                 turns ahead. Context is crowding. You may want to respond more \
                 briefly, or say so."
            )));
        }
        Ok(None)
    }

    /// Backfill whole turns (given newest-last) until min_messages is
    /// met, stopping if the next turn would push past the threshold.
    fn backfill_floor(&mut self, below_turns: Vec<Vec<HotEntry>>) {
        let threshold = self.knobs.compaction_threshold * self.knobs.limit as f64;
        let mut prepend: Vec<Vec<HotEntry>> = Vec::new();
        for turn_entries in below_turns.into_iter().rev() {
            if self.hot.len() + prepend.iter().map(Vec::len).sum::<usize>()
                >= self.knobs.min_messages as usize
            {
                break;
            }
            let added: f64 = turn_entries
                .iter()
                .map(|e| self.estimator.estimate(&e.content))
                .sum();
            if self.estimate_total() + added > threshold {
                break;
            }
            prepend.push(turn_entries);
        }
        // prepend holds newest-first; reassemble chronologically.
        let mut new_hot: Vec<HotEntry> = Vec::new();
        for turn_entries in prepend.into_iter().rev() {
            new_hot.extend(turn_entries);
        }
        new_hot.append(&mut self.hot);
        self.hot = new_hot;
    }

    /// Arc budget: moves newest-first until the fill target, then
    /// presented oldest-first. Old moves fall off here; they remain
    /// in the record.
    fn reload_arc(&mut self, workspace: &Path) -> anyhow::Result<()> {
        let all = record::read_moves(&record::moves_path(workspace))?;
        let budget = self.knobs.fill_target * self.knobs.limit as f64;
        let mut chosen: Vec<MoveLine> = Vec::new();
        let mut used = 0.0;
        for move_line in all.into_iter().rev() {
            let cost = self.estimator.estimate(&move_line.summary);
            if used + cost > budget && !chosen.is_empty() {
                break;
            }
            if used + cost > budget {
                break;
            }
            used += cost;
            chosen.push(move_line);
        }
        chosen.reverse();
        self.arc = chosen;
        Ok(())
    }

    /// Assemble for the model: system string in wall order, hot as
    /// the message list. Captures the estimate for calibration.
    pub fn messages(&mut self) -> (String, Vec<ChatMessage>) {
        let system = self.system_string();
        let list: Vec<ChatMessage> = self
            .hot
            .iter()
            .map(|e| match e.role {
                RecordRole::Assistant => {
                    ChatMessage::assistant(e.content.clone(), e.tool_calls.clone())
                }
                RecordRole::User | RecordRole::System => ChatMessage::user(e.content.clone()),
                RecordRole::Tool => match &e.tool_call_id {
                    Some(id) => ChatMessage::tool_result(id.clone(), e.content.clone()),
                    None => ChatMessage::user(format!("[tool result] {}", e.content)),
                },
            })
            .collect();
        self.last_estimate = self.estimate_total();
        (system, list)
    }

    fn system_string(&self) -> String {
        let mut system = self.system_prompt.clone();
        if !self.arc.is_empty() {
            system.push_str("\n\n[Conversation arc]\n");
            for move_line in &self.arc {
                system.push_str(&format!("turn {}: {}\n", move_line.turn, move_line.summary));
            }
        }
        if !self.memory_slot.is_empty() {
            system.push_str("\n\n[Memory]\n");
            system.push_str(&self.memory_slot);
        }
        system
    }

    /// Content plus tool-call payloads (wall ch. 03).
    pub fn estimate_total(&self) -> f64 {
        let mut total = self.estimator.estimate(&self.system_string());
        for entry in &self.hot {
            total += self.estimator.estimate(&entry.content);
            for call in &entry.tool_calls {
                total += self.estimator.estimate(&call.arguments);
            }
        }
        total
    }

    /// Feed the model-reported prompt token count back (wall ch. 03).
    pub fn calibrate(&mut self, reported_prompt_tokens: Option<u64>) {
        if let Some(reported) = reported_prompt_tokens {
            self.estimator.calibrate(reported, self.last_estimate);
        }
    }

    pub fn len(&self) -> usize {
        self.hot.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{RecordRole, TurnRecord};

    fn small_knobs() -> ContextConfig {
        ContextConfig {
            limit: 1000,
            compaction_threshold: 0.80,
            fill_target: 0.40,
            min_messages: 2,
        }
    }

    fn write_moves(workspace: &Path, moves: &[(u64, &str)]) {
        let path = record::moves_path(workspace);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = String::new();
        for (turn, summary) in moves {
            text.push_str(&format!(
                "{}\n",
                serde_json::json!({"id": ulid::Ulid::new().to_string(), "turn": turn, "summary": summary})
            ));
        }
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn estimator_base_and_calibration() {
        let mut est = Estimator::new();
        assert_eq!(est.estimate("abcdefgh"), 6.0); // 8/4 + 4
        est.calibrate(12, 6.0); // ratio ← 0.7 + 0.3·2 = 1.3
        assert!((est.ratio() - 1.3).abs() < 1e-9);
        est.calibrate(0, 6.0); // zero-skip
        assert!((est.ratio() - 1.3).abs() < 1e-9);
        est.calibrate(12, 0.0); // zero-skip
        assert!((est.ratio() - 1.3).abs() < 1e-9);
    }

    #[test]
    fn assembly_order_is_system_arc_memory_hot() {
        let dir = tempfile::tempdir().unwrap();
        write_moves(dir.path(), &[(1, "you greeted cass")]);
        let mut ctx = PersistentContext::build(
            dir.path(),
            "local_main",
            "IDENTITY".into(),
            small_knobs(),
        )
        .unwrap();
        ctx.append(2, RecordRole::User, "[local_main] cass: hi");

        let (system, list) = ctx.messages();
        let identity_pos = system.find("IDENTITY").unwrap();
        let arc_pos = system.find("[Conversation arc]").unwrap();
        assert!(identity_pos < arc_pos);
        assert!(system.contains("turn 1: you greeted cass"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].role, Role::User);
    }

    #[test]
    fn cursor_zero_drops_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // Tiny limit so ten messages genuinely crowd the context.
        let mut ctx = PersistentContext::build(
            dir.path(),
            "local_main",
            "sys".into(),
            ContextConfig {
                limit: 100,
                ..small_knobs()
            },
        )
        .unwrap();
        for turn in 1..=10 {
            ctx.append(turn, RecordRole::User, format!("message {turn}"));
        }
        let warning = ctx.compact(dir.path(), "sys".into(), 10).unwrap();
        assert_eq!(ctx.len(), 10, "lossless: cursor 0 means nothing droppable");
        // 10 turns ahead of cursor 0 and crowded → the lag warning fires.
        assert!(warning.is_some());
    }

    #[test]
    fn compaction_drops_whole_turns_at_or_below_cursor() {
        let dir = tempfile::tempdir().unwrap();
        write_moves(
            dir.path(),
            &[(1, "turn one happened"), (2, "turn two happened")],
        );
        let mut ctx = PersistentContext::build(
            dir.path(),
            "local_main",
            "sys".into(),
            ContextConfig {
                min_messages: 1,
                ..small_knobs()
            },
        )
        .unwrap();
        for turn in 1..=3u64 {
            ctx.append(turn, RecordRole::User, format!("q{turn}"));
            ctx.append(turn, RecordRole::Assistant, format!("a{turn}"));
        }

        ctx.compact(dir.path(), "sys".into(), 3).unwrap();

        // Cursor = 2: turns 1-2 dropped whole; turn 3 kept whole.
        assert_eq!(ctx.len(), 2);
        let (system, list) = ctx.messages();
        assert!(list.iter().all(|m| m.content.ends_with('3')));
        assert!(system.contains("turn 1: turn one happened"));
        assert!(system.contains("turn 2: turn two happened"));
    }

    #[test]
    fn floor_backfills_whole_turns_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        write_moves(dir.path(), &[(1, "one"), (2, "two"), (3, "three")]);
        let mut ctx = PersistentContext::build(
            dir.path(),
            "local_main",
            "sys".into(),
            ContextConfig {
                min_messages: 3,
                ..small_knobs()
            },
        )
        .unwrap();
        for turn in 1..=3u64 {
            ctx.append(turn, RecordRole::User, format!("q{turn}"));
            ctx.append(turn, RecordRole::Assistant, format!("a{turn}"));
        }

        ctx.compact(dir.path(), "sys".into(), 3).unwrap();

        // Everything is ≤ cursor (3): all droppable, but the floor
        // (3) backfills the newest whole turn (2 messages → still
        // under 3 → next turn too).
        assert_eq!(ctx.len(), 4);
        let (_, list) = ctx.messages();
        assert!(list[0].content.ends_with('2'), "whole turns, in order");
    }

    #[test]
    fn arc_budget_keeps_newest_presented_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let moves: Vec<(u64, String)> = (1..=50)
            .map(|i| (i, format!("move number {i} with some padding text")))
            .collect();
        let move_refs: Vec<(u64, &str)> =
            moves.iter().map(|(t, s)| (*t, s.as_str())).collect();
        write_moves(dir.path(), &move_refs);

        let mut ctx = PersistentContext::build(
            dir.path(),
            "local_main",
            "sys".into(),
            ContextConfig {
                limit: 200,
                ..small_knobs()
            },
        )
        .unwrap();

        let (system, _) = ctx.messages();
        assert!(system.contains("move number 50"), "newest always rides");
        assert!(!system.contains("move number 1\n"), "oldest fell off");
        let pos49 = system.find("move number 49");
        let pos50 = system.find("move number 50").unwrap();
        if let Some(pos49) = pos49 {
            assert!(pos49 < pos50, "presented oldest-first");
        }
    }

    #[test]
    fn build_collects_whole_touching_turns_from_record() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut rec = TurnRecord::open(dir.path()).unwrap();
            rec.append(1, "local_main", RecordRole::User, Some("q1")).unwrap();
            rec.append(1, "local_main", RecordRole::Assistant, Some("a1")).unwrap();
            rec.append(2, "discord_g", RecordRole::User, Some("q2")).unwrap();
            rec.append(2, "local_main", RecordRole::Assistant, Some("a2")).unwrap();
            rec.append(3, "discord_g", RecordRole::User, Some("q3")).unwrap();
        }

        let mut local = PersistentContext::build(
            dir.path(),
            "local_main",
            "sys".into(),
            small_knobs(),
        )
        .unwrap();
        // Turns 1 and 2 touch local_main; turn 3 does not.
        assert_eq!(local.len(), 4);
        let (_, list) = local.messages();
        assert!(list.iter().any(|m| m.content == "q2"), "whole turn rides in");

        let discord = PersistentContext::build(
            dir.path(),
            "discord_g",
            "sys".into(),
            small_knobs(),
        )
        .unwrap();
        // Turns 2 and 3 touch discord_g — turn 2 whole (2 entries) + turn 3.
        assert_eq!(discord.len(), 3);
    }

    #[test]
    fn calibration_uses_last_assembled_estimate() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = PersistentContext::build(
            dir.path(),
            "local_main",
            "12345678".into(),
            small_knobs(),
        )
        .unwrap();
        let (_, _) = ctx.messages();
        let estimated = ctx.estimate_total();
        ctx.calibrate(Some((estimated * 2.0) as u64));
        assert!(ctx.estimator.ratio() > 1.0);
        let before = ctx.estimator.ratio();
        ctx.calibrate(None);
        assert_eq!(ctx.estimator.ratio(), before);
    }
}
