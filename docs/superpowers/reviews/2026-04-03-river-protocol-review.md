# river-protocol Code Review

> Reviewer: Claude (Senior Code Reviewer)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-03-river-protocol-spec.md
> Implementation: crates/river-protocol/

---

## Executive Summary

The `river-protocol` crate implementation is **structurally complete** but has **critical gaps** in testing and **several spec deviations** that need addressing. The code compiles and is well-organized, but the complete absence of tests violates the explicit testing requirement in the spec.

**Overall Assessment: NEEDS WORK**

---

## 1. Spec Compliance Analysis

### 1.1 Module Structure

| Requirement | Status | Notes |
|-------------|--------|-------|
| `lib.rs` | MET | Correct module declarations and exports |
| `identity.rs` | MET | File exists with identity types |
| `registry.rs` | MET | File exists with registry types |
| `model.rs` | MET | File exists with ModelConfig |
| `registration.rs` | MET | File exists with registration types |

### 1.2 Type Definitions

#### identity.rs

| Type | Status | Issues |
|------|--------|--------|
| `Side` | PARTIAL | Uses `snake_case` but spec says `lowercase` |
| `Baton` | PARTIAL | Uses `snake_case` but spec says `lowercase` |
| `Ground` | DEVIATION | Spec has `{channel, adapter}` but impl has `{name, id, channel}` |
| `Channel` | MET | Matches spec exactly |
| `Author` | MET | Matches spec exactly |
| `Attachment` | DEVIATION | Implementation adds `id` field, changes `size` from `Option<u64>` to `u64` |

**Critical: `Ground` struct mismatch**

Spec defines:
```rust
pub struct Ground {
    pub channel: Channel,
    pub adapter: String,
}
```

Implementation has:
```rust
pub struct Ground {
    pub name: String,
    pub id: String,
    pub channel: Channel,
}
```

This is a **breaking change** from the spec. The implementation appears to describe a human operator while the spec describes a channel/adapter pair. One of these is wrong.

**Important: serde rename_all discrepancy**

Spec specifies `#[serde(rename_all = "lowercase")]` for `Side` and `Baton`, but implementation uses `#[serde(rename_all = "snake_case")]`. This affects wire protocol compatibility:
- Spec: `"left"`, `"right"`, `"actor"`, `"spectator"`
- Impl: `"left"`, `"right"`, `"actor"`, `"spectator"` (same in this case, but wrong annotation)

Actually, for these enum variants, `lowercase` and `snake_case` produce identical output. However, the annotation should match the spec for consistency and intent clarity.

**Important: Attachment changes**

Spec:
```rust
pub struct Attachment {
    pub url: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}
```

Implementation:
```rust
pub struct Attachment {
    pub id: String,           // ADDED
    pub filename: String,
    pub url: String,
    pub size: u64,            // Changed from Option<u64>
    pub content_type: Option<String>,
}
```

The addition of `id` and making `size` required are potentially justified improvements, but they deviate from the spec without documented rationale.

#### registry.rs

| Type | Status | Issues |
|------|--------|--------|
| `ProcessEntry` | PARTIAL | Uses `snake_case` but spec says `lowercase` |
| `Registry` | MET | Matches spec |

**Added functionality (not in spec):**
- `ProcessEntry::endpoint(&self)` method - useful helper
- `Registry::embed_endpoint(&self)` method - useful helper
- `Registry::adapter_endpoint(&self, adapter_type)` method - useful helper
- `Registry::worker_endpoint(&self, dyad, side)` method - useful helper

These additions are **beneficial deviations** that improve usability.

#### model.rs

| Type | Status | Issues |
|------|--------|--------|
| `ModelConfig` | MET | Matches spec exactly |

#### registration.rs

| Type | Status | Issues |
|------|--------|--------|
| `WorkerRegistration` | MET | Matches spec |
| `WorkerRegistrationRequest` | MET | Matches spec |
| `WorkerRegistrationResponse` | MET | Matches spec |
| `AdapterRegistration` | MET | Matches spec |
| `AdapterRegistrationRequest` | MET | Matches spec |
| `AdapterRegistrationResponse` | MET | Matches spec |

