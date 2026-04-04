# river-adapter Code Review

**Date:** 2026-04-03
**Reviewer:** Claude (Code Review Agent)
**Spec:** `docs/superpowers/specs/2026-04-01-adapter-library-design.md`
**Crate:** `crates/river-adapter/`

## Executive Summary

The `river-adapter` crate implementation is **mostly complete** with good adherence to the spec. The core types, traits, and enums are well-implemented with proper derive macros and serialization. However, there are several notable deviations and missing elements that need attention.

**Overall Assessment:** 7/10 - Solid implementation with gaps in tests and one missing file.

---

## Spec Compliance Checklist

### Crate Structure

| Spec Requirement | Status | Notes |
|-----------------|--------|-------|
| `Cargo.toml` | PASS | Present and correct |
| `src/lib.rs` | PASS | Re-exports and OpenAPI doc generation |
| `src/trait.rs` | DEVIATION | Named `traits.rs` (minor, acceptable) |
| `src/feature.rs` | PASS | FeatureId and OutboundRequest |
| `src/event.rs` | PASS | InboundEvent, EventMetadata, EventType |
| `src/response.rs` | PASS | OutboundResponse, ResponseData, ResponseError |
| `src/author.rs` | MISSING | Types moved to river-protocol |
| `src/error.rs` | PASS | AdapterError enum |
| `openapi.json` | CRITICAL MISSING | Not generated/committed |

### FeatureId Enum

| Requirement | Status | Notes |
|------------|--------|-------|
| `#[repr(u16)]` | PASS | Correct |
| Clone, Copy, PartialEq, Eq, Hash | PASS | All derived |
| Serialize, Deserialize, ToSchema | PASS | All derived |
| `is_required()` method | PASS | Correct implementation |
| All feature variants (0-900 range) | PASS | All 24 variants present with correct values |

**Added functionality not in spec:**
- `#[serde(rename_all = "snake_case")]` - Acceptable improvement
- `Debug` derive - Good addition
- `TryFrom<u16>` implementation - Useful addition, not in spec

### OutboundRequest Enum

| Requirement | Status | Notes |
|------------|--------|-------|
| All 15 variants | PASS | All present |
| Serialize, Deserialize, ToSchema | PASS | All derived |
| `feature_id()` method | PASS | Correct mappings |
| SendAttachment data field | DEVIATION | Uses base64 encoding (improvement) |

**Deviations:**
- Added `#[serde(rename_all = "snake_case")]` (acceptable)
- Added `Clone, Debug` derives (good)
- Added base64 serialization for `SendAttachment.data` field - This is a GOOD deviation as raw `Vec<u8>` would not serialize well to JSON

### InboundEvent / EventMetadata / EventType

| Requirement | Status | Notes |
|------------|--------|-------|
| InboundEvent struct | PASS | Correct fields |
| EventType enum (22 variants) | PASS | All present |
| EventMetadata enum (22 variants) | PASS | All present with correct fields |
| `event_type()` method | PASS | Correct implementation |
| `#[serde(rename_all = "snake_case")]` | PASS | Applied correctly |

### Supporting Types

| Type | Status | Notes |
|------|--------|-------|
| Author | PASS | Re-exported from river-protocol |
| Channel | PASS | Re-exported from river-protocol |
| Attachment | PASS | Re-exported from river-protocol |
| Baton | PASS | Re-exported from river-protocol |
| Side | PASS | Re-exported from river-protocol |
| Ground | PASS | Re-exported from river-protocol |

**Architectural Decision:** Types moved to `river-protocol` crate and re-exported. This is a reasonable architectural decision to prevent duplication across crates, though it deviates from the spec's file structure.

### Adapter Trait

| Requirement | Status | Notes |
|------------|--------|-------|
| `#[async_trait]` | PASS | Applied |
| `Send + Sync` bounds | PASS | Present |
| `adapter_type()` | PASS | Returns `&str` |
| `features()` | PASS | Returns `Vec<FeatureId>` |
| `supports()` default impl | PASS | Correct |
| `start()` async method | PASS | Correct signature |
| `execute()` async method | PASS | Correct signature |
| `health()` async method | PASS | Correct signature |

