# Shared Reference

Reference material for both workers in the dyad.

## Tools

### File Operations

| Tool | Description |
|------|-------------|
| `read` | Read file contents. Params: `path`, optional `start_line`, `end_line` |
| `write` | Write file. Params: `path`, `content`, optional `mode` (overwrite/append/insert), `at_line` |
| `delete` | Delete file. Params: `path` |

### Shell

| Tool | Description |
|------|-------------|
| `bash` | Execute shell command. Params: `command`, optional `timeout_seconds`, `working_directory` |

### Communication

| Tool | Description |
|------|-------------|
| `speak` | Send message to channel. Params: `content`, optional `adapter`, `channel`, `reply_to` |
| `adapter` | Execute adapter operation. Params: `adapter`, `request` (OutboundRequest) |
| `switch_channel` | Change current channel. Params: `adapter`, `channel` |

### Memory

| Tool | Description |
|------|-------------|
| `search_embeddings` | Search vector store. Params: `query`. Returns first result + cursor |
| `next_embedding` | Continue search. Params: `cursor`. Returns next result |
| `create_flash` | Send flash to another worker. Params: `target_dyad`, `target_side`, `content`, optional `ttl_minutes` |
| `create_move` | Create move summary. Params: `channel`, `content`, `start_message_id`, `end_message_id` |
| `create_moment` | Create moment summary. Params: `channel`, `content`, `start_move_id`, `end_move_id` |

### Control

| Tool | Description |
|------|-------------|
| `sleep` | Pause loop. Params: optional `minutes` (None = indefinite) |
| `watch` | Manage wake channels. Params: optional `add`, `remove` (channel lists) |
| `summary` | Exit loop with summary. Params: `summary` |
| `switch_roles` | Switch actor/spectator with partner. No params |
| `request_model` | Switch LLM model. Params: `model` |

## Workspace Structure

```
workspace/
├── roles/           # Role definitions (actor.md, spectator.md)
├── left/            # Left worker's identity and context
├── right/           # Right worker's identity and context
├── shared/          # Shared reference material (this file)
├── conversations/   # Chat history by adapter/channel
├── moves/           # Per-turn summaries (spectator writes)
├── moments/         # Arc summaries (spectator writes)
├── embeddings/      # Files indexed for semantic search
├── memory/          # Long-term memory
├── notes/           # Working notes
└── artifacts/       # Generated files
```

## Conversation File Format

```
# === Tail (append-only since last compaction) ===
[+] 2026-04-03T14:30:00Z 1234567893 <alice:111> message text
[r] 2026-04-03T14:30:30Z 1234567893
[>] 2026-04-03T14:30:15Z 1234567895 <river:999> response text

# === Compacted (sorted, statuses resolved) ===
[x] 2026-04-03T14:30:00Z 1234567893 <alice:111> message text
[>] 2026-04-03T14:30:15Z 1234567895 <river:999> response text
```

| Prefix | Meaning |
|--------|---------|
| `[x]` | Incoming, read |
| `[ ]` | Incoming, unread |
| `[>]` | Outgoing |
| `[+]` | New arrival (tail) |
| `[r]` | Read receipt (tail) |
| `[!]` | Failed to send |
