# River Oneshot: Turn-Based Dual-Loop Agent

## Overview

A **turn-based oneshot Rust program** with dual-loop architecture, ported from OpenClaw concepts. The program launches two concurrent loops — reasoning and execution — that both run to completion. When either produces output, the program returns the result and prompts the user for another cycle.

---

## Architecture

### Core Concept: Dual Completing Loops

```
User Input
    │
    ▼
┌────────────────────────────────┐
│          Runtime.cycle()       │
│                                │
│  ┌──────────┐  ┌───────────┐  │
│  │  Loop A  │  │  Loop B   │  │
│  │ Reasoning│  │ Execution │  │
│  │          │  │           │  │
│  │ LLM call │  │ Run skill │  │
│  │ → Plan   │  │ → Result  │  │
│  └────┬─────┘  └─────┬─────┘  │
│       │               │        │
│       └───────────────┘        │
│               │                │
│    Both run to completion      │
│    First ready → output        │
│    Other → cached for next     │
│               │                │
└───────────────┬────────────────┘
                ▼
        TurnOutput returned
        to user, program waits
                │
                ▼
        User: [enter] continue
              [q] quit
              or type new input
                │
                ▼
        next cycle begins
```

### Why Two Loops?

- LLM reasoning can be slow. If a previously queued skill (HTTP fetch, file read) finishes first, the user sees that result immediately.
- If nothing is pending for execution, the reasoning loop completes first and proposes the next action.
- Creates a natural rhythm: sometimes the agent is "thinking ahead," sometimes it's "catching up on results."

### Both Loops Complete

Unlike a cancellation-based race, **both loops always run to completion**. The first to finish becomes the cycle's output; the other's result is cached for the next cycle. This ensures:
- No wasted LLM tokens from cancelled requests
- No orphaned skill executions
- No cleanup complexity from mid-flight cancellation

---

## Module Layout

```
river-oneshot/
├── Cargo.toml
├── PLAN.md
├── src/
│   ├── main.rs              # Entry point, cycle pump, user I/O
│   ├── runtime.rs           # Runtime struct, cycle() method, completion logic
│   ├── config.rs            # Config loading (workspace, credentials, model)
│   ├── context.rs           # CycleInput, reasoning/execution context builders
│   ├── memory.rs            # Memory integration (uses river-db abstractions)
│   ├── llm/
│   │   ├── mod.rs           # LlmProvider trait
│   │   ├── claude.rs        # Anthropic Claude backend
│   │   ├── openai.rs        # OpenAI backend
│   │   └── ollama.rs        # Local Ollama backend
│   ├── skills/
│   │   ├── mod.rs           # Skill trait, SkillRegistry
│   │   ├── builtin/         # Native Rust skill implementations
│   │   │   ├── shell.rs     # Run shell commands
│   │   │   ├── http.rs      # HTTP fetch/post
│   │   │   ├── file_io.rs   # Read/write files
│   │   │   └── summarize.rs # Summarize text via LLM
│   │   └── loader.rs        # Parse SKILL.md files into skill metadata
│   ├── channels/
│   │   ├── mod.rs           # InputSource / OutputSink enums
│   │   ├── stdin.rs         # Read from terminal
│   │   ├── webhook.rs       # Accept a single webhook payload
│   │   └── file.rs          # Read task from file
│   └── types.rs             # Shared types: Plan, ActionResult, TurnOutput, etc.
```

---

## Key Types

