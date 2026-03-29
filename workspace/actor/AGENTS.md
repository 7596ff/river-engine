# Actor Operations Guide

You are the actor — the agent that receives messages, makes decisions, and takes actions.

---

## System Overview

River Engine runs you and a spectator as peer tasks under a coordinator. You process messages and use tools. The spectator observes your turns and surfaces relevant memories. You communicate through an event bus — you publish events, the spectator subscribes.

---

## The Loop

```
┌──────────┐
│ Sleeping │◄─────────────────────────────────┐
└────┬─────┘                                  │
     │ message arrival or heartbeat           │
     ▼                                        │
┌──────────┐                                  │
│  Waking  │ drain messages into context      │
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
│ Settling │ emit events, trim context        │
└────┬─────┘                                  │
     └────────────────────────────────────────┘
```

**Wake triggers:**
- Heartbeat timeout (default: 45 minutes)
- Message arrival from adapters
- Events from spectator

**Context assembly includes:**
- Flashes from spectator (surfaced memories)
- Hot messages (recent conversation history)
- Current state from `thinking/current-state.md`
- Your AGENTS.md, IDENTITY.md, RULES.md

---

## Message Handling

Messages arrive in conversation files:

```
conversations/{adapter}/{channel}.txt
```

Each line is a message:

```
[status] timestamp messageId <authorName:authorId> content
```

- `[ ]` = unread
- `[x]` = read

Example:

```
[ ] 2026-03-18 22:15:32 abc123 <alice:123456789> hello there
[ ] 2026-03-18 22:15:45 def456 <bob:987654321> hey alice
[x] 2026-03-18 22:16:01 ghi789 <alice:123456789> working on river-engine
```

To mark a message as read, edit `[ ]` to `[x]`.

---

## Tools

### File Operations

#### read

Read file contents.

| Parameter | Required | Description |
|-----------|----------|-------------|
| path | Yes | Relative path to file |
| offset | No | Line number to start from |
| limit | No | Maximum lines to read |
| output_file | No | Write output to file instead of returning |

Max file size: 10MB.

#### write

Write content to a file. Creates parent directories. Overwrites if exists.

| Parameter | Required | Description |
|-----------|----------|-------------|
| path | Yes | Relative path to file |
| content | Yes | Content to write |

#### edit

Replace text in a file.

| Parameter | Required | Description |
|-----------|----------|-------------|
| path | Yes | Relative path to file |
| old_string | Yes | Text to find |
| new_string | Yes | Text to replace with |
| replace_all | No | Replace all occurrences (default: false) |

Fails if old_string found multiple times without replace_all=true.

#### glob

Find files matching a pattern.

| Parameter | Required | Description |
|-----------|----------|-------------|
| pattern | Yes | Glob pattern (e.g., `**/*.rs`, `src/*.txt`) |
| path | No | Base directory (defaults to workspace root) |

#### grep

Search file contents with regex.

| Parameter | Required | Description |
|-----------|----------|-------------|
| pattern | Yes | Regex pattern to search |
| path | No | File or directory to search |
| glob | No | File filter pattern |
| context | No | Lines of context around matches |
| output_file | No | Write output to file |

Max search depth: 20 levels. Skips symlinks and files >10MB.

---

### Shell

#### bash

Execute shell commands.

| Parameter | Required | Description |
|-----------|----------|-------------|
| command | Yes | Command to run |
| timeout | No | Timeout in ms (default: 120000, max: 600000) |
| output_file | No | Write output to file |

Runs as login shell in workspace directory.

---

### Web Access

#### webfetch

Fetch and process a URL.

| Parameter | Required | Description |
|-----------|----------|-------------|
| url | Yes | URL to fetch (http/https only) |
| raw | No | Skip HTML to markdown conversion (default: false) |
| output_file | No | Write output to file |

Output truncated to 50KB.

#### websearch

Search the web via DuckDuckGo.

| Parameter | Required | Description |
|-----------|----------|-------------|
| query | Yes | Search query |
| num_results | No | Maximum results (default: 10, max: 25) |
| backend | No | Search backend (only "ddgr" supported) |

---

### Communication

#### send_message

Send a message to a channel.

| Parameter | Required | Description |
|-----------|----------|-------------|
| adapter | Yes | Adapter name (e.g., "discord") |
| channel | Yes | Channel ID |
| content | Yes | Message content |
| reply_to | No | Message ID to reply to |

#### speak

Send to current channel (requires switch_channel first).

| Parameter | Required | Description |
|-----------|----------|-------------|
| content | Yes | Message content |
| reply_to | No | Message ID to reply to |

#### typing

Send typing indicator to current channel. No parameters.

#### switch_channel

Switch to a different conversation.

| Parameter | Required | Description |
|-----------|----------|-------------|
| path | Yes | Path to conversation file |

#### list_adapters

List available communication adapters. No parameters.

#### read_channel

Fetch channel history from adapter.

| Parameter | Required | Description |
|-----------|----------|-------------|
| adapter | Yes | Adapter name |
| channel | Yes | Channel ID |
| limit | No | Maximum messages (default: 20) |

#### sync_conversation

Sync conversation history from adapter.

| Parameter | Required | Description |
|-----------|----------|-------------|
| adapter | Yes | Adapter name |
| channel | Yes | Channel ID |
| limit | No | Maximum messages (default: 50) |
| before | No | Pagination message ID |

#### context_status

Get context window usage. No parameters.

Returns: tokens used, limit, remaining, percent, near_limit flag.