### Response Types

| Requirement | Status | Notes |
|------------|--------|-------|
| OutboundResponse struct | PASS | Correct fields and skip_serializing_if |
| ResponseData enum (15 variants) | PASS | All present |
| HistoryMessage struct | PASS | Correct fields |
| ResponseError struct | PASS | Correct fields |
| ErrorCode enum (6 variants) | PASS | All present |

**Added functionality:**
- `OutboundResponse::success()` and `failure()` constructors - Good addition
- `ResponseError::new()` constructor - Good addition

### AdapterError Enum

| Requirement | Status | Notes |
|------------|--------|-------|
| Connection variant | PASS | |
| Timeout variant | PASS | |
| Unsupported variant | PASS | |
| RateLimited variant | PASS | With retry_after_ms |
| Platform variant | PASS | |
| InvalidRequest variant | PASS | |
| thiserror derive | PASS | Correct error messages |

### OpenAPI Generation

| Requirement | Status | Notes |
|------------|--------|-------|
| AdapterApiDoc struct | PASS | Correct |
| All schemas registered | PASS | 13 types registered |
| `openapi_json()` function | PASS | Returns pretty JSON |
| openapi.json file committed | CRITICAL MISSING | File not present in repo |

### Dependencies

| Dependency | Spec | Implementation | Status |
|-----------|------|----------------|--------|
| serde | workspace | workspace | PASS |
| serde_json | workspace | workspace | PASS |
| thiserror | workspace | workspace | PASS |
| utoipa | workspace | workspace | PASS |
| async-trait | workspace | workspace | PASS |
| river-protocol | - | added | DEVIATION |
| base64 | - | added | DEVIATION |

---

## Issues by Severity

### Critical

1. **Missing openapi.json file**
   - **Location:** `crates/river-adapter/openapi.json`
   - **Spec says:** "The `openapi.json` file is generated and committed to the repo."
   - **Impact:** API consumers cannot reference the schema without building the crate
   - **Fix:** Generate and commit the file

### Important

2. **No unit tests**
   - **Location:** `crates/river-adapter/src/`
   - **Observation:** Running `cargo test -p river-adapter` shows 0 unit tests, only 1 doc test
   - **Impact:** No verification of serialization correctness, feature_id() mappings, or event_type() mappings
   - **Recommended tests:**
     - FeatureId serialization roundtrip
     - FeatureId::is_required() returns true only for SendMessage/ReceiveMessage
     - FeatureId::try_from() for all values
     - OutboundRequest::feature_id() returns correct feature for each variant
     - EventMetadata::event_type() returns correct type for each variant
     - OutboundResponse serialization with skip_serializing_if behavior
     - base64 encoding/decoding for SendAttachment

3. **Missing author.rs file**
   - **Spec says:** Create `src/author.rs` with Author and Attachment structs
   - **Implementation:** Types defined in river-protocol, re-exported
   - **Assessment:** This is an acceptable architectural deviation but should be documented

### Suggestions

4. **Add PartialEq/Eq to more types**
   - **Location:** `src/event.rs`, `src/response.rs`
   - **Observation:** `InboundEvent`, `EventMetadata`, `OutboundResponse`, `ResponseData` lack PartialEq
   - **Impact:** Cannot easily compare events/responses in tests
   - **Fix:** Add `#[derive(PartialEq)]` where sensible

5. **Consider Default implementations**
   - **Location:** `src/response.rs`
   - **Suggestion:** Add `Default` for `OutboundResponse` to create empty success responses

6. **Add From implementations for AdapterError**
   - **Location:** `src/error.rs`
   - **Suggestion:** Add `From<std::io::Error>` and other common error conversions

7. **Documentation improvements**
   - Module-level docs are minimal
   - Consider adding examples in doc comments for each public type

