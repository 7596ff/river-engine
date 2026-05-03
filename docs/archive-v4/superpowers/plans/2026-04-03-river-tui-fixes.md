# river-tui Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix river-tui issues including zero test coverage, silent error handling, vestigial /start endpoint, blocking I/O in async context, and missing tracing initialization.

**Architecture:** The river-tui crate is a terminal debugger consisting of four modules: `main.rs` (entry point and context tailing), `adapter.rs` (state management), `tui.rs` (UI rendering and input handling), and `http.rs` (HTTP server for worker communication). State is shared via `Arc<RwLock<AdapterState>>` across async tasks.

**Tech Stack:** Rust, Tokio (async runtime), Axum (HTTP server), Ratatui/Crossterm (TUI), Reqwest (HTTP client), Clap (CLI args), Tracing (logging)

---

## File Structure

```
crates/river-tui/
  src/
    main.rs      - Entry point, CLI args, context tailing (Issues 4, 5, 7, 9)
    adapter.rs   - State management, message types (Issue 1 tests)
    tui.rs       - TUI rendering, input handling (Issues 2, 10, 11, 13, tests)
    http.rs      - HTTP endpoints (Issues 3, 12, tests)
  Cargo.toml     - Dependencies (Issue 5 optional cleanup)
```

---

## Task 1: Initialize Tracing Subscriber

**Issue:** #5 - Tracing dependencies exist but are never initialized, making all `tracing::info!` calls no-ops.

**Files:** `crates/river-tui/src/main.rs`

### Steps

- [ ] **1.1** Add tracing subscriber initialization at the start of main()

In `crates/river-tui/src/main.rs`, add after `let args = Args::parse();`:

```rust
// Initialize tracing - output to stderr so it doesn't interfere with TUI
tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()),
    )
    .init();
```

Also add the import at the top:
```rust
use tracing_subscriber::prelude::*;
```

- [ ] **1.2** Add `env-filter` feature to tracing-subscriber in Cargo.toml

In `crates/river-tui/Cargo.toml`, change:
```toml
tracing-subscriber = { workspace = true, features = ["env-filter"] }
```

- [ ] **1.3** Verify tracing works by running with `RUST_LOG=info`

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

**Commit:** `fix(river-tui): initialize tracing subscriber`

---

## Task 2: Replace std::fs with tokio::fs in Context Tailing

**Issue:** #4 - `std::fs::File::open()` blocks the tokio runtime.

**Files:** `crates/river-tui/src/main.rs`

### Steps

- [ ] **2.1** Change `read_context_from_line` to async function

Replace the entire function signature and body:

```rust
/// Read context entries starting from a specific line.
async fn read_context_from_line(path: &PathBuf, skip_lines: usize) -> std::io::Result<Vec<OpenAIMessage>> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let file = tokio::fs::File::open(path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let mut entries = Vec::new();
    let mut line_num = 0;

    while let Some(line) = lines.next_line().await? {
        if line_num >= skip_lines && !line.trim().is_empty() {
            if let Ok(msg) = serde_json::from_str::<OpenAIMessage>(&line) {
                entries.push(msg);
            }
        }
        line_num += 1;
    }

    Ok(entries)
}
```

- [ ] **2.2** Update imports - remove std::io imports, add tokio imports

Remove from imports:
```rust
use std::io::{BufRead, BufReader};
```

The tokio imports are added inside the function to keep them scoped.

- [ ] **2.3** Update call site to await the async function

In `tail_context`, change:
```rust
let new_entries = match read_context_from_line(&path, lines_read).await {
```

- [ ] **2.4** Verify build succeeds

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

**Commit:** `fix(river-tui): use tokio::fs for non-blocking context file reads`

---

## Task 3: Log Context Read Errors

**Issue:** #7 - Context file read errors are silently swallowed.

**Files:** `crates/river-tui/src/main.rs`

### Steps

- [ ] **3.1** Add error logging in `tail_context` error handler

Replace the error handling in `tail_context`:

```rust
let new_entries = match read_context_from_line(&path, lines_read).await {
    Ok(entries) => entries,
    Err(e) => {
        tracing::warn!(side = side, error = %e, "Context read error");
        tokio::time::sleep(Duration::from_millis(500)).await;
        continue;
    }
};
```

- [ ] **3.2** Verify build succeeds

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

**Commit:** `fix(river-tui): log context file read errors instead of silently ignoring`

---

## Task 4: Handle /notify Send Errors

**Issue:** #2 - Silent error handling when sending messages to worker.

**Files:** `crates/river-tui/src/tui.rs`

