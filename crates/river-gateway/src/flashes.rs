//! Flash subsystem (spec:
//! `docs/superpowers/specs/2026-07-13-flash-subsystem-design.md`).
//!
//! Absorbs the standalone connect duty into a per-turn flash pass:
//! one embed of the turn transcript feeds a shared candidate pool
//! for Connection/Echo/Return plus a shape-space pool for Bridge.
//! Each type's predicate module returns a `Vec<FlashFrame>`; every
//! qualifying candidate fires. The witness sends frames through
//! `flash_tx` and the turn loop appends `[flash: <type>] ...` as
//! system-role lines on `record/turns.jsonl` — the connect path's
//! single-writer discipline, generalized.
//!
//! Correction is stubbed. Danger is out of scope.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use river_core::config::{BridgeConfig, ConnectionConfig, EchoConfig, FlashConfig, ReturnConfig};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::memory::{Memory, SearchHit};
use crate::model::{Chat, ChatMessage};
use crate::shape::LoadedPrompt;

const NOTHING_TO_CONNECT: &str = "NOTHING_TO_CONNECT";

/// One flash event sent from the witness to the turn loop. The turn
/// loop's `build_flash_frame_body` renders the `body` field into a
/// system-role line on `record/turns.jsonl`.
#[derive(Debug, Clone)]
pub struct FlashFrame {
    pub turn: u64,
    pub channel: String,
    pub flash_type: FlashType,
    pub target_ref: String,
    pub target_path: PathBuf,
    pub score: f32,
    pub body: String,
    /// Bridge-only: the turn's shape gloss and the target's shape
    /// gloss, surfaced in the frame body and receipt log so the
    /// signal is legible after the fact.
    pub bridge_extras: Option<BridgeExtras>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashType {
    Connection,
    Echo,
    Return,
    Bridge,
    Correction,
}

impl FlashType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FlashType::Connection => "connection",
            FlashType::Echo => "echo",
            FlashType::Return => "return",
            FlashType::Bridge => "bridge",
            FlashType::Correction => "correction",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BridgeExtras {
    pub turn_shape: String,
    pub target_shape: String,
}

/// One receipt-log line for a fired flash. Bridge extras stay
/// optional so the row format is a superset for the four current
/// types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashLogEntry {
    #[serde(rename = "type")]
    pub flash_type: FlashType,
    pub turn: u64,
    pub channel: String,
    pub target_ref: String,
    pub target_path: String,
    pub score: f32,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_shape: Option<String>,
}

/// Per-type refractory state, loaded from the tail of
/// `witness/flashes.jsonl` at witness startup. Connection has no
/// refractory by design. Correction is stubbed.
#[derive(Debug, Default)]
pub struct State {
    pub config: FlashConfig,
    pub log_path: PathBuf,
    pub echo_last: HashMap<String, u64>,
    pub return_last: HashMap<String, u64>,
    pub bridge_last: HashMap<String, u64>,
}

impl State {
    pub fn new(config: FlashConfig, workspace: &Path) -> anyhow::Result<Self> {
        let log_path = workspace.join("witness").join("flashes.jsonl");
        log::migrate_legacy_connect_log(workspace)?;
        let mut state = Self {
            config,
            log_path,
            echo_last: HashMap::new(),
            return_last: HashMap::new(),
            bridge_last: HashMap::new(),
        };
        log::recover(&mut state)?;
        Ok(state)
    }
}

/// Everything a flash pass needs, gathered once at the call site.
pub struct FlashPassCtx<'a, C: Chat + Sync> {
    pub turn: u64,
    pub channel: String,
    pub transcript: String,
    pub memory: &'a Memory,
    pub workspace: &'a Path,
    pub client: &'a C,
    pub identity: &'a str,
    pub state: &'a mut State,
    /// `witness/flashes/on-connection.md` — Connection's compose-why
    /// prompt. `None` disables Connection.
    pub on_connection: Option<&'a str>,
    /// `witness/on-shape.md` — Bridge's gloss_turn prompt. `None`
    /// disables Bridge.
    pub shape_prompt: Option<&'a LoadedPrompt>,
    /// Paths the agent recently wrote via `write` / `edit` /
    /// `create_moment` / `write_atomic` — the self-write guard for
    /// Connection.
    pub recent_agent_writes: Vec<PathBuf>,
    pub sender: &'a mpsc::Sender<FlashFrame>,
}

