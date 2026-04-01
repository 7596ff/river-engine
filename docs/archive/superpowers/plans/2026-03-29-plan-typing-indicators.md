# Typing Indicators Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `typing` tool that sends typing indicators to the current channel.

**Architecture:** The `typing` tool uses `channel_context` (like `speak`) to get the current channel, checks if the adapter supports `TypingIndicator` feature, and POSTs to the adapter's `/typing` endpoint. Silent success if unsupported.

**Tech Stack:** Rust, axum, Twilight (Discord), existing river-gateway infrastructure.

**Spec:** `docs/superpowers/specs/2026-03-29-typing-indicators-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/river-gateway/src/tools/communication.rs` | Modify | Add `features` to `AdapterConfig`, `supports()` to `AdapterRegistry`, add `TypingTool` |
| `crates/river-discord/src/outbound.rs` | Modify | Add `/typing` endpoint |

---

## Task 1: Add features to AdapterConfig and AdapterRegistry

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Add Feature import**

Add at top of file after line 7:
```rust
use river_adapter::Feature;
use std::collections::HashSet;
```

- [ ] **Step 2: Add features field to AdapterConfig**

Update the `AdapterConfig` struct (around line 17):
```rust
/// Adapter endpoint configuration
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Adapter name (e.g., "discord", "slack")
    pub name: String,
    /// Outbound webhook URL (for sending messages)
    pub outbound_url: String,
    /// Read URL (for fetching channel history), optional
    pub read_url: Option<String>,
    /// Supported features
    pub features: HashSet<Feature>,
}
```

- [ ] **Step 3: Add supports method to AdapterRegistry**

Add after the `names()` method (around line 52):
```rust
    pub fn supports(&self, name: &str, feature: Feature) -> bool {
        self.adapters
            .get(name)
            .map(|c| c.features.contains(&feature))
            .unwrap_or(false)
    }
```

- [ ] **Step 4: Add test for supports method**

Add to the tests module:
```rust
    #[test]
    fn test_adapter_registry_supports() {
        let mut registry = AdapterRegistry::new();

        let mut features = HashSet::new();
        features.insert(Feature::TypingIndicator);

        registry.register(AdapterConfig {
            name: "discord".to_string(),
            outbound_url: "http://localhost:8080/send".to_string(),
            read_url: None,
            features,
        });

        assert!(registry.supports("discord", Feature::TypingIndicator));
        assert!(!registry.supports("discord", Feature::Reactions));
        assert!(!registry.supports("nonexistent", Feature::TypingIndicator));
    }
```

- [ ] **Step 5: Fix existing tests**

Update `test_adapter_registry` to include the features field:
```rust
        registry.register(AdapterConfig {
            name: "discord".to_string(),
            outbound_url: "http://localhost:8080/send".to_string(),
            read_url: None,
            features: HashSet::new(),
        });
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-gateway tools::communication`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(tools): add feature tracking to AdapterRegistry

Adds features field to AdapterConfig and supports() method to
AdapterRegistry for checking adapter capabilities.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add TypingTool

**Files:**
- Modify: `crates/river-gateway/src/tools/communication.rs`

- [ ] **Step 1: Add TypingTool struct**

Add after `SpeakTool` implementation:

```rust
/// Send typing indicator to the current channel
pub struct TypingTool {
    registry: Arc<RwLock<AdapterRegistry>>,
    http_client: reqwest::Client,
    channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
}

impl TypingTool {
    pub fn new(
        registry: Arc<RwLock<AdapterRegistry>>,
        channel_context: Arc<RwLock<Option<crate::agent::ChannelContext>>>,
    ) -> Self {
        Self {
            registry,
            http_client: reqwest::Client::new(),
            channel_context,
        }
    }
}

impl Tool for TypingTool {
    fn name(&self) -> &str {
        "typing"
    }

    fn description(&self) -> &str {
        "Send a typing indicator to the current channel"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(&self, _args: Value) -> Result<ToolResult, RiverError> {
        let registry = self.registry.clone();
        let http_client = self.http_client.clone();
        let channel_context = self.channel_context.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Get channel context
                let ctx_guard = channel_context.read().await;
                let ctx = ctx_guard.as_ref().ok_or_else(|| {
                    error!("TypingTool: No channel selected");
                    RiverError::tool("No channel selected. Use switch_channel first.")
                })?;

                let adapter = ctx.adapter.clone();
                let channel_id = ctx.channel_id.clone();

                drop(ctx_guard); // Release lock before async call

                let registry = registry.read().await;

                // Check if adapter supports typing
                if !registry.supports(&adapter, Feature::TypingIndicator) {
                    debug!(adapter = %adapter, "Adapter doesn't support typing indicators");
                    return Ok(ToolResult::success("Typing indicator sent"));
                }

                let config = registry.get(&adapter).ok_or_else(|| {
                    RiverError::tool(format!("Adapter '{}' not registered", adapter))
                })?;

                // Build typing URL (same base as outbound, but /typing endpoint)
                let typing_url = config.outbound_url
                    .trim_end_matches("/send")
                    .to_string() + "/typing";

                let payload = serde_json::json!({
                    "channel": channel_id,
                });

                info!(
                    url = %typing_url,
                    adapter = %adapter,
                    channel_id = %channel_id,
                    "Sending typing indicator"
                );

                let response = http_client
                    .post(&typing_url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| {
                        error!(error = %e, "Typing indicator request failed");
                        RiverError::tool(format!("Failed to send typing indicator: {}", e))
                    })?;

                if response.status().is_success() {
                    Ok(ToolResult::success("Typing indicator sent"))
                } else {
                    // Silent failure - just log and return success
                    warn!(status = %response.status(), "Typing indicator returned non-success");
                    Ok(ToolResult::success("Typing indicator sent"))
                }
            })
        })
    }
}
```