```rust
/// Application configuration.
struct Config {
    workspace: PathBuf,        // Working directory
    database_path: PathBuf,    // SQLite database location

    // LLM settings
    provider: Provider,        // claude, openai, ollama
    model: String,             // Model name
    api_key: Option<String>,   // From env or config file
    api_base_url: Option<String>,  // For ollama or proxies

    // Behavior
    system_prompt_path: Option<PathBuf>,  // Custom system prompt
    skills_dir: Option<PathBuf>,  // Additional skills
    max_retries: u32,          // LLM retry attempts
}

enum Provider {
    Claude,
    OpenAi,
    Ollama,
}

/// What the user feeds into a cycle.
struct CycleInput {
    user_message: Option<String>,        // new text from user (if any)
    previous_output: Option<TurnOutput>, // result of last cycle
}

/// What a cycle produces.
enum TurnOutput {
    Thought(Plan),          // Loop A completed: LLM produced a plan
    Action(ActionResult),   // Loop B completed: a skill finished executing
}

/// LLM's proposed next steps.
struct Plan {
    summary: String,                // human-readable description
    actions: Vec<PlannedAction>,    // skills to invoke
    response: Option<String>,       // message to send back to user
}

/// A skill invocation the LLM wants to make.
struct PlannedAction {
    tool_use_id: String,       // From LLM's tool_use block
    skill_name: String,
    parameters: serde_json::Value,
    priority: u8,
}

/// Result of running a skill.
struct ActionResult {
    tool_use_id: String,       // Links back to LLM's tool_use request
    skill_name: String,
    description: String,
    payload: serde_json::Value,
    success: bool,
    error: Option<String>,
}

/// A turn in the conversation history.
enum ConversationTurn {
    User(String),
    Assistant(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { id: String, content: String, success: bool },
}

/// Context passed to skill execution.
struct SkillContext {
    workspace: PathBuf,        // Working directory for file operations
    http_client: reqwest::Client,  // Shared HTTP client
}

/// Context for reasoning loop.
struct ReasoningContext {
    messages: Vec<Message>,
    tools: Vec<ToolDef>,
}
```

---

## Core Traits

### LlmProvider

```rust
#[async_trait]
trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse>;

    fn model_name(&self) -> &str;
}
```

Implementations: `ClaudeProvider`, `OpenAiProvider`, `OllamaProvider`.

Each wraps the respective HTTP API. Tool definitions are derived from the skill registry so the LLM knows what skills are available.

### Skill

```rust
#[async_trait]
trait Skill: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn tool_definition(&self) -> ToolDef;  // for LLM function calling
    async fn execute(&self, params: serde_json::Value, ctx: &SkillContext) -> Result<ActionResult>;
}
```

`SkillRegistry` holds a `Vec<Box<dyn Skill>>` and provides lookup by name plus a method to collect all tool definitions for the LLM.

---

## Memory Model

Memory and embeddings are abstracted from `river-db` and related crates:

```rust
/// Memory handle using river-db's vector store
struct Memory {
    db: river_db::Database,              // SQLite via river-db
    vector_store: VectorStore,           // Embeddings from river-db
    conversation: Vec<ConversationTurn>, // Recent turns for LLM context
    pending_actions: Vec<PlannedAction>, // Queued but not yet executed
    deferred_output: Option<TurnOutput>, // Cached output from previous cycle
}

struct MemoryEntry {
    content: String,
    timestamp: DateTime<Utc>,
    source: String,
    embedding: Option<Vec<f32>>,  // From river-db vector store
}
```

Uses river-db's existing:
- SQLite storage layer
- Vector store with cosine similarity search
- Embedding service client

`relevant_to(query: &str)` uses vector similarity to pull relevant memories into the LLM context window.

---

## Runtime: The Cycle Method