/// Per settled turn. Reads the transcript, retrieves the shared
/// candidate pool, evaluates each enabled type's predicate, sends
/// every qualifying frame through `sender`, appends receipt lines
/// to `flashes.jsonl`, and updates per-target refractory state.
pub async fn flash_pass<C: Chat + Sync>(ctx: FlashPassCtx<'_, C>) -> anyhow::Result<()> {
    // Embed once. Threaded through both the shared pool scan and
    // Bridge's per-candidate text_sim check.
    let query_vec = ctx.memory.embed_query(&ctx.transcript).await?;

    // Shared pool for the text-sim types (Connection, Echo, Return).
    let pool = ctx
        .memory
        .search_no_bump_with_vec(&query_vec, ctx.state.config.top_k)
        .unwrap_or_default();

    let mut frames: Vec<FlashFrame> = Vec::new();

    if ctx.state.config.types.connection.enabled {
        frames.extend(
            types::connection::evaluate(
                &pool,
                &ctx.state.config.types.connection,
                ctx.turn,
                &ctx.channel,
                &ctx.recent_agent_writes,
                ctx.workspace,
                ctx.client,
                ctx.identity,
                ctx.on_connection,
                &ctx.transcript,
            )
            .await,
        );
    }
    if ctx.state.config.types.echo.enabled {
        frames.extend(types::echo::evaluate(
            &pool,
            &ctx.state.config.types.echo,
            ctx.turn,
            &ctx.channel,
            ctx.memory,
            &ctx.state.echo_last,
        ));
    }
    if ctx.state.config.types.return_.enabled {
        frames.extend(types::return_::evaluate(
            &pool,
            &ctx.state.config.types.return_,
            ctx.turn,
            &ctx.channel,
            ctx.memory,
            &ctx.state.return_last,
        ));
    }
    if ctx.state.config.types.bridge.enabled {
        let bridge_frames = types::bridge::evaluate(
            &ctx.state.config.types.bridge,
            ctx.turn,
            &ctx.channel,
            ctx.memory,
            ctx.workspace,
            ctx.client,
            ctx.identity,
            ctx.shape_prompt,
            &ctx.transcript,
            &query_vec,
            &ctx.state.bridge_last,
        )
        .await;
        frames.extend(bridge_frames);
    }
    // Correction is stubbed; its predicate is empty. Skip.

    for frame in frames {
        let turn = frame.turn;
        let flash_type = frame.flash_type;
        if let Err(e) = deliver_and_log(ctx.sender, ctx.state, frame) {
            tracing::warn!(
                turn,
                error = %e,
                flash_type = flash_type.as_str(),
                "flash delivery failed"
            );
        }
    }
    Ok(())
}

/// Send a frame, append its receipt, update refractory — in that
/// order, with each step guarding the next. The ordering is the
/// invariant: no log line describes an undelivered frame, no
/// refractory state marks a target as recently-fired without a
/// receipt to back it. Errors at any step short-circuit the rest so
/// state stays consistent.
fn deliver_and_log(
    sender: &mpsc::Sender<FlashFrame>,
    state: &mut State,
    frame: FlashFrame,
) -> anyhow::Result<()> {
    let entry = frame_to_log_entry(&frame);
    let target_ref = frame.target_ref.clone();
    let turn = frame.turn;
    let flash_type = frame.flash_type;
    sender
        .try_send(frame)
        .map_err(|e| anyhow::anyhow!("send failed: {e}"))?;
    log::append(&state.log_path, &entry)?;
    match flash_type {
        FlashType::Echo => {
            state.echo_last.insert(target_ref, turn);
        }
        FlashType::Return => {
            state.return_last.insert(target_ref, turn);
        }
        FlashType::Bridge => {
            state.bridge_last.insert(target_ref, turn);
        }
        FlashType::Connection | FlashType::Correction => {}
    }
    Ok(())
}

fn frame_to_log_entry(frame: &FlashFrame) -> FlashLogEntry {
    let (turn_shape, target_shape) = match &frame.bridge_extras {
        Some(extras) => (Some(extras.turn_shape.clone()), Some(extras.target_shape.clone())),
        None => (None, None),
    };
    FlashLogEntry {
        flash_type: frame.flash_type,
        turn: frame.turn,
        channel: frame.channel.clone(),
        target_ref: frame.target_ref.clone(),
        target_path: frame.target_path.display().to_string(),
        score: frame.score,
        at: jiff::Timestamp::now().to_string(),
        turn_shape,
        target_shape,
    }
}

pub mod signals {
    //! Pure signals over ambient state. All read-only.
    use std::collections::HashMap;

    /// Turns elapsed since a target's last cognitive access,
    /// approximated as (now_turn - last_touched_turn). Returns
    /// `u64::MAX` for unknown targets (never touched → maximally
    /// stale). Waiting for the Kanban's
    /// `activation.last_touched_turn` follow-up before Return can
    /// consume it.
    #[allow(dead_code)]
    pub fn staleness_turns(last_touched: &HashMap<String, u64>, target: &str, now_turn: u64) -> u64 {
        match last_touched.get(target) {
            Some(&last) => now_turn.saturating_sub(last),
            None => u64::MAX,
        }
    }
}

pub mod types {
    pub mod connection {
        use super::super::*;

        pub async fn evaluate<C: Chat + Sync>(
            pool: &[SearchHit],
            config: &ConnectionConfig,
            turn: u64,
            channel: &str,
            recent_writes: &[PathBuf],
            workspace: &Path,
            client: &C,
            identity: &str,
            on_connection: Option<&str>,
            transcript: &str,
        ) -> Vec<FlashFrame> {
            let Some(template) = on_connection else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for hit in pool {
                if hit.score < config.threshold {
                    continue;
                }
                if config.self_write_window > 0
                    && recent_writes.iter().any(|w| paths_match(w, &hit.file_path))
                {
                    continue;
                }
                let target_path = PathBuf::from(&hit.file_path);
                let target_ref = crate::memory::target_ref_for_path(&target_path);
                let body_head = load_target_head(workspace, &target_path);

                let prompt = template
                    .replace("{transcript}", transcript)
                    .replace("{target_path}", &hit.file_path)
                    .replace("{target_excerpt}", &hit.text);
                let messages = [ChatMessage::user(prompt)];
                let why = match client.chat(identity, &messages, &[]).await {
                    Ok(resp) => {
                        let text = resp.content.trim().to_string();
                        if text.is_empty() || text.eq_ignore_ascii_case(NOTHING_TO_CONNECT) {
                            continue;
                        }
                        text
                    }
                    Err(e) => {
                        tracing::warn!(
                            turn,
                            error = %e,
                            "connection compose failed; skipping"
                        );
                        continue;
                    }
                };
                let body = format!(
                    "[flash: connection] turn {turn} connects to [[{target_ref}]]: {why}\n\n{body_head}"
                );
                out.push(FlashFrame {
                    turn,
                    channel: channel.to_string(),
                    flash_type: FlashType::Connection,
                    target_ref,
                    target_path,
                    score: hit.score,
                    body,
                    bridge_extras: None,
                });
            }
            out
        }
    }

