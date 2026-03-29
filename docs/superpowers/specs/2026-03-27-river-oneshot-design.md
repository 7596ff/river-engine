# River Oneshot — Design Spec

> Turn-based dual-loop agent CLI for interactive sessions.
> Ported from OpenClaw concepts, integrated with River Engine infrastructure.
>
> Date: 2026-03-27
> Authors: Cass, Claude

---

## 1. Executive Summary

River Oneshot is a **turn-based CLI agent** that complements River Gateway's always-on model. It runs two concurrent loops — **reasoning** (LLM) and **execution** (skills) — both completing every cycle. The first to finish becomes the turn output; the other is cached for the next cycle.

**Key properties:**
- Both loops always complete (no cancellation, no wasted tokens)
- One action per cycle (predictable turns)
- Memory via river-db (SQLite + vector store)
- Native Rust skill implementations
- Claude/OpenAI/Ollama provider support

---

## 2. Architecture Overview

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

LLM reasoning is slow. If a previously queued skill (HTTP fetch, file read) finishes first, the user sees that result immediately. Creates a natural rhythm: sometimes thinking ahead, sometimes catching up on results.

### Both Loops Complete

Unlike a race-to-cancel pattern, **both loops always run to completion**:
- No wasted LLM tokens from cancelled requests
- No orphaned skill executions
- No cleanup complexity from mid-flight cancellation

---

## 3. Crate Structure

### Location

`crates/river-oneshot/`

### Module Layout

```
river-oneshot/
├── Cargo.toml
├── PLAN.md
├── src/
│   ├── main.rs              # Entry point, cycle pump, user I/O
│   ├── runtime.rs           # Runtime struct, cycle() method
│   ├── config.rs            # Config loading (TOML)
│   ├── context.rs           # CycleInput, context builders
│   ├── memory.rs            # Memory via river-db
│   ├── llm/
│   │   ├── mod.rs           # LlmProvider trait
│   │   ├── claude.rs        # Anthropic Claude
│   │   ├── openai.rs        # OpenAI
│   │   └── ollama.rs        # Local Ollama
│   ├── skills/
│   │   ├── mod.rs           # Skill trait, registry
│   │   ├── builtin/         # Native implementations
│   │   │   ├── shell.rs
│   │   │   ├── http.rs
│   │   │   ├── file_io.rs
│   │   │   └── summarize.rs
│   │   └── loader.rs        # SKILL.md parser
│   ├── channels/
│   │   ├── mod.rs           # Input/output sources
│   │   ├── stdin.rs
│   │   ├── webhook.rs
│   │   └── file.rs
│   └── types.rs             # Shared types
```

### Dependencies

Reuses from River Engine:
- `river-core`: Types, errors
- `river-db`: SQLite, vector store, embedding client

Does NOT use:
- `river-gateway`: Oneshot is standalone CLI
- `river-orchestrator`: Uses external LLM APIs
- `river-adapter`: No platform adapters

---

## 4. Core Types

```rust
/// Application configuration.
struct Config {
    workspace: PathBuf,
    database_path: PathBuf,
    provider: Provider,        // claude, openai, ollama
    model: String,
    api_key: Option<String>,
    api_base_url: Option<String>,
    system_prompt_path: Option<PathBuf>,
    skills_dir: Option<PathBuf>,
    max_retries: u32,
}

enum Provider { Claude, OpenAi, Ollama }

/// What the user feeds into a cycle.
struct CycleInput {
    user_message: Option<String>,
    previous_output: Option<TurnOutput>,
}

/// What a cycle produces.
enum TurnOutput {
    Thought(Plan),
    Action(ActionResult),
}

/// LLM's proposed next steps.
struct Plan {
    summary: String,
    actions: Vec<PlannedAction>,
    response: Option<String>,
}

/// A skill invocation.
struct PlannedAction {
    tool_use_id: String,
    skill_name: String,
    parameters: serde_json::Value,
    priority: u8,
}

/// Result of running a skill.
struct ActionResult {
    tool_use_id: String,
    skill_name: String,
    description: String,
    payload: serde_json::Value,
    success: bool,
    error: Option<String>,
}

/// Conversation history entry.
enum ConversationTurn {
    User(String),
    Assistant(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { id: String, content: String, success: bool },
}
```

