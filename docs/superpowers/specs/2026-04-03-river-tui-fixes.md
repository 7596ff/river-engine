# river-tui Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: Medium

## Summary

river-tui provides a functional TUI debugger with good spec compliance (85%). Main issues: zero test coverage, silent error handling, /start endpoint is vestigial (never used), and std::fs used in async context. The TUI works well for its intended purpose. Estimated effort: 1-2 days.

## Critical Issues

### Issue 1: Zero test coverage

- **Source:** Both reviews
- **Problem:** No unit tests, no integration tests.
- **Fix:** Add comprehensive tests:
  ```rust
  // adapter.rs tests
  #[test] fn test_adapter_state_new() { ... }
  #[test] fn test_add_user_message_unique_ids() { ... }
  #[test] fn test_add_system_message() { ... }
  #[test] fn test_add_context_entry_increments_counter() { ... }
  #[test] fn test_supported_features() { ... }

  // http.rs tests
  #[tokio::test] async fn test_start_binds_worker() { ... }
  #[tokio::test] async fn test_start_rejects_if_bound() { ... }
  #[tokio::test] async fn test_execute_send_message() { ... }
  #[tokio::test] async fn test_health_returns_ok() { ... }

  // tui.rs tests
  #[test] fn test_truncate_str_short() { ... }
  #[test] fn test_truncate_str_exact() { ... }
  #[test] fn test_truncate_str_adds_ellipsis() { ... }
  #[test] fn test_format_context_entry_user() { ... }
  #[test] fn test_format_context_entry_assistant() { ... }
  #[test] fn test_format_context_entry_tool_call() { ... }

  // main.rs tests
  #[test] fn test_read_context_from_line_skips() { ... }
  #[test] fn test_read_context_from_line_parses_json() { ... }
  #[test] fn test_read_context_from_line_handles_empty() { ... }
  ```
- **Files:** Create tests in each source file or `crates/river-tui/tests/`
- **Tests:** State management, HTTP endpoints, message formatting, context parsing

### Issue 2: Silent error handling on /notify

- **Source:** Both reviews
- **Problem:** `let _ = http_client.post(...).send().await;` silently discards errors. Users don't know when messages fail.
- **Fix:** Display error via system message:
  ```rust
  match http_client.post(...).send().await {
      Ok(_) => {},
      Err(e) => {
          state.add_system_message(format!("Failed to send: {}", e));
      }
  }
  ```
- **Files:** `crates/river-tui/src/tui.rs`
- **Tests:** Test error message displayed on send failure

## Important Issues

### Issue 3: /start endpoint is vestigial

- **Source:** Both reviews
- **Problem:** Worker endpoint is set during registration. /start checks if already bound and always returns error since binding happens pre-/start.
- **Fix:** Either:
  1. Remove /start endpoint (direct registration works), or
  2. Don't set worker_endpoint during registration
- **Files:** `crates/river-tui/src/main.rs`, `crates/river-tui/src/http.rs`
- **Tests:** Verify chosen approach works

### Issue 4: std::fs used in async context

- **Source:** Both reviews
- **Problem:** `std::fs::File::open(path)` in `read_context_from_line()` blocks the tokio runtime.
- **Fix:** Use `tokio::fs::File` and `tokio::io::BufReader`
- **Files:** `crates/river-tui/src/main.rs`
- **Tests:** Existing behavior unchanged

### Issue 5: No tracing initialization

- **Source:** Both reviews
- **Problem:** `tracing` and `tracing-subscriber` in Cargo.toml but never initialized. All `tracing::info!` calls are no-ops.
- **Fix:** Initialize tracing in main:
  ```rust
  tracing_subscriber::fmt::init();
  ```
  Or remove unused dependencies.
- **Files:** `crates/river-tui/src/main.rs`, `Cargo.toml`
- **Tests:** Verify logs appear

### Issue 6: Context entries use current timestamp

- **Source:** Brutal review
- **Problem:** `add_context_entry` uses `Utc::now()` instead of the actual time the entry was written to context.jsonl.
- **Fix:** OpenAIMessage doesn't have timestamp. Consider either:
  1. Accept this limitation (display order = read order)
  2. Extract timestamp from context file modification time
- **Files:** `crates/river-tui/src/adapter.rs`
- **Tests:** N/A (design decision)

### Issue 7: Context file read errors silently swallowed

- **Source:** First review
- **Problem:** `Err(_) => { continue; }` in context tailing loop hides errors.
- **Fix:** Log errors:
  ```rust
  Err(e) => {
      tracing::warn!("Context read error: {}", e);
      tokio::time::sleep(Duration::from_millis(500)).await;
      continue;
  }
  ```
- **Files:** `crates/river-tui/src/main.rs`
- **Tests:** N/A

## Minor Issues

### Issue 8: System message format inconsistency

- **Source:** First review
- **Problem:** Spec table says `sys>` but implementation uses `[sys]`.
- **Fix:** Clarify spec or update implementation to match
- **Files:** `crates/river-tui/src/tui.rs` or spec
- **Tests:** N/A

### Issue 9: Context tailing in wrong module

- **Source:** First review
- **Problem:** Spec says tui.rs handles "context tailing" but it's in main.rs.
- **Fix:** Move `tail_context()` and `read_context_from_line()` to tui.rs
- **Files:** `crates/river-tui/src/main.rs`, `crates/river-tui/src/tui.rs`
- **Tests:** N/A

### Issue 10: Hardcoded truncation lengths

- **Source:** Both reviews
- **Problem:** Magic numbers 30, 40 for truncation don't adapt to terminal width.
- **Fix:** Calculate based on available width or extract to constants
- **Files:** `crates/river-tui/src/tui.rs`
- **Tests:** N/A

### Issue 11: Hardcoded user identity

- **Source:** First review
- **Problem:** `author.id = "user-1"`, `author.name = "Human"` hardcoded.
- **Fix:** Consider making configurable via CLI `--user-name`
- **Files:** `crates/river-tui/src/tui.rs`, `crates/river-tui/src/main.rs`
- **Tests:** N/A

### Issue 12: History only returns user messages

- **Source:** Brutal review
- **Problem:** ReadHistory filters to only local user input, not context entries.
- **Fix:** Document this as mock behavior or include all messages
- **Files:** `crates/river-tui/src/http.rs`
- **Tests:** N/A

### Issue 13: Missing PageUp/PageDown

- **Source:** Brutal review
- **Problem:** Only Up/Down for scrolling. PageUp/PageDown would be useful.
- **Fix:** Add PageUp/PageDown handlers
- **Files:** `crates/river-tui/src/tui.rs`
- **Tests:** N/A

### Issue 14: Missing documentation

- **Source:** First review
- **Problem:** Minimal doc comments on public API.
- **Fix:** Add crate-level and type documentation
- **Files:** All source files
- **Tests:** N/A

## Spec Updates Needed

1. Clarify `sys>` vs `[sys]` format

## Verification Checklist

- [ ] Unit tests for state management
- [ ] Unit tests for message formatting
- [ ] Unit tests for HTTP endpoints
- [ ] Error displayed when /notify fails
- [ ] /start endpoint fixed or removed
- [ ] tokio::fs used instead of std::fs
- [ ] Tracing initialized or deps removed
- [ ] Context read errors logged
- [ ] At least 50% test coverage