### Steps

- [ ] **4.1** Capture state reference before the HTTP call for error handling

In the `run` function, after constructing `event`, replace the silent send:

```rust
// Send to worker's /notify endpoint
let notify_result = http_client
    .post(format!("{}/notify", worker_endpoint))
    .json(&event)
    .timeout(Duration::from_secs(5))
    .send()
    .await;

match notify_result {
    Ok(response) => {
        if !response.status().is_success() {
            let mut s = state.write().await;
            s.add_system_message(&format!(
                "Send failed: HTTP {}",
                response.status()
            ));
        }
    }
    Err(e) => {
        let mut s = state.write().await;
        s.add_system_message(&format!("Send failed: {}", e));
    }
}
```

- [ ] **4.2** Verify build succeeds

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

**Commit:** `fix(river-tui): display error message when /notify send fails`

---

## Task 5: Remove Vestigial /start Endpoint

**Issue:** #3 - Worker endpoint is set during registration, making /start always fail.

**Files:** `crates/river-tui/src/http.rs`

### Steps

- [ ] **5.1** Remove /start route from router

In `router()` function, remove the `/start` route:

```rust
pub fn router(state: SharedState, ui_tx: mpsc::Sender<UiEvent>) -> Router {
    let http_state = HttpState { state, ui_tx };
    Router::new()
        .route("/execute", post(execute))
        .route("/health", get(health))
        .with_state(http_state)
}
```

- [ ] **5.2** Remove StartRequest and StartResponse structs

Delete lines 33-45:
```rust
/// Start request body.
#[derive(Debug, Deserialize, Serialize)]
pub struct StartRequest {
    pub worker_endpoint: String,
}

/// Start response.
#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

- [ ] **5.3** Remove the `start` handler function

Delete the entire `start` async function (lines 48-71).

- [ ] **5.4** Remove unused Deserialize import if no longer needed

Check if `Deserialize` is still used elsewhere in the file. If `StartRequest` was the only struct using it, the import can remain since `OutboundRequest` likely uses it.

- [ ] **5.5** Verify build succeeds

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

**Commit:** `refactor(river-tui): remove vestigial /start endpoint`

---

## Task 6: Add PageUp/PageDown Scrolling

**Issue:** #13 - Only Up/Down available for scrolling.

**Files:** `crates/river-tui/src/tui.rs`

### Steps

- [ ] **6.1** Add PageUp handler after the Down handler

In the `match (key.code, key.modifiers)` block, add after the `KeyCode::Down` case:

```rust
(KeyCode::PageUp, _) => {
    let mut s = state.write().await;
    let page_size = 10; // Scroll 10 lines at a time
    s.conversation_scroll = s
        .conversation_scroll
        .saturating_add(page_size)
        .min(s.messages.len().saturating_sub(1));
}
(KeyCode::PageDown, _) => {
    let mut s = state.write().await;
    let page_size = 10;
    s.conversation_scroll = s.conversation_scroll.saturating_sub(page_size);
}
```

- [ ] **6.2** Update input help text to mention PageUp/PageDown

In `draw_input`, change the title:

```rust
.title(" Type message (Enter=send, Up/Down/PgUp/PgDn=scroll, Ctrl+C=quit) ")
```

- [ ] **6.3** Verify build succeeds

```bash
cd /home/cassie/river-engine && cargo build -p river-tui
```

**Commit:** `feat(river-tui): add PageUp/PageDown for faster scrolling`

---

## Task 7: Add Unit Tests for adapter.rs

**Issue:** #1 - Zero test coverage for state management.

**Files:** `crates/river-tui/src/adapter.rs`

### Steps

- [ ] **7.1** Add test module at the end of adapter.rs

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_state_new() {
        let state = AdapterState::new(
            "test-dyad".into(),
            "mock".into(),
            "general".into(),
        );

        assert_eq!(state.dyad, "test-dyad");
        assert_eq!(state.adapter_type, "mock");
        assert_eq!(state.channel, "general");
        assert!(state.worker_endpoint.is_none());
        assert!(state.messages.is_empty());
        assert_eq!(state.conversation_scroll, 0);
        assert_eq!(state.left_lines_read, 0);
        assert_eq!(state.right_lines_read, 0);
        assert!(state.input.is_empty());
    }

    #[test]
    fn test_add_user_message_unique_ids() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());

        let id1 = state.add_user_message("hello");
        let id2 = state.add_user_message("world");

        assert_ne!(id1, id2);
        assert_eq!(state.messages.len(), 2);
    }

    #[test]
    fn test_add_user_message_resets_scroll() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());
        state.conversation_scroll = 5;

        state.add_user_message("test");

        assert_eq!(state.conversation_scroll, 0);
    }

    #[test]
    fn test_add_system_message() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());

        state.add_system_message("Connected");

        assert_eq!(state.messages.len(), 1);
        match &state.messages[0] {
            DisplayMessage::System { content, .. } => {
                assert_eq!(content, "Connected");
            }
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn test_add_context_entry_increments_left_counter() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());

        let entry = OpenAIMessage {
            role: "user".into(),
            content: Some("hello".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        state.add_context_entry("left", entry);

        assert_eq!(state.left_lines_read, 1);
        assert_eq!(state.right_lines_read, 0);
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn test_add_context_entry_increments_right_counter() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());

        let entry = OpenAIMessage {
            role: "assistant".into(),
            content: Some("response".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        state.add_context_entry("right", entry);

        assert_eq!(state.left_lines_read, 0);
        assert_eq!(state.right_lines_read, 1);
    }

    #[test]
    fn test_context_lines_read() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());
        state.left_lines_read = 5;
        state.right_lines_read = 3;

        assert_eq!(state.context_lines_read("left"), 5);
        assert_eq!(state.context_lines_read("right"), 3);
        assert_eq!(state.context_lines_read("unknown"), 0);
    }

    #[test]
    fn test_supported_features() {
        let features = supported_features();

        assert!(features.contains(&FeatureId::SendMessage));
        assert!(features.contains(&FeatureId::ReceiveMessage));
        assert!(features.contains(&FeatureId::ReadHistory));
        assert_eq!(features.len(), 7);
    }

    #[test]
    fn test_generate_message_id_unique() {
        let mut state = AdapterState::new("d".into(), "m".into(), "c".into());

        let id1 = state.generate_message_id();
        let id2 = state.generate_message_id();

        assert_ne!(id1, id2);
    }
}
```