### 1.3 Dependencies (Cargo.toml)

| Requirement | Status | Notes |
|-------------|--------|-------|
| `serde` workspace | MET | |
| `serde_json` workspace | MET | |
| `utoipa` workspace | MET | |
| Zero river-* deps | MET | No internal dependencies |

### 1.4 Public Exports

| Export | Status |
|--------|--------|
| `Attachment` | MET |
| `Author` | MET |
| `Baton` | MET |
| `Channel` | MET |
| `Ground` | MET |
| `Side` | MET |
| `ModelConfig` | MET |
| `ProcessEntry` | MET |
| `Registry` | MET |
| `AdapterRegistration` | MET |
| `AdapterRegistrationRequest` | MET |
| `AdapterRegistrationResponse` | MET |
| `WorkerRegistration` | MET |
| `WorkerRegistrationRequest` | MET |
| `WorkerRegistrationResponse` | MET |

### 1.5 Testing

| Requirement | Status | Notes |
|-------------|--------|-------|
| Serde round-trip tests | **NOT MET** | Zero tests exist |
| ProcessEntry tagged enum tests | **NOT MET** | Zero tests exist |

**This is a critical gap.** The spec explicitly states:

> Unit tests for serde round-trips on all types, especially `ProcessEntry` with the new tagged enum format.

The crate has **zero tests**.

---

## 2. Code Quality Assessment

### 2.1 Strengths

1. **Clean module organization** - Each file has a single responsibility
2. **Consistent derive macros** - All types derive `Debug`, `Clone`, `Serialize`, `Deserialize`, `ToSchema`
3. **Good documentation** - Field-level doc comments on all public fields
4. **Useful helper methods** - `Side::opposite()`, `ProcessEntry::endpoint()`, `Registry::*` lookups
5. **Proper use of tagged enums** - `#[serde(tag = "type")]` on `ProcessEntry` as spec requires

### 2.2 Issues

#### Critical

1. **No tests** - Complete absence of unit tests

2. **Ground struct deviation** - Implementation differs significantly from spec

#### Important

3. **Missing trait derivations**

`Side` in spec derives `Hash` but implementation already has it. Good.

`Attachment` and `Author` might benefit from `PartialEq` for testing and comparison.

`Ground` might benefit from `PartialEq` for testing.

`ModelConfig` might benefit from `PartialEq` for testing.

4. **Inconsistent Copy derivation**

`Side` and `Baton` derive `Copy` (good for small enums), but this isn't specified in the spec. This is a beneficial addition.

#### Suggestions

5. **Missing Default implementations**

Consider adding `Default` for types that have sensible defaults. Currently only `Registry` has `Default`.

6. **API key security**

`ModelConfig` stores `api_key` as a plain `String`. Consider:
- Implementing `Debug` manually to redact the key
- Using a `SecretString` type

Current `Debug` derive will print API keys in logs:
```rust
// This will expose the API key in debug output
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelConfig {
    pub api_key: String,  // Will appear in Debug output!
}
```

7. **Feature flags consideration**

The `utoipa` dependency adds OpenAPI schema generation to all types. Consider making this optional via a feature flag for consumers who don't need it.

---

## 3. Test Coverage Gaps

### Required Tests (per spec)

The spec requires "Unit tests for serde round-trips on all types". Here's what's missing:

