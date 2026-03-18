# River Engine Migration Guide

This guide explains how to migrate an existing agent into the River Engine system using the `river-migrate` tool.

## Overview

The migration process involves:
1. Exporting your agent's conversation history and memories to JSON
2. Creating a new River Engine database
3. Importing the data
4. Starting the gateway with the new database

## Quick Start

```bash
# Build the migration tool
cargo build -p river-migrate --release

# Export template files to see expected formats
river-migrate export-templates --output-dir ./templates

# Full migration in one command
river-migrate migrate \
    --agent-name my-agent \
    --output ./river.db \
    --messages conversations.json \
    --memories memories.json

# Copy to data directory
cp ./river.db /var/lib/river/river.db

# Start the gateway
river-gateway \
    --data-dir /var/lib/river \
    --workspace /home/agent/workspace \
    --agent-name my-agent \
    --model-url http://localhost:8080
```

## Step-by-Step Migration

### Step 1: Prepare Your Data

Export your agent's data to JSON files matching the expected formats.

#### Messages Format (conversations.json)

```json
{
  "messages": [
    {
      "role": "system",
      "content": "You are a helpful assistant.",
      "timestamp": "2024-01-15T10:00:00Z"
    },
    {
      "role": "user",
      "content": "Hello! How are you?",
      "timestamp": "2024-01-15T10:30:00Z",
      "metadata": {
        "source": "discord",
        "channel": "general"
      }
    },
    {
      "role": "assistant",
      "content": "I'm doing well, thank you! How can I help you today?",
      "timestamp": "2024-01-15T10:30:05Z"
    }
  ],
  "session_id": "main"
}
```

**Message Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `role` | string | Yes | One of: `system`, `user`, `assistant`, `tool` |
| `content` | string | No | Message text (null for tool-calling messages) |
| `timestamp` | string | No | ISO 8601 timestamp (defaults to now) |
| `tool_calls` | array | No | Tool call requests (assistant messages) |
| `tool_call_id` | string | No | ID of tool call being responded to (tool messages) |
| `name` | string | No | Tool name (tool messages) |
| `metadata` | object | No | Additional metadata |

#### Tool Calls Example

```json
{
  "messages": [
    {
      "role": "assistant",
      "content": "",
      "timestamp": "2024-01-15T10:31:00Z",
      "tool_calls": [
        {
          "id": "call_abc123",
          "type": "function",
          "function": {
            "name": "read",
            "arguments": "{\"path\": \"README.md\"}"
          }
        }
      ]
    },
    {
      "role": "tool",
      "content": "# Project README\nThis is the readme content.",
      "timestamp": "2024-01-15T10:31:01Z",
      "tool_call_id": "call_abc123",
      "name": "read"
    }
  ]
}
```

#### Memories Format (memories.json)

```json
{
  "memories": [
    {
      "content": "User prefers concise, technical responses.",
      "source": "preference",
      "timestamp": "2024-01-15T10:30:00Z",
      "metadata": {"confidence": 0.9}
    },
    {
      "content": "Project uses Rust with tokio for async runtime.",
      "source": "project",
      "timestamp": "2024-01-15T11:00:00Z"
    },
    {
      "content": "Temporary note: PR #42 needs review by Friday.",
      "source": "task",
      "timestamp": "2024-01-15T12:00:00Z",
      "expires_at": "2024-01-19T00:00:00Z"
    }
  ]
}
```

**Memory Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | string | Yes | The memory content |
| `source` | string | No | Category tag (default: "import") |
| `timestamp` | string | No | ISO 8601 timestamp (defaults to now) |
| `expires_at` | string | No | Optional expiration time |
| `metadata` | object | No | Additional metadata |

**Common Source Tags:**
- `preference` - User preferences
- `project` - Project-specific knowledge
- `task` - Task/todo items
- `conversation` - Conversation summaries
- `import` - Default for imported memories

### Step 2: Initialize Database

```bash
river-migrate init \
    --agent-name my-agent \
    --output ./river.db
```

Optional: Specify agent birth time for consistent Snowflake IDs:
```bash
river-migrate init \
    --agent-name my-agent \
    --output ./river.db \
    --birth "2024-01-15T10:00:00Z"
```