---

## Code Quality Assessment

### Strengths

1. **Clean type definitions** - All types are well-structured with appropriate derives
2. **Good use of serde attributes** - `skip_serializing_if` for Option fields, `rename_all` for consistent casing
3. **Proper async trait usage** - Trait is correctly defined with async-trait
4. **Base64 encoding for binary data** - Smart decision for JSON serialization of Vec<u8>
5. **Separation of concerns** - Moving shared types to river-protocol prevents duplication
6. **Constructor methods** - `OutboundResponse::success/failure` and `ResponseError::new` improve ergonomics

### Weaknesses

1. **Lack of tests** - Critical gap in quality assurance
2. **Missing generated artifact** - openapi.json should be committed
3. **No integration examples** - Would help users understand usage patterns
4. **TryFrom returns raw u16 on error** - Could use a custom error type

---

## File-by-File Notes

### `/home/cassie/river-engine/crates/river-adapter/Cargo.toml`

```toml
[dependencies]
river-protocol = { path = "../river-protocol" }  # Added, not in spec
base64 = { workspace = true }  # Added for SendAttachment encoding
```

The additional dependencies are justified improvements.

### `/home/cassie/river-engine/crates/river-adapter/src/lib.rs`

Well-organized with clear re-exports. Doc example compiles and demonstrates basic usage.

### `/home/cassie/river-engine/crates/river-adapter/src/feature.rs`

The `base64_bytes` module (lines 199-215) is a clean implementation for handling binary data in JSON.

```rust
mod base64_bytes {
    // Custom serde serializer/deserializer for Vec<u8> as base64
}
```

### `/home/cassie/river-engine/crates/river-adapter/src/traits.rs`

Matches spec exactly. File named `traits.rs` instead of spec's `trait.rs` (Rust convention).

### `/home/cassie/river-engine/crates/river-adapter/src/event.rs`

All 22 EventMetadata variants present with correct fields. Uses `river_protocol::{Attachment, Author}` imports.

### `/home/cassie/river-engine/crates/river-adapter/src/response.rs`

Adds useful constructor methods not in spec:

```rust
impl OutboundResponse {
    pub fn success(data: ResponseData) -> Self { ... }
    pub fn failure(error: ResponseError) -> Self { ... }
}
```

### `/home/cassie/river-engine/crates/river-adapter/src/error.rs`

Exactly matches spec. Uses `crate::feature::FeatureId` for the Unsupported variant.

---

## Recommendations

### Immediate Actions

1. **Generate and commit openapi.json:**
   ```bash
   # Add a build script or test that generates this
   cargo test -p river-adapter generate_openapi -- --nocapture
   ```

2. **Add comprehensive tests:**
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_required_features() {
           assert!(FeatureId::SendMessage.is_required());
           assert!(FeatureId::ReceiveMessage.is_required());
           assert!(!FeatureId::EditMessage.is_required());
       }

       #[test]
       fn test_feature_id_roundtrip() {
           let id = FeatureId::SendMessage;
           let json = serde_json::to_string(&id).unwrap();
           let parsed: FeatureId = serde_json::from_str(&json).unwrap();
           assert_eq!(id, parsed);
       }

       // ... more tests
   }
   ```

### Future Improvements

1. Consider adding a `FeatureSet` type for managing collections of features
2. Add `#[non_exhaustive]` to enums to allow future extension without breaking changes
3. Consider adding validation methods (e.g., `OutboundRequest::validate()`)

---

## Conclusion

The `river-adapter` crate is a solid implementation that closely follows the spec with sensible improvements. The main gaps are:

1. Missing `openapi.json` file (critical)
2. No unit tests (important)
3. Documentation could be more comprehensive

The architectural decision to source shared types from `river-protocol` is a good design choice that should be documented in the spec or as a spec amendment.

**Recommendation:** Address the critical and important issues before considering this implementation complete. The crate is functional but lacks the quality assurance that tests would provide.
