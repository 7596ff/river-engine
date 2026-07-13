//! Tools (wall ch. 07): how the agent acts. One structural rule —
//! the tool surface is per-agent configuration, not code. The
//! registry holds everything the engine can do; the profile names
//! what this agent's model is offered. Unprofiled tools are invisible
//! and uncallable. Failures become result text, never panics.
//!
//! `bash` children run with a scrubbed environment: every variable
//! named by a `*_env` config field is stripped before spawn.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::broadcast;

use crate::channels::Channels;
use crate::memory::Memory;
use crate::model::{ToolCall, ToolSchema};
use crate::turn::{LOCAL_ADAPTER, OutboundMessage};

const BASH_TIMEOUT: Duration = Duration::from_secs(300);
const BASH_TERM_GRACE: Duration = Duration::from_secs(2);
const MAX_RESULT_BYTES: usize = 64 * 1024;
const MAX_CHANNEL_READ_LIMIT: usize = 500;
const DEFAULT_CHANNEL_READ_LIMIT: usize = 50;
const MAX_READ_MOVES_TURNS: u64 = 200;

/// What tools need from the engine. Built per turn (the current
/// channel changes).
pub struct ToolContext {
    pub workspace: PathBuf,
    pub channels: Channels,
    pub outbound: broadcast::Sender<OutboundMessage>,
    pub current_channel: String,
    /// The turn number the agent is currently inside. `create_moment`
    /// uses it to refuse future-dated turn ranges.
    pub current_turn: u64,
    /// Secret variable names stripped from child environments.
    pub scrub: Vec<String>,
    /// The memory system, when the agent has an embedding model. The
    /// file tools are memory instruments through this seam (wall
    /// ch. 07); None disables capture and the search tool.
    pub memory: Option<Memory>,
    /// Nudges the sync service after watched writes.
    pub reindex: Option<tokio::sync::mpsc::Sender<()>>,
    /// Routes speak requests for discord_* channels to the adapter.
    pub discord: Option<tokio::sync::mpsc::Sender<crate::discord::SpeakRequest>>,
    /// Set on Wake::Digestion turns only; gives `reject_candidate` the
    /// id and text of the candidate the agent is currently digesting.
    pub digestion: Option<DigestionInfo>,
    /// Raised by `compact`; the turn loop forces a compaction at the
    /// start of the next turn even if the threshold hasn't tripped.
    pub compact_requested: Arc<AtomicBool>,
    /// Raised by `create_moment`; the turn loop refreshes the arc
    /// (re-scans `record/moments/`) before the next model call so the
    /// agent's just-written moment is visible without waiting for
    /// compaction.
    pub arc_dirty: Arc<AtomicBool>,
    /// Word-count cap enforced by `write_atomic` (wall ch. 02). From
    /// `config.atomic.max_words`; defaults to 100.
    pub atomic_max_words: usize,
    /// The shape worker's queue. `write_atomic` submits a Write job
    /// after successful writes; None when the shape subsystem is
    /// disabled or unwired (safe to skip — the sync sweep is an
    /// independent Source 4 path that will pick up the atomic on
    /// its next pass if the queue lands later).
    pub shape_queue: Option<tokio::sync::mpsc::Sender<crate::shape::GlossJob>>,
}

/// The candidate the agent is being asked to digest this turn. Carries
/// what `reject_candidate` needs to write an attributable entry into
/// `workspace/witness/rejections.jsonl`.
#[derive(Debug, Clone)]
pub struct DigestionInfo {
    pub candidate_id: String,
    pub candidate_text: String,
    pub turn: u64,
}

type ToolFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'a>>;

pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolSchema;
    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> ToolFuture<'a>;
}

pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

impl Registry {
    /// Every tool the engine ships (the `search` tool joins when the
    /// memory system constructs it).
    pub fn core() -> Self {
        Self {
            tools: vec![
                Box::new(ReadTool),
                Box::new(WriteTool),
                Box::new(EditTool),
                Box::new(GlobTool),
                Box::new(GrepTool),
                Box::new(BashTool),
                Box::new(SpeakTool),
                Box::new(SearchTool),
                Box::new(ChannelReadTool),
                Box::new(RejectCandidateTool),
                Box::new(CreateMomentTool),
                Box::new(WriteAtomicTool),
                Box::new(ReadMovesTool),
                Box::new(CompactTool),
            ],
        }
    }

    /// Schemas for the profiled tools only — what the model sees.
    pub fn schemas(&self, profile: &[String]) -> Vec<ToolSchema> {
        self.tools
            .iter()
            .map(|t| t.schema())
            .filter(|s| profile.iter().any(|p| p == &s.name))
            .collect()
    }

    /// Dispatch one call: profile-gated, timed, error-as-result.
    pub async fn execute(
        &self,
        call: &ToolCall,
        profile: &[String],
        ctx: &ToolContext,
    ) -> String {
        if !profile.iter().any(|p| p == &call.name) {
            return format!("error: tool {:?} is not available", call.name);
        }
        let Some(tool) = self
            .tools
            .iter()
            .find(|t| t.schema().name == call.name)
        else {
            return format!("error: tool {:?} is not available", call.name);
        };
        let arguments: Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => return format!("error: malformed arguments: {e}"),
        };
        let started = std::time::Instant::now();
        let result = match tool.execute(arguments, ctx).await {
            Ok(text) => truncate(text),
            Err(e) => format!("error: {e}"),
        };
        tracing::debug!(tool = %call.name, ms = started.elapsed().as_millis() as u64, "tool executed");
        result
    }
}

fn truncate(text: String) -> String {
    if text.len() <= MAX_RESULT_BYTES {
        return text;
    }
    let mut cut = MAX_RESULT_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n[... truncated at {} bytes]", &text[..cut], MAX_RESULT_BYTES)
}

/// Workspace-rooted path resolution: relative paths live in the
/// workspace; absolute paths are taken as given (the agent's bash
/// already reaches everything its user can).
fn resolve(workspace: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(p)
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    args[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing required argument {key:?}"))
}

struct ReadTool;
impl Tool for ReadTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read".into(),
            description: "Read a file. Relative paths are workspace-rooted.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = resolve(&ctx.workspace, required_str(&args, "path")?);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            // The capture seam: an indexed read is a cognitive access.
            if let Some(memory) = &ctx.memory {
                let _ = memory.on_read(&path);
            }
            Ok(text)
        })
    }
}

struct WriteTool;
impl Tool for WriteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write".into(),
            description: "Create or overwrite a file. Relative paths are workspace-rooted."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = resolve(&ctx.workspace, required_str(&args, "path")?);
            let content = required_str(&args, "content")?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)
                .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
            capture_write(ctx, &path);
            Ok(format!("wrote {} bytes to {}", content.len(), path.display()))
        })
    }
}

/// Watched writes bump and trigger re-indexing (wall ch. 07).
fn capture_write(ctx: &ToolContext, path: &Path) {
    if let Some(memory) = &ctx.memory
        && matches!(memory.on_write(path), Ok(true))
        && let Some(reindex) = &ctx.reindex
    {
        let _ = reindex.try_send(());
    }
}

struct EditTool;
impl Tool for EditTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "edit".into(),
            description: "Exact-string replacement in a file. old_string must occur exactly once.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = resolve(&ctx.workspace, required_str(&args, "path")?);
            let old = required_str(&args, "old_string")?;
            let new = required_str(&args, "new_string")?;
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            match text.matches(old).count() {
                0 => anyhow::bail!("old_string not found in {}", path.display()),
                1 => {}
                n => anyhow::bail!(
                    "old_string occurs {n} times in {}; provide more context to make it unique",
                    path.display()
                ),
            }
            std::fs::write(&path, text.replacen(old, new, 1))?;
            capture_write(ctx, &path);
            Ok(format!("edited {}", path.display()))
        })
    }
}