    pub mod echo {
        use super::super::*;

        pub fn evaluate(
            pool: &[SearchHit],
            config: &EchoConfig,
            turn: u64,
            channel: &str,
            memory: &Memory,
            last_fire: &HashMap<String, u64>,
        ) -> Vec<FlashFrame> {
            let mut out = Vec::new();
            for hit in pool {
                if hit.score < config.threshold {
                    continue;
                }
                let target_path = PathBuf::from(&hit.file_path);
                let target_ref = crate::memory::target_ref_for_path(&target_path);
                let warmth = warmth_for(memory, &target_ref);
                if warmth < config.warmth_min as f32 {
                    continue;
                }
                if config.min_new_turns_target > 0
                    && let Some(&last) = last_fire.get(&target_ref)
                    && turn.saturating_sub(last) < config.min_new_turns_target
                {
                    continue;
                }
                let body_head = load_target_head(memory.workspace_root(), &target_path);
                let body = format!(
                    "[flash: echo] turn {turn} echoes [[{target_ref}]] — you were thinking this recently.\n\n{body_head}"
                );
                out.push(FlashFrame {
                    turn,
                    channel: channel.to_string(),
                    flash_type: FlashType::Echo,
                    target_ref,
                    target_path,
                    score: hit.score,
                    body,
                    bridge_extras: None,
                });
            }
            out
        }
    }

    pub mod return_ {
        use super::super::*;

        pub fn evaluate(
            pool: &[SearchHit],
            config: &ReturnConfig,
            turn: u64,
            channel: &str,
            memory: &Memory,
            last_fire: &HashMap<String, u64>,
        ) -> Vec<FlashFrame> {
            let mut out = Vec::new();
            for hit in pool {
                if hit.score < config.threshold {
                    continue;
                }
                let target_path = PathBuf::from(&hit.file_path);
                let target_ref = crate::memory::target_ref_for_path(&target_path);
                let warmth = warmth_for(memory, &target_ref);
                if warmth > config.warmth_max as f32 {
                    continue;
                }
                if config.min_new_turns_target > 0
                    && let Some(&last) = last_fire.get(&target_ref)
                    && turn.saturating_sub(last) < config.min_new_turns_target
                {
                    continue;
                }
                let body_head = load_target_head(memory.workspace_root(), &target_path);
                let body = format!(
                    "[flash: return] turn {turn} returns to [[{target_ref}]] — you haven't been thinking about this.\n\n{body_head}"
                );
                out.push(FlashFrame {
                    turn,
                    channel: channel.to_string(),
                    flash_type: FlashType::Return,
                    target_ref,
                    target_path,
                    score: hit.score,
                    body,
                    bridge_extras: None,
                });
            }
            out
        }
    }

    pub mod bridge {
        use super::super::*;

        #[allow(clippy::too_many_arguments)]
        pub async fn evaluate<C: Chat + Sync>(
            config: &BridgeConfig,
            turn: u64,
            channel: &str,
            memory: &Memory,
            workspace: &Path,
            client: &C,
            identity: &str,
            shape_prompt: Option<&LoadedPrompt>,
            transcript: &str,
            query_vec: &[f32],
            last_fire: &HashMap<String, u64>,
        ) -> Vec<FlashFrame> {
            // Missing signal is silent: without on-shape.md we have
            // no way to gloss the turn.
            let Some(prompt) = shape_prompt else {
                return Vec::new();
            };
            let turn_shape = match crate::shape::gloss_turn(client, prompt, identity, transcript).await {
                Ok(g) if !g.is_empty() => g,
                Ok(_) => return Vec::new(),
                Err(e) => {
                    tracing::warn!(turn, error = %e, "bridge gloss_turn failed; skipping");
                    return Vec::new();
                }
            };
            let gloss_vec = match memory.embed(&turn_shape).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(turn, error = %e, "bridge gloss embed failed; skipping");
                    return Vec::new();
                }
            };
            let shape_hits = match memory.search_shapes(&gloss_vec, config.top_k) {
                Ok(hits) => hits,
                Err(e) => {
                    tracing::warn!(turn, error = %e, "search_shapes failed; skipping");
                    return Vec::new();
                }
            };