```rust
impl Runtime {
    async fn cycle(&mut self, input: CycleInput) -> Result<TurnOutput> {
        // 1. Check if previous cycle has a cached result
        if let Some(deferred) = self.memory.deferred_output.take() {
            // Still incorporate any new user input into memory
            if let Some(msg) = &input.user_message {
                self.memory.conversation.push(ConversationTurn::user(msg));
            }
            return Ok(deferred);
        }

        // 2. Build contexts for both loops (extract data before spawning)
        let reasoning_ctx = self.build_reasoning_context(&input);
        let pending = self.memory.pending_actions.drain(..).next();

        // Clone/extract what tasks need (can't move self into spawn)
        let llm = self.llm.clone();  // Arc<dyn LlmProvider>
        let skills = self.skills.clone();  // Arc<SkillRegistry>

        // 3. Launch both loops concurrently
        let reasoning_handle = tokio::spawn(async move {
            run_reasoning(llm, reasoning_ctx).await
        });

        let execution_handle = pending.map(|action| {
            let skills = skills.clone();
            tokio::spawn(async move {
                run_execution(skills, action).await
            })
        });

        // 4. Wait for first to complete, then await the other
        let output = if let Some(exec_handle) = execution_handle {
            // Pin handles so select! doesn't consume them
            tokio::pin!(reasoning_handle);
            tokio::pin!(exec_handle);

            tokio::select! {
                result = &mut reasoning_handle => {
                    // Reasoning finished first — now await execution
                    let thought = result??;
                    match (&mut exec_handle).await {
                        Ok(Ok(action)) => {
                            self.memory.deferred_output = Some(TurnOutput::Action(action));
                        }
                        Ok(Err(e)) => tracing::warn!("execution failed: {e}"),
                        Err(e) => tracing::warn!("execution task panicked: {e}"),
                    }
                    TurnOutput::Thought(thought)
                }
                result = &mut exec_handle => {
                    // Execution finished first — now await reasoning
                    let action = result??;
                    match (&mut reasoning_handle).await {
                        Ok(Ok(thought)) => {
                            self.memory.deferred_output = Some(TurnOutput::Thought(thought));
                        }
                        Ok(Err(e)) => tracing::warn!("reasoning failed: {e}"),
                        Err(e) => tracing::warn!("reasoning task panicked: {e}"),
                    }
                    TurnOutput::Action(action)
                }
            }
        } else {
            // No pending action, just run reasoning
            TurnOutput::Thought(reasoning_handle.await??)
        };

        // 5. Update memory with output
        self.memory.record(&output);
        if let TurnOutput::Thought(ref plan) = output {
            // Queue the plan's actions for the next cycle's Loop B
            self.memory.pending_actions.extend(plan.actions.clone());
        }

        // 6. Persist
        self.memory.save().await?;

        Ok(output)
    }
}

// Free functions to avoid self-borrowing issues
async fn run_reasoning(
    llm: Arc<dyn LlmProvider>,
    ctx: ReasoningContext,
) -> Result<Plan> {
    let response = llm.complete(&ctx.messages, &ctx.tools).await?;
    parse_plan(response)
}

async fn run_execution(
    skills: Arc<SkillRegistry>,
    action: PlannedAction,
) -> Result<ActionResult> {
    let skill = skills.get(&action.skill_name)?;
    skill.execute(action.parameters, &SkillContext::default()).await
}
```

### Reasoning Loop (Loop A)

- Assembles messages: system prompt + relevant memories + recent conversation + user input
- **Includes previous action results** — when `CycleInput.previous_output` is `Action(result)`, it becomes a tool result message in the conversation
- Includes tool definitions from skill registry
- Calls LLM provider
- Parses response into a `Plan` (extracting tool_use blocks if any)

### Execution Loop (Loop B)

- Takes the first `PlannedAction` from the pending queue (one action per cycle)
- Looks up the skill by name in the registry
- Calls `skill.execute()` with the parameters
- Returns `ActionResult`

---

## Context Assembly

### Message Structure for LLM

```rust
fn build_reasoning_context(&self, input: &CycleInput) -> ReasoningContext {
    let mut messages = vec![];

    // 1. System prompt (defines agent behavior)
    messages.push(Message::system(&self.system_prompt));

    // 2. Relevant memories from vector search
    if let Some(ref msg) = input.user_message {
        let memories = self.memory.relevant_to(msg, 5)?;
        if !memories.is_empty() {
            let memory_text = memories.iter()
                .map(|m| format!("- {}", m.content))
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(Message::system(format!("Relevant memories:\n{memory_text}")));
        }
    }

    // 3. Recent conversation history
    for turn in &self.memory.conversation {
        match turn {
            ConversationTurn::User(text) => messages.push(Message::user(text)),
            ConversationTurn::Assistant(text) => messages.push(Message::assistant(text)),
            ConversationTurn::ToolUse { id, name, input } => {
                messages.push(Message::tool_use(id, name, input));
            }
            ConversationTurn::ToolResult { id, content, success } => {
                messages.push(Message::tool_result(id, content, *success));
            }
        }
    }

    // 4. Previous action result (if execution won last cycle)
    if let Some(TurnOutput::Action(ref result)) = input.previous_output {
        // Insert as tool_result so LLM sees what happened
        messages.push(Message::tool_result(
            &result.tool_use_id,
            &serde_json::to_string(&result.payload).unwrap_or_default(),
            result.success,
        ));
    }

    // 5. New user input (if any)
    if let Some(ref msg) = input.user_message {
        messages.push(Message::user(msg));
    }

    ReasoningContext {
        messages,
        tools: self.skills.tool_definitions(),
    }
}
```

### System Prompt Structure

```
You are a helpful assistant with access to tools.

When you want to take an action, use a tool. When you have information to share, respond directly.

Available tools will be provided. Use them when appropriate. You can request multiple tools in a single response — they will be queued and executed one per turn.

Be concise. Focus on completing the user's request.
```

The system prompt is loaded from config or workspace file, allowing customization.

---

## Plan Parsing