---

## 5. Core Traits

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

### Skill

```rust
#[async_trait]
trait Skill: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn tool_definition(&self) -> ToolDef;
    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &SkillContext,
    ) -> Result<ActionResult>;
}
```

`SkillRegistry` provides lookup by name and collects tool definitions for LLM.

---

## 6. Runtime: The Cycle Method

```rust
impl Runtime {
    async fn cycle(&mut self, input: CycleInput) -> Result<TurnOutput> {
        // 1. Return cached result from previous cycle if present
        if let Some(deferred) = self.memory.deferred_output.take() {
            if let Some(msg) = &input.user_message {
                self.memory.conversation.push(ConversationTurn::user(msg));
            }
            return Ok(deferred);
        }

        // 2. Build contexts, extract Arc'd resources
        let reasoning_ctx = self.build_reasoning_context(&input);
        let pending = self.memory.pending_actions.drain(..).next();
        let llm = self.llm.clone();
        let skills = self.skills.clone();

        // 3. Spawn both loops
        let reasoning_handle = tokio::spawn(async move {
            run_reasoning(llm, reasoning_ctx).await
        });

        let execution_handle = pending.map(|action| {
            let skills = skills.clone();
            tokio::spawn(async move {
                run_execution(skills, action).await
            })
        });

        // 4. Select first, await second
        let output = if let Some(exec_handle) = execution_handle {
            tokio::pin!(reasoning_handle);
            tokio::pin!(exec_handle);

            tokio::select! {
                result = &mut reasoning_handle => {
                    let thought = result??;
                    // Await execution, cache if successful
                    if let Ok(Ok(action)) = (&mut exec_handle).await {
                        self.memory.deferred_output = Some(TurnOutput::Action(action));
                    }
                    TurnOutput::Thought(thought)
                }
                result = &mut exec_handle => {
                    let action = result??;
                    // Await reasoning, cache if successful
                    if let Ok(Ok(thought)) = (&mut reasoning_handle).await {
                        self.memory.deferred_output = Some(TurnOutput::Thought(thought));
                    }
                    TurnOutput::Action(action)
                }
            }
        } else {
            TurnOutput::Thought(reasoning_handle.await??)
        };

        // 5. Update memory, queue new actions
        self.memory.record(&output);
        if let TurnOutput::Thought(ref plan) = output {
            self.memory.pending_actions.extend(plan.actions.clone());
        }

        // 6. Persist
        self.memory.save().await?;

        Ok(output)
    }
}
```

---

## 7. Context Assembly

### Message Structure

1. **System prompt** — agent behavior definition
2. **Relevant memories** — vector search on user message
3. **Conversation history** — recent turns
4. **Previous action result** — if execution won last cycle
5. **New user input** — if any

### Action Results in Context

When execution wins a cycle, the `ActionResult` becomes a `tool_result` message in the next reasoning cycle. This closes the loop: LLM requests tool → skill executes → result fed back to LLM.

### System Prompt

Default:
```
You are a helpful assistant with access to tools.
When you want to take an action, use a tool.
When you have information to share, respond directly.
Be concise. Focus on completing the user's request.
```

Customizable via `--config` or workspace file.

---

## 8. Memory Model

Uses river-db:

```rust
struct Memory {
    db: river_db::Database,
    vector_store: VectorStore,
    conversation: Vec<ConversationTurn>,
    pending_actions: Vec<PlannedAction>,
    deferred_output: Option<TurnOutput>,
}
```

### Relevant Memory Retrieval

```rust
fn relevant_to(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
    let embedding = self.embed(query)?;
    self.vector_store.search(&embedding, limit)
}
```

---

