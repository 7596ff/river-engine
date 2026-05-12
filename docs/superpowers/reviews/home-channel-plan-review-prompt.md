# Home Channel Plan Review Prompt

Paste the plan below into Gemini or another reviewer, followed by this prompt. Also include the spec (`docs/superpowers/specs/2026-05-12-home-channel.md`) for reference.

---

You are reviewing an implementation plan for the **home channel** feature of river-engine, a Rust multi-agent orchestration system. The plan has 10 tasks that transform the agent's invisible internal context into a visible, append-only JSONL log.

This is a critical review. Your job is to find the places where the plan will fail when an engineer tries to execute it. Find missing steps, wrong assumptions about existing code, ordering problems between tasks, and code that won't compile.

## 1. Compilation Order

The plan modifies interconnected modules. Will the codebase compile after each task, or do some tasks create broken intermediate states?

- Task 1 adds new entry types. Task 2 uses them. Task 3 uses them. Are the imports and module visibility correct at each step?
- Task 5 modifies `AgentTask` heavily while Task 7 removes `ChannelContext`. Can Task 5 compile before Task 7 runs? Or does Task 5 assume `ChannelContext` is already gone?
- Task 7 says "fix all compilation errors" — this is a placeholder. What specific files will break?

## 2. The `serde(untagged)` Problem

`ChannelEntry` currently uses `#[serde(untagged)]` (see `entry.rs` line 9). Untagged enums try each variant in order during deserialization. Adding `ToolEntry` and `HeartbeatEntry` to this enum will cause deserialization ambiguity:

- A `ToolEntry` has `kind`, `tool_name`, `tool_call_id` — fields that don't exist on `MessageEntry`. Will serde correctly distinguish them with `untagged`?
- If not, the plan needs to switch to `#[serde(tag = "type")]` or another discriminator. This is a **migration-breaking change** for existing JSONL files.

## 3. The Tool Name Threading Gap

The plan's Task 5 Step 4 admits: `"unknown".to_string()` for tool name in results. The comment says "needs threading." But:

- The `tool_results` variable in the current code is `Vec<(String, String)>` — `(tool_call_id, result_text)`. The tool name is not preserved through execution.
- This means every `ToolEntry::result` in the home channel will have `tool_name: "unknown"`. The context builder, spectator, and any log viewer will see meaningless tool entries.
- This isn't a minor issue — it's a data quality problem that affects every tool call. The plan should either fix the threading or explicitly acknowledge the limitation.

## 4. The Context Builder's Incomplete Tool Call Mapping

Task 3's context builder has a comment: `// (simplified — full implementation maps to ToolCallRequest)` for `tool_call` entries. But:

- The model expects assistant messages with `tool_use` to have a specific structure (function name, arguments, ID). The context builder must reconstruct this exactly or the model will reject the messages.
- The plan doesn't show how `ToolEntry::call` maps back to `ChatMessage::assistant` with tool calls. This is the hardest part of the context builder and it's hand-waved.

## 5. The `MessageEntry::user` Tag Format

Task 1 creates a `MessageEntry::user` constructor that embeds the tag in the content: `[user:discord:789012/general] hello`. But:

- The original content is now mixed with metadata in the `content` field. If anything needs to extract the raw message content later (adapter log, search, display), it has to parse the tag back out.
- The `author` and `author_id` fields are set, but the adapter/channel info is only in the content string. Should there be dedicated `source_adapter` and `source_channel` fields on `MessageEntry`?

## 6. The Write-Ahead Without Transaction

Task 6 writes to the home channel first, then the adapter log. But:

- If the home channel write succeeds and the adapter log write fails, the home channel has an entry the adapter log doesn't. The spec says this is acceptable ("the home channel entry still exists"). But the adapter log is supposed to be a complete record of what happened on that platform. A missing entry means the adapter log is incomplete.
- Is this actually acceptable? Or should both writes be attempted and failures logged but not blocking?

## 7. The `PersistentContext` Removal Gap

The plan says `PersistentContext` is replaced by the home channel context builder. But:

- Task 3 creates the context builder.
- Task 5 wires the home channel into the turn cycle.
- But no task explicitly removes `PersistentContext` or migrates `AgentTask` to use `build_context()` instead of `self.context`.
- When does `AgentTask` stop using `self.context: PersistentContext` and start using `build_context()`? This is the actual switchover and it's not explicitly tasked.

## 8. The SQL Removal Deferral

The spec says SQL message storage is eliminated. The plan's self-review says "SQL database removal not explicitly tasked — this is a larger migration." But:

- `AgentTask` currently calls `persist_turn_messages` which writes to the DB. If this isn't removed, the system is triple-writing (home channel + adapter log + SQL).
- The `Database` is used by the spectator to load moves. If SQL is deferred, how does the spectator find moves? From files? From the DB? The plan's Task 8 says "update spectator" but doesn't clarify the move storage.

## 9. Missing: Home Channel Initialization

When does the home channel file get created? Task 10 Step 3 says "create home channel directory on birth." But:

- What about existing agents that don't have a home channel? Is there a migration path?
- If the gateway starts and the file doesn't exist, does the writer create it? The current `ChannelLog::append_entry` creates the file on first write. Is this sufficient?

## 10. Test Coverage Gaps

Several tasks say "write tests" without specifying what to test:

- Task 5 (the largest, most complex task) says "run tests" but doesn't specify new tests for the home channel writes. This is where bugs will live.
- Task 8 (spectator) says "run tests" but the spectator's relationship to the home channel is fundamentally different from its relationship to `PersistentContext`. The existing tests may pass while the new behavior is untested.

Be specific. Cite task numbers and step numbers. Suggest concrete fixes.
