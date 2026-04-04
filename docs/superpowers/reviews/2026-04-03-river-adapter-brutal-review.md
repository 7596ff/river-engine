# river-adapter Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-01-adapter-library-design.md

## Spec Completion Assessment

### Module Structure - PARTIAL

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| lib.rs | YES | |
| trait.rs | YES | Named `traits.rs` |
| feature.rs | YES | |
| event.rs | YES | |
| response.rs | YES | |
| author.rs | **NO** | Re-exports from river-protocol |
| error.rs | YES | |
| openapi.json | **NO** | Generated file not committed |

### Feature System - PASS

| Feature | Implemented | Notes |
|---------|-------------|-------|
| FeatureId enum | YES | All 24 variants match spec |
| repr(u16) | YES | |
| is_required() | YES | |
| TryFrom<u16> | YES | |
| OutboundRequest enum | YES | All 15 variants match spec |
| feature_id() method | YES | |

### Inbound Events - PASS

| Type | Implemented | Notes |
|------|-------------|-------|
| InboundEvent | YES | |
| EventType enum | YES | All 22 variants |
| EventMetadata enum | YES | All 22 variants |
| event_type() method | YES | |

### Response Types - PASS

| Type | Implemented | Notes |
|------|-------------|-------|
| OutboundResponse | YES | With convenience constructors (bonus) |
| ResponseData | YES | All 15 variants |
| ResponseError | YES | |
| ErrorCode | YES | All 6 variants |
| HistoryMessage | YES | |

### Adapter Trait - PASS

| Method | Implemented | Notes |
|--------|-------------|-------|
| adapter_type() | YES | |
| features() | YES | |
| supports() | YES | Default impl |
| start() | YES | |
| execute() | YES | |
| health() | YES | |

### Supporting Types - PASS (via re-export)

| Type | Implemented | Notes |
|------|-------------|-------|
| Author | YES | From river-protocol |
| Channel | YES | From river-protocol |
| Attachment | YES | From river-protocol |
| Baton | YES | From river-protocol |
| Side | YES | From river-protocol |
| Ground | YES | From river-protocol |

## CRITICAL ISSUES

None. This is a well-implemented crate.

## IMPORTANT ISSUES

### 1. No openapi.json committed

**Spec says:**
> The `openapi.json` file is generated and committed to the repo. Regenerated when types change.

**Implementation:** Has `openapi_json()` function but no committed `openapi.json` file.

This is a spec violation but minor — the function exists to generate it.

### 2. Supporting types not defined locally

**Spec defines:**
```rust
// In author.rs
pub struct Author { ... }
pub struct Channel { ... }
pub struct Attachment { ... }
pub struct Baton { ... }
pub struct Side { ... }
pub struct Ground { ... }
```

**Implementation:**
```rust
// In lib.rs
pub use river_protocol::{Attachment, Author, Baton, Channel, Ground, Side};
```

The spec shows these defined in `author.rs`, but implementation re-exports from `river-protocol`. This is actually **better** than spec — single source of truth. But spec should be updated.

### 3. Serde rename strategy not in spec

**Spec shows OutboundRequest without serde attributes:**
```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub enum OutboundRequest {
    SendMessage { ... },
    ...
}
```

**Implementation uses snake_case:**
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboundRequest {
    SendMessage { ... },  // Serializes as "send_message"
    ...
}
```

JSON will be `{"send_message": {...}}` not `{"SendMessage": {...}}`. This is probably better, but consumers must know the format.

Same applies to EventMetadata.

## MINOR ISSUES

### 4. SendAttachment uses base64 encoding

**Spec shows:**
```rust
SendAttachment {
    ...
    data: Vec<u8>,
    ...
}
```

**Implementation adds custom serialization:**
```rust
SendAttachment {
    ...
    #[serde(with = "base64_bytes")]
    data: Vec<u8>,
    ...
}
```

This is good for JSON transport but not in spec. The `base64` dependency was added for this.

### 5. No tests

No test files exist. For a types-only library, serde round-trip tests would be valuable.

### 6. Module naming inconsistency

Spec says `trait.rs`, implementation has `traits.rs`. Minor but inconsistent.

### 7. Debug derive not consistently in spec

Implementation adds `Debug` derive to most types. Spec doesn't always show it. Good addition.

## Code Quality Assessment

### Strengths

1. **Excellent spec adherence** - All 24 FeatureId values, all 15 OutboundRequest variants, all 22 EventMetadata variants match exactly
2. **Clean re-exports** - Uses river-protocol for shared types (single source of truth)
3. **Convenience constructors** - `OutboundResponse::success()`, `OutboundResponse::failure()`, `ResponseError::new()`
4. **Proper error handling** - Uses thiserror as specified
5. **Good documentation** - Module docs with usage examples in lib.rs
6. **TryFrom<u16> for FeatureId** - Allows validation of wire values
7. **base64 encoding** - Smart addition for JSON transport of binary data
8. **OpenAPI generation** - Function exists and includes all types

### Weaknesses

1. **No committed openapi.json** - Spec says commit it
2. **No tests** - Should have serde round-trip tests
3. **Spec drift** - Some serde attributes not documented

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 90% | Missing openapi.json, author.rs module |
| Code Quality | 95% | Clean, well-documented |
| Documentation | 90% | Good module docs, missing inline comments |
| Testing | 0% | No tests |

### Blocking Issues

None.

### Recommended Actions

1. Generate and commit `openapi.json` file
2. Add serde round-trip tests for all major types
3. Update spec to reflect:
   - snake_case serde rename on OutboundRequest and EventMetadata
   - base64 encoding for SendAttachment.data
   - Re-export of shared types from river-protocol
4. Consider adding tests for FeatureId::try_from() edge cases
