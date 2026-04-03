# river-adapter Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add comprehensive tests, PartialEq derives for testability, and generate/commit the openapi.json file.

**Architecture:** This is a types-only crate. All changes are additive: adding PartialEq derives, creating a test module, and generating the OpenAPI spec. No functional changes to existing code.

**Tech Stack:** Rust, serde, serde_json, utoipa, base64

---

## File Structure

| File | Responsibility | Changes |
|------|----------------|---------|
| `crates/river-adapter/src/lib.rs` | Module exports, OpenAPI | Add test module |
| `crates/river-adapter/src/event.rs` | Event types | Add PartialEq to InboundEvent, EventMetadata |
| `crates/river-adapter/src/response.rs` | Response types | Add PartialEq to OutboundResponse, ResponseData, HistoryMessage, ResponseError |
| `crates/river-adapter/src/feature.rs` | Feature types | Add PartialEq to OutboundRequest |
| `crates/river-adapter/openapi.json` | OpenAPI spec | Generate and commit |

---

### Task 1: Add PartialEq to event.rs types

**Files:**
- Modify: `crates/river-adapter/src/event.rs:9` (InboundEvent)
- Modify: `crates/river-adapter/src/event.rs:46` (EventMetadata)

- [ ] **Step 1: Add PartialEq to InboundEvent struct**