            let mut out = Vec::new();
            for (note_id, shape_sim) in shape_hits {
                if shape_sim < config.shape_sim_min {
                    continue;
                }
                // Look up the note's file_path via memory to compute
                // text_sim from segments.
                let Some(row) = memory.read_shape(&note_id).ok().flatten() else {
                    continue;
                };
                let text_sim = match memory.text_sim(&row.file_path, query_vec) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if text_sim > config.text_sim_max {
                    continue;
                }
                let target_path = workspace.join(&row.file_path);
                let target_ref = crate::memory::target_ref_for_path(&target_path);
                if config.min_new_turns_target > 0
                    && let Some(&last) = last_fire.get(&target_ref)
                    && turn.saturating_sub(last) < config.min_new_turns_target
                {
                    continue;
                }
                let target_shape = row.gloss.clone();
                let body_head = load_target_head(workspace, &target_path);
                let body = format!(
                    "[flash: bridge] turn {turn} and [[{target_ref}]] make the same move in different words.\n\n  shape: {turn_shape}\n  matches: {target_shape}\n\n{body_head}"
                );
                out.push(FlashFrame {
                    turn,
                    channel: channel.to_string(),
                    flash_type: FlashType::Bridge,
                    target_ref,
                    target_path,
                    score: shape_sim,
                    body,
                    bridge_extras: Some(BridgeExtras {
                        turn_shape: turn_shape.clone(),
                        target_shape,
                    }),
                });
            }
            out
        }
    }
}

pub mod log {
    use super::*;

    /// Idempotently migrate any legacy `witness/connect-log.jsonl`
    /// into `witness/flashes.jsonl` with `type: "connection"`. Runs
    /// once at witness startup; a subsequent run finds the new file
    /// already present and skips.
    pub fn migrate_legacy_connect_log(workspace: &Path) -> anyhow::Result<()> {
        let flashes = workspace.join("witness").join("flashes.jsonl");
        let legacy = workspace.join("witness").join("connect-log.jsonl");
        if flashes.exists() || !legacy.exists() {
            return Ok(());
        }
        let text = std::fs::read_to_string(&legacy)
            .with_context(|| format!("reading {}", legacy.display()))?;
        std::fs::create_dir_all(flashes.parent().unwrap())?;
        let mut migrated = String::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, line, "skipping malformed legacy connect-log line");
                    continue;
                }
            };
            let entry = FlashLogEntry {
                flash_type: FlashType::Connection,
                turn: value.get("turn").and_then(|v| v.as_u64()).unwrap_or(0),
                channel: value
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                target_ref: value
                    .get("target_ref")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                target_path: value
                    .get("target_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                score: value
                    .get("score")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                at: value
                    .get("at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                turn_shape: None,
                target_shape: None,
            };
            migrated.push_str(&serde_json::to_string(&entry)?);
            migrated.push('\n');
        }
        std::fs::write(&flashes, migrated)?;
        std::fs::remove_file(&legacy)?;
        tracing::info!("migrated legacy connect-log.jsonl → flashes.jsonl");
        Ok(())
    }

    /// Append one receipt line to `witness/flashes.jsonl`. fsync per
    /// line — same discipline as glean-log, shape-log.
    pub fn append(path: &Path, entry: &FlashLogEntry) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string(entry)?;
        json.push('\n');
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        file.write_all(json.as_bytes())?;
        file.sync_data()?;
        Ok(())
    }

    /// Bounded tail scan (last ~1000 lines) to rebuild per-target
    /// refractory maps. Torn/malformed lines skip with a warning.
    pub fn recover(state: &mut State) -> anyhow::Result<()> {
        let text = match std::fs::read_to_string(&state.log_path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", state.log_path.display())),
        };
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(1000);
        for line in &lines[start..] {
            if line.trim().is_empty() {
                continue;
            }
            let entry: FlashLogEntry = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(error = %err, line, "skipping malformed flashes.jsonl line");
                    continue;
                }
            };
            match entry.flash_type {
                FlashType::Echo => {
                    state.echo_last.insert(entry.target_ref, entry.turn);
                }
                FlashType::Return => {
                    state.return_last.insert(entry.target_ref, entry.turn);
                }
                FlashType::Bridge => {
                    state.bridge_last.insert(entry.target_ref, entry.turn);
                }
                FlashType::Connection | FlashType::Correction => {}
            }
        }
        Ok(())
    }
}

/// The activation table exposes warmth for a note (memory ch. 02).
/// This wraps the lookup so flashes.rs stays decoupled from
/// activation's internal representation.
fn warmth_for(memory: &Memory, target_ref: &str) -> f32 {
    memory
        .activation(target_ref)
        .ok()
        .flatten()
        .map(|v| v as f32)
        .unwrap_or(0.0)
}

/// The first ~200 words / ~1200 chars of a target file, used as the
/// tail of every flash body so the agent sees the target's opening
/// on the next turn.
fn load_target_head(workspace: &Path, target_path: &Path) -> String {
    let abs = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        workspace.join(target_path)
    };
    let text = std::fs::read_to_string(&abs).unwrap_or_default();
    let mut trimmed = String::new();
    for (i, c) in text.chars().enumerate() {
        if i >= 1200 {
            break;
        }
        trimmed.push(c);
    }
    trimmed
}

