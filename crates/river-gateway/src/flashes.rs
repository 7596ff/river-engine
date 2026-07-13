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
        // Send first; if send fails, don't log the receipt (avoid a
        // torn log line that describes a phantom frame — same
        // discipline as connect today).
        if let Err(e) = ctx.sender.try_send(frame.clone()) {
            tracing::warn!(
                turn = ctx.turn,
                error = %e,
                flash_type = frame.flash_type.as_str(),
                "flash frame send failed; dropping"
            );
            continue;
        }
        let entry = frame_to_log_entry(&frame);
        if let Err(e) = log::append(&ctx.state.log_path, &entry) {
            tracing::warn!(
                turn = ctx.turn,
                error = %e,
                "flash-log append failed"
            );
            continue;
        }
        // Update refractory only for successfully-appended entries.
        match frame.flash_type {
            FlashType::Echo => {
                ctx.state.echo_last.insert(frame.target_ref, frame.turn);
            }
            FlashType::Return => {
                ctx.state
                    .return_last
                    .insert(frame.target_ref, frame.turn);
            }
            FlashType::Bridge => {
                ctx.state
                    .bridge_last
                    .insert(frame.target_ref, frame.turn);
            }
            FlashType::Connection | FlashType::Correction => {}
        }
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
}
