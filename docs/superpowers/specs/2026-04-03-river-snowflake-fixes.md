# river-snowflake Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: Critical

## Summary

river-snowflake has a critical race condition in ID generation that can produce duplicate IDs under concurrent load. The bit layout is correct but comments are confusing. Missing graceful shutdown and limited test coverage. Estimated effort: 2-3 days.

## Critical Issues

### Issue 1: Race condition in SnowflakeGenerator

- **Source:** Both reviews
- **Problem:** Between `load(Ordering::Acquire)` and `store(Ordering::Release)`, two threads can both see `relative_micros > last`, both reset sequence to 0, producing duplicate IDs.
- **Fix:** Use `compare_exchange` loop for timestamp update:
  ```rust
  loop {
      let last = self.last_timestamp.load(Ordering::Acquire);
      if relative_micros > last {
          match self.last_timestamp.compare_exchange(
              last, relative_micros,
              Ordering::AcqRel, Ordering::Relaxed
          ) {
              Ok(_) => {
                  self.sequence.store(0, Ordering::Release);
                  return (relative_micros, 0);
              }
              Err(_) => continue, // Retry
          }
      } else {
          // Same timestamp, increment sequence
          let seq = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
          // ...
      }
  }
  ```
- **Files:** `crates/river-snowflake/src/snowflake/generator.rs`
- **Tests:** Add concurrent generation test with multiple threads

### Issue 2: Blocking sleep on sequence overflow

- **Source:** Brutal review
- **Problem:** When sequence overflows (0xFFFFF), code calls `thread::sleep` and recursively retries, blocking the tokio runtime and risking stack overflow.
- **Fix:** Return error instead of blocking, or use async sleep if in async context
- **Files:** `crates/river-snowflake/src/snowflake/generator.rs`
- **Tests:** Test sequence overflow handling

### Issue 3: Missing graceful shutdown

- **Source:** Both reviews
- **Problem:** Spec requires graceful shutdown on SIGINT/SIGTERM but no signal handling exists.
- **Fix:** Add tokio signal handler:
  ```rust
  tokio::select! {
      _ = axum::serve(listener, app) => {},
      _ = tokio::signal::ctrl_c() => {
          tracing::info!("Shutting down...");
      }
  }
  ```
- **Files:** `crates/river-snowflake/src/main.rs`
- **Tests:** Manual testing

## Important Issues

### Issue 4: No validation on AgentBirth::from_u64()

- **Source:** Both reviews
- **Problem:** Accepts any u64, including garbage values that produce nonsensical dates.
- **Fix:** Add `try_from_u64` that validates, or rename to `from_u64_unchecked`
- **Files:** `crates/river-snowflake/src/snowflake/birth.rs`
- **Tests:** Test with invalid values

### Issue 5: Sequence overflow check is wrong

- **Source:** First review
- **Problem:** Uses `>=` instead of `>`. Since `fetch_add` returns previous value and we add 1, `seq` could validly be `0xFFFFF`.
- **Fix:** Change `if seq >= 0xFFFFF` to `if seq > 0xFFFFF`
- **Files:** `crates/river-snowflake/src/snowflake/generator.rs`
- **Tests:** Test at exactly 0xFFFFF sequences

### Issue 6: Duplicate is_leap_year function

- **Source:** Both reviews
- **Problem:** Same function in `birth.rs` and `extract.rs`
- **Fix:** Move to shared utility module
- **Files:** `crates/river-snowflake/src/snowflake/birth.rs`, `crates/river-snowflake/src/extract.rs`
- **Tests:** Existing tests cover functionality

### Issue 7: clap should be feature-gated

- **Source:** Both reviews
- **Problem:** clap is always compiled, spec says binary-only
- **Fix:** Make clap optional with server feature
- **Files:** `crates/river-snowflake/Cargo.toml`
- **Tests:** Build without server feature

## Minor Issues

### Issue 8: Confusing bit-packing comment

- **Source:** Both reviews
- **Problem:** Comment mentions "38 bits" but implementation correctly uses 36 bits
- **Fix:** Update comment to match implementation
- **Files:** `crates/river-snowflake/src/snowflake/birth.rs`
- **Tests:** N/A

### Issue 9: Missing FromStr/Display traits

- **Source:** First review
- **Problem:** `SnowflakeType` and `Snowflake` have methods but not standard traits
- **Fix:** Implement `FromStr` for `SnowflakeType` and `Snowflake`
- **Files:** `crates/river-snowflake/src/snowflake/types.rs`, `crates/river-snowflake/src/parse.rs`
- **Tests:** Add trait-based parsing tests

### Issue 10: No server endpoint tests

- **Source:** Both reviews
- **Problem:** Zero HTTP endpoint tests
- **Fix:** Add axum::test integration tests
- **Files:** `crates/river-snowflake/src/server.rs` or `tests/server_tests.rs`
- **Tests:** Test all three endpoints

## Spec Updates Needed

None - implementation follows spec, just has bugs.

## Verification Checklist

- [ ] Race condition fixed with CAS loop
- [ ] Sequence overflow returns error (not blocking)
- [ ] Graceful shutdown works
- [ ] AgentBirth validation added
- [ ] Concurrent generation test passes
- [ ] Server endpoint tests pass
- [ ] is_leap_year deduplicated