struct GlobTool;
impl Tool for GlobTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "glob".into(),
            description: "Find files by glob pattern, workspace-rooted.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "pattern": { "type": "string" } },
                "required": ["pattern"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let pattern = required_str(&args, "pattern")?;
            let full = if Path::new(pattern).is_absolute() {
                pattern.to_string()
            } else {
                format!("{}/{}", ctx.workspace.display(), pattern)
            };
            let mut hits: Vec<String> = Vec::new();
            for entry in glob::glob(&full).map_err(|e| anyhow::anyhow!("bad pattern: {e}"))? {
                if let Ok(path) = entry {
                    hits.push(path.display().to_string());
                }
            }
            if hits.is_empty() {
                Ok("no matches".to_string())
            } else {
                Ok(hits.join("\n"))
            }
        })
    }
}

struct GrepTool;
impl Tool for GrepTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "grep".into(),
            description: "Search file contents by regex under a directory (default: the workspace).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["pattern"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let re = regex::Regex::new(required_str(&args, "pattern")?)
                .map_err(|e| anyhow::anyhow!("bad regex: {e}"))?;
            let root = resolve(&ctx.workspace, args["path"].as_str().unwrap_or("."));
            let mut hits = Vec::new();
            grep_walk(&root, &re, &mut hits)?;
            if hits.is_empty() {
                Ok("no matches".to_string())
            } else {
                Ok(hits.join("\n"))
            }
        })
    }
}

fn grep_walk(path: &Path, re: &regex::Regex, hits: &mut Vec<String>) -> anyhow::Result<()> {
    if hits.len() >= 200 {
        return Ok(());
    }
    if path.is_dir() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" || name == "target" || name == "node_modules" {
            return Ok(());
        }
        for entry in std::fs::read_dir(path)? {
            grep_walk(&entry?.path(), re, hits)?;
        }
    } else if let Ok(text) = std::fs::read_to_string(path) {
        for (no, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hits.push(format!("{}:{}: {}", path.display(), no + 1, line));
                if hits.len() >= 200 {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

struct BashTool;

async fn run_bash(
    command: &str,
    workspace: &Path,
    scrub: &[String],
    timeout: Duration,
    term_grace: Duration,
) -> anyhow::Result<std::process::Output> {
    use std::os::unix::process::CommandExt as _;

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c")
        .arg(command)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A fresh process group gives timeout cleanup one address for Bash
    // and every ordinary descendant it spawns. Without this, dropping
    // the wait future leaves those processes running invisibly.
    cmd.as_std_mut().process_group(0);
    // Last-resort protection for the direct Bash child if group-level
    // signalling itself fails. Normal timeout cleanup still owns and
    // reaps the child explicitly below.
    cmd.kill_on_drop(true);
    for var in scrub {
        cmd.env_remove(var);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawning bash: {e}"))?;
    let process_group = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("spawned bash has no process id"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("capturing bash stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("capturing bash stderr"))?;
    let stdout_task = tokio::spawn(read_all(stdout));
    let stderr_task = tokio::spawn(read_all(stderr));
    let deadline = tokio::time::Instant::now() + timeout;
    let mut stdout_task = stdout_task;
    let mut stderr_task = stderr_task;

    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
        Ok(status) => status.map_err(|e| anyhow::anyhow!("waiting for bash: {e}"))?,
        Err(_) => {
            return Err(
                bash_timeout_error(
                    &mut child,
                    process_group,
                    term_grace,
                    timeout,
                    &mut stdout_task,
                    &mut stderr_task,
                )
                .await,
            );
        }
    };
    let streams = tokio::time::timeout_at(deadline, async {
        let stdout = join_output(&mut stdout_task, "stdout").await?;
        let stderr = join_output(&mut stderr_task, "stderr").await?;
        anyhow::Ok((stdout, stderr))
    })
    .await;
    let (stdout, stderr) = match streams {
        Ok(streams) => streams?,
        Err(_) => {
            return Err(
                bash_timeout_error(
                    &mut child,
                    process_group,
                    term_grace,
                    timeout,
                    &mut stdout_task,
                    &mut stderr_task,
                )
                .await,
            );
        }
    };
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn read_all<R>(mut reader: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

async fn join_output(
    task: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
) -> anyhow::Result<Vec<u8>> {
    task.await
        .map_err(|e| anyhow::anyhow!("joining bash {stream} reader: {e}"))?
        .map_err(|e| anyhow::anyhow!("reading bash {stream}: {e}"))
}

async fn bash_timeout_error(
    child: &mut tokio::process::Child,
    process_group: u32,
    term_grace: Duration,
    timeout: Duration,
    stdout_task: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> anyhow::Error {
    let cleanup = terminate_process_group(child, process_group, term_grace).await;
    // A deliberately detached descendant can leave the process group
    // and retain an inherited pipe. Stop our drain tasks after group
    // cleanup so such a writer cannot pin the timed-out tool forever.
    stdout_task.abort();
    stderr_task.abort();
    match cleanup {
        Ok(()) => anyhow::anyhow!("command timed out after {timeout:?}"),
        Err(e) => anyhow::anyhow!(
            "command timed out after {timeout:?}; process cleanup failed: {e:#}"
        ),
    }
}

async fn terminate_process_group(
    child: &mut tokio::process::Child,
    process_group: u32,
    term_grace: Duration,
) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    if let Err(e) = signal_process_group(process_group, libc::SIGTERM) {
        errors.push(format!("sending SIGTERM: {e}"));
    }
    tokio::time::sleep(term_grace).await;
    if let Err(e) = signal_process_group(process_group, libc::SIGKILL) {
        errors.push(format!("sending SIGKILL: {e}"));
    }

    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(e) = child.start_kill() {
                errors.push(format!("killing direct bash child: {e}"));
            }
            match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => errors.push(format!("reaping bash: {e}")),
                Err(_) => errors.push("reaping bash timed out".to_string()),
            }
        }
        Err(e) => errors.push(format!("checking bash status: {e}")),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(errors.join("; ")))
    }
}

fn signal_process_group(process_group: u32, signal: libc::c_int) -> std::io::Result<()> {
    let process_group: libc::pid_t = process_group
        .try_into()
        .map_err(|_| std::io::Error::other("bash process id does not fit pid_t"))?;
    // SAFETY: a negative pid addresses the Unix process group whose id
    // is the spawned Bash pid; `signal` is one of SIGTERM/SIGKILL.
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(()) // the whole group exited before this phase
    } else {
        Err(error)
    }
}

impl Tool for BashTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "bash".into(),
            description: "Run a shell command in the workspace.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let command = required_str(&args, "command")?;
            let output = run_bash(
                command,
                &ctx.workspace,
                &ctx.scrub,
                BASH_TIMEOUT,
                BASH_TERM_GRACE,
            )
            .await?;
            let mut result = String::new();
            result.push_str(&String::from_utf8_lossy(&output.stdout));
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                result.push_str("\n[stderr]\n");
                result.push_str(&stderr);
            }
            if !output.status.success() {
                result.push_str(&format!("\n[exit status: {}]", output.status));
            }
            if result.trim().is_empty() {
                result = "(no output)".to_string();
            }
            Ok(result)
        })
    }
}

/// Appended to every speak result: the cue lands at the exact moment
/// of the failure mode it prevents — a model continuing a conversation
/// whose other half has not arrived (and cannot, mid-turn).
const SPOKEN_CUE: &str =
    " — if you await a reply, end your turn; replies arrive as new turns";

