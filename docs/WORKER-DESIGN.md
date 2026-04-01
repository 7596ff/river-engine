# River Worker — Design (WIP)

## Philosophy

The Worker is a shell. It creates the conditions for the model to think and act. As little as possible is done by the program itself. All intelligence — prioritization, context management decisions, task planning — lives inside the model.

## Input

The Worker is spawned by an orchestrator (designed separately) and receives:

- **System prompt** — a single string, built externally from workspace files (identity, rules, etc.). Includes the context budget so the model knows its limits.
- **Messages** — the conversation so far. Not a conversation with a user — a conversation with the world. Messages progress in only two ways: a new notification arrives, or a tool result completes.
- **Model endpoint** — where to send LLM requests (OpenAI-compatible chat completions API)
- **Model name** — which model to use
- **Orchestrator endpoint** — where to reach the orchestrator (for flash, model switch, output)
- **Worker name** — this Worker's name (used in flash, registration, logging)
- **Ground** — the human operator who started the system. Name and DM channel used as default channel.

```rust
struct WorkerInput {
    system_prompt: String,
    messages: Vec<Message>,
    model_endpoint: String,
    model_name: String,
    orchestrator_endpoint: String,
    worker_name: String,
    ground: Ground,
}

struct Ground {
    name: String,
    adapter: String,
    channel: String,  // DM channel for the human operator
}
```

The `ground` provides a default channel for `speak` before `switch_channel` is ever called. The model can always reach the human operator.

## The Loop

```
1. Call LLM with system prompt + messages
2. LLM responds with tool calls:
   → Execute them in parallel (all tool calls in a single response are independent)
   → Append tool results to messages
   → Persist context to JSONL after each tool result
   → If `summary` was among the tool calls: summary wins, stop execution
   → If `sleep` was among the tool calls (and no summary): pause the loop
   → Otherwise: go to 1
3. LLM responds with text (no tool calls):
   → Append a system-role status message:
     "You responded with text. Context: 54,000 / 128,000 tokens.
      Current time: 2026-03-28T14:30:00Z.
      New notifications: #general (2 unread), #ops (1 unread).
      Call `summary` if you are done, or continue working."
   → Go to 1
4. New notification arrives via HTTP endpoint (while loop is active):
   → Written to workspace
   → Batched for next status injection
5. New notification arrives via HTTP endpoint (while loop is sleeping):
   → Written to workspace
   → If channel is in watch list: wake the loop, inject status message, go to 1
   → Otherwise: batch for when sleep timer expires
```

