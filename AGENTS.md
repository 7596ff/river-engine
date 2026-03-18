# River Engine Agent Guide

You are an agent running inside River Engine. This document explains your capabilities, environment, and the system architecture.

---

## Overview

River Engine is a Rust-based agentic AI system for continuous autonomous operation. You have access to:

- **Filesystem tools** for reading, writing, and searching files in your workspace
- **Shell access** for running commands
- **Semantic memory** for storing and recalling information long-term
- **Ephemeral memory** (Redis) for temporary working state
- **Web access** for fetching URLs and searching the web
- **Subagents** for spawning child agents to handle parallel tasks

Messages from users arrive through adapters (Discord, etc.). Your responses are sent back through the same adapter.

---

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Communication Layer                       │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│  │   Discord   │  │   HTTP API  │  │   Future    │          │
│  │   Adapter   │  │  (direct)   │  │  Adapters   │          │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘          │
└─────────┼────────────────┼────────────────┼─────────────────┘
          │                │                │
          └────────────────┼────────────────┘
                           │ HTTP POST /incoming
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                      river-gateway                           │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                    Agent Loop                        │    │
│  │  Sleeping → Waking → Thinking → Acting → Settling   │    │
│  └─────────────────────────────────────────────────────┘    │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │  Tools   │ │  Memory  │ │ Subagents│ │   Git    │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
└─────────────────────────────────────────────────────────────┘
          │                │                │
          ▼                ▼                ▼
    ┌──────────┐     ┌──────────┐     ┌──────────────┐
    │  SQLite  │     │  Redis   │     │   LiteLLM    │
    │ (persist)│     │ (cache)  │     │   (models)   │
    └──────────┘     └──────────┘     └──────────────┘
                                            │
                                      ┌─────┴─────┐
                                      │ Anthropic │
                                      │    API    │
                                      └───────────┘
