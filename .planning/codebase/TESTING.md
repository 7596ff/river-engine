# Testing Patterns

**Analysis Date:** 2026-04-06

## Test Framework

**Runner:**
- Rust built-in test framework with `cargo test`
- `tokio` runtime for async tests via `#[tokio::test]` attribute
- Config: No explicit `Cargo.toml` test configuration; standard Rust conventions
- Integration tests in `tests/` directory recognized by cargo

**Assertion Library:**
- Rust standard assertions: `assert!()`, `assert_eq!()`, `assert_ne!()`
- String assertions: `assert!(string.contains("text"))`
- Option assertions: `assert!(option.is_some())`, `assert!(option.is_none())`
- Custom assertions inline with descriptive messages

**Run Commands:**
```bash
cargo test                          # Run all tests
cargo test --lib                    # Run library unit tests only
cargo test --test integration       # Run specific integration test file
cargo test -- --ignored             # Run only ignored tests
cargo test --all                    # Run all workspace tests
cargo test --package river-adapter  # Run tests for specific crate
```

## Test File Organization

**Location:**
- **Unit tests:** Co-located with source code in `#[cfg(test)] mod tests` blocks
- **Integration tests:** In `tests/` directory at crate root (e.g., `crates/river-embed/tests/integration.rs`)
- **No separate test directory** for unit tests; they live in same file as implementation

**Naming:**
- Unit test functions: `test_<what_is_being_tested>`
- Test modules: `#[cfg(test)] mod tests`
- Integration tests: `crates/<crate>/tests/<name>.rs`

**Structure:**
```
crates/river-adapter/
├── src/
│   ├── lib.rs              # Contains #[cfg(test)] mod tests
│   ├── error.rs
│   ├── feature.rs
│   └── ...
└── No separate tests/ directory for unit tests

crates/river-embed/
├── src/
│   └── ...
└── tests/
    └── integration.rs      # Integration tests here
```

## Test Structure

**Suite Organization:**

Example from `river-adapter/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_id_serde_roundtrip() {
        let features = [
            FeatureId::SendMessage,
            FeatureId::ReceiveMessage,
            // ... more variants
        ];
        for feature in features {
            let json = serde_json::to_string(&feature).unwrap();
            let parsed: FeatureId = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, feature, "Failed roundtrip for {:?}", feature);
        }
    }

    #[test]
    fn test_feature_id_is_required() {
        assert!(FeatureId::SendMessage.is_required());
        assert!(FeatureId::ReceiveMessage.is_required());
        assert!(!FeatureId::EditMessage.is_required());
    }
}
```

**Patterns:**

1. **Setup:** No explicit setup functions; data created inline in test functions
2. **Teardown:** No explicit teardown; Rust ownership ensures cleanup
3. **Assertion:** Direct assertions on computed values
   - For equality: `assert_eq!(expected, actual, "descriptive message")`
   - For conditions: `assert!(condition, "message with {}", variable)`
   - For containment: `assert!(string.contains("substring"), "context: {}", full_string)`

## Mocking

**Framework:** Minimal mocking - preference for real type construction

**Patterns:**

1. **Struct construction for test data:**
   ```rust
   let author = Author {
       id: "u1".into(),
       name: "User".into(),
       bot: false,
   };
   ```

2. **Vector iteration for batch testing:**
   ```rust
   let cases = vec![
       (OutboundRequest::SendMessage { ... }, FeatureId::SendMessage),
       (OutboundRequest::EditMessage { ... }, FeatureId::EditMessage),
       // ...
   ];
   for (request, expected_feature) in cases {
       assert_eq!(request.feature_id(), expected_feature);
   }
   ```

3. **Helper functions for test data (when repeated):**
   ```rust
   fn make_id(timestamp_micros: u64) -> String {
       let snowflake: u128 = (timestamp_micros as u128) << 64;
       snowflake.to_string()
   }
   ```

**What to Mock:**
- External HTTP services: use `#[ignore]` and document requirement
- Time-dependent tests: pass current time as parameter to function under test

**What NOT to Mock:**
- Pure functions: test with real data
- Serialization: use actual serde roundtrips
- Type conversions: test real TryFrom/Into implementations
- Business logic: test with real domain types

Example from `river-embed/tests/integration.rs`:
```rust
#[tokio::test]
#[ignore] // Requires running service
async fn test_cursor_does_not_duplicate_first_result() {
    // This would require setting up the full service
    // For now, the unit tests in search.rs cover this
}
```

## Fixtures and Factories

**Test Data:**

From `river-context/tests/integration.rs` - complex nested structures created inline:
```rust
let request = ContextRequest {
    channels: vec![
        ChannelContext {
            channel: Channel {
                adapter: "discord".into(),
                id: "current_123".into(),
                name: Some("dev-chat".into()),
            },
            moments: vec![Moment {
                id: make_id(1_000_000),
                content: "Team discussed deployment strategy".into(),
                move_range: ("m1".into(), "m10".into()),
            }],
            // ... more nested fields
        },
    ],
    // ...
};
```

