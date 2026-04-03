# river-context Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: High

## Summary

river-context has correct basic structure but missing timestamp-based interspersing for flashes and embeddings. The `id.rs` module for timestamp extraction is completely missing. Test coverage is severely inadequate. Estimated effort: 2-3 days.

## Critical Issues

### Issue 1: Missing id.rs for timestamp extraction

- **Source:** Both reviews
- **Problem:** Spec explicitly requires `id.rs` to "extract timestamp from ID for ordering". Without it, cannot implement proper interspersing.
- **Fix:** Create `id.rs` with:
  ```rust
  /// Extract timestamp from snowflake ID for ordering.
  pub fn extract_timestamp(id: &str) -> Option<u64> {
      // Parse snowflake ID and extract timestamp component
      let snowflake = id.parse::<u128>().ok()?;
      let high = (snowflake >> 64) as u64;  // Timestamp in microseconds
      Some(high)
  }
  ```
- **Files:** Create `crates/river-context/src/id.rs`, update `lib.rs`
- **Tests:** Test timestamp extraction from known IDs

### Issue 2: Flashes not interspersed by timestamp

- **Source:** Both reviews
- **Problem:** Spec says flashes should be "interspersed globally by timestamp". Implementation just appends them at the end.
- **Fix:**
  1. Extract timestamps from flash IDs
  2. Sort flashes with other messages by timestamp
  3. Insert at correct positions in timeline
- **Files:** `crates/river-context/src/assembly.rs`
- **Tests:** Test that flashes appear at correct temporal positions

### Issue 3: Embeddings not interspersed by timestamp

- **Source:** Both reviews
- **Problem:** Spec says embeddings should "Merge within their channel by ID timestamp". Implementation just appends them.
- **Fix:** Same approach as flashes - extract timestamp and merge into timeline
- **Files:** `crates/river-context/src/assembly.rs`
- **Tests:** Test embedding ordering within channel

### Issue 4: String-based TTL comparison fragile

- **Source:** Both reviews
- **Problem:** TTL filtering uses string comparison `expires_at > now`. While ISO8601 is lexicographically sortable, different timezones or precision could cause issues.
- **Fix:** Consider using chrono to parse and compare timestamps properly, or document ISO8601 UTC requirement strictly
- **Files:** `crates/river-context/src/assembly.rs`
- **Tests:** Test with edge case timestamps

## Important Issues

### Issue 5: Missing thiserror dependency

- **Source:** Both reviews
- **Problem:** Spec lists thiserror but it's missing. Error types manually implement traits.
- **Fix:** Add `thiserror` to Cargo.toml and use derive macro
- **Files:** `crates/river-context/Cargo.toml`, `crates/river-context/src/response.rs`
- **Tests:** Existing error tests should pass

### Issue 6: Inconsistent import source

- **Source:** Both reviews
- **Problem:** `workspace.rs` imports from `river-adapter` but `lib.rs` re-exports from `river-protocol`
- **Fix:** Use `river-protocol` consistently in all modules
- **Files:** `crates/river-context/src/workspace.rs`
- **Tests:** N/A

### Issue 7: ToolCall field naming

- **Source:** First review
- **Problem:** Spec uses `r#type`, implementation uses `call_type` with serde rename. Functionally equivalent but inconsistent.
- **Fix:** Either update spec or change to `r#type`
- **Files:** `crates/river-context/src/openai.rs` or spec
- **Tests:** Serde roundtrip test

### Issue 8: Severely inadequate test coverage

- **Source:** Both reviews
- **Problem:** Only 3 unit tests. Missing tests for: multi-channel scenarios, all format functions, TTL filtering, over-budget error, ordering correctness.
- **Fix:** Add comprehensive test suite:
  - `format_moment`, `format_move`, `format_flash`, `format_embedding` tests
  - Multi-channel assembly tests
  - TTL filtering tests
  - Over-budget error tests
  - Ordering verification tests
- **Files:** `crates/river-context/src/lib.rs` or `tests/`
- **Tests:** Comprehensive suite

## Minor Issues

### Issue 9: No PartialEq on response types

- **Source:** First review
- **Problem:** `ContextResponse` and `ContextError` lack `PartialEq`, making testing harder
- **Fix:** Add `PartialEq` derive
- **Files:** `crates/river-context/src/response.rs`
- **Tests:** Enables assertion-based testing

### Issue 10: No Default for ContextRequest

- **Source:** First review
- **Problem:** No default makes test setup verbose
- **Fix:** Add `Default` implementation
- **Files:** `crates/river-context/src/request.rs`
- **Tests:** Simplifies test code

### Issue 11: Inconsistent string comparison style

- **Source:** First review
- **Problem:** One place uses `f.expires_at > *now`, another uses `embedding.expires_at.as_str() > now`
- **Fix:** Use consistent pattern
- **Files:** `crates/river-context/src/assembly.rs`
- **Tests:** N/A

## Spec Updates Needed

1. Clarify whether `ToolCall.type` should be `r#type` or `call_type` with rename

## Verification Checklist

- [ ] id.rs created with timestamp extraction
- [ ] Flashes interspersed by timestamp globally
- [ ] Embeddings interspersed by timestamp within channel
- [ ] thiserror added and used
- [ ] Import source consistent (river-protocol)
- [ ] All format functions tested
- [ ] Multi-channel assembly tested
- [ ] TTL filtering tested
- [ ] Over-budget error tested
- [ ] Ordering correctness verified