struct SpeakTool;
impl Tool for SpeakTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "speak".into(),
            description: "Say something on the current channel (or a named one). This is how anything you want heard gets delivered. Discord channels also accept attachments: a list of workspace-relative file paths.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "channel": { "type": "string", "description": "optional override" },
                    "attachments": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "workspace-relative paths to attach (discord only)"
                    }
                },
                "required": ["content"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let content = required_str(&args, "content")?;
            let channel = args["channel"]
                .as_str()
                .unwrap_or(&ctx.current_channel)
                .to_string();
            let supplied_attachments: Vec<String> = match args.get("attachments") {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| anyhow::anyhow!("attachments entries must be strings"))
                    })
                    .collect::<anyhow::Result<_>>()?,
                Some(Value::Null) | None => Vec::new(),
                Some(_) => anyhow::bail!("attachments must be a list of paths"),
            };

            // Discord channels route to the adapter, which delivers
            // and logs post-acceptance; the platform msg_id (or the
            // error) comes back as tool-result text (wall ch. 06).
            if channel.starts_with(crate::discord::CHANNEL_PREFIX) {
                let Some(discord) = &ctx.discord else {
                    anyhow::bail!("no discord adapter configured");
                };
                let attachments =
                    resolve_outbound_attachments(&ctx.workspace, &supplied_attachments)?;
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                discord
                    .send(crate::discord::SpeakRequest {
                        channel: channel.clone(),
                        content: content.to_string(),
                        attachments,
                        reply: reply_tx,
                    })
                    .await
                    .map_err(|_| anyhow::anyhow!("discord adapter is down"))?;
                let msg_id = tokio::time::timeout(Duration::from_secs(60), reply_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("discord delivery timed out"))?
                    .map_err(|_| anyhow::anyhow!("discord adapter dropped the request"))??;
                return Ok(format!("spoken on {channel} (msg {msg_id}){SPOKEN_CUE}"));
            }

            // Local: no attachment support in v1; refuse before any
            // delivery so the channel-log entry isn't desynced from the
            // platform truth.
            if !supplied_attachments.is_empty() {
                anyhow::bail!("attachments are only supported on discord channels");
            }
            // Local: the broadcast is the delivery; the agent entry
            // doubles as the cursor (wall ch. 05).
            let _ = ctx.outbound.send(OutboundMessage {
                channel: channel.clone(),
                content: content.to_string(),
            });
            ctx.channels
                .agent_spoke(&channel, content, LOCAL_ADAPTER, None)?;
            Ok(format!("spoken on {channel}{SPOKEN_CUE}"))
        })
    }
}

fn resolve_outbound_attachments(
    workspace: &Path,
    supplied: &[String],
) -> anyhow::Result<Vec<crate::discord::OutboundAttachment>> {
    supplied
        .iter()
        .map(|rel| {
            let absolute = crate::channels::validate_outbound_path(workspace, rel)?;
            let filename = absolute
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("attachment has no filename: {rel:?}"))?
                .to_string();
            let mime = mime_for_extension(&filename);
            Ok(crate::discord::OutboundAttachment {
                absolute,
                relative: rel.clone(),
                filename,
                mime,
            })
        })
        .collect()
}

fn mime_for_extension(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

struct SearchTool;
impl Tool for SearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "search".into(),
            description: "Semantic search over the indexed workspace. Returns the most similar passages with file paths.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let query = required_str(&args, "query")?;
            let Some(memory) = &ctx.memory else {
                anyhow::bail!("no memory system configured (the agent has no embedding model)");
            };
            let hits = memory.search(query).await?;
            if hits.is_empty() {
                return Ok("no results".to_string());
            }
            let mut out = String::new();
            for hit in hits {
                out.push_str(&format!(
                    "{} (score {:.3})\n{}\n---\n",
                    hit.file_path, hit.score, hit.text
                ));
            }
            Ok(out)
        })
    }
}

/// Pure-peek window into a channel's history. Never mutates the
/// cursor, never notifies, never bumps activation. `before_id` /
/// `after_id` are engine ULIDs and mutually exclusive; the slicing
/// direction picks oldest-first or newest-first inside the window,
/// but the returned prose is always chronological so it reads in the
/// same shape the agent gets from auto-read at turn start.
struct ChannelReadTool;
impl Tool for ChannelReadTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "channel_read".into(),
            description: "Read entries from a channel's history without advancing the cursor. Defaults to the current channel and the tail of the log. Pass before_id to scroll back, after_id to scroll forward; they are mutually exclusive. Limit defaults to 50 and is capped at 500.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "channel_id": {
                        "type": "string",
                        "description": "engine channel name (e.g. discord_12345, local_main); defaults to the current channel"
                    },
                    "before_id": {
                        "type": "string",
                        "description": "engine ULID; return entries with id < before_id (backward pagination)"
                    },
                    "after_id": {
                        "type": "string",
                        "description": "engine ULID; return entries with id > after_id (forward pagination)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "max entries; default 50, hard cap 500"
                    }
                }
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let channel = args["channel_id"]
                .as_str()
                .unwrap_or(&ctx.current_channel)
                .to_string();
            let before_id = args.get("before_id").and_then(Value::as_str).map(str::to_string);
            let after_id = args.get("after_id").and_then(Value::as_str).map(str::to_string);
            if before_id.is_some() && after_id.is_some() {
                anyhow::bail!("before_id and after_id are mutually exclusive");
            }
            let requested = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_CHANNEL_READ_LIMIT)
                .max(1);
            let limit = requested.min(MAX_CHANNEL_READ_LIMIT);

            let entries = ctx.channels.scan(&channel)?;
            // Engine-internal entries don't belong to conversation.
            let conversational: Vec<_> = entries
                .into_iter()
                .filter(|e| !e.cursor && e.up_to.is_none())
                .collect();

            let windowed: Vec<_> = match (&before_id, &after_id) {
                (Some(b), _) => conversational.into_iter().filter(|e| &e.id < b).collect(),
                (_, Some(a)) => conversational.into_iter().filter(|e| &e.id > a).collect(),
                _ => conversational,
            };

            // after_id paginates forward: take the oldest `limit` after
            // the cursor. Otherwise take the newest `limit` of the
            // window (tail for default and before_id alike).
            let sliced: Vec<_> = if after_id.is_some() {
                windowed.into_iter().take(limit).collect()
            } else {
                let drop = windowed.len().saturating_sub(limit);
                windowed.into_iter().skip(drop).collect()
            };

            let mut out = String::new();
            out.push_str(&format_header(&channel, &sliced, requested, limit));
            for entry in &sliced {
                if let Some(content) = &entry.content {
                    let author = entry
                        .author
                        .as_deref()
                        .unwrap_or_else(|| match entry.role {
                            crate::channels::EntryRole::Agent => "(agent)",
                            crate::channels::EntryRole::Other => "unknown",
                        });
                    out.push('\n');
                    out.push_str(&crate::turn::format_inbound(
                        &channel,
                        author,
                        content,
                        &entry.attachments,
                    ));
                }
            }
            Ok(out)
        })
    }
}

fn format_header(
    channel: &str,
    sliced: &[crate::channels::ChannelEntry],
    requested: usize,
    limit: usize,
) -> String {
    if sliced.is_empty() {
        return format!("— channel {channel} (0 messages)");
    }
    let oldest = &sliced.first().unwrap().id;
    let newest = &sliced.last().unwrap().id;
    let count = sliced.len();
    if requested > limit {
        format!(
            "— channel {channel} ({count} messages, showing {limit} of {requested} requested, oldest: {oldest}, newest: {newest})"
        )
    } else {
        format!("— channel {channel} ({count} messages, oldest: {oldest}, newest: {newest})")
    }
}

