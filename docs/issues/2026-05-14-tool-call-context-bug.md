# Bug: Context builder splits assistant content from tool calls

**Severity:** Critical — breaks tool calling with DeepSeek and any strict OpenAI-compatible API

**Error:** `"An assistant message with 'tool_calls' must be followed by tool messages responding to each 'tool_call_id'. (insufficient tool messages following tool_calls message)"`

## The problem

Two separate bugs combine to break the tool call loop:

### Bug 1: Agent task writes content and tool calls as separate entries

In `agent/task.rs`, when the model returns BOTH content and tool calls (which deepseek does), the code writes them as separate home channel entries:

1. Line ~285: If content exists, write a `MessageEntry::agent(content)` to the home channel
2. Line ~308: Then write `ToolEntry::call()` entries for each tool call

This produces the home channel sequence:
```
msg  agent   | "Sure, let me try that."     ← content only
tool_call    | send_message(...)             ← tool calls separate
tool_result  | send_message → success
```

### Bug 2: Context builder doesn't re-merge them

In `home_context.rs`:

- Line 46: When it sees a `msg agent`, it creates `ChatMessage::assistant(Some(content), None)` — **no tool calls**
- Line 60-86: When it sees `tool_call` entries, it creates a SECOND `ChatMessage::assistant(None, Some(tool_calls))`

This produces the API message sequence:
```
assistant: "Sure, let me try that."          ← no tool_calls field
assistant: null content, tool_calls: [...]   ← tool_calls here
tool: result for tool_call_id X
```

DeepSeek (and any strict OpenAI-compatible API) rejects this because the first assistant message has no tool_calls, the second has tool_calls but no content, and it doesn't match the expected pattern.

### Bug 3: Tool call loop breaks after one iteration

Because bug 2 causes a 400 error on the second model call (after tool results), the loop breaks at the `Err(e) => break` on line 258-259. The turn ends with only one iteration. The agent never gets to see its tool results and respond.

From the logs:
```
Turn 8: iteration=1, tool_calls=5, Turn complete
Turn 9: iteration=1, tool_calls=3, Turn complete
```

Every turn is iteration=1 — the loop never reaches iteration=2 because the second model call fails.

## Evidence

### Home channel showing the split
```
 96 msg  agent   | Boop received. Still here.\n\nLet me keep reading...
 97 tool_call    | read (tcid=call_00_9UDBzhS9WhF4)
 98 tool_call    | read (tcid=call_01_SkFjDde8jALX)
 99 tool_call    | read (tcid=call_02_btg6HfqnbYrS)
100 tool_result  | read (tcid=call_00_9UDBzhS9WhF4)
101 tool_result  | read (tcid=call_01_SkFjDde8jALX)
102 tool_result  | read (tcid=call_02_btg6HfqnbYrS)
103 msg  bystander | beep      ← user had to manually advance
```

### API error from DeepSeek
```
Model API error 400: {"error":{"message":"An assistant message with 'tool_calls' must be followed by tool messages responding to each 'tool_call_id'. (insufficient tool messages following tool_calls message)"}}
```

### Debug logs showing single iteration
```
Loop iteration start | iteration=1
Model response | iteration=1, tool_calls=3, has_content=true
Has tool calls — executing | iteration=1, tool_count=3
Tool execution complete | iteration=1, results=3
Continuing loop — will call model again | iteration=1
Loop iteration start | iteration=2
Model call failed | error=Model API error 400: ...
Turn complete | Turn 1 completed: 3 tool calls (0 failed)
```

## Fix

### Option A: Merge in the context builder (recommended)

In `home_context.rs`, when processing the tail, look ahead: if an `agent` message is immediately followed by `tool_call` entries, merge them into a single `ChatMessage::assistant(Some(content), Some(tool_calls))`.

### Option B: Merge in the agent task writer

In `agent/task.rs`, don't write the agent content and tool calls as separate entries. Either:
- Write a single entry that contains both content and tool calls (needs a new entry type or extend MessageEntry)
- Skip writing the content when tool calls are present (the content is often just narration)

### Option C: Both

Option A fixes the context builder for existing logs. Option B prevents the problem from occurring in new logs.

## Affected files

- `crates/river-gateway/src/agent/home_context.rs` — context builder
- `crates/river-gateway/src/agent/task.rs` — agent task writer (lines 285-319)

## Workaround

Send a bystander message after each tool execution to trigger a new turn. The model sees the tool results in the home channel context on the next turn and can respond. This is what was done during the May 14 session with viola.
