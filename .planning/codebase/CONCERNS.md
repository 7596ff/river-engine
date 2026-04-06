# Codebase Concerns

**Analysis Date:** 2026-04-06

## Tech Debt

### Pervasive Unwrap/Expect Usage

**Issue:** The codebase contains 456+ instances of `unwrap()`, `expect()`, and `unwrap_or_default()` patterns across the workspace. While some are intentional for initialization, many occur in runtime code paths.

**Files:**
- `crates/river-worker/src/tools.rs` - Multiple unwraps in JSON serialization (lines 212-213, 469, 476, 567-568, 580, 619, 667)
- `crates/river-worker/src/workspace_loader.rs` - Unsafe unwraps in message parsing (lines 27, 31, 35, 341, 352, 361)
- `crates/river-embed/src/store.rs` - SystemTime conversion (line 162)
- `crates/river-worker/src/llm.rs` - Response parsing with unwrap_or_default (lines 172, 195)
- `crates/river-worker/src/worker_loop.rs` - JSON serialization fallback (lines 212-213)

**Impact:** Potential panic crashes if error conditions occur. Silently swallows errors with `unwrap_or_default()` making bugs harder to diagnose.

**Fix approach:**
1. Replace `unwrap()` with `?` operator in Result-returning functions
2. Use explicit error handling instead of `unwrap_or_default()` for critical paths
3. Only use `unwrap()` in functions annotated with panic safety guarantees
4. Add validation tests for error paths

### Unsafe FFI Code Without Documentation

**Issue:** `crates/river-embed/src/store.rs` contains unsafe code (line 13-16) for sqlite-vec initialization using `transmute()`. The safety invariants are not documented.

**Files:** `crates/river-embed/src/store.rs` (lines 13-16)

**Current mitigation:** Uses `Once` to ensure single initialization, but the transmute itself is undocumented.

**Recommendations:**
1. Add SAFETY comments explaining function pointer casts are valid
2. Document why transmute is necessary here (function pointer type erasure for sqlite3 ABI)
3. Consider safer alternatives if sqlite-vec API supports them

## Known Bugs

### Message Parsing Panics on Malformed Input

**Issue:** `parse_message_line()` in `crates/river-worker/src/workspace_loader.rs` uses `.unwrap()` on successful regex captures, which will panic if the regex structure changes or unexpected input arrives.

**Files:** `crates/river-worker/src/workspace_loader.rs` (lines 341, 352, 361)

**Trigger:** Malformed conversation files with inconsistent message format

**Workaround:** None - will crash the worker

**Fix approach:** Return Option/Result from parse_message_line, handle None cases explicitly in caller

### Discord Connection Restoration Has Edge Cases

**Issue:** Discord gateway reconnection logic in `crates/river-discord/src/discord.rs` creates new Shard instances in an infinite loop (line 62). If event channel is dropped while reconnecting, the loop doesn't break cleanly.

**Files:** `crates/river-discord/src/discord.rs` (lines 51-127)

**Potential issue:** Could spawn unlimited shard instances if event_tx.send() repeatedly fails during a reconnect loop

**Fix approach:** Track failed send attempts and break from reconnection loop after N failures

### SQLite Concurrency Without Connection Pool

**Issue:** `crates/river-embed/src/store.rs` wraps a single `rusqlite::Connection` in `Mutex<Store>` (line 100 of main.rs). SQLite allows concurrent reads but the Mutex serializes all access.

**Files:**
- `crates/river-embed/src/store.rs` (line 66: `conn: Connection`)
- `crates/river-embed/src/main.rs` (line 100: `store: Mutex<Store>`)

**Impact:** High contention under concurrent search/index operations; search requests block index requests and vice versa

**Scaling path:**
1. Consider pooling (r2d2 or sqlx with pooling)
2. Or use rusqlite with WAL mode for better concurrent reads
3. Profile actual contention under load

## Security Considerations

### Bash Command Execution Without Escaping

**Issue:** `execute_bash()` in `crates/river-worker/src/tools.rs` passes arbitrary user command strings to `sh -c` without validation or escaping (lines 352-354).

**Files:** `crates/river-worker/src/tools.rs` (lines 311-375)

**Current mitigation:**
- Timeout applied (capped at 600s)
- Working directory validation
- No shell escape applied

**Risk:** If LLM or user can control `command` argument, arbitrary code execution possible. However, model outputs are controlled by the user, so this is by design.

**Recommendations:**
1. Document that this is intentional capability delegation to LLM
2. Log all bash commands executed for audit trails
3. Consider sandboxing via containers/nix-shell if needed