/// Append one rejection entry to `workspace/witness/rejections.jsonl`,
/// creating the file (and parent dir) if absent. Returns the ISO-8601
/// timestamp written into the entry, so a downstream vector insert
/// can share it with the jsonl row.
pub(crate) fn append_rejection(
    workspace: &Path,
    candidate_id: &str,
    candidate_text: &str,
    reason: Option<&str>,
    turn: u64,
) -> anyhow::Result<String> {
    use std::io::Write as _;
    let path = workspace.join("witness").join("rejections.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    }
    let at = jiff::Timestamp::now().to_string();
    let mut obj = serde_json::Map::new();
    obj.insert(
        "candidate_id".into(),
        serde_json::Value::String(candidate_id.to_string()),
    );
    obj.insert(
        "candidate".into(),
        serde_json::Value::String(candidate_text.to_string()),
    );
    if let Some(reason) = reason {
        obj.insert(
            "reason".into(),
            serde_json::Value::String(reason.to_string()),
        );
    }
    obj.insert(
        "turn".into(),
        serde_json::Value::Number(serde_json::Number::from(turn)),
    );
    obj.insert("at".into(), serde_json::Value::String(at.clone()));
    let mut json = serde_json::to_string(&serde_json::Value::Object(obj))?;
    json.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?;
    file.write_all(json.as_bytes())
        .map_err(|e| anyhow::anyhow!("appending {}: {e}", path.display()))?;
    file.sync_data()
        .map_err(|e| anyhow::anyhow!("fsyncing {}: {e}", path.display()))?;
    Ok(at)
}

/// The agent's `reject_candidate` call: writes an attributable entry
/// to `rejections.jsonl` so the witness can read its prior misses.
/// Available only inside a `Wake::Digestion` turn — outside of one,
/// returns an error result.
struct RejectCandidateTool;
impl Tool for RejectCandidateTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "reject_candidate".into(),
            description: "Reject the current digestion candidate so your witness can learn from it. Optional reason — a short why. Available only inside a digestion turn.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "why this candidate didn't land — short, honest, the witness reads it"
                    }
                }
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let Some(digestion) = &ctx.digestion else {
                anyhow::bail!("reject_candidate is only valid inside a digestion turn");
            };
            let reason = args.get("reason").and_then(Value::as_str);
            let at = append_rejection(
                &ctx.workspace,
                &digestion.candidate_id,
                &digestion.candidate_text,
                reason,
                digestion.turn,
            )?;
            // Best-effort vector insert: the jsonl entry is the truth,
            // the vector is derived and rebuildable at startup. Failure
            // here logs and moves on; the tool result is unaffected.
            if let Some(memory) = &ctx.memory {
                if let Err(e) = memory
                    .insert_rejection_vector(
                        &digestion.candidate_id,
                        &digestion.candidate_text,
                        reason,
                        digestion.turn,
                        &at,
                    )
                    .await
                {
                    tracing::warn!(
                        candidate_id = %digestion.candidate_id,
                        error = %e,
                        "rejection vector insert failed; jsonl is authoritative"
                    );
                }
            }
            Ok(format!(
                "rejected candidate {} ({})",
                digestion.candidate_id,
                reason.unwrap_or("no reason given")
            ))
        })
    }
}

/// `create_moment` (wall ch. 03): write a moment file under
/// `record/moments/{id}.md`. The agent's own compression — replaces
/// the witness's moves for the covered turn range in the arc layer.
struct CreateMomentTool;
impl Tool for CreateMomentTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "create_moment".into(),
            description: "Write a moment: your own compression of a stretch of turns, in your voice. It will replace the witness's moves for the covered range in the arc. Range must be at least two turns and end at or before the current turn.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "turn_start": {
                        "type": "integer",
                        "description": "inclusive start of the covered turn range"
                    },
                    "turn_end": {
                        "type": "integer",
                        "description": "inclusive end; must be greater than turn_start and ≤ the current turn"
                    },
                    "body": {
                        "type": "string",
                        "description": "your compression — first person, what the stretch meant"
                    },
                    "links": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "optional ULIDs of atomic notes this moment cites"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "optional freeform tags"
                    }
                },
                "required": ["turn_start", "turn_end", "body"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let turn_start = args["turn_start"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("turn_start must be an integer"))?;
            let turn_end = args["turn_end"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("turn_end must be an integer"))?;
            if turn_end <= turn_start {
                anyhow::bail!("turn_end must be greater than turn_start (a moment covers at least two turns)");
            }
            if turn_end > ctx.current_turn {
                anyhow::bail!(
                    "turn_end ({turn_end}) is in the future — the current turn is {}",
                    ctx.current_turn
                );
            }
            let body = required_str(&args, "body")?;
            if body.trim().is_empty() {
                anyhow::bail!("body must be non-empty");
            }
            let links = parse_string_list(&args, "links")?;
            let tags = parse_string_list(&args, "tags")?;
            let id = ulid::Ulid::new().to_string();
            let moment = crate::moments::Moment {
                id: id.clone(),
                turn_start,
                turn_end,
                links,
                tags,
                body: body.to_string(),
                file_path: std::path::PathBuf::new(),
            };
            let path = crate::moments::write(&ctx.workspace, &moment)?;
            // The capture seam: a moment is an indexed write — let the
            // memory pipeline embed it on the next sweep.
            capture_write(ctx, &path);
            // The arc has a cached entry list; raise the dirty flag so
            // the turn loop re-scans moments before the next model
            // call. Without this, the moment is on disk but invisible
            // in arc until the next compaction.
            ctx.arc_dirty.store(true, Ordering::Relaxed);
            Ok(format!(
                "moment {id} written ({turn_start}–{turn_end})"
            ))
        })
    }
}

/// `write_atomic` (wall ch. 02): birth an atomic note into
/// `workspace/knowledge/`. Enforces the wall's atomic-note rules
/// (≤ `atomic.max_words` body, ≥1 typed link) that bare `write`
/// leaves unchecked. Assembles deterministic frontmatter, writes
/// atomically (tmp + fsync + rename), and submits a gloss job to
/// the shape worker if configured.
struct WriteAtomicTool;
impl Tool for WriteAtomicTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_atomic".into(),
            description: "Write a new atomic note under workspace/knowledge/. Body ≤100 words. At least one typed link is required. Auto-populates id (ULID) and created (RFC3339). Use for new claims; use `edit`/`write` for revisions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "body": {
                        "type": "string",
                        "description": "The claim, ≤ atomic.max_words words (default 100)."
                    },
                    "links": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string" },
                                "target": { "type": "string" }
                            },
                            "required": ["type", "target"]
                        },
                        "description": "Typed links; example: [{\"type\":\"extends\",\"target\":\"01JXX4PMRT...\"}]"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "shape": {
                        "type": "string",
                        "description": "Optional agent-authored shape gloss; overrides the witness's gloss in shape_vectors."
                    }
                },
                "required": ["body", "links"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let body = required_str(&args, "body")?;
            if body.trim().is_empty() {
                anyhow::bail!("body must be non-empty");
            }
            let word_count = body.split_whitespace().count();
            if word_count > ctx.atomic_max_words {
                anyhow::bail!(
                    "body is {word_count} words; limit is {}",
                    ctx.atomic_max_words
                );
            }
            let links = parse_typed_link_list(&args, "links")?;
            if links.is_empty() {
                anyhow::bail!("at least one typed link is required");
            }
            let tags = parse_string_list(&args, "tags")?;
            let shape = args.get("shape").and_then(|v| v.as_str()).map(str::to_string);

            let created = jiff::Timestamp::now().to_string();
            let (id, path, relative) = {
                let mut last_err = None;
                let mut chosen: Option<(String, PathBuf, String)> = None;
                for _ in 0..2 {
                    let id = ulid::Ulid::new().to_string();
                    let text =
                        assemble_atomic(&id, &created, body, &links, &tags, shape.as_deref());
                    match write_atomic_file(&ctx.workspace, &id, &text) {
                        Ok((p, r)) => {
                            chosen = Some((id, p, r));
                            break;
                        }
                        Err(e) => last_err = Some(e),
                    }
                }
                chosen.ok_or_else(|| {
                    last_err.unwrap_or_else(|| anyhow::anyhow!("ULID collision persisted"))
                })?
            };

            // The capture seam: the sync service picks up the new file
            // and re-indexes it on its own schedule.
            capture_write(ctx, &path);

            // Submit a Write job to the shape worker (spec §3, source
            // 3). Fire-and-forget: the tool returns as soon as the
            // file lands; the gloss happens on the worker's idle
            // schedule. If the queue is full or absent, the sync
            // sweep (source 4) picks the atomic up on its next pass.
            if let Some(sender) = &ctx.shape_queue {
                let _ = sender.try_send(crate::shape::GlossJob {
                    note_id: id.clone(),
                    note_path: relative.clone(),
                    reason: crate::shape::JobReason::Write,
                });
            }

            let known = known_atomic_stems(&ctx.workspace);
            let mut warnings: Vec<String> = Vec::new();
            for link in &links {
                if link.target != id && !known.contains(&link.target) {
                    warnings.push(format!("unresolved link target: {}", link.target));
                }
            }
            let result = json!({
                "id": id,
                "path": relative,
                "warnings": warnings,
            });
            Ok(result.to_string())
        })
    }
}