- [ ] **7.2** Run tests to verify they pass

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

**Commit:** `test(river-tui): add unit tests for AdapterState`

---

## Task 8: Add Unit Tests for tui.rs Helper Functions

**Issue:** #1 - Zero test coverage for UI formatting.

**Files:** `crates/river-tui/src/tui.rs`

### Steps

- [ ] **8.1** Add test module at the end of tui.rs

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use river_context::OpenAIMessage;

    #[test]
    fn test_truncate_str_short() {
        let result = truncate_str("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        let result = truncate_str("hello", 5);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_str_adds_ellipsis() {
        let result = truncate_str("hello world", 8);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_str_very_short_max() {
        let result = truncate_str("hello", 3);
        assert_eq!(result, "...");
    }

    #[test]
    fn test_format_message_user() {
        let msg = DisplayMessage::User {
            id: "123".into(),
            content: "hello".into(),
            timestamp: Utc::now(),
        };

        let items = format_message(&msg, 80);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_message_system() {
        let msg = DisplayMessage::System {
            content: "Connected".into(),
            timestamp: Utc::now(),
        };

        let items = format_message(&msg, 80);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_user() {
        let entry = OpenAIMessage {
            role: "user".into(),
            content: Some("test message".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);
        assert!(!items.is_empty());
    }

    #[test]
    fn test_format_context_entry_assistant() {
        let entry = OpenAIMessage {
            role: "assistant".into(),
            content: Some("response".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("right", &entry, &timestamp, 80);
        assert!(!items.is_empty());
    }

    #[test]
    fn test_format_context_entry_tool_call() {
        use river_context::{ToolCall, FunctionCall};

        let entry = OpenAIMessage {
            role: "assistant".into(),
            content: None,
            name: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_123".into(),
                r#type: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: r#"{"path": "/tmp/test.txt"}"#.into(),
                },
            }]),
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);
        assert!(!items.is_empty());
    }

    #[test]
    fn test_format_context_entry_tool_result() {
        let entry = OpenAIMessage {
            role: "tool".into(),
            content: Some("file contents here".into()),
            name: None,
            tool_calls: None,
            tool_call_id: Some("call_123".into()),
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_format_context_entry_empty_content() {
        let entry = OpenAIMessage {
            role: "assistant".into(),
            content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);
        assert!(!items.is_empty());
    }

    #[test]
    fn test_format_context_entry_multiline() {
        let entry = OpenAIMessage {
            role: "assistant".into(),
            content: Some("line1\nline2\nline3".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let timestamp = Utc::now();

        let items = format_context_entry("left", &entry, &timestamp, 80);
        assert_eq!(items.len(), 3);
    }
}
```

- [ ] **8.2** Run tests to verify they pass

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

**Commit:** `test(river-tui): add unit tests for TUI formatting functions`

---

## Task 9: Add Unit Tests for http.rs Endpoints

**Issue:** #1 - Zero test coverage for HTTP endpoints.

**Files:** `crates/river-tui/src/http.rs`

### Steps

- [ ] **9.1** Add test dependencies to Cargo.toml

In `crates/river-tui/Cargo.toml`, add dev-dependencies:

```toml
[dev-dependencies]
axum-test = "0.14"
tower = { workspace = true, features = ["util"] }
```

- [ ] **9.2** Add test module at the end of http.rs

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::AdapterState;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn create_test_state() -> HttpState {
        let state = Arc::new(RwLock::new(AdapterState::new(
            "test-dyad".into(),
            "mock".into(),
            "general".into(),
        )));
        let (ui_tx, _ui_rx) = mpsc::channel(16);
        HttpState { state, ui_tx }
    }

    #[tokio::test]
    async fn test_health_returns_ok() {
        let http_state = create_test_state();
        let app = Router::new()
            .route("/health", get(health))
            .with_state(http_state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_execute_send_message() {
        let http_state = create_test_state();
        {
            let mut s = http_state.state.write().await;
            s.worker_endpoint = Some("http://localhost:9999".into());
        }

        let app = Router::new()
            .route("/execute", post(execute))
            .with_state(http_state);

        let request = OutboundRequest::SendMessage {
            channel: "general".into(),
            content: "hello".into(),
            reply_to: None,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert!(matches!(resp.data, Some(ResponseData::MessageSent { .. })));
    }

    #[tokio::test]
    async fn test_execute_edit_message() {
        let http_state = create_test_state();
        let app = Router::new()
            .route("/execute", post(execute))
            .with_state(http_state);

        let request = OutboundRequest::EditMessage {
            channel: "general".into(),
            message_id: "msg-123".into(),
            content: "edited".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_execute_typing_indicator() {
        let http_state = create_test_state();
        let app = Router::new()
            .route("/execute", post(execute))
            .with_state(http_state);

        let request = OutboundRequest::TypingIndicator {
            channel: "general".into(),
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_execute_read_history_empty() {
        let http_state = create_test_state();
        let app = Router::new()
            .route("/execute", post(execute))
            .with_state(http_state);

        let request = OutboundRequest::ReadHistory {
            channel: "general".into(),
            limit: Some(10),
            before: None,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert!(matches!(resp.data, Some(ResponseData::History { messages }) if messages.is_empty()));
    }

    #[tokio::test]
    async fn test_execute_read_history_with_messages() {
        let http_state = create_test_state();
        {
            let mut s = http_state.state.write().await;
            s.add_user_message("hello");
            s.add_user_message("world");
        }

        let app = Router::new()
            .route("/execute", post(execute))
            .with_state(http_state);

        let request = OutboundRequest::ReadHistory {
            channel: "general".into(),
            limit: Some(10),
            before: None,
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: OutboundResponse = serde_json::from_slice(&body).unwrap();

        if let Some(ResponseData::History { messages }) = resp.data {
            assert_eq!(messages.len(), 2);
        } else {
            panic!("Expected History response");
        }
    }
}
```

- [ ] **9.3** Run tests to verify they pass

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

**Commit:** `test(river-tui): add unit tests for HTTP endpoints`

---

## Task 10: Add Tests for Context Parsing in main.rs

**Issue:** #1 - Zero test coverage for context file parsing.

**Files:** `crates/river-tui/src/main.rs`

### Steps

- [ ] **10.1** Add test module at the end of main.rs

Note: Testing async file I/O requires temporary files. We'll create integration-style tests.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_read_context_from_line_empty_file() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();

        let entries = read_context_from_line(&path, 0).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_read_context_from_line_parses_json() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role":"user","content":"hello"}}"#).unwrap();
        writeln!(file, r#"{{"role":"assistant","content":"hi"}}"#).unwrap();
        file.flush().unwrap();

        let path = file.path().to_path_buf();

        let entries = read_context_from_line(&path, 0).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_read_context_from_line_skips_lines() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role":"user","content":"first"}}"#).unwrap();
        writeln!(file, r#"{{"role":"user","content":"second"}}"#).unwrap();
        writeln!(file, r#"{{"role":"user","content":"third"}}"#).unwrap();
        file.flush().unwrap();

        let path = file.path().to_path_buf();

        let entries = read_context_from_line(&path, 2).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content.as_deref(), Some("third"));
    }

    #[tokio::test]
    async fn test_read_context_from_line_skips_empty_lines() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role":"user","content":"hello"}}"#).unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "   ").unwrap();
        writeln!(file, r#"{{"role":"user","content":"world"}}"#).unwrap();
        file.flush().unwrap();

        let path = file.path().to_path_buf();

        let entries = read_context_from_line(&path, 0).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_read_context_from_line_handles_invalid_json() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"role":"user","content":"valid"}}"#).unwrap();
        writeln!(file, "not valid json").unwrap();
        writeln!(file, r#"{{"role":"assistant","content":"also valid"}}"#).unwrap();
        file.flush().unwrap();

        let path = file.path().to_path_buf();

        let entries = read_context_from_line(&path, 0).await.unwrap();
        // Invalid JSON lines are skipped
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_read_context_from_line_file_not_found() {
        let path = PathBuf::from("/nonexistent/path/context.jsonl");

        let result = read_context_from_line(&path, 0).await;
        assert!(result.is_err());
    }
}
```

- [ ] **10.2** Add tempfile dev-dependency

In `crates/river-tui/Cargo.toml`:

```toml
[dev-dependencies]
axum-test = "0.14"
tower = { workspace = true, features = ["util"] }
tempfile = "3"
```

- [ ] **10.3** Run tests to verify they pass

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

**Commit:** `test(river-tui): add unit tests for context file parsing`

---

## Task 11: Extract Truncation Constants

**Issue:** #10 - Hardcoded magic numbers for truncation.

**Files:** `crates/river-tui/src/tui.rs`

### Steps

- [ ] **11.1** Add constants at the top of tui.rs (after imports)

```rust
/// Maximum length for tool call arguments display.
const TOOL_ARGS_MAX_LEN: usize = 30;

/// Maximum length for tool result content display.
const TOOL_RESULT_MAX_LEN: usize = 40;
```

- [ ] **11.2** Replace hardcoded values in format_context_entry

Replace `truncate_str(&tc.function.arguments, 30)` with:
```rust
truncate_str(&tc.function.arguments, TOOL_ARGS_MAX_LEN)
```

Replace `truncate_str(content, 40)` with:
```rust
truncate_str(content, TOOL_RESULT_MAX_LEN)
```

- [ ] **11.3** Verify build and tests pass

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

**Commit:** `refactor(river-tui): extract truncation length constants`

---

## Task 12: Final Verification and Cleanup

### Steps

- [ ] **12.1** Run all tests

```bash
cd /home/cassie/river-engine && cargo test -p river-tui
```

- [ ] **12.2** Run clippy for linting

```bash
cd /home/cassie/river-engine && cargo clippy -p river-tui -- -D warnings
```

- [ ] **12.3** Run with tracing to verify logs appear

```bash
cd /home/cassie/river-engine && RUST_LOG=info cargo run -p river-tui -- --help
```

- [ ] **12.4** Update any documentation if needed

No README changes required unless explicitly requested.

**Commit:** `chore(river-tui): final cleanup and verification`

---

## Summary

| Task | Issue(s) | Time Est. | Dependencies |
|------|----------|-----------|--------------|
| 1. Initialize tracing | #5 | 3 min | None |
| 2. Use tokio::fs | #4 | 5 min | None |
| 3. Log context errors | #7 | 2 min | Task 1 |
| 4. Handle /notify errors | #2 | 3 min | None |
| 5. Remove /start endpoint | #3 | 3 min | None |
| 6. Add PageUp/PageDown | #13 | 3 min | None |
| 7. Tests for adapter.rs | #1 | 5 min | None |
| 8. Tests for tui.rs | #1 | 5 min | None |
| 9. Tests for http.rs | #1 | 5 min | Task 5 |
| 10. Tests for main.rs | #1 | 5 min | Task 2 |
| 11. Extract constants | #10 | 2 min | None |
| 12. Final verification | N/A | 3 min | All |

**Total Estimated Time:** ~44 minutes

## Verification Checklist

- [ ] Unit tests for state management (Task 7)
- [ ] Unit tests for message formatting (Task 8)
- [ ] Unit tests for HTTP endpoints (Task 9)
- [ ] Unit tests for context parsing (Task 10)
- [ ] Error displayed when /notify fails (Task 4)
- [ ] /start endpoint removed (Task 5)
- [ ] tokio::fs used instead of std::fs (Task 2)
- [ ] Tracing initialized (Task 1)
- [ ] Context read errors logged (Task 3)
- [ ] All tests pass
- [ ] Clippy passes