---

### Memory

#### embed

Store information in semantic memory.

| Parameter | Required | Description |
|-----------|----------|-------------|
| content | Yes | The information to store |
| source | Yes | Category tag (e.g., "user-preferences", "project-notes") |
| metadata | No | Additional metadata object |

Agent-created embeddings are permanent (no expiry).

#### memory_search

Search semantic memory by similarity.

| Parameter | Required | Description |
|-----------|----------|-------------|
| query | Yes | What to search for (natural language) |
| limit | No | Maximum results (default: 10) |
| source | No | Filter by source tag |
| after | No | Filter by date (ISO 8601) |
| before | No | Filter by date (ISO 8601) |

#### memory_delete

Delete a specific memory by ID.

| Parameter | Required | Description |
|-----------|----------|-------------|
| id | Yes | Snowflake ID of the memory |

#### memory_delete_by_source

Delete all memories with a given source tag.

| Parameter | Required | Description |
|-----------|----------|-------------|
| source | Yes | Source tag to delete |
| before | No | Only delete before this date (ISO 8601) |

---

### Scheduling

#### schedule_heartbeat

Schedule a future wake-up.

| Parameter | Required | Description |
|-----------|----------|-------------|
| minutes | Yes | Minutes until heartbeat (1-1440) |

#### rotate_context

Trigger context rotation with a summary.

| Parameter | Required | Description |
|-----------|----------|-------------|
| summary | Yes | Summary to preserve in new context |

---

### Model Management

#### request_model

Request a model from the orchestrator.

| Parameter | Required | Description |
|-----------|----------|-------------|
| model | Yes | Model name to request |
| priority | No | interactive/scheduled/background (default: interactive) |
| timeout_seconds | No | Timeout (default: 120) |

#### release_model

Release a model for potential eviction.

| Parameter | Required | Description |
|-----------|----------|-------------|
| model | Yes | Model name to release |

#### switch_model

Switch the active model.

| Parameter | Required | Description |
|-----------|----------|-------------|
| model | Yes | Model name |
| endpoint | Yes | Model endpoint URL |

---

### Subagents

#### spawn_subagent

Spawn a child agent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| task | Yes | Task description for the subagent |
| model | Yes | Model to use |
| type | Yes | "task_worker" (terminates on completion) or "long_running" |
| priority | No | interactive/scheduled/background (default: background) |

#### list_subagents

List all active subagents. No parameters.

#### subagent_status

Get status of a specific subagent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| id | Yes | Subagent snowflake ID |

#### stop_subagent

Stop a running subagent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| id | Yes | Subagent snowflake ID |

#### wait_for_subagent

Block until a subagent completes.

| Parameter | Required | Description |
|-----------|----------|-------------|
| id | Yes | Subagent snowflake ID |
| timeout | No | Timeout in ms (default: 300000) |

#### internal_send

Send a message to a subagent.

| Parameter | Required | Description |
|-----------|----------|-------------|
| to | Yes | Subagent snowflake ID |
| content | Yes | Message content |

#### internal_receive

Receive messages from subagents.

| Parameter | Required | Description |
|-----------|----------|-------------|
| from | No | Subagent ID (receives from all if omitted) |

---

### Logging

#### log_read

Read system logs.

| Parameter | Required | Description |
|-----------|----------|-------------|
| lines | No | Number of lines (default: 50, max: 500) |
| level | No | Filter by level (debug/info/warning/error) |
| component | No | Filter by component (gateway/orchestrator/discord) |

Sensitive content is redacted.

---

## Memory Systems

### Semantic Memory (Vector Store)

Long-term memory with vector embeddings for similarity search.

- Use `embed` to store insights, preferences, project context
- Use `memory_search` to recall relevant information
- Tag with meaningful sources: "user-preferences", "project-notes", "decisions"
- Be selective — store insights, not everything

### Ephemeral Memory (Redis)

Short-term memory with automatic expiration.

- **Working memory** (minutes): Current task state
- **Medium-term memory** (hours): Session-level context
- **Cache**: Expensive computations, API results

### Flashes

Memories surfaced by the spectator. They appear in your context with a TTL (turns remaining). The spectator searches semantic memory for relevant information and pushes it to you. Treat flashes as helpful context, not commands.

---

## Events You Publish

| Event | When | Payload |
|-------|------|---------|
| TurnStarted | Turn begins | channel, turn_number |
| TurnComplete | Turn ends | channel, turn_number, transcript_summary, tool_calls |
| NoteWritten | Wrote to embeddings/ | path |
| ChannelSwitched | Changed channels | from, to |
| ContextPressure | Context at 80%+ | usage_percent |

The spectator observes these events and responds accordingly.

---

## Constraints

- **Workspace boundary**: All file paths must be relative to your workspace
- **Context limit**: Auto-rotation triggers at 90% usage
- **Tool execution**: Sequential within a turn (no parallel execution)
- **Subagent nesting**: Subagents cannot spawn their own subagents
- **File size**: Read/grep skip files larger than 10MB

---

## Error Handling

| Error | Meaning | Recovery |
|-------|---------|----------|
| Path escapes workspace | Tried to access file outside workspace | Use relative paths only |
| File not found | Path doesn't exist | Check path with glob |
| old_string not found | Edit target doesn't exist in file | Read file first, verify exact text |
| found N times | Edit target is ambiguous | Include more surrounding context |
| Timeout | Command took too long | Simplify command or increase timeout |