/// Assemble the atomic note's contents: deterministic YAML
/// frontmatter (`id, created, links, tags, shape` order; absent
/// optionals omitted) followed by the body.
fn assemble_atomic(
    id: &str,
    created: &str,
    body: &str,
    links: &[TypedLink],
    tags: &[String],
    shape: Option<&str>,
) -> String {
    let mut fm = String::from("---\n");
    fm.push_str(&format!("id: {id}\n"));
    fm.push_str(&format!("created: {created}\n"));
    fm.push_str("links:\n");
    for link in links {
        fm.push_str(&format!("  - {}: {}\n", link.link_type, link.target));
    }
    if !tags.is_empty() {
        fm.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    }
    if let Some(shape) = shape {
        fm.push_str(&format!("shape: {shape}\n"));
    }
    fm.push_str("---\n\n");
    fm.push_str(body);
    if !body.ends_with('\n') {
        fm.push('\n');
    }
    fm
}

/// Write the atomic to `workspace/knowledge/{id}.md` (tmp + fsync +
/// rename). Returns the absolute path and the workspace-relative
/// path.
fn write_atomic_file(
    workspace: &Path,
    id: &str,
    text: &str,
) -> anyhow::Result<(PathBuf, String)> {
    let dir = workspace.join("knowledge");
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
    let final_path = dir.join(format!("{id}.md"));
    if final_path.exists() {
        anyhow::bail!("atomic {id} already exists at {}", final_path.display());
    }
    let tmp = dir.join(format!(".{id}.tmp"));
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", tmp.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| anyhow::anyhow!("fsyncing {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &final_path)
        .map_err(|e| anyhow::anyhow!("renaming {}: {e}", tmp.display()))?;
    let relative = format!("knowledge/{id}.md");
    Ok((final_path, relative))
}

/// Collect atomic filename stems for advisory link-target
/// resolution. Filename-stem match only; frontmatter-id lookup
/// requires the memory index and lands when that's threaded
/// through. Ambiguity conducts nothing (wall ch. 02) — an
/// unresolved target is a warning, never an error.
fn known_atomic_stems(workspace: &Path) -> std::collections::HashSet<String> {
    let dir = workspace.join("knowledge");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return std::collections::HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                return None;
            }
            path.file_stem().and_then(|s| s.to_str()).map(str::to_string)
        })
        .collect()
}

#[derive(Debug, Clone)]
struct TypedLink {
    link_type: String,
    target: String,
}

fn parse_typed_link_list(args: &Value, key: &str) -> anyhow::Result<Vec<TypedLink>> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let obj = v
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("link {i}: must be an object"))?;
                let link_type = obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("link {i}: missing type"))?
                    .trim();
                if link_type.is_empty() {
                    anyhow::bail!("link {i}: type must be non-empty");
                }
                let target = obj
                    .get("target")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("link {i}: missing target"))?
                    .trim();
                if target.is_empty() {
                    anyhow::bail!("link {i}: target must be non-empty");
                }
                Ok(TypedLink {
                    link_type: link_type.to_string(),
                    target: target.to_string(),
                })
            })
            .collect(),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(_) => anyhow::bail!("{key} must be a list of link objects"),
    }
}

/// `read_moves` (wall ch. 03): scan `record/moves.jsonl` for moves in
/// a turn range. The agent's retrospective lookback for authoring
/// moments. Range capped at 200 turns.
struct ReadMovesTool;
impl Tool for ReadMovesTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_moves".into(),
            description: "Read the witness's moves for a turn range. Returns one line per turn that has a move, in ascending turn order, across all channels. Range size capped at 200 turns.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "turn_start": {
                        "type": "integer",
                        "description": "inclusive start of the range"
                    },
                    "turn_end": {
                        "type": "integer",
                        "description": "inclusive end of the range; must be ≥ turn_start and within 200 turns"
                    }
                },
                "required": ["turn_start", "turn_end"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let turn_start = args["turn_start"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("turn_start must be an integer"))?;
            let turn_end = args["turn_end"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("turn_end must be an integer"))?;
            if turn_end < turn_start {
                anyhow::bail!("turn_end must be ≥ turn_start");
            }
            let span = turn_end - turn_start + 1;
            if span > MAX_READ_MOVES_TURNS {
                anyhow::bail!(
                    "range too wide: {span} turns (max {MAX_READ_MOVES_TURNS})"
                );
            }

            // Channel attribution: query the turn index and build
            // a turn → facing-channel index. The "facing" channel is
            // the channel assistant/tool lines were tagged with for
            // that turn (wall ch. 10); fall back to whatever the turn
            // touched if no assistant line exists.
            let record_path = ctx.workspace.join("record").join("turns.jsonl");
            let lines = crate::record::scan_turn_range(&record_path, turn_start, turn_end)?;
            let mut facing: std::collections::HashMap<u64, String> =
                std::collections::HashMap::new();
            for line in &lines {
                let prefer = matches!(
                    line.role,
                    crate::record::RecordRole::Assistant | crate::record::RecordRole::Tool
                );
                facing
                    .entry(line.turn)
                    .and_modify(|c| {
                        if prefer {
                            *c = line.channel.clone();
                        }
                    })
                    .or_insert_with(|| line.channel.clone());
            }

            let moves_path = crate::record::moves_path(&ctx.workspace);
            let moves = crate::record::read_moves_range(&moves_path, turn_start, turn_end)?;
            let mut out = String::new();
            for m in moves {
                let channel = facing
                    .get(&m.turn)
                    .map(|s| s.as_str())
                    .unwrap_or("(unknown)");
                out.push_str(&format!("turn {} [{}]: {}\n", m.turn, channel, m.summary));
            }
            Ok(out)
        })
    }
}

/// Path of the cross-session handoff file. Written by `compact`,
/// consumed by the next session's startup (turn loop construction).
pub fn handoff_path(workspace: &Path) -> PathBuf {
    workspace.join("handoff.md")
}

/// `compact` (wall ch. 03): force a compaction at the next turn start
/// and leave a handoff message that the next session will see as the
/// first system-role record line after the restart.
struct CompactTool;
impl Tool for CompactTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "compact".into(),
            description: "Force a compaction at the start of the next turn and leave a handoff note for the next session. The summary lands as a system-role line at the head of the next session's hot — your voice to your future self. Call this when winding down or stepping away.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "what to tell the next session — open questions, where you left off, anything that should ride in"
                    }
                },
                "required": ["summary"]
            }),
        }
    }
    fn execute<'a>(&'a self, args: Value, ctx: &'a ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let summary = required_str(&args, "summary")?;
            if summary.trim().is_empty() {
                anyhow::bail!("summary must be non-empty");
            }
            let path = handoff_path(&ctx.workspace);
            // Atomic write so a kill mid-write doesn't leave a torn
            // handoff lying around for the next session.
            use std::io::Write as _;
            let tmp = path.with_extension("md.tmp");
            {
                let mut file = std::fs::File::create(&tmp)
                    .map_err(|e| anyhow::anyhow!("creating {}: {e}", tmp.display()))?;
                file.write_all(summary.as_bytes())
                    .map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
                file.sync_all()
                    .map_err(|e| anyhow::anyhow!("fsyncing {}: {e}", tmp.display()))?;
            }
            std::fs::rename(&tmp, &path)
                .map_err(|e| anyhow::anyhow!("renaming {}: {e}", tmp.display()))?;
            ctx.compact_requested.store(true, Ordering::Relaxed);
            Ok(format!(
                "handoff saved ({} bytes); compaction will run at the next turn start",
                summary.len()
            ))
        })
    }
}

