# river-protocol Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add comprehensive serde round-trip tests, fix API key exposure in Debug output, add PartialEq derives for testability, and add Default implementations.

**Architecture:** This is a types-only crate. All changes are additive: adding derives, implementing traits manually for ModelConfig, and adding a test module. No existing functionality changes.

**Tech Stack:** Rust, serde, serde_json, utoipa

---

## File Structure

| File | Responsibility | Changes |
|------|----------------|---------|
| `crates/river-protocol/src/lib.rs` | Module exports, test module | Add `#[cfg(test)]` module with all serde tests |
| `crates/river-protocol/src/identity.rs` | Identity types | Add `PartialEq` to Author, Attachment, Ground |
| `crates/river-protocol/src/model.rs` | Model config | Manual `Debug` impl to redact api_key, add `PartialEq` |
| `crates/river-protocol/src/registry.rs` | Registry types | Add `PartialEq` to ProcessEntry, Registry |
| `crates/river-protocol/src/registration.rs` | Registration types | Add `PartialEq` to all types |

---

### Task 1: Add PartialEq to identity.rs types

**Files:**
- Modify: `crates/river-protocol/src/identity.rs:7-15` (Author)
- Modify: `crates/river-protocol/src/identity.rs:29-41` (Attachment)
- Modify: `crates/river-protocol/src/identity.rs:72-80` (Ground)

- [ ] **Step 1: Add PartialEq to Author struct**