In `crates/river-adapter/src/event.rs`, change line 9 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct InboundEvent {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InboundEvent {
```

- [ ] **Step 2: Add PartialEq to EventMetadata enum**

In `crates/river-adapter/src/event.rs`, change line 46 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventMetadata {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventMetadata {
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p river-adapter`
Expected: Compiles without errors

- [ ] **Step 4: Commit**

```bash
git add crates/river-adapter/src/event.rs
git commit -m "feat(river-adapter): add PartialEq to InboundEvent, EventMetadata"
```

---

### Task 2: Add PartialEq to response.rs types

**Files:**
- Modify: `crates/river-adapter/src/response.rs:9` (OutboundResponse)
- Modify: `crates/river-adapter/src/response.rs:42` (ResponseData)
- Modify: `crates/river-adapter/src/response.rs:63` (HistoryMessage)
- Modify: `crates/river-adapter/src/response.rs:73` (ResponseError)

- [ ] **Step 1: Add PartialEq to OutboundResponse struct**

In `crates/river-adapter/src/response.rs`, change line 9 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutboundResponse {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct OutboundResponse {
```

- [ ] **Step 2: Add PartialEq to ResponseData enum**

Change line 42 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseData {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseData {
```

- [ ] **Step 3: Add PartialEq to HistoryMessage struct**

Change line 63 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct HistoryMessage {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HistoryMessage {
```

- [ ] **Step 4: Add PartialEq to ResponseError struct**

Change line 73 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ResponseError {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResponseError {
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p river-adapter`
Expected: Compiles without errors

- [ ] **Step 6: Commit**

```bash
git add crates/river-adapter/src/response.rs
git commit -m "feat(river-adapter): add PartialEq to response types"
```

---

### Task 3: Add PartialEq to OutboundRequest

**Files:**
- Modify: `crates/river-adapter/src/feature.rs:100` (OutboundRequest)

- [ ] **Step 1: Add PartialEq to OutboundRequest enum**

In `crates/river-adapter/src/feature.rs`, change line 100 from:
```rust
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboundRequest {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboundRequest {
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p river-adapter`
Expected: Compiles without errors

- [ ] **Step 3: Commit**

```bash
git add crates/river-adapter/src/feature.rs
git commit -m "feat(river-adapter): add PartialEq to OutboundRequest"
```

---

### Task 4: Add FeatureId tests

**Files:**
- Modify: `crates/river-adapter/src/lib.rs`

- [ ] **Step 1: Add test module with FeatureId tests**

In `crates/river-adapter/src/lib.rs`, add after the `openapi_json()` function:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_id_serde_roundtrip() {
        let features = [
            FeatureId::SendMessage,
            FeatureId::ReceiveMessage,
            FeatureId::EditMessage,
            FeatureId::DeleteMessage,
            FeatureId::ReadHistory,
            FeatureId::PinMessage,
            FeatureId::UnpinMessage,
            FeatureId::BulkDeleteMessages,
            FeatureId::AddReaction,
            FeatureId::RemoveReaction,
            FeatureId::RemoveAllReactions,
            FeatureId::Attachments,
            FeatureId::TypingIndicator,
            FeatureId::CreateThread,
            FeatureId::ThreadEvents,
            FeatureId::CreatePoll,
            FeatureId::PollVote,
            FeatureId::PollEvents,
            FeatureId::VoiceStateEvents,
            FeatureId::PresenceEvents,
            FeatureId::MemberEvents,
            FeatureId::ScheduledEvents,
            FeatureId::ChannelEvents,
            FeatureId::ConnectionEvents,
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
        assert!(!FeatureId::DeleteMessage.is_required());
        assert!(!FeatureId::AddReaction.is_required());
        assert!(!FeatureId::ConnectionEvents.is_required());
    }

    #[test]
    fn test_feature_id_try_from_valid() {
        assert_eq!(FeatureId::try_from(0u16), Ok(FeatureId::SendMessage));
        assert_eq!(FeatureId::try_from(1u16), Ok(FeatureId::ReceiveMessage));
        assert_eq!(FeatureId::try_from(10u16), Ok(FeatureId::EditMessage));
        assert_eq!(FeatureId::try_from(20u16), Ok(FeatureId::AddReaction));
        assert_eq!(FeatureId::try_from(100u16), Ok(FeatureId::VoiceStateEvents));
        assert_eq!(FeatureId::try_from(900u16), Ok(FeatureId::ConnectionEvents));
    }

    #[test]
    fn test_feature_id_try_from_invalid() {
        assert_eq!(FeatureId::try_from(2u16), Err(2u16));
        assert_eq!(FeatureId::try_from(99u16), Err(99u16));
        assert_eq!(FeatureId::try_from(9999u16), Err(9999u16));
    }

    #[test]
    fn test_feature_id_u16_values() {
        assert_eq!(FeatureId::SendMessage as u16, 0);
        assert_eq!(FeatureId::ReceiveMessage as u16, 1);
        assert_eq!(FeatureId::EditMessage as u16, 10);
        assert_eq!(FeatureId::ConnectionEvents as u16, 900);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p river-adapter`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-adapter/src/lib.rs
git commit -m "test(river-adapter): add FeatureId tests"
```

---

### Task 5: Add OutboundRequest tests

**Files:**
- Modify: `crates/river-adapter/src/lib.rs`

- [ ] **Step 1: Add OutboundRequest tests to existing test module**

In `crates/river-adapter/src/lib.rs`, add inside the `mod tests` block:

```rust
    #[test]
    fn test_outbound_request_feature_id_mapping() {
        let cases = [
            (
                OutboundRequest::SendMessage {
                    channel: "ch".into(),
                    content: "hi".into(),
                    reply_to: None,
                },
                FeatureId::SendMessage,
            ),
            (
                OutboundRequest::EditMessage {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    content: "edited".into(),
                },
                FeatureId::EditMessage,
            ),
            (
                OutboundRequest::DeleteMessage {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                },
                FeatureId::DeleteMessage,
            ),
            (
                OutboundRequest::ReadHistory {
                    channel: "ch".into(),
                    limit: Some(10),
                    before: None,
                },
                FeatureId::ReadHistory,
            ),
            (
                OutboundRequest::PinMessage {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                },
                FeatureId::PinMessage,
            ),
            (
                OutboundRequest::UnpinMessage {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                },
                FeatureId::UnpinMessage,
            ),
            (
                OutboundRequest::BulkDeleteMessages {
                    channel: "ch".into(),
                    message_ids: vec!["m1".into(), "m2".into()],
                },
                FeatureId::BulkDeleteMessages,
            ),
            (
                OutboundRequest::AddReaction {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    emoji: "👍".into(),
                },
                FeatureId::AddReaction,
            ),
            (
                OutboundRequest::RemoveReaction {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    emoji: "👍".into(),
                },
                FeatureId::RemoveReaction,
            ),
            (
                OutboundRequest::RemoveAllReactions {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                },
                FeatureId::RemoveAllReactions,
            ),
            (
                OutboundRequest::SendAttachment {
                    channel: "ch".into(),
                    filename: "file.txt".into(),
                    data: vec![1, 2, 3],
                    content_type: Some("text/plain".into()),
                },
                FeatureId::Attachments,
            ),
            (
                OutboundRequest::TypingIndicator {
                    channel: "ch".into(),
                },
                FeatureId::TypingIndicator,
            ),
            (
                OutboundRequest::CreateThread {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    name: "thread".into(),
                },
                FeatureId::CreateThread,
            ),
            (
                OutboundRequest::CreatePoll {
                    channel: "ch".into(),
                    question: "Vote?".into(),
                    options: vec!["Yes".into(), "No".into()],
                    duration_hours: Some(24),
                },
                FeatureId::CreatePoll,
            ),
            (
                OutboundRequest::PollVote {
                    channel: "ch".into(),
                    poll_id: "p1".into(),
                    option_index: 0,
                },
                FeatureId::PollVote,
            ),
        ];

        for (request, expected_feature) in cases {
            assert_eq!(
                request.feature_id(),
                expected_feature,
                "Wrong feature_id for {:?}",
                request
            );
        }
    }

    #[test]
    fn test_outbound_request_serde_roundtrip() {
        let request = OutboundRequest::SendMessage {
            channel: "general".into(),
            content: "Hello!".into(),
            reply_to: Some("msg123".into()),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""send_message""#), "Should use snake_case: {}", json);
        let parsed: OutboundRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn test_outbound_request_base64_attachment() {
        let request = OutboundRequest::SendAttachment {
            channel: "ch".into(),
            filename: "test.bin".into(),
            data: vec![0x48, 0x65, 0x6c, 0x6c, 0x6f], // "Hello" in bytes
            content_type: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        // "Hello" base64 encoded is "SGVsbG8="
        assert!(json.contains("SGVsbG8="), "Should contain base64 data: {}", json);
        let parsed: OutboundRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p river-adapter`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-adapter/src/lib.rs
git commit -m "test(river-adapter): add OutboundRequest tests"
```

---

### Task 6: Add EventMetadata tests

**Files:**
- Modify: `crates/river-adapter/src/lib.rs`

- [ ] **Step 1: Add EventMetadata tests to existing test module**

In `crates/river-adapter/src/lib.rs`, add inside the `mod tests` block:

```rust
    #[test]
    fn test_event_metadata_event_type_mapping() {
        use river_protocol::{Attachment, Author};

        let author = Author {
            id: "u1".into(),
            name: "User".into(),
            bot: false,
        };

        let cases: Vec<(EventMetadata, EventType)> = vec![
            (
                EventMetadata::MessageCreate {
                    channel: "ch".into(),
                    author: author.clone(),
                    content: "hi".into(),
                    message_id: "m1".into(),
                    timestamp: "2026-01-01T00:00:00Z".into(),
                    reply_to: None,
                    attachments: vec![],
                },
                EventType::MessageCreate,
            ),
            (
                EventMetadata::MessageUpdate {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    content: "edited".into(),
                    timestamp: "2026-01-01T00:00:00Z".into(),
                },
                EventType::MessageUpdate,
            ),
            (
                EventMetadata::MessageDelete {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                },
                EventType::MessageDelete,
            ),
            (
                EventMetadata::ReactionAdd {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    user_id: "u1".into(),
                    emoji: "👍".into(),
                },
                EventType::ReactionAdd,
            ),
            (
                EventMetadata::ReactionRemove {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    user_id: "u1".into(),
                    emoji: "👍".into(),
                },
                EventType::ReactionRemove,
            ),
            (
                EventMetadata::TypingStart {
                    channel: "ch".into(),
                    user_id: "u1".into(),
                },
                EventType::TypingStart,
            ),
            (
                EventMetadata::MemberJoin {
                    user_id: "u1".into(),
                    username: "newuser".into(),
                },
                EventType::MemberJoin,
            ),
            (
                EventMetadata::MemberLeave {
                    user_id: "u1".into(),
                },
                EventType::MemberLeave,
            ),
            (
                EventMetadata::PresenceUpdate {
                    user_id: "u1".into(),
                    status: "online".into(),
                },
                EventType::PresenceUpdate,
            ),
            (
                EventMetadata::VoiceStateUpdate {
                    user_id: "u1".into(),
                    channel: Some("voice-ch".into()),
                },
                EventType::VoiceStateUpdate,
            ),
            (
                EventMetadata::ChannelCreate {
                    channel: "ch".into(),
                    name: "new-channel".into(),
                },
                EventType::ChannelCreate,
            ),
            (
                EventMetadata::ChannelUpdate {
                    channel: "ch".into(),
                    name: "renamed".into(),
                },
                EventType::ChannelUpdate,
            ),
            (
                EventMetadata::ChannelDelete {
                    channel: "ch".into(),
                },
                EventType::ChannelDelete,
            ),
            (
                EventMetadata::ThreadCreate {
                    channel: "thread-ch".into(),
                    parent_channel: "ch".into(),
                    name: "thread".into(),
                },
                EventType::ThreadCreate,
            ),
            (
                EventMetadata::ThreadUpdate {
                    channel: "thread-ch".into(),
                    name: "renamed-thread".into(),
                },
                EventType::ThreadUpdate,
            ),
            (
                EventMetadata::ThreadDelete {
                    channel: "thread-ch".into(),
                },
                EventType::ThreadDelete,
            ),
            (
                EventMetadata::PinUpdate {
                    channel: "ch".into(),
                    message_id: "m1".into(),
                    pinned: true,
                },
                EventType::PinUpdate,
            ),
            (
                EventMetadata::PollVote {
                    channel: "ch".into(),
                    poll_id: "p1".into(),
                    user_id: "u1".into(),
                    option_index: 0,
                    added: true,
                },
                EventType::PollVote,
            ),
            (
                EventMetadata::ScheduledEvent {
                    event_id: "e1".into(),
                    name: "Event".into(),
                    start_time: "2026-01-01T12:00:00Z".into(),
                },
                EventType::ScheduledEvent,
            ),
            (
                EventMetadata::ConnectionLost {
                    reason: "network error".into(),
                    reconnecting: true,
                },
                EventType::ConnectionLost,
            ),
            (
                EventMetadata::ConnectionRestored {
                    downtime_seconds: 30,
                },
                EventType::ConnectionRestored,
            ),
            (
                EventMetadata::Unknown(serde_json::json!({"custom": "data"})),
                EventType::Unknown,
            ),
        ];

        for (metadata, expected_type) in cases {
            assert_eq!(
                metadata.event_type(),
                expected_type,
                "Wrong event_type for {:?}",
                metadata
            );
        }
    }

    #[test]
    fn test_inbound_event_serde_roundtrip() {
        let event = InboundEvent {
            adapter: "discord".into(),
            metadata: EventMetadata::MessageCreate {
                channel: "general".into(),
                author: Author {
                    id: "u1".into(),
                    name: "Alice".into(),
                    bot: false,
                },
                content: "Hello!".into(),
                message_id: "m123".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                reply_to: None,
                attachments: vec![],
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""message_create""#), "Should use snake_case: {}", json);
        let parsed: InboundEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p river-adapter`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-adapter/src/lib.rs
git commit -m "test(river-adapter): add EventMetadata tests"
```

---

### Task 7: Add OutboundResponse tests

**Files:**
- Modify: `crates/river-adapter/src/lib.rs`

- [ ] **Step 1: Add OutboundResponse tests to existing test module**

In `crates/river-adapter/src/lib.rs`, add inside the `mod tests` block:

```rust
    #[test]
    fn test_outbound_response_success() {
        let response = OutboundResponse::success(ResponseData::MessageSent {
            message_id: "m123".into(),
        });
        assert!(response.ok);
        assert!(response.data.is_some());
        assert!(response.error.is_none());

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""message_sent""#), "Should use snake_case: {}", json);
        // Verify skip_serializing_if works - error should not appear
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_outbound_response_failure() {
        let response = OutboundResponse::failure(ResponseError::new(
            ErrorCode::NotFound,
            "Message not found",
        ));
        assert!(!response.ok);
        assert!(response.data.is_none());
        assert!(response.error.is_some());

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains(r#""ok":false"#));
        assert!(json.contains(r#""not_found""#));
        // Verify skip_serializing_if works - data should not appear
        assert!(!json.contains(r#""data""#));
    }

    #[test]
    fn test_response_data_serde_roundtrip() {
        let variants = [
            ResponseData::MessageSent { message_id: "m1".into() },
            ResponseData::MessageEdited { message_id: "m1".into() },
            ResponseData::MessageDeleted,
            ResponseData::MessagesPinned,
            ResponseData::MessagesUnpinned,
            ResponseData::MessagesDeleted { count: 5 },
            ResponseData::ReactionAdded,
            ResponseData::ReactionRemoved,
            ResponseData::ReactionsCleared,
            ResponseData::AttachmentSent { message_id: "m1".into() },
            ResponseData::TypingStarted,
            ResponseData::History {
                messages: vec![HistoryMessage {
                    message_id: "m1".into(),
                    channel: "ch".into(),
                    author: Author {
                        id: "u1".into(),
                        name: "User".into(),
                        bot: false,
                    },
                    content: "Hello".into(),
                    timestamp: "2026-01-01T00:00:00Z".into(),
                }],
            },
            ResponseData::ThreadCreated { thread_id: "t1".into() },
            ResponseData::PollCreated { poll_id: "p1".into() },
            ResponseData::PollVoted,
        ];

        for data in variants {
            let json = serde_json::to_string(&data).unwrap();
            let parsed: ResponseData = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, data, "Failed roundtrip for {:?}", data);
        }
    }

    #[test]
    fn test_error_code_serde_roundtrip() {
        let codes = [
            ErrorCode::UnsupportedFeature,
            ErrorCode::InvalidPayload,
            ErrorCode::PlatformError,
            ErrorCode::RateLimited,
            ErrorCode::NotFound,
            ErrorCode::Unauthorized,
        ];

        for code in codes {
            let json = serde_json::to_string(&code).unwrap();
            let parsed: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, code);
        }
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p river-adapter`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-adapter/src/lib.rs
git commit -m "test(river-adapter): add OutboundResponse tests"
```

---

### Task 8: Generate and commit openapi.json

**Files:**
- Create: `crates/river-adapter/openapi.json`
- Modify: `crates/river-adapter/src/lib.rs` (add test)

- [ ] **Step 1: Add test that generates openapi.json**

In `crates/river-adapter/src/lib.rs`, add inside the `mod tests` block:

```rust
    #[test]
    fn test_openapi_json_generation() {
        let json = openapi_json();
        assert!(json.contains("FeatureId"));
        assert!(json.contains("OutboundRequest"));
        assert!(json.contains("InboundEvent"));
        assert!(json.contains("EventMetadata"));
        assert!(json.contains("OutboundResponse"));
        assert!(json.contains("ResponseData"));
        // Verify it's valid JSON
        let _: serde_json::Value = serde_json::from_str(&json).expect("Invalid JSON");
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p river-adapter test_openapi_json_generation`
Expected: PASS

- [ ] **Step 3: Generate openapi.json file**

Create a small Rust script to generate the file. Run:
```bash
cd /home/cassie/river-engine && cargo build -p river-adapter && \
  echo 'fn main() { print!("{}", river_adapter::openapi_json()); }' | \
  rustc --edition 2021 -L target/debug/deps --extern river_adapter=target/debug/libriver_adapter.rlib \
  --extern river_protocol=target/debug/libriver_protocol.rlib \
  --extern serde=target/debug/libserde.rlib \
  --extern serde_json=target/debug/libserde_json.rlib \
  --extern utoipa=target/debug/libutoipa.rlib \
  -o /tmp/gen_openapi - && /tmp/gen_openapi > crates/river-adapter/openapi.json
```

Alternative approach - add a simple binary in the test:
```rust
// In the test, write to file
#[test]
#[ignore] // Run manually with: cargo test -p river-adapter generate_openapi_file -- --ignored
fn generate_openapi_file() {
    let json = openapi_json();
    std::fs::write("openapi.json", json).expect("Failed to write openapi.json");
}
```

Run: `cargo test -p river-adapter generate_openapi_file -- --ignored`
Then: `mv crates/river-adapter/openapi.json crates/river-adapter/openapi.json` (if needed)

- [ ] **Step 4: Verify openapi.json exists and is valid**

Run: `cat crates/river-adapter/openapi.json | head -20`
Expected: Valid JSON with OpenAPI structure

- [ ] **Step 5: Commit**

```bash
git add crates/river-adapter/src/lib.rs crates/river-adapter/openapi.json
git commit -m "feat(river-adapter): generate and commit openapi.json"
```

---

### Task 9: Final verification

**Files:**
- All river-adapter source files

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p river-adapter -- --nocapture`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p river-adapter -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Verify build**

Run: `cargo build -p river-adapter`
Expected: Builds without warnings

- [ ] **Step 4: Run dependent crate tests**

Run: `cargo test -p river-discord -p river-tui -p river-worker 2>&1 | tail -20`
Expected: Tests pass (or same status as before)

- [ ] **Step 5: Final commit if cleanup needed**

If any changes:
```bash
git add -A
git commit -m "chore(river-adapter): final cleanup and verification"
```

---

## Summary

After completing all tasks, the river-adapter crate will have:

1. **PartialEq** on all types (InboundEvent, EventMetadata, OutboundRequest, OutboundResponse, ResponseData, HistoryMessage, ResponseError)
2. **Comprehensive tests** covering:
   - FeatureId serde roundtrip (all 24 variants)
   - FeatureId::is_required() behavior
   - FeatureId::try_from() valid and invalid cases
   - OutboundRequest::feature_id() mapping (all 15 variants)
   - OutboundRequest serde with snake_case verification
   - Base64 encoding for SendAttachment
   - EventMetadata::event_type() mapping (all 22 variants)
   - InboundEvent serde roundtrip
   - OutboundResponse success/failure constructors
   - ResponseData serde roundtrip (all variants)
   - ErrorCode serde roundtrip
   - OpenAPI JSON generation
3. **openapi.json** generated and committed

Total estimated tasks: 9
Total estimated commits: 8-9
