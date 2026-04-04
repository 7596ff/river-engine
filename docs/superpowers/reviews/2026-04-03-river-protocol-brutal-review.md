# river-protocol Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-03-river-protocol-spec.md

## Spec Completion Assessment

### Required Types - PASS

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| `Side` enum | YES | Has `opposite()` helper (bonus) |
| `Baton` enum | YES | |
| `Ground` struct | PARTIAL | **Different fields than spec** |
| `Channel` struct | YES | |
| `Author` struct | YES | |
| `Attachment` struct | PARTIAL | **Different fields than spec** |
| `ProcessEntry` enum | YES | Tagged enum as specified |
| `Registry` struct | YES | Has helper methods (bonus) |
| `ModelConfig` struct | YES | |
| `WorkerRegistration` | YES | |
| `WorkerRegistrationRequest` | YES | |
| `WorkerRegistrationResponse` | YES | |
| `AdapterRegistration` | YES | |
| `AdapterRegistrationRequest` | YES | |
| `AdapterRegistrationResponse` | YES | |

### CRITICAL ISSUES

#### 1. `Ground` struct diverges from spec

**Spec says:**
```rust
pub struct Ground {
    pub channel: Channel,
    pub adapter: String,  // <-- adapter field
}
```

**Implementation has:**
```rust
pub struct Ground {
    pub name: String,      // <-- NEW
    pub id: String,        // <-- NEW
    pub channel: Channel,
}
```

The implementation added `name` and `id` for human operator info but removed `adapter`. This is a semantic change. The spec describes `Ground` as a destination, the implementation describes it as a person.

**Verdict:** Implementation may be better, but spec is out of date. **Spec needs update or implementation needs correction.**

#### 2. `Attachment` struct diverges from spec

**Spec says:**
```rust
pub struct Attachment {
    pub url: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}
```

**Implementation has:**
```rust
pub struct Attachment {
    pub id: String,                    // <-- NEW required field
    pub filename: String,
    pub url: String,
    pub size: u64,                     // <-- NOT optional
    pub content_type: Option<String>,
}
```

- Added required `id` field
- Changed `size` from `Option<u64>` to `u64`

**Verdict:** Breaking change from spec. Consumers expecting optional size will fail.

### IMPORTANT ISSUES

#### 3. Missing serde round-trip tests

Spec explicitly requires:
> Unit tests for serde round-trips on all types, especially `ProcessEntry` with the new tagged enum format.

**No tests exist.** Zero test files in the crate.

```
$ find crates/river-protocol -name "*.rs" | xargs grep -l "#\[test\]"
(nothing)
```

**Verdict:** SPEC VIOLATION. Testing requirement unmet.

#### 4. `serde(rename_all)` inconsistency

Spec uses `"lowercase"`:
```rust
#[serde(rename_all = "lowercase")]
pub enum Side { Left, Right }

#[serde(rename_all = "lowercase")]
pub enum Baton { Actor, Spectator }
```

Implementation uses `"snake_case"`:
```rust
#[serde(rename_all = "snake_case")]
pub enum Baton { ... }

#[serde(rename_all = "snake_case")]
pub enum Side { ... }
```

For single-word variants (`Left`, `Right`, `Actor`, `Spectator`), this produces identical output. But the spec and implementation disagree on convention. If variants gain underscores, they'll serialize differently.

**Verdict:** Minor. Functionally equivalent for current variants.

### MINOR ISSUES

#### 5. Module visibility

All modules are `mod` (private), then re-exported via `pub use`. This is fine, but the spec shows `pub mod` in the structure diagram. Inconsistent but not wrong.

#### 6. Doc comments excellent

Implementation has better doc comments than spec showed. Good.

## Code Quality Assessment

### Strengths

1. **Clean module separation** - Each type group in its own file
2. **Consistent derive order** - `Clone, Debug, Serialize, Deserialize, ToSchema`
3. **ToSchema for OpenAPI** - All types have utoipa derives
4. **Helper methods on Registry** - `embed_endpoint()`, `adapter_endpoint()`, `worker_endpoint()` are useful
5. **Helper method on ProcessEntry** - `endpoint()` accessor
6. **Side::opposite()** - Useful helper

### Weaknesses

1. **No tests** - See above
2. **No validation** - Strings can be empty, endpoints can be malformed URLs
3. **No Default impls** - Only `Registry` has Default
4. **api_key in ModelConfig** - Sensitive data in a struct that gets logged via Debug

### Security Concern

`ModelConfig` derives `Debug`, which will print `api_key`. Consider:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    #[serde(skip_serializing)]  // Don't accidentally send in responses
    pub api_key: String,
    pub context_limit: usize,
}
```

Or implement Debug manually to redact the key.

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 70% | Two structs diverge, no tests |
| Code Quality | 80% | Clean but missing tests and validation |
| Documentation | 90% | Good doc comments |
| Security | 60% | API key in Debug output |

### Blocking Issues

1. **Missing tests** - Spec explicitly requires serde round-trip tests
2. **Ground/Attachment spec mismatch** - Need to either update spec or fix implementation

### Recommended Actions

1. Add `tests/` module with serde round-trip tests for all types
2. Reconcile Ground and Attachment with spec (update spec if implementation is intentionally better)
3. Consider redacting api_key from Debug output
4. Add validation helpers or document that validation is caller's responsibility