### File Path Operations Without Normalization

**Issue:** File path handling in `execute_read()`, `execute_write()`, and `execute_delete()` doesn't prevent directory traversal in relative paths.

**Files:** `crates/river-worker/src/tools.rs` (lines 104-309)

**Current approach:**
- Absolute paths used as-is
- Relative paths joined with workspace (lines 112, 181, 238)
- No canonicalization

**Risk:** Limited because workspace is workspace root, but `../` paths could escape workspace if not careful.

**Recommendations:**
1. Canonicalize paths after joining
2. Verify final path is still within workspace
3. Add test cases for `../` traversal attempts

### API Key Exposure in Responses

**Issue:** Model configuration including API keys is returned in worker registration response `crates/river-orchestrator/src/http.rs` (lines 68-83).

**Files:** `crates/river-orchestrator/src/http.rs` (lines 75-83: `api_key` field)

**Current mitigation:** Response is returned to worker that needs key

**Risk:** If orchestrator logs are captured or network traffic is sniffed, API keys are exposed

**Recommendations:**
1. Review whether `api_key` must be in response or can be stored in worker config
2. Add TLS requirement between components
3. Consider using header-based auth instead of embedding keys in response bodies

## Performance Bottlenecks

### Message Parsing Efficiency

**Issue:** `load_conversation()` in `crates/river-worker/src/workspace_loader.rs` reads and parses entire conversation files into memory (line 156-250).

**Files:** `crates/river-worker/src/workspace_loader.rs` (lines 156-250)

**Cause:** Synchronous file read and regex parsing for each message

**Improvement path:**
1. Add pagination/cursor support for large conversation files
2. Pre-compile regex patterns
3. Consider lazy loading for historical messages beyond context window

### LLM Context Assembly Every Loop

**Issue:** `build_context()` in `crates/river-worker/src/worker_loop.rs` assembles full context from multiple channels on every LLM call (around line 100+).

**Files:** `crates/river-worker/src/worker_loop.rs` (>100 lines)

**Cause:** No caching of assembled context between iterations

**Impact:** Quadratic time complexity as context grows

**Improvement path:**
1. Cache assembled context and only rebuild if channel data changed
2. Implement incremental context updates
3. Profile actual assembly time

### Snowflake Generator Thread Spawn

**Issue:** `crates/river-snowflake/src/snowflake/generator.rs` spawns a native thread for timestamp updates (line 149).

**Files:** `crates/river-snowflake/src/snowflake/generator.rs` (line 149)

**Impact:** Creates OS thread per generator instance; not ideal for embedded use

**Improvement path:**
1. Use tokio::task for async generators
2. Consider atomic-based time tracking instead of background task

## Fragile Areas

### Workspace File Format Dependencies

**Files:**
- `crates/river-worker/src/workspace_loader.rs` - Message format parsing (lines 339-388)
- Fixture tests in `crates/river-context/tests/fixtures/` - Format examples

**Why fragile:**
- Regex-based parsing of conversation format with no schema validation
- No versioning of message format
- Single `.unwrap()` on successful captures means any format change crashes worker

**Safe modification:**
1. Add format version header to conversation files
2. Implement format readers for each version
3. Write extensive format tests before changing

**Test coverage gaps:**
- No tests for format evolution
- No tests for truncated/corrupted conversation files
- No tests for extremely large conversations

### Discord Event Handling and Channel Management

**Files:** `crates/river-discord/src/discord.rs` (865 lines total)

**Why fragile:**
- Large monolithic file with event conversion logic
- Event loop creates new Shard on every reconnect
- No rate limiting on event processing
- Resource cleanup not explicit

**Safe modification:**
1. Split event handling into separate modules
2. Add explicit Shard lifecycle management
3. Add backpressure handling for event queue

**Test coverage gaps:**
- No integration tests with actual Discord gateway
- Reconnection scenarios only in comments
- Event ordering assumptions not tested

### Role Switching Protocol State Machine

**Files:**
- `crates/river-orchestrator/src/http.rs` - Switch handling (lines 200+)
- `crates/river-worker/src/worker_loop.rs` - Worker-side coordination

**Why fragile:**
- Distributed protocol with 5-second timeout per phase (line 24 in http.rs)
- No explicit state machine; state tracked in multiple fields
- Timeout/abort logic not symmetrical

**Safe modification:**
1. Implement explicit state machine (Prepare → Commit/Abort → Done)
2. Add logging at each phase transition
3. Test timeout scenarios