- [ ] **Step 2: Add tests for TypingTool**

Add to tests module:

```rust
    #[test]
    fn test_typing_tool_schema() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let channel_context = Arc::new(RwLock::new(None));

        let tool = TypingTool::new(registry, channel_context);

        assert_eq!(tool.name(), "typing");
        assert_eq!(tool.description(), "Send a typing indicator to the current channel");
        let params = tool.parameters();
        assert_eq!(params["properties"], serde_json::json!({}));
        assert_eq!(params["required"], serde_json::json!([]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_typing_without_channel_selected() {
        let registry = Arc::new(RwLock::new(AdapterRegistry::new()));
        let channel_context = Arc::new(RwLock::new(None)); // No channel set

        let tool = TypingTool::new(registry, channel_context);

        let result = tool.execute(serde_json::json!({}));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No channel selected"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_typing_unsupported_adapter_silent_success() {
        let mut registry = AdapterRegistry::new();
        registry.register(AdapterConfig {
            name: "test".to_string(),
            outbound_url: "http://localhost:9999/send".to_string(),
            read_url: None,
            features: HashSet::new(), // No TypingIndicator feature
        });

        let registry = Arc::new(RwLock::new(registry));
        let channel_context = Arc::new(RwLock::new(Some(crate::agent::ChannelContext {
            path: PathBuf::from("test.txt"),
            adapter: "test".to_string(),
            channel_id: "123".to_string(),
            channel_name: Some("test".to_string()),
            guild_id: None,
        })));

        let tool = TypingTool::new(registry, channel_context);

        let result = tool.execute(serde_json::json!({}));

        assert!(result.is_ok());
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-gateway tools::communication`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(tools): add typing tool for typing indicators

Sends typing indicator to current channel. Returns silent success
if adapter doesn't support TypingIndicator feature.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add /typing endpoint to Discord adapter

**Files:**
- Modify: `crates/river-discord/src/outbound.rs`

- [ ] **Step 1: Add TypingRequest struct**

Add after `DiscordSendRequest` struct (around line 70):

```rust
/// Typing indicator request
#[derive(Debug, Deserialize)]
pub struct TypingRequest {
    pub channel: String,
}

/// Typing response
#[derive(Debug, Serialize)]
pub struct TypingResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

- [ ] **Step 2: Add typing handler function**

Add after the other handler functions:

```rust
/// Handle typing indicator request
async fn handle_typing(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TypingRequest>,
) -> Result<Json<TypingResponse>, (StatusCode, Json<TypingResponse>)> {
    let channel_id: u64 = request.channel.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(TypingResponse {
                success: false,
                error: Some("Invalid channel ID".to_string()),
            }),
        )
    })?;

    let channel_id = twilight_model::id::Id::new(channel_id);

    state
        .discord
        .trigger_typing(channel_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, channel_id = %request.channel, "Failed to send typing indicator");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TypingResponse {
                    success: false,
                    error: Some(format!("Failed to send typing indicator: {}", e)),
                }),
            )
        })?;

    tracing::info!(channel_id = %request.channel, "Typing indicator sent");

    Ok(Json(TypingResponse {
        success: true,
        error: None,
    }))
}
```

- [ ] **Step 3: Add trigger_typing to DiscordSender**

In `crates/river-discord/src/client.rs`, add method to `DiscordSender`:

```rust
    /// Send a typing indicator to a channel
    pub async fn trigger_typing(&self, channel_id: Id<ChannelMarker>) -> Result<(), Error> {
        self.http
            .create_typing_trigger(channel_id)
            .await
            .map_err(|e| Error::Discord(e.to_string()))?;
        Ok(())
    }
```

- [ ] **Step 4: Add /typing route**

Update `create_router` function (around line 188):

```rust
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/capabilities", get(capabilities))
        .route("/send", post(handle_send))
        .route("/typing", post(handle_typing))
        .route("/read", get(handle_read))
        .route("/channels", get(list_channels))
        .route("/channels", post(add_channel))
        .route("/channels/{id}", delete(remove_channel))
        .route("/history/{channel}", get(history))
        .with_state(state)
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-discord`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(discord): add /typing endpoint for typing indicators

Uses Twilight's create_typing_trigger to show "Bot is typing..."
in Discord channels.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Update roadmap

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Update typing indicators status**

Find the Communication section (around line 113) and update:

```markdown
| Typing indicators | 🟢 | `typing` tool shows typing while agent thinks |
```

- [ ] **Step 2: Commit**

```bash
git add docs/roadmap.md && git commit -m "$(cat <<'EOF'
docs: mark typing indicators as complete in roadmap

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Add features to AdapterConfig/Registry | `tools/communication.rs` |
| 2 | Add TypingTool | `tools/communication.rs` |
| 3 | Add /typing endpoint to Discord | `outbound.rs`, `client.rs` |
| 4 | Update roadmap | `docs/roadmap.md` |