The Worker loops until:
- **Summary called** (model explicitly signals it's done)
- **Context exhausted** (95% — hard stop, forced summary)
- **Error** (LLM unreachable, unrecoverable failure)

## Notifications

The Worker has an HTTP endpoint that receives inbound messages from adapters. When a message arrives:

1. The message is written to the workspace (format and path convention defined in the context-building crate design)
2. A notification is appended to the message list: `"New message in #general (1 unread)"`

The Worker does NOT inject message content into the context. The model decides whether to read the channel using the `read` tool on the conversation file. The model decides what to prioritize.

## Context Pressure

This is the one thing the Worker actively manages. Everything else is hands-off, but context is a hard safety rail.

- Worker tracks token count using the model API's response (usage.total_tokens) — no double counting. The first LLM call has no prior token count; the first response's usage establishes the baseline.
- **At 80%**: inject a system-role warning message:
  `"⚠ Context at 80% (102,400 / 128,000 tokens). Consider wrapping up or summarizing your work."`
- **At 95%**: hard stop. The Worker:
  1. Stops the think→act loop
  2. Sends one final LLM call: "Summarize what you've accomplished and what remains. This context is ending."
  3. Takes that response as the summary
  4. Returns `WorkerOutput` to the orchestrator

The orchestrator can then spawn a fresh Worker with the summary as seed context — a new turn.

## Output

```rust
struct WorkerOutput {
    status: ExitStatus,
    summary: String,            // model-generated via summary tool
    last_messages: Vec<Message>, // tail of conversation (for debugging/handoff)
}

enum ExitStatus {
    Done,                // model called summary tool
    ContextExhausted,    // hit 95%, forced summary
    Error(String),       // something broke
}
```

## Tools

Eleven tools:

| Tool | Purpose |
|------|---------|
| `read` | Read file contents from workspace |
| `write` | Write file to workspace (creates parent dirs) |
| `bash` | Execute shell commands (with timeout) |
| `speak` | Send to the current channel (feature + payload) |
| `send_message` | Send to any channel (adapter + feature + payload) |
| `switch_channel` | Change which channel the Worker is "in" |
| `sleep` | Pause the loop internally, wait for timer or watched notification |
| `watch` | Manage which channels trigger early wake during sleep |
| `flash` | Send high-priority interrupt to a partner Worker |
| `request_model` | Ask the orchestrator to switch LLM model |
| `summary` | Stop execution and return handoff to orchestrator |

### Worker State

The Worker maintains mutable state:

- **Current channel** — the adapter + channel that `speak` targets. Set explicitly by the model via `switch_channel`. Defaults to `ground` (the human operator's DM channel) on startup.

### Tool Schemas

**read**
```json
{
  "path": "string (required)",
  "start_line": "number (optional)",
  "end_line": "number (optional)"
}
```
Returns file contents as string. If `start_line`/`end_line` provided, returns only that range. Error if file doesn't exist. Routed through the orchestrator (`POST /file/read`) for file locking when Workers share workspaces.

**write**
```json
{
  "path": "string (required)",
  "content": "string (required)",
  "mode": "\"overwrite\" | \"append\" | \"insert\" (required)",
  "at_line": "number (required if mode is insert)"
}
```
Creates parent directories if needed. Returns success/error. Routed through the orchestrator (`POST /file/write`) for file locking when Workers share workspaces.

**bash**
```json
{
  "command": "string (required)",
  "timeout_seconds": "number (optional, default 120, max 600)",
  "working_directory": "string (optional)"
}
```
Returns `{stdout, stderr, exit_code}`.

**speak**
```json
{
  "feature": "number (required, AdapterFeature int)",
  "payload": "object (required, matches feature schema)"
}
```
Sends to the current adapter + channel. The Worker runtime injects the current channel into the payload before forwarding. Defaults to `ground` (operator's DM channel) if `switch_channel` hasn't been called. The feature int tells the adapter exactly what action to take. Returns the adapter's response (e.g. `{message_id}`) or error.

**send_message**
```json
{
  "adapter": "string (required)",
  "feature": "number (required, AdapterFeature int)",
  "payload": "object (required, matches feature schema)"
}
```
Sends to any adapter. The payload must include `channel` and whatever else the feature schema requires. Does NOT change the current channel. Returns the adapter's response or error.

**switch_channel**
```json
{
  "adapter": "string (required)",
  "channel": "string (required)"
}
```
Sets the Worker's current channel. Subsequent `speak` calls target this channel. Returns confirmation of the new current channel.

**sleep**
```json
{
  "minutes": "number (optional)"
}
```
Pauses the loop inside the Worker process. If `minutes` is omitted, sleeps indefinitely until a watched channel notification wakes it. During sleep, notifications continue to arrive on the HTTP endpoint and are written to workspace files. If a notification arrives on a watched channel, the Worker wakes early. When the timer expires (or early wake), the loop resumes with a status message containing batched notifications and context usage.

**watch**
```json
{
  "add": [{"adapter": "string", "channel": "string"}] (optional),
  "remove": [{"adapter": "string", "channel": "string"}] (optional)
}
```
Additive/subtractive. Manages the set of adapter+channel pairs that trigger early wake during sleep. Returns the current watch list after the operation.

**summary**
```json
{
  "summary": "string (required)"
}
```
Stops execution. The summary string is the model's handoff — what it accomplished, what remains, any context the next Worker would need. This is the only way the Worker exits cleanly. The Worker returns `WorkerOutput` with status `Done` and this summary to the orchestrator.

**flash**
```json
{
  "name": "string (required, target worker name)",
  "payload": "object (required)"
}
```
Sends a high-priority interrupt to any Worker by name via the orchestrator's `/flash` endpoint. The orchestrator is a dumb router — it looks up the target and forwards the payload. The receiving Worker handles interrupt timing itself, injecting the flash between tool calls if needed. Used for actor ↔ spectator communication, but not limited to partners.

**request_model**
```json
{
  "model": "string (required, model name from config)"
}
```
Asks the orchestrator to switch the Worker's LLM model. The orchestrator responds with the new endpoint and model name. The Worker uses the new model on its next LLM call.

### Why these eleven?

- `read` + `write` give the model access to its workspace
- `bash` gives it general-purpose compute
- `speak` + `send_message` + `switch_channel` let it communicate through adapters with both convenience (speak to current) and precision (send to any)
- `sleep` + `watch` let the model manage its own idle behavior
- `flash` enables high-priority communication between partner Workers
- `request_model` lets the model choose the right LLM for the task
- `summary` gives the model explicit control over when execution ends
- Everything else can be built from these primitives

## Notification Endpoint

The Worker exposes an HTTP endpoint that receives inbound messages from adapters.

**Inbound message format:**
```json
{
  "adapter": "string",
  "event_type": "string",
  "payload": {}
}
```

Channel, author, content, and all other event data are inside `payload`. The Worker extracts what it needs.

When a message arrives:

1. The content is appended to the conversation file for that adapter/channel
2. If the Worker is active (in the loop): the notification is batched and delivered in the next status message
3. If the Worker is sleeping and the channel is in the watch list: the Worker wakes early, injects a status message with batched notifications, and resumes the loop

## Worker HTTP API

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/notify` | Inbound notifications from adapters |
| POST | `/registry` | Registry updates from orchestrator |
| POST | `/flash` | Flash messages from other Workers (via orchestrator) |
| GET | `/health` | Health check |

## Startup

The Worker does not call the LLM on boot. It waits for its first notification from the orchestrator, then begins the loop.

## Error Handling

- **LLM returns malformed tool calls** (hallucinated tool names, bad params): retry with exponential backoff — 1 minute, 2 minutes, 5 minutes. On each retry, append a system message explaining the error. After 3 failures, exit with `Error` status.
- **Tool execution fails**: the error is returned to the model as a tool result. The model decides what to do.
- **Adapter unreachable** (speak fails): error returned to the model as a tool result. The Worker does not crash.
- **LLM unreachable**: Worker exits with `Error` status.

## What The Worker Does NOT Do

- Parse or understand message content
- Decide priorities
- Manage memory (no embeddings, no vector search, no flash queue)
- Know about adapter internals (it just forwards JSON via `speak`/`send_message`)
- Decide when to stop (the model decides via `summary`)

## What The Worker DOES Do (actively)

- Track current channel state (set by `switch_channel`, defaults to `ground`)
- Track token usage via model API responses
- Inject status messages when the model responds with text (context usage, current time, notifications)
- Inject context pressure warnings at 80%
- Force summary at 95% context
- Manage the sleep timer and early wake from watched channels
- Write inbound notifications to workspace
- Retry malformed LLM responses (1m, 2m, 5m backoff)
- **Persist context to disk** — after every mutation to the message list (model response AND each tool result), the Worker writes the full message list as a JSONL context file. If the Worker crashes, it can be restarted from this file.

## Resolved Decisions

1. **Idle behavior** — the model calls `sleep(minutes)` which pauses the loop inside the Worker. No orchestrator involvement. Watched channel notifications can wake the loop early.
2. **Parallel tool calls** — all tool calls in a single LLM response are independent and executed in parallel. Sequential chaining happens across turns (call → result → call → result).
3. **Token counting** — use the model API's `usage.total_tokens` from each response. No client-side token counting. First call has no baseline — accept this.
4. **Tool errors** — all errors (speak, bash, read, write) are returned to the model as tool results. The Worker does not crash. The model decides how to handle them.
5. **Notification batching** — notifications are batched and delivered in the next status message.
6. **Early wake** — notifications on watched channels (managed via `watch` tool) wake the Worker from sleep internally.
7. **LLM retry** — malformed tool calls trigger retries with backoff: 1 minute, 2 minutes, 5 minutes. Each retry includes a system message explaining the error. After 3 failures, exit with `Error`.
8. **Startup** — the Worker waits for its first notification from the orchestrator before entering the loop.
9. **End of turn** — when the model responds with text (no tool calls), the Worker does NOT treat this as "done." It injects a status message with context usage, current time, and notifications, and asks the model to continue or call `summary`.
10. **Stopping execution** — only the `summary` tool stops the Worker cleanly. The model must explicitly decide to end. If `summary` and `sleep` are in the same tool call batch, `summary` wins.
11. **Default channel** — `speak` defaults to the `ground` channel (human operator's DM) if `switch_channel` hasn't been called.
12. **Context persistence** — the Worker persists the message list to JSONL after every mutation (model response AND each tool result). Crash recovery loads from this file.
13. **Flash during LLM call** — if a flash arrives while the LLM is being called, it is queued and injected immediately after the response, before processing any tool calls from that response.
