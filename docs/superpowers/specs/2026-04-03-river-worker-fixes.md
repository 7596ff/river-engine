# river-worker Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: Critical

## Summary

river-worker has core think-act loop functioning but critical gaps in spec compliance: sequential tool execution (should be parallel), missing conversation file format (60+ lines of spec unimplemented), no retry logic for malformed tool calls, and LlmClient not updated after model switch. Estimated effort: 3-4 days.

## Critical Issues

### Issue 1: Tools not executed in parallel

- **Source:** Both reviews
- **Problem:** Spec requires `execute_tools_parallel(calls, state, config).await`. Implementation uses sequential `for call in &calls { execute_tool(...).await }` loop.
- **Fix:**
  ```rust
  use futures::future::join_all;

  let futures: Vec<_> = calls.iter().map(|call| {
      execute_tool(call, &state, config, &mut generator, client)
  }).collect();
  let results = join_all(futures).await;
  ```
- **Files:** `crates/river-worker/src/worker_loop.rs`
- **Tests:** Test with multiple concurrent tool calls, verify execution time

### Issue 2: Conversation file format not implemented

- **Source:** Both reviews
- **Problem:** Spec defines 60+ lines for conversation file format with paths `workspace/conversations/{adapter}/{guild_id}-{guild_name}/{channel_id}-{channel_name}.txt`, line types `[x]`, `[>]`, `[ ]`, `[+]`, `[r]`, `[!]`, and compaction logic. None of this exists.
- **Fix:** Implement conversation file module with:
  1. Line type enum and parsing
  2. File path generation from channel metadata
  3. Append on notify (` [ ]` unread)
  4. Append on speak (`[>]` sent)
  5. Update to `[x]` when read
  6. Compaction logic for sorted section
- **Files:** Create `crates/river-worker/src/conversation.rs`, update `http.rs`, `tools.rs`
- **Tests:** Test all line type parsing, file creation, compaction

### Issue 3: No malformed tool call retry logic

- **Source:** Both reviews
- **Problem:** Spec requires: "Retry with backoff: 1 minute, 2 minutes, 5 minutes. Inject system message explaining error on each retry. After 3 failures: exit with Error status."
- **Fix:** Add retry logic:
  ```rust
  struct MalformedRetryState {
      count: u32,
      backoffs: [Duration; 3],  // 1m, 2m, 5m
  }

  // On malformed call:
  // 1. Increment count
  // 2. If count > 3, return ExitStatus::Error
  // 3. Inject system message explaining error
  // 4. Sleep for backoff[count-1]
  // 5. Retry
  ```
- **Files:** `crates/river-worker/src/worker_loop.rs`
- **Tests:** Test retry sequence, verify exit after 3 failures

### Issue 4: LlmClient not updated after model switch

- **Source:** Both reviews
- **Problem:** `request_model` tool updates `state.model_config` but the `LlmClient` instance in the loop is not recreated. The client has an `update_config` method that's never called.
- **Fix:** After `request_model` success, call `llm_client.update_config(&new_config)` or recreate the client
- **Files:** `crates/river-worker/src/worker_loop.rs`, `crates/river-worker/src/tools.rs`
- **Tests:** Test that model switch actually changes LLM endpoint

## Important Issues

### Issue 5: prepare_switch doesn't check busy state

- **Source:** Both reviews
- **Problem:** Spec requires checking "not mid-tool-execution" and "not mid-LLM-call". Implementation only checks `switch_pending` flag.
- **Fix:** Add execution state tracking:
  ```rust
  pub struct WorkerState {
      // ...
      pub executing_tool: bool,
      pub calling_llm: bool,
  }
  ```
  Check these in `prepare_switch` handler.
- **Files:** `crates/river-worker/src/state.rs`, `crates/river-worker/src/http.rs`
- **Tests:** Test that prepare_switch fails when mid-operation

### Issue 6: Summary clears context before acknowledgment

- **Source:** Brutal review
- **Problem:** Context is cleared before orchestrator confirms receipt. If orchestrator fails to receive, context is lost.
- **Fix:** Return output first, wait for orchestrator ack, then clear context
- **Files:** `crates/river-worker/src/worker_loop.rs`
- **Tests:** Test context preserved on failed output delivery

### Issue 7: commit_switch doesn't reload role file

- **Source:** First review
- **Problem:** Externally-triggered role switches (via HTTP from orchestrator) don't reload role file from `workspace/roles/{new_baton}.md`.
- **Fix:** Add role reload in `handle_commit_switch`
- **Files:** `crates/river-worker/src/http.rs`
- **Tests:** Test role content changes after HTTP-triggered switch

### Issue 8: No river-context integration for context building

- **Source:** Both reviews
- **Problem:** Spec implies using `river-context` for context assembly. Implementation uses raw `Vec<OpenAIMessage>`. Channel switches don't properly reorder context.
- **Fix:** Integrate `river-context::assemble_context()` for proper context building
- **Files:** `crates/river-worker/src/worker_loop.rs`
- **Tests:** Test context reordering on channel switch

### Issue 9: Sleep tool returns wrong format

- **Source:** First review
- **Problem:** Spec says return `{ "sleeping": true, "until": "2026-04-02T15:30:00Z" }`. Implementation returns `{ "sleeping": true, "minutes": N }`.
- **Fix:** Calculate `until` timestamp and return ISO8601 string
- **Files:** `crates/river-worker/src/worker_loop.rs`
- **Tests:** Verify sleep response format

## Minor Issues

### Issue 10: Polling loops instead of async notify

- **Source:** Both reviews
- **Problem:** `wait_for_activation` and `sleep_until_wake` poll every 100ms using `tokio::time::sleep`.
- **Fix:** Use `tokio::sync::Notify` for efficient wake handling
- **Files:** `crates/river-worker/src/worker_loop.rs`
- **Tests:** N/A (performance improvement)

### Issue 11: std::fs in async context

- **Source:** Brutal review
- **Problem:** Uses `std::fs::{File, OpenOptions}` which blocks the tokio runtime.
- **Fix:** Use `tokio::fs` throughout
- **Files:** `crates/river-worker/src/persistence.rs`
- **Tests:** Existing tests should pass

### Issue 12: Side parsed manually

- **Source:** Brutal review
- **Problem:** Manual string matching for "left"/"right" instead of using clap value_enum or serde.
- **Fix:** Use clap's `ValueEnum` derive for Side type
- **Files:** `crates/river-worker/src/main.rs`
- **Tests:** N/A (cleanup)

### Issue 13: Missing tool tests

- **Source:** Both reviews
- **Problem:** 17 tools, 1000+ lines, only persistence.rs tested.
- **Fix:** Add unit tests for each tool's argument parsing and basic execution
- **Files:** `crates/river-worker/src/tools.rs`
- **Tests:** Test each tool type

## Spec Updates Needed

None - implementation should match spec.

## Verification Checklist

- [ ] Tool execution is parallel (use join_all)
- [ ] Conversation file format implemented with all line types
- [ ] Malformed call retry with backoff works
- [ ] LlmClient updated after model switch
- [ ] prepare_switch checks mid-operation state
- [ ] Summary waits for ack before clearing context
- [ ] commit_switch reloads role file
- [ ] river-context used for context building
- [ ] Sleep returns ISO8601 `until` timestamp
- [ ] Tool tests added