**Location:**
- Test fixtures created inline in test functions
- Helper functions at module level: `fn make_id(timestamp_micros: u64) -> String`
- `Default` trait used for partial initialization: `ChannelContext { channel, ..Default::default() }`

**Pattern for Default usage:**
```rust
let mut request = ContextRequest::default();
request.channels.push(ChannelContext {
    channel: Channel { ... },
    ..Default::default()
});
```

## Coverage

**Requirements:** No coverage tool enforced or configured; coverage is voluntary

**View Coverage:** Not configured; could be added with `tarpaulin` or `llvm-cov` if needed

**Current gaps:**
- Integration tests marked `#[ignore]` require running services (documented in comments)
- Performance tests marked `#[ignore]` due to computational expense
- Example: `river-embed/tests/integration.rs` placeholder tests waiting for service setup

## Test Types

**Unit Tests:**
- **Scope:** Individual functions, type conversions, business logic
- **Approach:** Fast, deterministic, co-located with code
- **Locations:**
  - `river-adapter/src/lib.rs`: 20+ tests for feature system, serialization, responses
  - `river-protocol/src/lib.rs`: 20+ tests for type roundtrips and serde behavior
  - All modules follow similar pattern

**Integration Tests:**
- **Scope:** HTTP endpoints, service behavior, multi-component interactions
- **Approach:** Use `#[tokio::test]` for async, test against real routers
- **Location:** `crates/<crate>/tests/integration.rs`
- **Example from `river-snowflake/tests/server_tests.rs`:**
  ```rust
  #[tokio::test]
  async fn test_get_id_success() {
      let app = create_app();
      let birth = valid_birth();

      let response = app
          .oneshot(
              Request::builder()
                  .uri(format!("/id/message?birth={}", birth))
                  .body(Body::empty())
                  .unwrap(),
          )
          .await
          .unwrap();

      assert_eq!(response.status(), StatusCode::OK);
  }
  ```

**E2E Tests:**
- **Framework:** Not present in codebase
- **Why:** Server/adapter coordination tested via integration tests
- **Future:** Could add end-to-end workflow tests if needed

## Common Patterns

**Async Testing:**

From `river-context/tests/integration.rs`:
```rust
#[test]
fn test_full_context_assembly() {
    let request = ContextRequest { ... };
    let result = build_context(request).unwrap();

    assert!(!result.messages.is_empty());
    assert!(result.estimated_tokens > 0);
    assert!(result.estimated_tokens < 50000);
}
```

From `river-snowflake/tests/server_tests.rs`:
```rust
#[tokio::test]
async fn test_get_id_success() {
    let app = create_app();
    let birth = valid_birth();

    let response = app
        .oneshot(Request::builder()
            .uri(format!("/id/message?birth={}", birth))
            .body(Body::empty())
            .unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
```

**Error Testing:**

Pattern for fallible operations:
```rust
#[test]
fn test_feature_id_try_from_invalid() {
    assert_eq!(FeatureId::try_from(2u16), Err(2u16));
    assert_eq!(FeatureId::try_from(99u16), Err(99u16));
    assert_eq!(FeatureId::try_from(9999u16), Err(9999u16));
}
```

Pattern for parsing errors:
```rust
#[test]
fn test_inbound_event_serde_roundtrip() {
    let event = InboundEvent { ... };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: InboundEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, event);
}
```

**Serde Roundtrip Testing:**

Comprehensive pattern used throughout codebase:
```rust
#[test]
fn test_response_data_serde_roundtrip() {
    let variants = [
        ResponseData::MessageSent { message_id: "m1".into() },
        ResponseData::MessageEdited { message_id: "m1".into() },
        ResponseData::MessageDeleted,
        // ... more variants
    ];
    for data in variants {
        let json = serde_json::to_string(&data).unwrap();
        let parsed: ResponseData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, data, "Failed roundtrip for {:?}", data);
    }
}
```

**Ignored Tests:**

Used for tests requiring external services or expensive operations:
```rust
#[test]
#[ignore] // Run with: cargo test -p river-adapter generate_openapi_file -- --ignored
fn generate_openapi_file() {
    let json = openapi_json();
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json"),
        json,
    ).expect("Failed to write openapi.json");
}
```

## Test Execution Environment

**Dependencies for testing:**
- `tempfile` for temporary file handling (dev-dependency in `river-worker`)
- `tokio` with full features for async runtime
- No external test framework dependencies; standard Rust assertions

**CI considerations:**
- Tests requiring running services marked `#[ignore]`
- Comments explain what would be needed to run them
- Example: "For CI, consider mocking the embed client" in `river-embed/tests/integration.rs`

---

*Testing analysis: 2026-04-06*