## 9. CLI Interface

```
USAGE:
    river-oneshot [OPTIONS] [INPUT]

OPTIONS:
    --config <PATH>       Config file [default: ~/.river/oneshot.toml]
    --workspace <PATH>    Workspace directory [default: ~/.river/workspace]
    --model <MODEL>       LLM model [default: claude-sonnet-4-20250514]
    --provider <NAME>     Provider: claude, openai, ollama [default: claude]
    --once                Single cycle mode
    -v, --verbose         Show reasoning traces

EXAMPLES:
    river-oneshot "summarize my inbox"
    echo "check weather" | river-oneshot --once
```

---

## 10. Error Handling

### LLM Failures

Exponential backoff: 1s, 2s, 4s... up to `max_retries`.

Don't retry:
- Auth errors (401, 403)
- Invalid request (400)

### Skill Failures

Non-fatal. `ActionResult.success = false` with error message. Fed back to LLM in next reasoning cycle so it can adapt.

### Rate Limiting

Track `retry-after` header on 429 responses. For oneshot CLI, simple backoff is sufficient.

---

## 11. Implementation Phases

### Phase 1: Skeleton
- [ ] Project setup, workspace integration
- [ ] Config loading (TOML)
- [ ] Types and traits
- [ ] Main loop (echo mode)
- [ ] CLI parsing

### Phase 2: Single Loop
- [ ] Claude provider
- [ ] Message assembly
- [ ] Plan parsing (tool_use extraction)
- [ ] Memory with river-db
- [ ] Working cycle: user → LLM → plan → display

### Phase 3: Dual Loop
- [ ] Skill trait and registry
- [ ] Built-in skills: shell, http, file_io
- [ ] Execution loop
- [ ] Both loops complete, first ready wins
- [ ] Deferred output caching

### Phase 4: Memory & Embeddings
- [ ] Vector store integration
- [ ] Embedding-based retrieval
- [ ] Context window management
- [ ] Conversation with semantic search

### Phase 5: Polish
- [ ] Error recovery
- [ ] OpenAI and Ollama providers
- [ ] SKILL.md parser
- [ ] `--once` mode
- [ ] Colored output

---

## 12. Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Async runtime | tokio | Need concurrent loops |
| Both loops complete | Yes | No wasted tokens |
| One action per cycle | Yes | Predictable turns |
| Skills | Native Rust | Performance, no Node.js |
| Memory | river-db | Reuse existing code |
| Deferred cache | Single | Fast cycles if both finish |
| Config | TOML | Rust standard |

---

## 13. Future Considerations

### Streaming Responses

Show partial output while reasoning streams. Complexity: handle tool_use blocks mid-stream.

### Multiple Pending Actions

Batch independent actions. Trade-off: throughput vs predictability.

### Conversation Compaction

When history exceeds context window:
1. Rolling window (drop oldest)
2. Summarization (compress with LLM)
3. Semantic retrieval (embed and search)

Could adapt I/You architecture's moves→moments compression.

---

## 14. Relationship to River Engine

**Complementary, not competing:**

| Use Case | Tool |
|----------|------|
| Interactive CLI sessions | `river-oneshot` |
| Persistent agents | `river-gateway` |
| Model management | `river-orchestrator` |
| Platform integrations | `river-discord` + adapters |

Oneshot provides a lightweight entry point for one-off tasks without running full gateway infrastructure.

---

## 15. Success Criteria

### Functional
- [ ] Single cycle works: user → LLM → response
- [ ] Dual loop works: reasoning and execution complete
- [ ] Memory persists across sessions
- [ ] Skills execute correctly

### Behavioral
- [ ] LLM latency masked by concurrent execution
- [ ] Action results feed back to reasoning
- [ ] Errors handled gracefully
- [ ] Context stays within window limits

### Qualitative
- [ ] Feels responsive (sub-second when skills win)
- [ ] Conversation flows naturally
- [ ] Memory retrieval is relevant
- [ ] CLI is intuitive