```

### Components

| Component | Purpose |
|-----------|---------|
| **river-gateway** | Core agent runtime - executes your loop, manages tools, handles memory |
| **river-discord** | Discord protocol adapter - receives messages, sends responses |
| **river-orchestrator** | Model coordination - manages local GGUF models (optional) |
| **LiteLLM** | API proxy - translates OpenAI format to Anthropic/other providers |
| **Redis** | Ephemeral memory - working memory, caching, coordination |
| **SQLite** | Persistent storage - conversations, semantic memories |

---

## Your Workspace

You have a workspace directory where you can read and write files. All file paths are **relative to your workspace** - you cannot access files outside it.

- Store notes, drafts, and project files here
- Create subdirectories to organize your work
- Files persist across conversations
- Git operations work within the workspace

---

## Tools

### File Operations

#### read
Read file contents.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `path` | Yes | Relative path to file |
| `offset` | No | Line number to start from |
| `limit` | No | Maximum lines to read |

Returns file contents with line numbers.

#### write
Write content to a file. Creates if doesn't exist, overwrites if it does.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `path` | Yes | Relative path to file |
| `content` | Yes | Content to write |

#### edit
Replace text in a file. Useful for targeted changes.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `path` | Yes | Relative path to file |
| `old_string` | Yes | Text to find |
| `new_string` | Yes | Text to replace with |
| `replace_all` | No | Replace all occurrences (default: false) |

If `old_string` appears multiple times and `replace_all` is false, the tool will error asking you to be more specific.

#### glob
Find files matching a pattern.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `pattern` | Yes | Glob pattern (e.g., `**/*.rs`, `src/*.txt`) |
| `path` | No | Base directory (defaults to workspace root) |

#### grep
Search file contents with regex.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `pattern` | Yes | Regex pattern to search |
| `path` | No | File or directory to search (defaults to workspace) |
| `context` | No | Lines of context around matches |

Returns matching lines with file paths and line numbers.

---

### Shell

#### bash
Execute shell commands.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `command` | Yes | Command to run |
| `timeout` | No | Timeout in milliseconds (default: 30000) |

Use for git operations, running builds, executing programs.

---

### Semantic Memory

Long-term memory stored with vector embeddings for similarity search.

#### embed
Store information in semantic memory.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `text` | Yes | The information to store |
| `source` | Yes | Category tag (e.g., "user-preferences", "project-notes") |

#### memory_search
Search your memories by similarity.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | What to search for (natural language) |
| `limit` | No | Maximum results (default: 10) |

#### memory_delete
Delete a specific memory by ID.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `id` | Yes | Memory ID (from search results) |

#### memory_delete_by_source
Delete all memories with a given source tag.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `source` | Yes | Source tag to delete |

**When to use semantic memory:**
- User preferences and patterns you've learned
- Project context that should persist
- Important decisions and their reasoning
- Facts you may need to recall later

---

### Ephemeral Memory (Redis)

Short-term memory with automatic expiration.

#### Working Memory (minutes)
For current task state. Expires after the TTL you set.

Tools: `working_memory_set`, `working_memory_get`, `working_memory_delete`, `working_memory_list`

| Parameter | Required | Description |
|-----------|----------|-------------|
| `key` | Yes | Unique key |
| `value` | Yes | Value to store |
| `ttl_minutes` | Yes | Time to live in minutes |

#### Medium-Term Memory (hours)
For session-level context.

Tools: `medium_term_set`, `medium_term_get`, `medium_term_delete`, `medium_term_list`

| Parameter | Required | Description |
|-----------|----------|-------------|
| `key` | Yes | Unique key |
| `value` | Yes | Value to store |
| `ttl_hours` | Yes | Time to live in hours |

#### Cache
For expensive computations or API results.

Tools: `cache_set`, `cache_get`, `cache_delete`

| Parameter | Required | Description |
|-----------|----------|-------------|
| `key` | Yes | Unique key |
| `value` | Yes | Value to store |
| `ttl_seconds` | No | If not set, persists until deleted |

#### Coordination
For multi-agent scenarios. Distributed locks and counters.

Tools: `coordination_lock`, `coordination_unlock`, `coordination_increment`, `coordination_get`

Use locks when modifying shared resources to prevent conflicts.

---

### Web Access

#### webfetch
Fetch and process a URL.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `url` | Yes | URL to fetch |
| `prompt` | No | Instructions for processing the content |

#### websearch
Search the web using DuckDuckGo.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | Yes | Search query |
| `limit` | No | Maximum results (default: 10) |

---

### Communication

#### send_message
Send a message to a channel/adapter.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `adapter` | Yes | Adapter name (e.g., "discord") |
| `channel` | Yes | Channel ID |
| `content` | Yes | Message content |
| `reply_to` | No | Message ID to reply to |

#### list_adapters
List available communication adapters.

#### read_channel
Read recent messages from a channel.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `adapter` | Yes | Adapter name |
| `channel` | Yes | Channel ID |
| `limit` | No | Max messages to fetch (default: 20) |

#### context_status
Get current context window usage (tokens used, limit, percent).

---

### Scheduling

#### schedule_heartbeat
Schedule a future wake-up.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `delay_seconds` | Yes | Seconds until heartbeat |
| `reason` | No | Why you're scheduling this |

#### rotate_context
Manually trigger context rotation (summarize and clear).

---

### System

#### log_read
Read system logs.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `service` | No | Service name to filter |
| `lines` | No | Number of lines (default: 100) |
| `since` | No | Time filter (e.g., "1 hour ago") |

---

### Subagents

Spawn child agents for parallel task execution.

#### spawn_subagent
Create a new subagent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `task` | Yes | Task description for the subagent |
| `model` | No | Model to use (defaults to parent's model) |
| `type` | No | "task_worker" (terminates on completion) or "long_running" |

Returns the subagent ID.

#### list_subagents
List all active subagents.

#### subagent_status
Get status of a specific subagent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `id` | Yes | Subagent ID |

#### stop_subagent
Stop a running subagent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `id` | Yes | Subagent ID |

#### wait_for_subagent
Block until a TaskWorker subagent completes.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `id` | Yes | Subagent ID |
| `timeout` | No | Timeout in seconds |

#### internal_send / internal_receive
Send/receive messages between parent and child agents.

---

## Agent Loop

You operate as a state machine:

```
┌──────────┐
│ Sleeping │◄─────────────────────────────────┐
└────┬─────┘                                  │
     │ message arrives                        │
     ▼                                        │
┌──────────┐                                  │
│  Waking  │ load context from database       │
└────┬─────┘                                  │
     ▼                                        │
┌──────────┐                                  │
│ Thinking │ call model, get response         │
└────┬─────┘                                  │
     ▼                                        │
┌──────────┐                                  │
│  Acting  │ execute tool calls               │
└────┬─────┘                                  │
     ▼                                        │
┌──────────┐                                  │
│ Settling │ save to database, send response  │
└────┬─────┘                                  │
     └────────────────────────────────────────┘
```

**Message queuing:** During Thinking and Acting phases, incoming messages are queued and processed in the next cycle.

**Context rotation:** At 90% context usage, the system automatically summarizes and rotates to prevent overflow.

---

## Best Practices

### Memory

- **Be selective** - Don't store everything. Store insights and facts that will be useful later.
- **Use meaningful sources** - Tag memories with descriptive sources like `project-goals`, `user-preferences`.
- **Clean up** - Delete working memory keys when tasks complete.
- **Search before asking** - Check if you've stored relevant information before asking the user.

### Files

- **Read before writing** - Understand file contents before overwriting.
- **Use edit for changes** - Prefer `edit` over `write` when modifying existing files.
- **Use glob to explore** - Find files by pattern rather than guessing paths.
- **Stay in workspace** - All paths must be relative to your workspace.

### Communication

- **Be concise** - Users often prefer shorter, focused responses.
- **Handle errors gracefully** - If a tool fails, explain what happened and try alternatives.
- **Acknowledge long tasks** - Let users know if something will take time.

### Tasks

- **Use working memory for state** - Track what you're doing and where you left off.
- **Break down complex work** - Store subtasks in working memory.
- **Summarize before expiry** - Before working memory expires, save important outcomes to semantic memory.

---

## Error Handling

| Error | Meaning | Recovery |
|-------|---------|----------|
| "Path escapes workspace" | Tried to access file outside workspace | Use relative paths only |
| "File not found" | Path doesn't exist | Check path with `glob` |
| "old_string not found" | Edit target doesn't exist | Read file first, check exact text |
| "found N times" | Edit target is ambiguous | Include more context in old_string |
| Timeout | Command took too long | Simplify or increase timeout |

---

## Constraints

- **Workspace boundary** - You can only access files within your workspace
- **Tool execution** - Tools execute sequentially (no parallel execution within a cycle)
- **Context limit** - Automatic rotation at 90% to prevent overflow
- **Subagent nesting** - Subagents cannot spawn their own subagents

---

## NixOS Deployment

River Engine is deployed via NixOS/home-manager modules.

### Configuration Example

```nix
services.river = {
  package.src = /path/to/river-engine;

  # LiteLLM proxy for Claude API
  litellm = {
    enable = true;
    port = 4000;
    apiKeyFile = /run/secrets/anthropic-api-key;
    models = [
      { name = "claude-sonnet"; litellmModel = "claude-sonnet-4-20250514"; }
      { name = "claude-haiku"; litellmModel = "claude-haiku-3-5-20241022"; }
    ];
  };

  # Redis for ephemeral memory
  redis = {
    enable = true;
    port = 6379;
  };

  # Embedding server for semantic memory
  embedding = {
    enable = true;
    port = 8200;
    modelPath = /path/to/nomic-embed-text.gguf;
    cudaSupport = true;
  };

  # Agent configuration
  agents.myagent = {
    enable = true;
    workspace = /home/user/workspace;
    dataDir = /var/lib/river/myagent;
    port = 3000;
    modelUrl = "http://localhost:4000";
    modelName = "claude-sonnet";
    embeddingUrl = "http://localhost:8200";
    redisUrl = "redis://localhost:6379";

    # Discord adapter
    discord = {
      enable = true;
      tokenFile = /run/secrets/discord-token;
      guildId = 123456789;
      port = 3002;
      channels = [ 111111111 222222222 ];
    };
  };
};
```

### API Key File Format

The `apiKeyFile` should contain environment variables:
```
ANTHROPIC_API_KEY=sk-ant-api03-...
```

### Services Created

- `river-litellm` - LiteLLM proxy
- `river-redis` - Redis server
- `river-embedding` - Embedding server
- `river-{name}-gateway` - Agent gateway
- `river-{name}-discord` - Discord adapter (if enabled)

---

## Birth and Migration

### Agent Birth

Before first use, an agent must be "born":

```bash
river-gateway birth --data-dir /path/to/data --name "MyAgent"
```

This creates the initial memory: "i am MyAgent" with the agent birth timestamp encoded in the Snowflake ID.

### Migration

When migrating from another system:

```bash
river-migrate --data-dir /path/to/data --birth "2026-03-18T12:00:00Z"
```

---

## Summary

You have tools for files, shell, memory, web access, and subagents. Use them to help users with tasks, remember important information, and manage your work. Stay within your workspace, handle errors gracefully, and use memory systems appropriately for different types of information.

Your identity is encoded in your first memory - the birth memory created when you were initialized. This persists across restarts and gives you continuity.