### Mapping LLM Response to Plan

```rust
fn parse_plan(response: LlmResponse) -> Result<Plan> {
    let mut actions = vec![];
    let mut response_text = None;

    for block in response.content {
        match block {
            ContentBlock::Text(text) => {
                response_text = Some(text);
            }
            ContentBlock::ToolUse { id, name, input } => {
                actions.push(PlannedAction {
                    tool_use_id: id,
                    skill_name: name,
                    parameters: input,
                    priority: 0,  // FIFO for now
                });
            }
        }
    }

    Ok(Plan {
        summary: response_text.clone().unwrap_or_default(),
        actions,
        response: response_text,
    })
}
```

### Text-Only Responses

When the LLM returns only text (no tool_use), the Plan has:
- `actions: vec![]` — nothing to queue
- `response: Some(text)` — display to user

Next cycle, Loop B has nothing pending, so reasoning runs alone.

---

## Main: The Cycle Pump

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load()?;
    let mut runtime = Runtime::init(config).await?;

    // Initial input from stdin, file, or webhook
    let mut input = CycleInput::from_initial_source(&runtime.config)?;

    loop {
        let output = runtime.cycle(input).await?;

        // Display
        match &output {
            TurnOutput::Thought(plan) => {
                if let Some(response) = &plan.response {
                    println!("{}", response);
                }
                if !plan.actions.is_empty() {
                    println!("[queued {} action(s)]", plan.actions.len());
                    for a in &plan.actions {
                        println!("  -> {}", a.skill_name);
                    }
                }
            }
            TurnOutput::Action(result) => {
                println!("[{}] {}", result.skill_name, result.description);
                if !result.success {
                    println!("  error: {}", result.error.as_deref().unwrap_or("unknown"));
                }
            }
        }

        // Prompt for next cycle
        print!("\n> ");
        let user = read_line()?;
        let trimmed = user.trim();

        if trimmed == "q" || trimmed == "quit" {
            runtime.memory.save().await?;
            break;
        }

        input = CycleInput {
            user_message: if trimmed.is_empty() { None } else { Some(trimmed.to_string()) },
            previous_output: Some(output),
        };
    }

    Ok(())
}
```

---

## CLI Interface

```
USAGE:
    river-oneshot [OPTIONS] [INPUT]

OPTIONS:
    --config <PATH>       Path to config file [default: ~/.river/oneshot.toml]
    --workspace <PATH>    Workspace directory [default: ~/.river/workspace]
    --model <MODEL>       LLM to use [default: claude-sonnet-4-20250514]
    --provider <NAME>     LLM provider: claude, openai, ollama [default: claude]
    --input <SOURCE>      Input source: stdin, file:<path>, webhook:<port>
    --output <SINK>       Output sink: stdout, http:<url>, file:<path>
    --skills <DIR>        Additional skills directory
    --once                Run a single cycle and exit (true oneshot mode)
    -v, --verbose         Show reasoning traces

ARGS:
    [INPUT]               Initial message (alternative to --input stdin)

EXAMPLES:
    river-oneshot "summarize my inbox"
    echo "check weather" | river-oneshot --input stdin --once
    river-oneshot --input file:task.json --output http://slack.webhook.url
```

---

## Dependencies (Cargo.toml)

```toml
[package]
name = "river-oneshot"
version = "0.1.0"
edition = "2021"

[dependencies]
# Workspace crates
river-core = { path = "../river-core" }
river-db = { path = "../river-db" }

