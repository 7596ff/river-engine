# river-adapter Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: High

## Summary

river-adapter is the best-implemented crate in the project (90% spec completion, 95% code quality). The main gaps are: missing openapi.json file (critical for consumers), zero test coverage, and spec drift on serde attributes. This is a types-only crate that should be the easiest to bring to 100%. Estimated effort: 1 day.

## Critical Issues

### Issue 1: Missing openapi.json file

- **Source:** Both reviews
- **Problem:** Spec says "The `openapi.json` file is generated and committed to the repo." The `openapi_json()` function exists but no file is committed.
- **Fix:**
  1. Generate the file: `cargo test -p river-adapter -- --nocapture > /dev/null && cargo run --example generate_openapi`
  2. Or add a build script that generates it
  3. Commit `crates/river-adapter/openapi.json`
- **Files:** Create `crates/river-adapter/openapi.json`
- **Tests:** Add test that verifies openapi.json is up-to-date

## Important Issues

### Issue 2: No unit tests

- **Source:** Both reviews
- **Problem:** Zero tests in a types-only crate. Serde correctness is unverified.
- **Fix:** Add comprehensive test module:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn test_feature_id_roundtrip() {
          let id = FeatureId::SendMessage;
          let json = serde_json::to_string(&id).unwrap();
          let parsed: FeatureId = serde_json::from_str(&json).unwrap();
          assert_eq!(id, parsed);
      }

      #[test]
      fn test_required_features() {
          assert!(FeatureId::SendMessage.is_required());
          assert!(FeatureId::ReceiveMessage.is_required());
          assert!(!FeatureId::EditMessage.is_required());
      }

      #[test]
      fn test_feature_id_try_from() {
          assert_eq!(FeatureId::try_from(0u16), Ok(FeatureId::SendMessage));
          assert!(FeatureId::try_from(9999u16).is_err());
      }

      #[test]
      fn test_outbound_request_feature_id() {
          let req = OutboundRequest::SendMessage { ... };
          assert_eq!(req.feature_id(), FeatureId::SendMessage);
      }

      // Tests for all 15 OutboundRequest variants
      // Tests for all 22 EventMetadata variants
      // Tests for OutboundResponse serialization with skip_serializing_if
      // Tests for base64 encoding in SendAttachment
  }
  ```
- **Files:** `crates/river-adapter/src/lib.rs` or `crates/river-adapter/tests/`
- **Tests:** Full serde roundtrip for all types

### Issue 3: Supporting types not defined locally

- **Source:** Both reviews
- **Problem:** Spec shows Author, Channel, Attachment, etc. defined in `author.rs`. Implementation re-exports from river-protocol.
- **Fix:** This is actually a good architectural decision (single source of truth). Update spec to reflect this.
- **Files:** Spec update only
- **Tests:** N/A

### Issue 4: Serde rename strategy not in spec

- **Source:** Brutal review
- **Problem:** OutboundRequest and EventMetadata use `#[serde(rename_all = "snake_case")]` which produces `{"send_message": {...}}` instead of `{"SendMessage": {...}}`. Not documented in spec.
- **Fix:** Update spec to document this (the snake_case is better for JSON APIs)
- **Files:** Spec update
- **Tests:** Serde tests will verify format

## Minor Issues

### Issue 5: Missing PartialEq on response types

- **Source:** First review
- **Problem:** `InboundEvent`, `EventMetadata`, `OutboundResponse`, `ResponseData` lack PartialEq, making testing harder.
- **Fix:** Add `#[derive(PartialEq)]` where sensible
- **Files:** `crates/river-adapter/src/event.rs`, `crates/river-adapter/src/response.rs`
- **Tests:** Enables assertion-based testing

### Issue 6: TryFrom returns raw u16 on error

- **Source:** First review
- **Problem:** `TryFrom<u16>` for FeatureId returns the raw u16 on error.
- **Fix:** Consider custom error type: `InvalidFeatureId(u16)`
- **Files:** `crates/river-adapter/src/feature.rs`
- **Tests:** Test error cases

### Issue 7: Module naming inconsistency

- **Source:** Brutal review
- **Problem:** Spec says `trait.rs`, implementation has `traits.rs`.
- **Fix:** Rename to match spec, or update spec (traits.rs is more Rust-idiomatic)
- **Files:** Either spec or `crates/river-adapter/src/traits.rs`
- **Tests:** N/A

### Issue 8: base64 encoding not documented

- **Source:** Brutal review
- **Problem:** SendAttachment.data uses `#[serde(with = "base64_bytes")]` for JSON transport. Not in spec.
- **Fix:** Update spec to document base64 encoding for binary data
- **Files:** Spec update
- **Tests:** Test base64 roundtrip

## Spec Updates Needed

1. Document that Author, Channel, Attachment, etc. are re-exported from river-protocol
2. Document `#[serde(rename_all = "snake_case")]` on OutboundRequest and EventMetadata
3. Document base64 encoding for SendAttachment.data
4. Clarify `trait.rs` vs `traits.rs` naming

## Verification Checklist

- [ ] openapi.json generated and committed
- [ ] Serde roundtrip tests for all types
- [ ] FeatureId::try_from() tested for all values
- [ ] OutboundRequest::feature_id() tested for all variants
- [ ] EventMetadata::event_type() tested for all variants
- [ ] PartialEq added to response types
- [ ] base64 encoding tested for SendAttachment
- [ ] Spec updated to reflect implementation decisions