**Test coverage:** Orchestrator role switching tests minimal

## Scaling Limits

### Memory Usage of Full Context Assembly

**Observed:** Context builds full channel history into memory before truncation

**Current capacity:** Effective context limited by model's context_limit (typically 8192 tokens, line 81 in http.rs)

**Limit:** When conversations exceed 10MB+ on disk, assembly becomes memory-intensive

**Scaling path:**
1. Implement streaming context assembly
2. Add sliding window approach for historical messages
3. Use embeddings-based retrieval instead of chronological assembly

### Embed Service Single SQLite Connection

**Observed:** All search/index operations serialize through single Mutex

**Current capacity:** ~100-1000 concurrent requests before lock contention

**Limit:** SQLite WAL mode helps, but still serialized at application layer

**Scaling path:** See SQLite Concurrency issue above

### Snowflake ID Generation Throughput

**Files:** `crates/river-snowflake/src/snowflake/generator.rs`

**Current:** Generator handles up to ~1M IDs per second per instance (thread-based)

**Limit:** Only one timestamp update thread per generator; clock resolution limits

**Scaling path:** Use cached timestamps and batch ID generation

## Dependencies at Risk

### No Explicit Dependency Pinning

**Risk:** Workspace dependencies in `Cargo.toml` use `1.0` ranges which allow breaking changes

**Files:** `Cargo.toml` (lines 13-42)

**Impact:**
- `tokio = 1.0` - Could change from 1.39 to 1.99 with breaking API changes
- `serde = 1.0` - Similar range
- `reqwest = 0.12` - Pre-1.0, higher risk

**Migration plan:**
1. Pin to specific versions `tokio = "=1.39.0"`
2. Review changelog before minor updates
3. Test integration after dependency updates

### Twilight Discord Library

**Risk:** Community-maintained; version 0.16 is stable but not widely adopted

**Files:** `crates/river-discord/src/discord.rs` - Heavy twilight usage

**Impact:** If unmaintained, Discord API changes won't be addressed

**Recommendation:** Monitor twilight repository activity; have serenity as fallback

## Missing Critical Features

### No Graceful Shutdown Coordination

**Issue:** Worker process can exit while in middle of role switch or tool execution

**Files:** `crates/river-worker/src/main.rs` - No signal handling for SIGTERM

**Blocks:** Long-running operations can be interrupted, leaving state inconsistent

### No Message Acknowledgment/Delivery Guarantees

**Issue:** Messages sent to channels have no delivery confirmation

**Files:** `crates/river-worker/src/tools.rs` (execute_speak function)

**Blocks:** Difficult to debug if messages silently fail to send

**Impact:** Could create confusion in multi-worker scenarios

### No Configuration Hot-Reload

**Issue:** Changes to dyad config require full orchestrator restart

**Files:** `crates/river-orchestrator/src/config.rs` - Config loaded at startup only

**Blocks:** Cannot update model assignments without downtime

## Test Coverage Gaps

### Orchestrator HTTP API

**What's not tested:**
- Role switching happy path and error paths
- Concurrent registration of same dyad
- Worker respawn after failure

**Files:**
- `crates/river-orchestrator/src/http.rs` (895 lines)
- `crates/river-orchestrator/src/supervisor.rs` (430 lines)

**Risk:** Complex protocol coordination with minimal tests

**Priority:** High - orchestrator is critical system component

### Embed Service Concurrency

**What's not tested:**
- Concurrent search/index operations
- Cursor expiration race conditions
- Database lock contention

**Files:**
- `crates/river-embed/src/http.rs` (274 lines)
- `crates/river-embed/src/store.rs` (429 lines)

**Risk:** Production load could expose deadlocks

**Priority:** High - vector search is new system

### Worker Tool Execution

**What's not tested:**
- Error cases for bash execution
- File access boundary violations
- Tool argument parsing with malformed JSON

**Files:** `crates/river-worker/src/tools.rs` (1318 lines)

**Risk:** Largest and most complex tool module with minimal unit tests

**Priority:** Critical - tools are main capability

### Discord Event Normalization

**What's not tested:**
- Emoji reaction handling (line 15 in discord.rs imports)
- Message type conversion (line 14)
- Author field edge cases

**Files:** `crates/river-discord/src/discord.rs` (865 lines)

**Risk:** Event misinterpretation could cause incorrect behavior

**Priority:** Medium - Discord-specific but affects main flow

---

*Concerns audit: 2026-04-06*