```rust
// Tests needed in identity.rs or tests/identity.rs
#[test]
fn side_serde_roundtrip() { ... }

#[test]
fn baton_serde_roundtrip() { ... }

#[test]
fn channel_serde_roundtrip() { ... }

#[test]
fn author_serde_roundtrip() { ... }

#[test]
fn attachment_serde_roundtrip() { ... }

#[test]
fn ground_serde_roundtrip() { ... }

// Tests needed in registry.rs
#[test]
fn process_entry_worker_serde_roundtrip() { ... }

#[test]
fn process_entry_adapter_serde_roundtrip() { ... }

#[test]
fn process_entry_embed_service_serde_roundtrip() { ... }

#[test]
fn registry_serde_roundtrip() { ... }

#[test]
fn process_entry_tagged_discrimination() {
    // Verify JSON has "type": "worker" etc.
}

// Tests needed in model.rs
#[test]
fn model_config_serde_roundtrip() { ... }

// Tests needed in registration.rs
#[test]
fn worker_registration_serde_roundtrip() { ... }

#[test]
fn worker_registration_request_serde_roundtrip() { ... }

#[test]
fn worker_registration_response_serde_roundtrip() { ... }

#[test]
fn adapter_registration_serde_roundtrip() { ... }

#[test]
fn adapter_registration_request_serde_roundtrip() { ... }

#[test]
fn adapter_registration_response_serde_roundtrip() { ... }
```

### Additional Recommended Tests

```rust
// Helper method tests
#[test]
fn side_opposite() { ... }

#[test]
fn registry_embed_endpoint_found() { ... }

#[test]
fn registry_embed_endpoint_not_found() { ... }

#[test]
fn registry_adapter_endpoint_found() { ... }

#[test]
fn registry_worker_endpoint_found() { ... }

#[test]
fn process_entry_endpoint_accessor() { ... }
```

---

## 4. Documentation Gaps

### Present

- Crate-level doc comment in `lib.rs`
- Module-level doc comments in all files
- Field-level doc comments on all structs

### Missing

1. **No examples in documentation** - Consider adding `# Examples` sections to key types

2. **No module documentation for complex types** - `ProcessEntry` variants could use more explanation

3. **No cross-references** - Types like `WorkerRegistrationResponse` reference `Ground` and `ModelConfig` but don't link to them

---

## 5. Recommendations

### Critical (Must Fix)

1. **Add serde round-trip tests for all types** - This is explicitly required by the spec

2. **Resolve Ground struct discrepancy** - Either:
   - Update the implementation to match spec, OR
   - Update the spec to match implementation with documented rationale

### Important (Should Fix)

3. **Fix serde rename_all annotations** - Change from `snake_case` to `lowercase` to match spec (even though output is same for current variants)

4. **Add PartialEq derives** - Add to `Attachment`, `Author`, `Ground`, `ModelConfig`, registration types for testability

5. **Secure ModelConfig debug output** - Implement `Debug` manually to redact `api_key`

### Suggestions (Nice to Have)

6. **Make utoipa optional** - Add feature flag: `utoipa = ["dep:utoipa"]`

7. **Add builder patterns** - For complex types like `WorkerRegistrationResponse`

8. **Document Attachment.id addition** - If keeping the added `id` field, document why it was added

---

## 6. Files Reviewed

| File | Lines | Status |
|------|-------|--------|
| `/home/cassie/river-engine/crates/river-protocol/Cargo.toml` | 13 | OK |
| `/home/cassie/river-engine/crates/river-protocol/src/lib.rs` | 18 | OK |
| `/home/cassie/river-engine/crates/river-protocol/src/identity.rs` | 81 | Issues found |
| `/home/cassie/river-engine/crates/river-protocol/src/registry.rs` | 83 | Minor issues |
| `/home/cassie/river-engine/crates/river-protocol/src/model.rs` | 18 | Security concern |
| `/home/cassie/river-engine/crates/river-protocol/src/registration.rs` | 62 | OK |

---

## 7. Summary

### What Was Done Well

- Clean, well-organized code structure
- Proper use of serde tagged enum for `ProcessEntry`
- Helpful utility methods added to `Registry` and `ProcessEntry`
- Good field-level documentation
- Zero dependencies on other river-* crates as required

### What Needs Work

1. **Zero tests** - Critical gap, explicitly required by spec
2. **Ground struct mismatch** - Major spec deviation needs resolution
3. **Attachment changes** - Undocumented additions/changes
4. **API key exposure** - Security concern in debug output

### Verdict

**Do not merge until:**
1. Tests are added for all types
2. Ground struct discrepancy is resolved
3. API key debug redaction is implemented

---

*Review generated by Senior Code Reviewer agent*
