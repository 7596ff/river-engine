# Spectator Redesign Implementation Plan Review — V2

**Plan Reviewed:** `docs/superpowers/specs/2026-05-12-spectator-redesign.md`
**Date:** 2026-05-12
**Reviewer:** Gemini CLI

## Summary of Improvements

The updated implementation plan addresses several critical flaws from the first review, most notably the **Spectator Feedback Loop** and the **Missing Prompt File**. The addition of a "Catch-up Loop" sanity check also improves the robustness of the sweep logic.

## 1. Resolved Issues

*   **Feedback Loop (Resolved):** Task 1 Step 3 now explicitly filters out system messages starting with `[spectator]`.
*   **Prompt File (Resolved):** Task 7 creates a comprehensive `on-sweep.md` prompt template.
*   **Catch-up Logic (Improved):** Task 4 Step 4 now checks for remaining *non-filtered* entries before deciding to loop, preventing infinite catch-up loops on noise.
*   **Observability (Resolved):** The use of the spectator's snowflake generator for the observability message is correctly tasked.

## 2. Critical Bugs & Compilation Errors (Remaining)

### The "Stall on Noise" Bug (Task 4 Step 4)
There is a logic error in how filtered entries (heartbeats, cursors) are handled:
*   **Issue:** If a sweep reads only filtered entries, `transcript.is_empty()` becomes true. The code says "// Advance cursor past them", but it only updates `self.last_sweep` (an in-memory timestamp) and returns `false`.
*   **Impact:** Since no move was written to `moves.jsonl`, `read_cursor()` on the next sweep will return the *same* old snowflake. The spectator will read the same heartbeats again and again, effectively stalling its progress until a non-filtered entry (like a message) finally arrives to "pull" the cursor forward.
*   **Fix:** If entries exist but the transcript is empty, the spectator must still "advance the cursor" by writing a no-op move or a dedicated cursor entry to `moves.jsonl`.

### The `cleanup_tool_results` Mismatch (Task 4 Step 4)
The call to `cleanup_tool_results` will fail to compile for two reasons:
1.  **Static vs Instance:** It is called as a static method `HomeChannelWriter::cleanup_tool_results(...)`, but it was defined as an instance method `&self` in the Home Channel plan. It should be called on `self.home_channel_writer`.
2.  **Argument Mismatch:** The call passes `&Path, &str, &str`, but the original definition expected `&[String]` (a list of snowflake IDs to delete). The spectator doesn't have the list of IDs here; it only has the range. The implementation of `cleanup_tool_results` must be updated to handle a range or the caller must provide the IDs.

### `read_home_since` vs `read_home_since_opt` (Task 3)
*   Task 3 Step 3 implements `read_home_since` and `read_home_since_opt`.
*   Task 4 Step 4 calls `log.read_home_since_opt(cursor.as_deref())`.
*   **Potential Issue:** Ensure `ChannelLog::read_all_home()` (used by `read_home_since`) is implemented correctly to return `HomeChannelEntry` (tagged) and not the old `ChannelEntry` (untagged). Task 3 Step 2 refers to `read_all_home`, which was added in Task 3 of the **Home Channel** plan.

## 3. Missing: Integration & Sweep Tests

*   **Logic Complexity:** Task 4's `sweep` function is the core of the feature and contains complex branching (cursor reading, catch-up, LLM handling, budget management).
*   **Recommendation:** Add a Task 4.1 to implement unit tests for the `sweep` state machine using a mock model client. Relying purely on integration "smoke tests" at the end of the plan is risky for such a central component.

## 4. Server Config Gaps

*   Task 5 wires the config in `server.rs`, but the plan still lacks steps to expose `sweep_interval` and `sweep_token_budget` in the project's configuration files (e.g., `river.example.json`) or CLI arguments. Users will be stuck with the hardcoded defaults in `SpectatorConfig`.

## Conclusion

The plan is much closer to execution-ready, but the **Stall on Noise** bug and the **Cleanup Compilation Error** must be fixed before implementation begins. The spectator's inability to advance past heartbeats without a narrative move is a significant functional gap.
