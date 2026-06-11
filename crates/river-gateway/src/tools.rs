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
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::channels::Channels;
use crate::memory::Memory;
use crate::model::{ToolCall, ToolSchema};
use crate::turn::{LOCAL_ADAPTER, OutboundMessage};

const BASH_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_RESULT_BYTES: usize = 64 * 1024;

/// What tools need from the engine. Built per turn (the current
/// channel changes).
pub struct ToolContext {
    pub workspace: PathBuf,
    pub channels: Channels,
    pub outbound: broadcast::Sender<OutboundMessage>,
    pub current_channel: String,
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
            ],
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
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
            let mut cmd = tokio::process::Command::new("bash");
            cmd.arg("-c").arg(command).current_dir(&ctx.workspace);
            // The scrub: secrets never reach tool children.
            for var in &ctx.scrub {
                cmd.env_remove(var);
            }
            let output = tokio::time::timeout(BASH_TIMEOUT, cmd.output())
                .await
                .map_err(|_| anyhow::anyhow!("command timed out after {BASH_TIMEOUT:?}"))?
                .map_err(|e| anyhow::anyhow!("spawning bash: {e}"))?;
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

struct SpeakTool;
impl Tool for SpeakTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "speak".into(),
            description: "Say something on the current channel (or a named one). This is how anything you want heard gets delivered.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "channel": { "type": "string", "description": "optional override" }
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
            // Discord channels route to the adapter, which delivers
            // and logs post-acceptance; the platform msg_id (or the
            // error) comes back as tool-result text (wall ch. 06).
            if channel.starts_with(crate::discord::CHANNEL_PREFIX) {
                let Some(discord) = &ctx.discord else {
                    anyhow::bail!("no discord adapter configured");
                };
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                discord
                    .send(crate::discord::SpeakRequest {
                        channel: channel.clone(),
                        content: content.to_string(),
                        reply: reply_tx,
                    })
                    .await
                    .map_err(|_| anyhow::anyhow!("discord adapter is down"))?;
                let msg_id = tokio::time::timeout(Duration::from_secs(15), reply_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("discord delivery timed out"))?
                    .map_err(|_| anyhow::anyhow!("discord adapter dropped the request"))??;
                return Ok(format!("spoken on {channel} (msg {msg_id})"));
            }

            // Local: the broadcast is the delivery; the agent entry
            // doubles as the cursor (wall ch. 05).
            let _ = ctx.outbound.send(OutboundMessage {
                channel: channel.clone(),
                content: content.to_string(),
            });
            ctx.channels
                .agent_spoke(&channel, content, LOCAL_ADAPTER, None)?;
            Ok(format!("spoken on {channel}"))
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn ctx() -> (ToolContext, tempfile::TempDir, broadcast::Receiver<OutboundMessage>) {
        let dir = tempfile::tempdir().unwrap();
        let (notify_tx, _notify_rx) = mpsc::channel(16);
        let channels = Channels::open(dir.path(), notify_tx).unwrap();
        let (outbound, outbound_rx) = broadcast::channel(16);
        let ctx = ToolContext {
            workspace: dir.path().to_path_buf(),
            channels,
            outbound,
            current_channel: "local_main".into(),
            scrub: vec!["SECRET_KEY".into()],
            memory: None,
            reindex: None,
            discord: None,
        };
        (ctx, dir, outbound_rx)
    }

    async fn run(registry: &Registry, ctx: &ToolContext, name: &str, args: Value) -> String {
        let call = ToolCall {
            id: "t1".into(),
            name: name.into(),
            arguments: args.to_string(),
        };
        let profile: Vec<String> =
            ["read", "write", "edit", "glob", "grep", "bash", "speak"]
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
    async fn speak_delivers_and_logs() {
        let (ctx, _dir, mut outbound) = ctx();
        let registry = Registry::core();
        let out = run(&registry, &ctx, "speak", json!({"content":"good morning"})).await;
        assert_eq!(out, "spoken on local_main");
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