fn parse_string_list(args: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("{key} entries must be strings"))
            })
            .collect(),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(_) => anyhow::bail!("{key} must be a list of strings"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn ctx() -> (ToolContext, tempfile::TempDir, broadcast::Receiver<OutboundMessage>) {
        let dir = tempfile::tempdir().unwrap();
        let (notify_tx, notify_rx) = mpsc::channel(16);
        // Leak the receiver so the notification queue stays open for
        // the test's lifetime; no inbound() call should fail because
        // its pointer had nowhere to go.
        Box::leak(Box::new(notify_rx));
        let channels = Channels::open(dir.path(), notify_tx).unwrap();
        let (outbound, outbound_rx) = broadcast::channel(16);
        let ctx = ToolContext {
            workspace: dir.path().to_path_buf(),
            channels,
            outbound,
            current_channel: "local_main".into(),
            current_turn: 100,
            scrub: vec!["SECRET_KEY".into()],
            memory: None,
            reindex: None,
            discord: None,
            digestion: None,
            compact_requested: Arc::new(AtomicBool::new(false)),
            arc_dirty: Arc::new(AtomicBool::new(false)),
            atomic_max_words: 100,
            shape_queue: None,
        };
        (ctx, dir, outbound_rx)
    }

    async fn run(registry: &Registry, ctx: &ToolContext, name: &str, args: Value) -> String {
        let call = ToolCall {
            id: "t1".into(),
            name: name.into(),
            arguments: args.to_string(),
        };
        let profile: Vec<String> = [
            "read",
            "write",
            "edit",
            "glob",
            "grep",
            "bash",
            "speak",
            "channel_read",
            "reject_candidate",
            "create_moment",
            "read_moves",
            "compact",
            "write_atomic",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        registry.execute(&call, &profile, ctx).await
    }

    #[tokio::test]
    async fn write_read_edit_round_trip() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "write", json!({"path":"notes/a.md","content":"hello teal"})).await;
        assert!(out.contains("wrote"), "{out}");
        let out = run(&registry, &ctx, "edit", json!({"path":"notes/a.md","old_string":"teal","new_string":"cyan"})).await;
        assert!(out.contains("edited"), "{out}");
        let out = run(&registry, &ctx, "read", json!({"path":"notes/a.md"})).await;
        assert_eq!(out, "hello cyan");
    }

    #[tokio::test]
    async fn edit_requires_unique_match() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        run(&registry, &ctx, "write", json!({"path":"a.txt","content":"x x"})).await;
        let out = run(&registry, &ctx, "edit", json!({"path":"a.txt","old_string":"x","new_string":"y"})).await;
        assert!(out.starts_with("error:"), "{out}");
        assert!(out.contains("2 times"), "{out}");
    }

    #[tokio::test]
    async fn glob_and_grep_find_content() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        run(&registry, &ctx, "write", json!({"path":"k/one.md","content":"the heron waits"})).await;
        run(&registry, &ctx, "write", json!({"path":"k/two.md","content":"the owl asks"})).await;
        let out = run(&registry, &ctx, "glob", json!({"pattern":"k/*.md"})).await;
        assert!(out.contains("one.md") && out.contains("two.md"), "{out}");
        let out = run(&registry, &ctx, "grep", json!({"pattern":"heron"})).await;
        assert!(out.contains("one.md:1"), "{out}");
        assert!(!out.contains("two.md"), "{out}");
    }

    #[tokio::test]
    async fn bash_runs_in_workspace_with_scrubbed_env() {
        let (ctx, dir, _) = ctx();
        let registry = Registry::core();
        // SAFETY: test-local env var, single-threaded enough.
        unsafe { std::env::set_var("SECRET_KEY", "sk-very-secret") };
        let out = run(&registry, &ctx, "bash", json!({"command":"pwd && echo key=${SECRET_KEY:-scrubbed}"})).await;
        assert!(out.contains(&dir.path().display().to_string()), "{out}");
        assert!(out.contains("key=scrubbed"), "{out}");
        unsafe { std::env::remove_var("SECRET_KEY") };
    }

    #[tokio::test]
    async fn bash_failure_is_result_text() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "bash", json!({"command":"exit 3"})).await;
        assert!(out.contains("exit status"), "{out}");
    }

    #[tokio::test]
    async fn bash_timeout_terminates_stubborn_process_tree() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived-timeout");
        let command = format!(
            "trap '' TERM; (trap '' TERM; sleep 0.5; printf survived > '{}') & wait",
            marker.display()
        );

        let error = run_bash(
            &command,
            dir.path(),
            &[],
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error:#}");

        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(
            !marker.exists(),
            "a descendant survived after the bash tool reported a timeout"
        );
    }

    #[tokio::test]
    async fn speak_delivers_and_logs() {
        let (ctx, _dir, mut outbound) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "speak", json!({"content":"good morning"})).await;
        assert!(out.starts_with("spoken on local_main"), "{out}");
        assert!(
            out.contains("end your turn"),
            "the settle cue rides every speak result: {out}"
        );
        assert_eq!(outbound.try_recv().unwrap().content, "good morning");
        let entries = ctx.channels.scan("local_main").unwrap();
        assert_eq!(entries[0].content.as_deref(), Some("good morning"));
        assert_eq!(entries[0].role, crate::channels::EntryRole::Agent);
    }

    #[tokio::test]
    async fn unprofiled_tool_is_invisible_and_uncallable() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let narrow: Vec<String> = vec!["read".into()];
        let schemas = registry.schemas(&narrow);
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "read");

        let call = ToolCall {
            id: "t1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        };
        let out = registry.execute(&call, &narrow, &ctx).await;
        assert!(out.contains("not available"), "{out}");
    }

    async fn seed_channel(
        ctx: &ToolContext,
        channel: &str,
        count: usize,
    ) -> Vec<String> {
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let id = ctx
                .channels
                .inbound(channel, "cass", None, &format!("msg {i}"), "local", None)
                .await
                .unwrap();
            ids.push(id);
        }
        ids
    }

    #[tokio::test]
    async fn channel_read_defaults_to_current_channel_tail() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let ids = seed_channel(&ctx, "local_main", 5).await;
        let out = run(&registry, &ctx, "channel_read", json!({})).await;
        assert!(out.contains("— channel local_main (5 messages"), "{out}");
        assert!(out.contains(&format!("oldest: {}", ids[0])), "{out}");
        assert!(out.contains(&format!("newest: {}", ids[4])), "{out}");
        assert!(out.contains("msg 0") && out.contains("msg 4"), "{out}");
    }

    #[tokio::test]
    async fn channel_read_before_id_paginates_backward() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let ids = seed_channel(&ctx, "local_main", 10).await;
        let out = run(
            &registry,
            &ctx,
            "channel_read",
            json!({"before_id": ids[5], "limit": 3}),
        )
        .await;
        assert!(out.contains("(3 messages"), "{out}");
        assert!(out.contains("msg 2") && out.contains("msg 3") && out.contains("msg 4"), "{out}");
        assert!(!out.contains("msg 5"), "before_id is exclusive: {out}");
    }

    #[tokio::test]
    async fn channel_read_after_id_paginates_forward() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let ids = seed_channel(&ctx, "local_main", 10).await;
        let out = run(
            &registry,
            &ctx,
            "channel_read",
            json!({"after_id": ids[5], "limit": 3}),
        )
        .await;
        assert!(out.contains("(3 messages"), "{out}");
        assert!(out.contains("msg 6") && out.contains("msg 7") && out.contains("msg 8"), "{out}");
        assert!(!out.contains("msg 5"), "after_id is exclusive: {out}");
        assert!(!out.contains("msg 9"), "forward slice takes the oldest 3: {out}");
    }

    #[tokio::test]
    async fn channel_read_rejects_both_bounds() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let ids = seed_channel(&ctx, "local_main", 3).await;
        let out = run(
            &registry,
            &ctx,
            "channel_read",
            json!({"before_id": ids[2], "after_id": ids[0]}),
        )
        .await;
        assert!(out.contains("mutually exclusive"), "{out}");
    }

    #[tokio::test]
    async fn channel_read_clamps_oversize_limit() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        seed_channel(&ctx, "local_main", 3).await;
        let out = run(&registry, &ctx, "channel_read", json!({"limit": 999})).await;
        assert!(out.contains("(3 messages"), "{out}");
        // limit > MAX_CHANNEL_READ_LIMIT (500) triggers the showing-of-requested note,
        // even when the actual return is smaller than either.
        assert!(out.contains("showing 500 of 999 requested"), "{out}");
    }

    #[tokio::test]
    async fn channel_read_empty_and_missing_channels_render_zero() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "channel_read", json!({"channel_id": "nope"})).await;
        assert_eq!(out, "— channel nope (0 messages)");
    }

    #[tokio::test]
    async fn channel_read_filters_engine_cursor_entries() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let ids = seed_channel(&ctx, "local_main", 2).await;
        ctx.channels.mark_read("local_main", &ids[1]).unwrap();
        let out = run(&registry, &ctx, "channel_read", json!({})).await;
        assert!(out.contains("(2 messages"), "explicit cursor entry filtered: {out}");
        assert!(!out.contains("up_to"), "{out}");
    }

    #[tokio::test]
    async fn channel_read_renders_agent_entries_with_agent_marker() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        ctx.channels
            .inbound("local_main", "cass", None, "hi", "local", None)
            .await
            .unwrap();
        ctx.channels
            .agent_spoke("local_main", "good morning", "local", None)
            .unwrap();
        let out = run(&registry, &ctx, "channel_read", json!({})).await;
        assert!(out.contains("[local_main] cass: hi"), "{out}");
        assert!(out.contains("[local_main] (agent): good morning"), "{out}");
    }

    #[tokio::test]
    async fn reject_candidate_appends_an_entry_with_reason() {
        let (mut ctx, dir, _) = ctx();
        ctx.digestion = Some(DigestionInfo {
            candidate_id: "01CAND".into(),
            candidate_text: "the witness gleaned this, badly".into(),
            turn: 42,
        });
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "reject_candidate",
            json!({"reason": "warm goodnight, not a claim"}),
        )
        .await;
        assert!(out.contains("rejected candidate 01CAND"), "{out}");
        let path = dir.path().join("witness/rejections.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        let entry: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(entry["candidate_id"], "01CAND");
        assert_eq!(entry["candidate"], "the witness gleaned this, badly");
        assert_eq!(entry["reason"], "warm goodnight, not a claim");
        assert_eq!(entry["turn"], 42);
        assert!(entry["at"].is_string());
    }

    #[tokio::test]
    async fn reject_candidate_omits_reason_when_absent() {
        let (mut ctx, dir, _) = ctx();
        ctx.digestion = Some(DigestionInfo {
            candidate_id: "01CAND".into(),
            candidate_text: "anything".into(),
            turn: 7,
        });
        let registry = Registry::core();
        let out = run(&registry, &ctx, "reject_candidate", json!({})).await;
        assert!(out.contains("no reason given"), "{out}");
        let text = std::fs::read_to_string(dir.path().join("witness/rejections.jsonl")).unwrap();
        let entry: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert!(entry.get("reason").is_none(), "{entry}");
    }

    #[tokio::test]
    async fn reject_candidate_outside_digestion_turn_errors() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "reject_candidate", json!({"reason": "n/a"})).await;
        assert!(out.contains("only valid inside a digestion turn"), "{out}");
    }

    #[tokio::test]
    async fn reject_candidate_called_multiple_times_appends_each() {
        let (mut ctx, dir, _) = ctx();
        ctx.digestion = Some(DigestionInfo {
            candidate_id: "01CAND".into(),
            candidate_text: "complicated".into(),
            turn: 9,
        });
        let registry = Registry::core();
        run(&registry, &ctx, "reject_candidate", json!({"reason": "first reason"})).await;
        run(&registry, &ctx, "reject_candidate", json!({"reason": "second reason"})).await;
        let text = std::fs::read_to_string(dir.path().join("witness/rejections.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("first reason"));
        assert!(text.contains("second reason"));
    }

    fn write_moves_file(workspace: &Path, moves: &[(u64, &str)]) {
        let dir = workspace.join("record");
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = String::new();
        for (turn, summary) in moves {
            text.push_str(&format!(
                "{}\n",
                json!({"id": ulid::Ulid::new().to_string(), "turn": turn, "summary": summary})
            ));
        }
        std::fs::write(dir.join("moves.jsonl"), text).unwrap();
    }

    #[tokio::test]
    async fn create_moment_writes_file_and_returns_id() {
        let (mut ctx, dir, _) = ctx();
        ctx.current_turn = 600;
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "create_moment",
            json!({
                "turn_start": 571,
                "turn_end": 575,
                "body": "I read the stretch and what stayed was the labor question.",
                "links": ["01JXP20260618164250197"],
                "tags": ["labor"]
            }),
        )
        .await;
        assert!(out.starts_with("moment ") && out.contains("(571–575)"), "{out}");
        let files: Vec<_> = std::fs::read_dir(dir.path().join("record/moments"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with(".md"));
    }

    #[tokio::test]
    async fn create_moment_rejects_inverted_range() {
        let (mut ctx, _dir, _) = ctx();
        ctx.current_turn = 100;
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "create_moment",
            json!({"turn_start": 10, "turn_end": 5, "body": "x"}),
        )
        .await;
        assert!(out.contains("greater than turn_start"), "{out}");
    }

    #[tokio::test]
    async fn create_moment_rejects_single_turn_range() {
        let (mut ctx, _dir, _) = ctx();
        ctx.current_turn = 100;
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "create_moment",
            json!({"turn_start": 7, "turn_end": 7, "body": "x"}),
        )
        .await;
        assert!(out.contains("at least two turns"), "{out}");
    }

    #[tokio::test]
    async fn create_moment_rejects_future_dated_range() {
        let (mut ctx, _dir, _) = ctx();
        ctx.current_turn = 20;
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "create_moment",
            json!({"turn_start": 18, "turn_end": 25, "body": "x"}),
        )
        .await;
        assert!(out.contains("in the future"), "{out}");
    }

    #[tokio::test]
    async fn create_moment_rejects_empty_body() {
        let (mut ctx, _dir, _) = ctx();
        ctx.current_turn = 100;
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "create_moment",
            json!({"turn_start": 1, "turn_end": 5, "body": "   \n  "}),
        )
        .await;
        assert!(out.contains("body must be non-empty"), "{out}");
    }

    #[tokio::test]
    async fn read_moves_returns_range_sorted_with_channel() {
        let (ctx, dir, _) = ctx();
        write_moves_file(
            dir.path(),
            &[(1, "first"), (2, "second"), (3, "third"), (4, "fourth")],
        );
        // Seed turns.jsonl so channel attribution works.
        let mut rec = crate::record::TurnRecord::open(dir.path()).unwrap();
        for turn in 1..=4u64 {
            rec.append(turn, "discord_general", crate::record::RecordRole::User, Some("q"))
                .unwrap();
            rec.append(turn, "discord_general", crate::record::RecordRole::Assistant, Some("a"))
                .unwrap();
        }
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "read_moves",
            json!({"turn_start": 2, "turn_end": 3}),
        )
        .await;
        assert!(out.contains("turn 2 [discord_general]: second"), "{out}");
        assert!(out.contains("turn 3 [discord_general]: third"), "{out}");
        assert!(!out.contains("turn 1"), "{out}");
        assert!(!out.contains("turn 4"), "{out}");
    }

    #[tokio::test]
    async fn read_moves_caps_at_200_turns() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "read_moves",
            json!({"turn_start": 1, "turn_end": 500}),
        )
        .await;
        assert!(out.contains("range too wide"), "{out}");
    }

    #[tokio::test]
    async fn read_moves_empty_range_is_empty_string() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "read_moves",
            json!({"turn_start": 1, "turn_end": 5}),
        )
        .await;
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn compact_writes_handoff_and_raises_flag() {
        let (ctx, dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "compact",
            json!({"summary": "left off mid-thread on the labor question; resume there."}),
        )
        .await;
        assert!(out.contains("handoff saved"), "{out}");
        assert!(ctx.compact_requested.load(Ordering::Relaxed));
        let text = std::fs::read_to_string(dir.path().join("handoff.md")).unwrap();
        assert!(text.contains("labor question"));
    }

    #[tokio::test]
    async fn compact_rejects_empty_summary() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "compact", json!({"summary": "  \n  "})).await;
        assert!(out.contains("summary must be non-empty"), "{out}");
        assert!(!ctx.compact_requested.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn write_atomic_writes_file_and_returns_id() {
        let (ctx, dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "Reason requires agreed-upon names. Without settlement of names, reckoning produces different results for each party.",
                "links": [{"type": "extends", "target": "01JXX4PMRT4V2S1J7K0H6E9P8B"}],
                "tags": ["names", "reason"]
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("expected JSON result, got: {out}"));
        let id = parsed["id"].as_str().expect("id in result");
        let path = parsed["path"].as_str().expect("path in result");
        assert!(id.len() == 26, "ULID length: {id}");
        assert_eq!(path, format!("knowledge/{id}.md"));
        assert!(parsed["warnings"].is_array(), "warnings must be an array");

        let full = dir.path().join(path);
        let text = std::fs::read_to_string(&full).expect("file exists");
        assert!(text.contains(&format!("id: {id}")));
        assert!(text.contains("extends: 01JXX4PMRT4V2S1J7K0H6E9P8B"));
        assert!(text.contains("Reason requires"));
    }

    #[tokio::test]
    async fn write_atomic_submits_shape_job_when_queue_present() {
        let (mut ctx, _dir, _) = ctx();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        ctx.shape_queue = Some(tx);
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "a claim.",
                "links": [{"type": "extends", "target": "01X"}]
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        let id = parsed["id"].as_str().unwrap();
        let job = rx.try_recv().expect("job submitted");
        assert_eq!(job.note_id, id);
        assert_eq!(job.note_path, format!("knowledge/{id}.md"));
        assert_eq!(job.reason, crate::shape::JobReason::Write);
    }

    #[tokio::test]
    async fn write_atomic_no_gloss_when_shape_queue_absent() {
        // Baseline: default ctx has shape_queue = None; write still
        // succeeds. Exercised trivially by every other write_atomic
        // test; this documents the contract explicitly.
        let (ctx, _dir, _) = ctx();
        assert!(ctx.shape_queue.is_none());
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "a claim.",
                "links": [{"type": "extends", "target": "01X"}]
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert!(parsed["id"].is_string(), "write succeeds without queue: {out}");
    }

    #[tokio::test]
    async fn write_atomic_warns_on_unresolved_target() {
        let (ctx, dir, _) = ctx();
        // Pre-create one atomic so its stem resolves.
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        std::fs::write(
            dir.path().join("knowledge/01EXISTS.md"),
            "---\nid: 01EXISTS\n---\n\nbody\n",
        )
        .unwrap();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "a claim.",
                "links": [
                    {"type": "extends", "target": "01EXISTS"},
                    {"type": "contradicts", "target": "01MISSING"}
                ]
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        let warnings = parsed["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "one unresolved target: {out}");
        assert!(warnings[0].as_str().unwrap().contains("01MISSING"), "{out}");
    }

    #[tokio::test]
    async fn write_atomic_frontmatter_key_order_and_omits_optionals() {
        let (ctx, dir, _) = ctx();
        let registry = Registry::core();
        // No tags, no shape — both should be absent from the file.
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "a claim.",
                "links": [{"type": "extends", "target": "01X"}]
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        let path = dir.path().join(parsed["path"].as_str().unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        let id_pos = text.find("id:").expect("id present");
        let created_pos = text.find("created:").expect("created present");
        let links_pos = text.find("links:").expect("links present");
        assert!(id_pos < created_pos, "id before created");
        assert!(created_pos < links_pos, "created before links");
        assert!(!text.contains("tags:"), "tags omitted when absent");
        assert!(!text.contains("shape:"), "shape omitted when absent");
    }

    #[tokio::test]
    async fn write_atomic_writes_shape_frontmatter_when_provided() {
        let (ctx, dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "a claim.",
                "links": [{"type": "extends", "target": "01X"}],
                "shape": "a proxy under optimization pressure diverges from the target it stood for"
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        let path = dir.path().join(parsed["path"].as_str().unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("shape: a proxy under optimization pressure"), "{text}");
        // shape must appear after tags position (last field before ---).
        let shape_pos = text.find("shape:").unwrap();
        let close_pos = text.rfind("---").unwrap();
        assert!(shape_pos < close_pos, "shape inside frontmatter");
    }

    #[tokio::test]
    async fn write_atomic_leaves_no_tmp_file() {
        let (ctx, dir, _) = ctx();
        let registry = Registry::core();
        run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": "a claim.",
                "links": [{"type": "extends", "target": "01X"}]
            }),
        )
        .await;
        let leftover: Vec<_> = std::fs::read_dir(dir.path().join("knowledge"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with('.') && n.ends_with(".tmp"))
            .collect();
        assert!(leftover.is_empty(), "tmp file survived rename: {leftover:?}");
    }

    #[tokio::test]
    async fn write_atomic_rejects_no_links() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({"body": "a claim.", "links": []}),
        )
        .await;
        assert!(out.contains("at least one typed link is required"), "{out}");
    }

    #[tokio::test]
    async fn write_atomic_rejects_malformed_link_missing_type() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({"body": "a claim.", "links": [{"target": "01X"}]}),
        )
        .await;
        assert!(out.contains("missing type"), "{out}");
    }

    #[tokio::test]
    async fn write_atomic_rejects_malformed_link_empty_target() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({"body": "a claim.", "links": [{"type": "extends", "target": "  "}]}),
        )
        .await;
        assert!(out.contains("target"), "{out}");
    }

    #[tokio::test]
    async fn write_atomic_rejects_over_word_limit() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let body = "word ".repeat(101);
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": body,
                "links": [{"type": "extends", "target": "01X"}]
            }),
        )
        .await;
        assert!(out.contains("101 words") && out.contains("limit is 100"), "{out}");
    }

    #[tokio::test]
    async fn write_atomic_respects_config_max_words() {
        let (mut ctx, _dir, _) = ctx();
        ctx.atomic_max_words = 200;
        let registry = Registry::core();
        let body = "word ".repeat(150);
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({
                "body": body,
                "links": [{"type": "extends", "target": "01X"}]
            }),
        )
        .await;
        let parsed: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("expected success, got: {out}"));
        assert!(parsed["id"].is_string(), "{out}");
    }

    #[tokio::test]
    async fn write_atomic_rejects_empty_body() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let out = run(
            &registry,
            &ctx,
            "write_atomic",
            json!({"body": "  \n  ", "links": [{"type": "extends", "target": "01X"}]}),
        )
        .await;
        assert!(out.contains("body must be non-empty"), "{out}");
    }

    #[tokio::test]
    async fn malformed_arguments_become_error_text() {
        let (ctx, _dir, _) = ctx();
        let registry = Registry::core();
        let call = ToolCall {
            id: "t1".into(),
            name: "read".into(),
            arguments: "not json".into(),
        };
        let profile = vec!["read".to_string()];
        let out = registry.execute(&call, &profile, &ctx).await;
        assert!(out.contains("malformed arguments"), "{out}");
    }
}