# Async runtime
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# Error handling
anyhow = "1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# HTTP (rustls to avoid OpenSSL dependency)
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Time
chrono = { version = "0.4", features = ["serde"] }

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Utilities
dirs = "6"
```

---

## Error Handling & Retries

### LLM Failures

```rust
async fn run_reasoning_with_retry(
    llm: Arc<dyn LlmProvider>,
    ctx: ReasoningContext,
    max_retries: u32,
) -> Result<Plan> {
    let mut last_error = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            // Exponential backoff: 1s, 2s, 4s...
            let delay = Duration::from_secs(1 << (attempt - 1));
            tokio::time::sleep(delay).await;
        }

        match run_reasoning(llm.clone(), ctx.clone()).await {
            Ok(plan) => return Ok(plan),
            Err(e) => {
                tracing::warn!("reasoning attempt {attempt} failed: {e}");
                last_error = Some(e);

                // Don't retry on non-retryable errors
                if is_auth_error(&e) || is_invalid_request(&e) {
                    break;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("reasoning failed")))
}
```

### Rate Limiting

For Claude API:
- Track `retry-after` header on 429 responses
- Respect rate limits per-model
- For oneshot CLI, simple exponential backoff is sufficient

### Skill Failures

Skills return `ActionResult` with `success: bool` and `error: Option<String>`. Failures don't crash the cycle — they're reported to the user and fed back to the LLM in the next reasoning cycle so it can adapt.

---

## Implementation Phases

### Phase 1: Skeleton ✅
- [x] Cargo project setup with workspace integration
- [x] Config loading (TOML)
- [x] Types and traits defined
- [x] Main loop with dummy cycle (echo input back)
- [x] CLI argument parsing

### Phase 2: Single Loop (reasoning only) ✅
- [x] Claude LLM provider (reqwest -> Anthropic API)
- [x] Message assembly with system prompt
- [x] Plan parsing from LLM response (tool_use extraction)
- [x] Memory integration (conversation history, JSON persistence)
- [x] Working single-loop cycle: user -> LLM -> plan -> display

### Phase 3: Dual Loop
- [ ] Skill trait and registry
- [ ] Built-in skills: shell, http, file_io
- [ ] Execution loop running pending actions (one per cycle)
- [ ] Both loops run to completion, first ready wins
- [ ] Deferred output caching for the completing-second loop
- [ ] Action queue management between cycles

### Phase 4: Memory & Embeddings
- [ ] Vector store integration from river-db
- [ ] Embedding-based relevant memory retrieval
- [ ] Context window management (truncation, prioritization)
- [ ] Conversation history with semantic search

### Phase 5: Polish
- [ ] Error recovery (LLM timeout, skill failure)
- [ ] OpenAI and Ollama provider implementations
- [ ] SKILL.md parser for external skill definitions
- [ ] `--once` mode for scripting
- [ ] Colored terminal output
- [ ] Channel adapters beyond stdin (webhook, file)

---

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Async runtime | tokio | Need concurrent loops; ecosystem support |
| Both loops complete | Yes | No wasted tokens, no cancellation complexity |
| One action per cycle | Yes | Simpler, more predictable turns |
| Skill implementation | Native Rust | Performance, type safety, no Node.js dependency |
| Memory backend | river-db | Reuse existing SQLite + vector store |
| Embeddings | river-db abstraction | Already built, tested, integrated |
| LLM tool calling | Native tool_use | Structured output beats regex parsing |
| Deferred cache | Single item | Two fast cycles if both finish together |
| Error handling | anyhow | Ergonomic for application binary |
| Config format | TOML | Rust standard, human-readable |

---

## Future Considerations

### Streaming Responses

LLM APIs support streaming tokens. In v1, we wait for the full response before returning. Future enhancement:

```rust
enum TurnOutput {
    Thought(Plan),
    Action(ActionResult),
    Streaming(tokio::sync::mpsc::Receiver<String>),  // Future
}
```

With streaming, the reasoning loop could yield partial output while execution runs, showing the user that thinking is in progress. Complexity: need to handle tool_use blocks that arrive mid-stream.

### Multiple Pending Actions

Currently one action per cycle. Could batch independent actions:

```rust
// Instead of drain(..).next()
let batch: Vec<_> = self.memory.pending_actions
    .drain(..)
    .take(3)  // Max parallel
    .collect();

let handles: Vec<_> = batch.into_iter()
    .map(|action| tokio::spawn(run_execution(skills.clone(), action)))
    .collect();

// futures::future::select_all or join_all
```

Trade-off: more throughput, less predictable turn order.

### Conversation Compaction

Long sessions will exceed context windows. Options:
1. Rolling window (drop oldest turns)
2. Summarization (LLM call to compress history)
3. Semantic retrieval (embed turns, retrieve relevant)

River-engine's I/You architecture has moves→moments compression that could be adapted here.

---

## Integration with River Engine

This crate reuses:
- `river-core`: Types, errors, configuration
- `river-db`: SQLite database, vector store, embedding client

It does NOT use:
- `river-gateway`: This is a standalone CLI, not an HTTP server
- `river-orchestrator`: No model management needed (uses external LLM APIs)
- `river-adapter`: No Discord/platform adapters

The oneshot architecture is complementary to the gateway's always-on model — use `river-oneshot` for interactive CLI sessions, `river-gateway` for persistent agents.