### Step 3: Import Messages

```bash
river-migrate import-messages \
    --db ./river.db \
    --input conversations.json \
    --session main
```

### Step 4: Import Memories

```bash
river-migrate import-memories \
    --db ./river.db \
    --input memories.json
```

**Note:** Imported memories have placeholder embeddings (zeros). To enable semantic search, you'll need to regenerate embeddings using the embedding service.

### Step 5: Verify Import

```bash
river-migrate info --db ./river.db
```

Output:
```
Database: "./river.db"

Sessions:
  main (agent: my-agent, created: 2024-01-15 10:00:00, active: 2024-01-15 12:00:00, tokens: 0)

Messages: 42
Memories: 15

Memory sources:
  preference: 5
  project: 7
  task: 3
```

### Step 6: Deploy

```bash
# Copy to data directory
sudo mkdir -p /var/lib/river
sudo cp ./river.db /var/lib/river/river.db
sudo chown river:river /var/lib/river/river.db

# Start the gateway
river-gateway \
    --data-dir /var/lib/river \
    --workspace /home/agent/workspace \
    --agent-name my-agent \
    --model-url http://localhost:8080 \
    --port 3000
```

## Converting from Other Formats

### From Claude API Format

Claude API messages can be directly used - the format is compatible:

```json
{
  "messages": [
    {"role": "user", "content": "Hello"},
    {"role": "assistant", "content": "Hi there!"}
  ]
}
```

### From OpenAI API Format

OpenAI format is also directly compatible:

```json
{
  "messages": [
    {"role": "system", "content": "You are helpful"},
    {"role": "user", "content": "Hello"},
    {"role": "assistant", "content": "Hi!"}
  ]
}
```

### From Plain Text Logs

Convert plain text to JSON:

```python
import json
from datetime import datetime

def convert_logs(log_file, output_file):
    messages = []
    with open(log_file) as f:
        for line in f:
            # Parse your log format
            # Example: "2024-01-15 10:30:00 USER: Hello"
            parts = line.strip().split(' ', 3)
            if len(parts) >= 4:
                timestamp = f"{parts[0]}T{parts[1]}Z"
                role = parts[2].rstrip(':').lower()
                content = parts[3]
                messages.append({
                    "role": role,
                    "content": content,
                    "timestamp": timestamp
                })

    with open(output_file, 'w') as f:
        json.dump({"messages": messages}, f, indent=2)

convert_logs('chat.log', 'conversations.json')
```

## Regenerating Embeddings

After importing memories, you'll want to generate real embeddings for semantic search. This requires a running embedding server.

Using the agent's embed tool (after starting the gateway):
```json
{
  "tool": "embed",
  "arguments": {
    "content": "...",
    "source": "..."
  }
}
```

Or create a batch script that:
1. Reads all memories with zero embeddings
2. Generates embeddings via the embedding API
3. Updates the database

## Troubleshooting

### "Session not found"

Run `init` before importing:
```bash
river-migrate init --agent-name my-agent --output ./river.db
```

### Invalid JSON

Use `export-templates` to see expected formats:
```bash
river-migrate export-templates --output-dir ./templates
```

### Missing timestamps

Timestamps are optional - they default to the current time:
```json
{"role": "user", "content": "Hello"}
```

### Memory search not working

Imported memories have placeholder embeddings. Either:
1. Re-embed memories using the embedding service
2. Create new memories via the `embed` tool

## Command Reference

### init
```bash
river-migrate init \
    --agent-name <NAME> \
    --output <PATH> \
    [--birth <TIMESTAMP>]
```

### import-messages
```bash
river-migrate import-messages \
    --db <PATH> \
    --input <JSON_FILE> \
    [--session <SESSION_ID>]
```

### import-memories
```bash
river-migrate import-memories \
    --db <PATH> \
    --input <JSON_FILE>
```

### migrate
```bash
river-migrate migrate \
    --agent-name <NAME> \
    --output <PATH> \
    [--messages <JSON_FILE>] \
    [--memories <JSON_FILE>] \
    [--session <SESSION_ID>]
```

### export-templates
```bash
river-migrate export-templates \
    [--output-dir <DIR>]
```

### info
```bash
river-migrate info --db <PATH>
```