In `crates/river-protocol/src/identity.rs`, change line 7 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Author {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Author {
```

- [ ] **Step 2: Add PartialEq to Attachment struct**

In `crates/river-protocol/src/identity.rs`, change line 29 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Attachment {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Attachment {
```

- [ ] **Step 3: Add PartialEq to Ground struct**

In `crates/river-protocol/src/identity.rs`, change line 72 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Ground {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Ground {
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p river-protocol`
Expected: Compiles without errors

- [ ] **Step 5: Commit**

```bash
git add crates/river-protocol/src/identity.rs
git commit -m "feat(river-protocol): add PartialEq to Author, Attachment, Ground"
```

---

### Task 2: Add PartialEq to registry.rs types

**Files:**
- Modify: `crates/river-protocol/src/registry.rs:8-10` (ProcessEntry)
- Modify: `crates/river-protocol/src/registry.rs:44-47` (Registry)

- [ ] **Step 1: Add PartialEq to ProcessEntry enum**

In `crates/river-protocol/src/registry.rs`, change line 8 from:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEntry {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEntry {
```

- [ ] **Step 2: Add PartialEq to Registry struct**

In `crates/river-protocol/src/registry.rs`, change line 44 from:
```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Registry {
```
to:
```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Registry {
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p river-protocol`
Expected: Compiles without errors

- [ ] **Step 4: Commit**

```bash
git add crates/river-protocol/src/registry.rs
git commit -m "feat(river-protocol): add PartialEq to ProcessEntry, Registry"
```

---

### Task 3: Add PartialEq to registration.rs types

**Files:**
- Modify: `crates/river-protocol/src/registration.rs`

- [ ] **Step 1: Add PartialEq to WorkerRegistration**

In `crates/river-protocol/src/registration.rs`, change line 10 from:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkerRegistration {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WorkerRegistration {
```

- [ ] **Step 2: Add PartialEq to WorkerRegistrationRequest**

Change line 17 from:
```rust
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkerRegistrationRequest {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct WorkerRegistrationRequest {
```

- [ ] **Step 3: Add PartialEq to WorkerRegistrationResponse**

Change line 24 from:
```rust
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WorkerRegistrationResponse {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
pub struct WorkerRegistrationResponse {
```

- [ ] **Step 4: Add PartialEq to AdapterRegistration**

Change line 39 from:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdapterRegistration {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AdapterRegistration {
```

- [ ] **Step 5: Add PartialEq to AdapterRegistrationRequest**

Change line 48 from:
```rust
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdapterRegistrationRequest {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct AdapterRegistrationRequest {
```

- [ ] **Step 6: Add PartialEq to AdapterRegistrationResponse**

Change line 55 from:
```rust
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AdapterRegistrationResponse {
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
pub struct AdapterRegistrationResponse {
```

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p river-protocol`
Expected: Compiles without errors

- [ ] **Step 8: Commit**

```bash
git add crates/river-protocol/src/registration.rs
git commit -m "feat(river-protocol): add PartialEq to all registration types"
```

---

### Task 4: Fix ModelConfig Debug to redact api_key

**Files:**
- Modify: `crates/river-protocol/src/model.rs`

- [ ] **Step 1: Write failing test for Debug redaction**

In `crates/river-protocol/src/model.rs`, add at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacts_api_key() {
        let config = ModelConfig {
            endpoint: "https://api.example.com".to_string(),
            name: "gpt-4".to_string(),
            api_key: "sk-secret-key-12345".to_string(),
            context_limit: 128000,
        };
        let debug_output = format!("{:?}", config);
        assert!(
            !debug_output.contains("sk-secret-key-12345"),
            "Debug output should not contain actual API key: {}",
            debug_output
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output should contain [REDACTED]: {}",
            debug_output
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-protocol test_debug_redacts_api_key -- --nocapture`
Expected: FAIL - the current derived Debug prints the actual api_key

- [ ] **Step 3: Remove Debug derive and implement manually**

Replace the entire `crates/river-protocol/src/model.rs` file with:

```rust
//! Model configuration types.

use serde::{Deserialize, Serialize};
use std::fmt;
use utoipa::ToSchema;

/// Model configuration from orchestrator.
#[derive(Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ModelConfig {
    /// LLM API endpoint URL.
    pub endpoint: String,
    /// Model name/identifier.
    pub name: String,
    /// API key for authentication.
    pub api_key: String,
    /// Maximum context window size in tokens.
    pub context_limit: usize,
}

impl fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelConfig")
            .field("endpoint", &self.endpoint)
            .field("name", &self.name)
            .field("api_key", &"[REDACTED]")
            .field("context_limit", &self.context_limit)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacts_api_key() {
        let config = ModelConfig {
            endpoint: "https://api.example.com".to_string(),
            name: "gpt-4".to_string(),
            api_key: "sk-secret-key-12345".to_string(),
            context_limit: 128000,
        };
        let debug_output = format!("{:?}", config);
        assert!(
            !debug_output.contains("sk-secret-key-12345"),
            "Debug output should not contain actual API key: {}",
            debug_output
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output should contain [REDACTED]: {}",
            debug_output
        );
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-protocol test_debug_redacts_api_key -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/river-protocol/src/model.rs
git commit -m "fix(river-protocol): redact api_key in ModelConfig Debug output"
```

---

### Task 5: Add serde round-trip tests for identity types

**Files:**
- Modify: `crates/river-protocol/src/lib.rs`

- [ ] **Step 1: Add test module with identity type tests**

In `crates/river-protocol/src/lib.rs`, add after the `pub use` statements:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_side_serde_roundtrip() {
        let left = Side::Left;
        let json = serde_json::to_string(&left).unwrap();
        assert_eq!(json, r#""left""#);
        let parsed: Side = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, left);

        let right = Side::Right;
        let json = serde_json::to_string(&right).unwrap();
        assert_eq!(json, r#""right""#);
        let parsed: Side = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, right);
    }

    #[test]
    fn test_baton_serde_roundtrip() {
        let actor = Baton::Actor;
        let json = serde_json::to_string(&actor).unwrap();
        assert_eq!(json, r#""actor""#);
        let parsed: Baton = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, actor);

        let spectator = Baton::Spectator;
        let json = serde_json::to_string(&spectator).unwrap();
        assert_eq!(json, r#""spectator""#);
        let parsed: Baton = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, spectator);
    }

    #[test]
    fn test_channel_serde_roundtrip() {
        let channel = Channel {
            adapter: "discord".to_string(),
            id: "123456789".to_string(),
            name: Some("general".to_string()),
        };
        let json = serde_json::to_string(&channel).unwrap();
        let parsed: Channel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, channel);

        // Test with None name
        let channel_no_name = Channel {
            adapter: "slack".to_string(),
            id: "C1234".to_string(),
            name: None,
        };
        let json = serde_json::to_string(&channel_no_name).unwrap();
        let parsed: Channel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, channel_no_name);
    }

    #[test]
    fn test_author_serde_roundtrip() {
        let author = Author {
            id: "user123".to_string(),
            name: "Alice".to_string(),
            bot: false,
        };
        let json = serde_json::to_string(&author).unwrap();
        let parsed: Author = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, author);

        let bot_author = Author {
            id: "bot456".to_string(),
            name: "Helper Bot".to_string(),
            bot: true,
        };
        let json = serde_json::to_string(&bot_author).unwrap();
        let parsed: Author = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, bot_author);
    }

    #[test]
    fn test_attachment_serde_roundtrip() {
        let attachment = Attachment {
            id: "attach123".to_string(),
            filename: "document.pdf".to_string(),
            url: "https://cdn.example.com/doc.pdf".to_string(),
            size: 1024000,
            content_type: Some("application/pdf".to_string()),
        };
        let json = serde_json::to_string(&attachment).unwrap();
        let parsed: Attachment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, attachment);

        // Test with None content_type
        let attachment_no_type = Attachment {
            id: "attach456".to_string(),
            filename: "unknown.bin".to_string(),
            url: "https://cdn.example.com/file.bin".to_string(),
            size: 512,
            content_type: None,
        };
        let json = serde_json::to_string(&attachment_no_type).unwrap();
        let parsed: Attachment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, attachment_no_type);
    }

    #[test]
    fn test_ground_serde_roundtrip() {
        let ground = Ground {
            name: "Cassie".to_string(),
            id: "user789".to_string(),
            channel: Channel {
                adapter: "discord".to_string(),
                id: "dm-channel-123".to_string(),
                name: Some("Direct Message".to_string()),
            },
        };
        let json = serde_json::to_string(&ground).unwrap();
        let parsed: Ground = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ground);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p river-protocol`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-protocol/src/lib.rs
git commit -m "test(river-protocol): add serde round-trip tests for identity types"
```

---

### Task 6: Add serde round-trip tests for registry types

**Files:**
- Modify: `crates/river-protocol/src/lib.rs`

- [ ] **Step 1: Add registry type tests to existing test module**

In `crates/river-protocol/src/lib.rs`, add these tests inside the `mod tests` block (after the ground test):

```rust
    #[test]
    fn test_process_entry_worker_roundtrip() {
        let entry = ProcessEntry::Worker {
            endpoint: "http://localhost:3001".to_string(),
            dyad: "river".to_string(),
            side: Side::Left,
            baton: Baton::Actor,
            model: "gpt-4".to_string(),
            ground: Ground {
                name: "Cassie".to_string(),
                id: "user123".to_string(),
                channel: Channel {
                    adapter: "discord".to_string(),
                    id: "ch123".to_string(),
                    name: Some("general".to_string()),
                },
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProcessEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_process_entry_adapter_roundtrip() {
        let entry = ProcessEntry::Adapter {
            endpoint: "http://localhost:3002".to_string(),
            adapter_type: "discord".to_string(),
            dyad: "river".to_string(),
            features: vec![0, 1, 100, 200],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProcessEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_process_entry_embed_roundtrip() {
        let entry = ProcessEntry::EmbedService {
            endpoint: "http://localhost:3003".to_string(),
            name: "embed-service".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProcessEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_process_entry_tagged_discrimination() {
        // Verify the JSON has the correct "type" field for tagged enum
        let worker = ProcessEntry::Worker {
            endpoint: "http://localhost:3001".to_string(),
            dyad: "river".to_string(),
            side: Side::Left,
            baton: Baton::Actor,
            model: "gpt-4".to_string(),
            ground: Ground {
                name: "Cassie".to_string(),
                id: "user123".to_string(),
                channel: Channel {
                    adapter: "discord".to_string(),
                    id: "ch123".to_string(),
                    name: None,
                },
            },
        };
        let json = serde_json::to_string(&worker).unwrap();
        assert!(json.contains(r#""type":"worker""#), "JSON should contain type:worker tag: {}", json);

        let adapter = ProcessEntry::Adapter {
            endpoint: "http://localhost:3002".to_string(),
            adapter_type: "discord".to_string(),
            dyad: "river".to_string(),
            features: vec![0, 1],
        };
        let json = serde_json::to_string(&adapter).unwrap();
        assert!(json.contains(r#""type":"adapter""#), "JSON should contain type:adapter tag: {}", json);

        let embed = ProcessEntry::EmbedService {
            endpoint: "http://localhost:3003".to_string(),
            name: "embed".to_string(),
        };
        let json = serde_json::to_string(&embed).unwrap();
        assert!(json.contains(r#""type":"embed_service""#), "JSON should contain type:embed_service tag: {}", json);
    }

    #[test]
    fn test_registry_serde_roundtrip() {
        let registry = Registry {
            processes: vec![
                ProcessEntry::Worker {
                    endpoint: "http://localhost:3001".to_string(),
                    dyad: "river".to_string(),
                    side: Side::Left,
                    baton: Baton::Actor,
                    model: "gpt-4".to_string(),
                    ground: Ground {
                        name: "Cassie".to_string(),
                        id: "user123".to_string(),
                        channel: Channel {
                            adapter: "discord".to_string(),
                            id: "ch123".to_string(),
                            name: Some("general".to_string()),
                        },
                    },
                },
                ProcessEntry::Adapter {
                    endpoint: "http://localhost:3002".to_string(),
                    adapter_type: "discord".to_string(),
                    dyad: "river".to_string(),
                    features: vec![0, 1],
                },
            ],
        };
        let json = serde_json::to_string(&registry).unwrap();
        let parsed: Registry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, registry);
    }

    #[test]
    fn test_registry_default() {
        let registry = Registry::default();
        assert!(registry.processes.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p river-protocol`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-protocol/src/lib.rs
git commit -m "test(river-protocol): add serde round-trip tests for registry types"
```

---

### Task 7: Add serde round-trip tests for model and registration types

**Files:**
- Modify: `crates/river-protocol/src/lib.rs`

- [ ] **Step 1: Add model and registration type tests to existing test module**

In `crates/river-protocol/src/lib.rs`, add these tests inside the `mod tests` block:

```rust
    #[test]
    fn test_model_config_serde_roundtrip() {
        let config = ModelConfig {
            endpoint: "https://api.openai.com/v1".to_string(),
            name: "gpt-4-turbo".to_string(),
            api_key: "sk-test-key".to_string(),
            context_limit: 128000,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ModelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn test_worker_registration_serde_roundtrip() {
        let reg = WorkerRegistration {
            dyad: "river".to_string(),
            side: Side::Left,
        };
        let json = serde_json::to_string(&reg).unwrap();
        let parsed: WorkerRegistration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reg);
    }

    #[test]
    fn test_worker_registration_request_serde_roundtrip() {
        let req = WorkerRegistrationRequest {
            endpoint: "http://localhost:3001".to_string(),
            worker: WorkerRegistration {
                dyad: "river".to_string(),
                side: Side::Right,
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        // WorkerRegistrationRequest only derives Serialize, test serialization works
        assert!(json.contains("endpoint"));
        assert!(json.contains("worker"));
    }

    #[test]
    fn test_worker_registration_response_serde_roundtrip() {
        let json = r#"{
            "accepted": true,
            "baton": "actor",
            "partner_endpoint": "http://localhost:3002",
            "model": {
                "endpoint": "https://api.openai.com",
                "name": "gpt-4",
                "api_key": "sk-key",
                "context_limit": 128000
            },
            "ground": {
                "name": "Cassie",
                "id": "user123",
                "channel": {
                    "adapter": "discord",
                    "id": "ch123",
                    "name": "general"
                }
            },
            "workspace": "/path/to/workspace",
            "initial_message": "Hello!",
            "start_sleeping": false
        }"#;
        let response: WorkerRegistrationResponse = serde_json::from_str(json).unwrap();
        assert!(response.accepted);
        assert_eq!(response.baton, Baton::Actor);
        assert_eq!(response.partner_endpoint, Some("http://localhost:3002".to_string()));
        assert_eq!(response.workspace, "/path/to/workspace");
    }

    #[test]
    fn test_adapter_registration_serde_roundtrip() {
        let reg = AdapterRegistration {
            adapter_type: "discord".to_string(),
            dyad: "river".to_string(),
            features: vec![0, 1, 100, 200, 300],
        };
        let json = serde_json::to_string(&reg).unwrap();
        // Verify the "type" rename works
        assert!(json.contains(r#""type":"discord""#), "Should rename adapter_type to type: {}", json);
        let parsed: AdapterRegistration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reg);
    }

    #[test]
    fn test_adapter_registration_request_serde_roundtrip() {
        let req = AdapterRegistrationRequest {
            endpoint: "http://localhost:3002".to_string(),
            adapter: AdapterRegistration {
                adapter_type: "discord".to_string(),
                dyad: "river".to_string(),
                features: vec![0, 1],
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("endpoint"));
        assert!(json.contains("adapter"));
    }

    #[test]
    fn test_adapter_registration_response_serde_roundtrip() {
        let json = r#"{
            "accepted": true,
            "config": {"token": "discord-token", "guild_id": 123456},
            "worker_endpoint": "http://localhost:3001"
        }"#;
        let response: AdapterRegistrationResponse = serde_json::from_str(json).unwrap();
        assert!(response.accepted);
        assert_eq!(response.worker_endpoint, "http://localhost:3001");
        assert!(response.config.is_object());
    }
```

- [ ] **Step 2: Run all tests to verify they pass**

Run: `cargo test -p river-protocol`
Expected: All tests pass (should be around 18-20 tests)

- [ ] **Step 3: Commit**

```bash
git add crates/river-protocol/src/lib.rs
git commit -m "test(river-protocol): add serde round-trip tests for model and registration types"
```

---

### Task 8: Final verification and cleanup

**Files:**
- All river-protocol source files

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p river-protocol -- --nocapture`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p river-protocol -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Verify the crate builds cleanly**

Run: `cargo build -p river-protocol`
Expected: Builds without warnings

- [ ] **Step 4: Run tests for dependent crates to ensure no regressions**

Run: `cargo test -p river-worker -p river-orchestrator -p river-adapter 2>&1 | tail -20`
Expected: Tests pass (or same status as before)

- [ ] **Step 5: Final commit if any cleanup needed**

If any changes were made:
```bash
git add -A
git commit -m "chore(river-protocol): final cleanup and verification"
```

---

## Summary

After completing all tasks, the river-protocol crate will have:

1. **PartialEq** on all types (Author, Attachment, Ground, ProcessEntry, Registry, ModelConfig, all registration types)
2. **Manual Debug** on ModelConfig that redacts api_key
3. **18+ serde round-trip tests** covering:
   - Side, Baton (enums with snake_case)
   - Channel (with optional name)
   - Author (human and bot)
   - Attachment (with optional content_type)
   - Ground (nested Channel)
   - ProcessEntry (all 3 variants, tagged enum verification)
   - Registry (with multiple entries)
   - ModelConfig
   - All registration types

Total estimated tasks: 8
Total estimated commits: 7-8