/// Path-equivalence with tolerance for common shapes (bare
/// filename, workspace-relative, absolute). Used by Connection's
/// self-write guard. Same shape as the current witness helper.
fn paths_match(a: &Path, b_str: &str) -> bool {
    let b = Path::new(b_str);
    if a == b {
        return true;
    }
    match (a.file_name(), b.file_name()) {
        (Some(af), Some(bf)) => af == bf,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::tests::FakeEmbedder;
    use crate::memory::Memory;
    use crate::model::{ChatMessage, ChatResponse, ToolSchema};
    use std::sync::Arc;

    /// Fake Chat that returns a scripted queue of replies. `chat`
    /// pops one per call; extra calls panic (a signal the test setup
    /// under-provisioned, which is itself a bug worth catching).
    struct ScriptedChat(std::sync::Mutex<Vec<String>>);

    impl ScriptedChat {
        fn new(replies: Vec<&str>) -> Self {
            Self(std::sync::Mutex::new(
                replies.into_iter().map(String::from).collect(),
            ))
        }
    }

    impl crate::model::Chat for ScriptedChat {
        async fn chat(
            &self,
            _system: &str,
            _messages: &[ChatMessage],
            _tools: &[ToolSchema],
        ) -> anyhow::Result<ChatResponse> {
            let mut q = self.0.lock().unwrap();
            if q.is_empty() {
                panic!("ScriptedChat exhausted — test asked for more replies than provided");
            }
            let content = q.remove(0);
            Ok(ChatResponse {
                content,
                tool_calls: Vec::new(),
                prompt_tokens: Some(0),
            })
        }
    }

    fn make_memory(dir: &Path) -> Memory {
        let workspace = dir.join("ws");
        std::fs::create_dir_all(workspace.join("knowledge")).unwrap();
        Memory::open(&dir.join("data"), &workspace, &[], Arc::new(FakeEmbedder)).unwrap()
    }

    fn make_state(log_path: PathBuf) -> State {
        State {
            config: FlashConfig::default(),
            log_path,
            echo_last: HashMap::new(),
            return_last: HashMap::new(),
            bridge_last: HashMap::new(),
        }
    }

    fn echo_frame(target: &str, turn: u64) -> FlashFrame {
        FlashFrame {
            turn,
            channel: "local_main".into(),
            flash_type: FlashType::Echo,
            target_ref: target.into(),
            target_path: PathBuf::from(format!("knowledge/{target}.md")),
            score: 0.9,
            body: "body".into(),
            bridge_extras: None,
        }
    }

    #[test]
    fn frame_to_log_entry_carries_bridge_extras() {
        let frame = FlashFrame {
            turn: 42,
            channel: "local_main".into(),
            flash_type: FlashType::Bridge,
            target_ref: "01T".into(),
            target_path: PathBuf::from("knowledge/01T.md"),
            score: 0.85,
            body: "body".into(),
            bridge_extras: Some(BridgeExtras {
                turn_shape: "a proxy diverges".into(),
                target_shape: "signal displaces state".into(),
            }),
        };
        let entry = frame_to_log_entry(&frame);
        assert_eq!(entry.flash_type, FlashType::Bridge);
        assert_eq!(entry.turn_shape.as_deref(), Some("a proxy diverges"));
        assert_eq!(entry.target_shape.as_deref(), Some("signal displaces state"));
    }

    #[test]
    fn frame_to_log_entry_omits_extras_for_non_bridge() {
        let frame = FlashFrame {
            turn: 1,
            channel: "local_main".into(),
            flash_type: FlashType::Echo,
            target_ref: "01T".into(),
            target_path: PathBuf::from("knowledge/01T.md"),
            score: 0.6,
            body: "body".into(),
            bridge_extras: None,
        };
        let entry = frame_to_log_entry(&frame);
        assert!(entry.turn_shape.is_none());
        assert!(entry.target_shape.is_none());
    }

    #[test]
    fn append_and_recover_rebuild_per_target_maps() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("witness/flashes.jsonl");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        let make = |ft: FlashType, target: &str, turn: u64| FlashLogEntry {
            flash_type: ft,
            turn,
            channel: "local_main".into(),
            target_ref: target.into(),
            target_path: format!("knowledge/{target}.md"),
            score: 0.7,
            at: "2026-07-13T00:00:00Z".into(),
            turn_shape: None,
            target_shape: None,
        };
        log::append(&log_path, &make(FlashType::Echo, "01A", 10)).unwrap();
        log::append(&log_path, &make(FlashType::Echo, "01A", 20)).unwrap();
        log::append(&log_path, &make(FlashType::Return, "01B", 15)).unwrap();
        log::append(&log_path, &make(FlashType::Bridge, "01C", 30)).unwrap();
        log::append(&log_path, &make(FlashType::Connection, "01D", 25)).unwrap();

        let mut state = State {
            config: FlashConfig::default(),
            log_path,
            echo_last: HashMap::new(),
            return_last: HashMap::new(),
            bridge_last: HashMap::new(),
        };
        log::recover(&mut state).unwrap();
        assert_eq!(state.echo_last.get("01A"), Some(&20), "last wins");
        assert_eq!(state.return_last.get("01B"), Some(&15));
        assert_eq!(state.bridge_last.get("01C"), Some(&30));
        // Connection has no refractory; no map to check.
    }

    #[test]
    fn migrate_legacy_connect_log_moves_and_tags_entries() {
        let dir = tempfile::tempdir().unwrap();
        let witness_dir = dir.path().join("witness");
        std::fs::create_dir_all(&witness_dir).unwrap();
        let legacy = witness_dir.join("connect-log.jsonl");
        std::fs::write(
            &legacy,
            "{\"turn\":5,\"channel\":\"local_main\",\"target_ref\":\"01A\",\"target_path\":\"knowledge/01A.md\",\"at\":\"t\"}\n",
        )
        .unwrap();
        log::migrate_legacy_connect_log(dir.path()).unwrap();

        assert!(!legacy.exists(), "legacy file removed");
        let new = witness_dir.join("flashes.jsonl");
        let text = std::fs::read_to_string(&new).unwrap();
        assert!(text.contains("\"type\":\"connection\""), "type tagged: {text}");
        assert!(text.contains("\"turn\":5"));

        // Second run is a no-op.
        log::migrate_legacy_connect_log(dir.path()).unwrap();
        assert!(!legacy.exists());
    }

    #[test]
    fn migrate_legacy_connect_log_skips_when_flashes_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let witness_dir = dir.path().join("witness");
        std::fs::create_dir_all(&witness_dir).unwrap();
        std::fs::write(witness_dir.join("connect-log.jsonl"), "junk\n").unwrap();
        std::fs::write(witness_dir.join("flashes.jsonl"), "").unwrap();
        log::migrate_legacy_connect_log(dir.path()).unwrap();
        assert!(witness_dir.join("connect-log.jsonl").exists(), "legacy untouched");
    }

    #[test]
    fn staleness_signal_returns_max_for_unknown_target() {
        let last = HashMap::new();
        assert_eq!(signals::staleness_turns(&last, "01A", 100), u64::MAX);
    }

    #[test]
    fn staleness_signal_computes_delta_for_known_target() {
        let mut last = HashMap::new();
        last.insert("01A".into(), 40);
        assert_eq!(signals::staleness_turns(&last, "01A", 100), 60);
        // now_turn < last_touched (shouldn't happen but must be safe):
        assert_eq!(signals::staleness_turns(&last, "01A", 20), 0);
    }

    // -------- hunt tests: deliver/log/refractory ordering --------
    //
    // These prove the "no phantom receipt" and "no false refractory"
    // invariants of deliver_and_log. Each names the drift it hunts.

    /// Hunts: someone reorders deliver_and_log so `log::append` runs
    /// before `try_send`, or ignores the send Err. Either would let
    /// flashes.jsonl accumulate receipts for frames that never
    /// reached the turn loop.
    #[test]
    fn deliver_and_log_send_failure_skips_log_and_refractory() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("flashes.jsonl");
        let mut state = make_state(log_path.clone());
        let (tx, rx) = mpsc::channel::<FlashFrame>(1);
        drop(rx); // closes the channel; every try_send fails with Closed.

        let err = deliver_and_log(&tx, &mut state, echo_frame("01A", 5)).unwrap_err();
        assert!(err.to_string().contains("send failed"), "err = {err}");

        // Log must not exist (never opened).
        assert!(!log_path.exists(), "log written despite send failure");
        // Refractory map must be untouched.
        assert!(state.echo_last.is_empty(), "refractory updated despite send failure");
    }

    /// Hunts: someone updates the refractory map before verifying the
    /// log append succeeded, causing a "recently fired" state with no
    /// receipt to back it — the target then never re-fires but no
    /// audit trail says why.
    #[test]
    fn deliver_and_log_log_failure_skips_refractory() {
        let dir = tempfile::tempdir().unwrap();
        // Point log_path at an existing DIRECTORY: OpenOptions::append
        // on a dir returns EISDIR, guaranteeing log::append fails.
        let bad_log = dir.path().join("nested_dir");
        std::fs::create_dir_all(&bad_log).unwrap();
        let mut state = make_state(bad_log);
        let (tx, mut rx) = mpsc::channel::<FlashFrame>(4);

        let err = deliver_and_log(&tx, &mut state, echo_frame("01A", 5)).unwrap_err();
        // Must be log-append flavored, not send-flavored.
        assert!(!err.to_string().contains("send failed"), "err classification wrong: {err}");
        // Frame reached the queue (send succeeded).
        assert!(rx.try_recv().is_ok(), "frame should have been sent before log attempted");
        // Refractory map untouched even though send worked.
        assert!(
            state.echo_last.is_empty(),
            "refractory updated despite log failure"
        );
    }

    // -------- hunt tests: recover + serde format --------

    /// Hunts: someone changes `warn+continue` on malformed lines to
    /// `bail!`, which would make the witness fail to start after one
    /// torn line at the tail. Also hunts silent-drop of valid entries
    /// that happen to appear after a bad line.
    #[test]
    fn recover_skips_malformed_lines_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("witness/flashes.jsonl");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        let good = |target: &str, turn: u64| {
            let entry = FlashLogEntry {
                flash_type: FlashType::Echo,
                turn,
                channel: "local_main".into(),
                target_ref: target.into(),
                target_path: format!("knowledge/{target}.md"),
                score: 0.7,
                at: "t".into(),
                turn_shape: None,
                target_shape: None,
            };
            serde_json::to_string(&entry).unwrap()
        };
        let mut body = String::new();
        body.push_str(&good("01A", 10));
        body.push('\n');
        body.push_str("{ this is not valid json\n");
        body.push_str(&good("01B", 20));
        body.push('\n');
        body.push_str("\n"); // empty line (already handled but confirm)
        body.push_str("garbage that isn't even an object\n");
        body.push_str(&good("01C", 30));
        body.push('\n');
        std::fs::write(&log_path, body).unwrap();

        let mut state = make_state(log_path);
        log::recover(&mut state).expect("recover must not bail on malformed lines");
        assert_eq!(state.echo_last.get("01A"), Some(&10));
        assert_eq!(state.echo_last.get("01B"), Some(&20));
        assert_eq!(state.echo_last.get("01C"), Some(&30));
        assert_eq!(state.echo_last.len(), 3, "no phantom entries from junk");
    }

    /// Hunts: someone removes or edits the `#[serde(rename_all =
    /// "lowercase")]` on FlashType, breaking every existing
    /// flashes.jsonl at recover time. Also hunts: someone renames a
    /// variant (Connection → Connect) without a migration.
    #[test]
    fn flash_type_serde_is_stable_lowercase() {
        assert_eq!(
            serde_json::from_str::<FlashType>("\"connection\"").unwrap(),
            FlashType::Connection
        );
        assert_eq!(
            serde_json::from_str::<FlashType>("\"echo\"").unwrap(),
            FlashType::Echo
        );
        assert_eq!(
            serde_json::from_str::<FlashType>("\"return\"").unwrap(),
            FlashType::Return
        );
        assert_eq!(
            serde_json::from_str::<FlashType>("\"bridge\"").unwrap(),
            FlashType::Bridge
        );
        assert_eq!(
            serde_json::from_str::<FlashType>("\"correction\"").unwrap(),
            FlashType::Correction
        );
        // Round-trip: variant → JSON → variant.
        for ft in [
            FlashType::Connection,
            FlashType::Echo,
            FlashType::Return,
            FlashType::Bridge,
            FlashType::Correction,
        ] {
            let j = serde_json::to_string(&ft).unwrap();
            let back: FlashType = serde_json::from_str(&j).unwrap();
            assert_eq!(ft, back, "round-trip mismatch: {j}");
            assert!(
                j.chars().all(|c| c == '"' || c.is_ascii_lowercase()),
                "expected lowercase, got {j}"
            );
        }
    }

    // -------- hunt tests: predicate boundaries --------

    fn make_hit(path: &str, score: f32) -> SearchHit {
        SearchHit {
            file_path: path.into(),
            text: "excerpt".into(),
            score,
        }
    }

    /// Hunts: `<` → `<=` (or vice versa) on the warmth gate in Echo.
    /// The predicate spec: warmth exactly at min → fires; warmth just
    /// below → skips. Also hunts the min_new_turns_target refractory
    /// boundary (turn - last == threshold should FIRE, meaning strict
    /// `<`).
    #[tokio::test]
    async fn echo_boundaries_fire_at_equality_and_skip_below() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        // Write a knowledge note so target_ref_for_path resolves and
        // load_target_head has content.
        std::fs::write(
            mem.workspace_root().join("knowledge/01A.md"),
            "---\nid: 01A\n---\nbody\n",
        )
        .unwrap();
        let mut config = EchoConfig::default();
        config.enabled = true;
        config.threshold = 0.5;
        config.warmth_min = 2.0;
        config.min_new_turns_target = 10;

        let pool = vec![make_hit("knowledge/01A.md", 0.9)];

        // Warmth just below min → skips.
        mem.set_activation_for_test("01A", 1.99).unwrap();
        let frames = types::echo::evaluate(&pool, &config, 100, "c", &mem, &HashMap::new());
        assert!(frames.is_empty(), "warmth < min must skip");

        // Warmth exactly at min → fires.
        mem.set_activation_for_test("01A", 2.0).unwrap();
        let frames = types::echo::evaluate(&pool, &config, 100, "c", &mem, &HashMap::new());
        assert_eq!(frames.len(), 1, "warmth == min must fire");

        // Refractory boundary. last_fire = 100, current turn 109
        // (delta 9 < threshold 10) → skips. current turn 110
        // (delta 10 == threshold) → fires.
        let mut last = HashMap::new();
        last.insert("01A".into(), 100u64);
        let frames = types::echo::evaluate(&pool, &config, 109, "c", &mem, &last);
        assert!(frames.is_empty(), "delta < min_new_turns_target must skip");
        let frames = types::echo::evaluate(&pool, &config, 110, "c", &mem, &last);
        assert_eq!(frames.len(), 1, "delta == min_new_turns_target must fire");
    }

    /// Hunts: `>` → `>=` on Return's warmth-max gate. Predicate spec:
    /// warmth exactly at max → fires; warmth just above → skips.
    #[tokio::test]
    async fn return_boundary_fires_at_equality_and_skips_above() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        std::fs::write(
            mem.workspace_root().join("knowledge/01A.md"),
            "---\nid: 01A\n---\nbody\n",
        )
        .unwrap();
        let mut config = ReturnConfig::default();
        config.enabled = true;
        config.threshold = 0.5;
        config.warmth_max = 3.0;
        config.min_new_turns_target = 0; // disable refractory for this test

        let pool = vec![make_hit("knowledge/01A.md", 0.9)];

        // Warmth just above max → skips.
        mem.set_activation_for_test("01A", 3.01).unwrap();
        let frames = types::return_::evaluate(&pool, &config, 100, "c", &mem, &HashMap::new());
        assert!(frames.is_empty(), "warmth > max must skip");

        // Warmth exactly at max → fires.
        mem.set_activation_for_test("01A", 3.0).unwrap();
        let frames = types::return_::evaluate(&pool, &config, 100, "c", &mem, &HashMap::new());
        assert_eq!(frames.len(), 1, "warmth == max must fire");
    }

    // -------- hunt tests: helpers --------

    /// Hunts: someone changes `chars().enumerate()` truncation to byte
    /// slicing, which panics on non-ASCII boundaries. Also hunts the
    /// 1200 constant flipping to a smaller value silently.
    #[test]
    fn load_target_head_utf8_safe_at_cap_and_short_file() {
        let dir = tempfile::tempdir().unwrap();
        // 2000 multi-byte chars (each 3 bytes). If truncation used
        // bytes, this would panic on a char boundary somewhere.
        let content: String = "字".repeat(2000);
        let path = dir.path().join("wide.md");
        std::fs::write(&path, &content).unwrap();

        let head = load_target_head(dir.path(), Path::new("wide.md"));
        assert_eq!(head.chars().count(), 1200, "cap is 1200 chars");
        // All chars remain the same multi-byte char — no torn boundary.
        assert!(head.chars().all(|c| c == '字'), "utf8 boundary torn");

        // Short file: whole thing.
        std::fs::write(dir.path().join("short.md"), "hi").unwrap();
        assert_eq!(load_target_head(dir.path(), Path::new("short.md")), "hi");
    }

    /// Hunts: someone changes `unwrap_or_default()` on the file read
    /// to `?` or `expect`, turning a missing target into a hard error
    /// that would kill the flash pass instead of quietly producing an
    /// empty tail.
    #[test]
    fn load_target_head_returns_empty_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let head = load_target_head(dir.path(), Path::new("nope/absent.md"));
        assert_eq!(head, "", "missing file must return empty, not panic");
    }

    /// Hunts: someone tightens paths_match (e.g., requiring absolute
    /// paths) and breaks the Connection self-write guard for
    /// workspace-relative recent_writes; or loosens it so different
    /// files with same stem collide unexpectedly.
    #[test]
    fn paths_match_equivalence_rules() {
        // Same path matches.
        assert!(paths_match(Path::new("knowledge/01A.md"), "knowledge/01A.md"));
        // Different directories, same filename: matches (deliberate —
        // the guard tolerates absolute vs relative renderings).
        assert!(paths_match(
            Path::new("/abs/knowledge/01A.md"),
            "knowledge/01A.md"
        ));
        // Different filenames: does NOT match.
        assert!(!paths_match(
            Path::new("knowledge/01A.md"),
            "knowledge/01B.md"
        ));
        // Root-ish path with no file_name doesn't spuriously match.
        assert!(!paths_match(Path::new("/"), "knowledge/01A.md"));
    }

    /// Hunts: someone changes eq_ignore_ascii_case → strict eq (so
    /// the witness saying "nothing_to_connect" or
    /// "Nothing_To_Connect" starts producing spurious frames), or
    /// drops the empty check (so blank replies land as blank flash
    /// bodies).
    #[tokio::test]
    async fn connection_skips_sentinel_and_whitespace_responses() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        std::fs::write(
            dir.path().join("knowledge/01A.md"),
            "---\nid: 01A\n---\nbody\n",
        )
        .unwrap();
        let mut config = ConnectionConfig::default();
        config.enabled = true;
        config.threshold = 0.5;
        // Four candidate hits, each triggers one chat call.
        let pool = vec![
            make_hit("knowledge/01A.md", 0.9),
            make_hit("knowledge/01A.md", 0.9),
            make_hit("knowledge/01A.md", 0.9),
            make_hit("knowledge/01A.md", 0.9),
        ];
        // Responses (in order): sentinel exact, sentinel lowercase,
        // pure whitespace, real content.
        let client = ScriptedChat::new(vec![
            "NOTHING_TO_CONNECT",
            "nothing_to_connect",
            "  \n\t  ",
            "the pattern from turn N shows up in [[01A]] as X",
        ]);
        let frames = types::connection::evaluate(
            &pool,
            &config,
            42,
            "c",
            &[],
            dir.path(),
            &client,
            "identity",
            Some("prompt template {transcript} {target_path} {target_excerpt}"),
            "transcript",
        )
        .await;
        assert_eq!(frames.len(), 1, "only the real response should produce a frame");
        assert!(
            frames[0].body.contains("the pattern from turn N"),
            "wrong frame kept: {}",
            frames[0].body
        );
    }

    /// Hunts: someone changes warmth_for to panic or return NaN on
    /// missing activation. NaN would defeat every warmth-gate
    /// comparison (`NaN < x` = false, `NaN > x` = false), producing
    /// silent spurious fires or silent silence.
    #[test]
    fn warmth_for_missing_activation_returns_zero_not_nan() {
        let dir = tempfile::tempdir().unwrap();
        let mem = make_memory(dir.path());
        let w = warmth_for(&mem, "01NEVER_SEEN");
        assert_eq!(w, 0.0);
        assert!(!w.is_nan(), "must not be NaN");
    }
}
