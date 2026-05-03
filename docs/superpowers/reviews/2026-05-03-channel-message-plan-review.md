# Review: Channel Messages Implementation Plan

**Spec Version:** 2026-05-03
**Plan Version:** 2026-05-03
**Reviewer:** Gemini CLI
**Status:** Approved with Critical Findings

## Summary

The implementation plan is highly detailed and correctly translates the specification into a series of actionable tasks. However, it introduces significant compilation breakage in Task 3 that persists across several tasks, and some tool dependencies are not fully addressed in the cleanup phase.

## Findings

### Severity: Critical

#### 1. Compilation Breakage (Tasks 3 through 6)
Task 3 changes the `MessageQueue` payload to `ChannelNotification`. This will immediately break `crates/river-gateway/src/api/routes.rs`, `crates/river-gateway/src/agent/task.rs`, and likely several tests.
*   **Risk:** The codebase will be in a non-compilable state for at least 4 tasks. This makes intermediate testing and CI/CD validation impossible.
*   **Recommendation:** Combine Tasks 3, 4, and 5 into a single logical "Switch Wire" task, or accept the breakage but prioritize moving through these tasks rapidly in a single session.

#### 2. Transitive Breakage of SyncConversationTool
Task 7 removes the `conversations` and `inbox` modules, but `crates/river-gateway/src/tools/sync.rs` (SyncConversationTool) heavily depends on them for `WriteOp`, `Author`, and `build_discord_path`.
*   **Risk:** The plan mentions fixing compilation errors in Task 7 Step 4, but `SyncConversationTool` needs a significant logic rewrite to use `ChannelLog::append_entry` instead of `mpsc::Sender<WriteOp>`.
*   **Recommendation:** Add a specific sub-task to Task 6 or 7 to refactor `SyncConversationTool` to use the new `channels` module.

### Severity: Important

#### 1. Missing Snowflakes in Tools
Task 6 notes that `SendMessageTool` and `SpeakTool` may need the `SnowflakeGenerator` added to their constructors.
*   **Risk:** Implementer might struggle with dependency injection if the plan isn't explicit about where the generator comes from.
*   **Recommendation:** Explicitly state that `server.rs` must pass `state.snowflake_gen` (or the generator created in `run()`) to these tool constructors.

#### 2. Test Format Obsolescence
Task 7 Step 5 mentions removing old tests referencing the inbox format.
*   **Risk:** Deleting tests without replacing them reduces confidence in the refactor.
*   **Recommendation:** Ensure Task 8 (Integration Test) is comprehensive enough to cover the behaviors previously tested by the deleted inbox tests.

#### 3. Cursor Scanning Performance
The plan implements `rposition` for cursor scanning (Task 2). As noted in the design review, this is $O(N)$ and slow for large logs.
*   **Risk:** Performance degradation over time.
*   **Recommendation:** While acceptable for an initial implementation, add a "TODO" or a future task to implement a cursor cache as recommended in the design review.

### Severity: Suggestion

#### 1. Deduplication Logic in Task 5
Task 5 Step 1 uses a `HashSet` to deduplicate channels from notifications. This is good practice.
*   **Improvement:** Consider if the agent should prioritize channels with "Interactive" notifications if the `MessageQueue` ever re-introduces priorities (currently Task 3 removes them).

#### 2. Sync vs Async I/O
The `ChannelLog` operations in Task 2 use `std::fs` (synchronous). 
*   **Note:** While `tokio::task::block_in_place` is used in tools, `AgentTask` is an async task. Using sync I/O in `AgentTask::turn_cycle` will block the executor thread.
*   **Recommendation:** Consider using `tokio::fs` for `ChannelLog` operations to keep the agent loop fully asynchronous and non-blocking.

## Conclusion

The plan is solid but "loud" (breaks compilation). If the implementer is aware of the transitive breakage in `SyncConversationTool` and the temporary broken state of the build, they can proceed. Addressing the Sync/Async I/O in Task 2 would be a significant quality-of-life improvement for the agent's performance.
