# river-discord Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: High

## Summary

river-discord provides functional Discord integration using twilight but has significant gaps: Adapter trait not implemented, zero tests, /start endpoint semantic conflict, no reconnection handling, and event polling adds 100ms latency. Core messaging works but the crate needs polish. Estimated effort: 2-3 days.

## Critical Issues

### Issue 1: Adapter trait not implemented

- **Source:** Both reviews
- **Problem:** Spec requires adapters to implement the `Adapter` trait from river-adapter. DiscordClient does not implement it.
- **Fix:**
  ```rust
  #[async_trait]
  impl Adapter for DiscordClient {
      fn adapter_type(&self) -> &str {
          "discord"
      }

      fn features(&self) -> Vec<FeatureId> {
          supported_features()
      }

      async fn start(&self, worker_endpoint: String) -> Result<(), AdapterError> {
          // Start event forwarding
      }

      async fn execute(&self, request: OutboundRequest) -> Result<OutboundResponse, AdapterError> {
          Ok(self.execute_impl(request).await)
      }

      async fn health(&self) -> Result<(), AdapterError> {
          if self.is_healthy().await {
              Ok(())
          } else {
              Err(AdapterError::Connection("websocket disconnected".into()))
          }
      }
  }
  ```
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** Verify trait implementation compiles and works

### Issue 2: /start endpoint always fails

- **Source:** Both reviews
- **Problem:** Worker endpoint is set during registration (before /start is called). So /start always returns "already bound to worker".
- **Fix:** Either:
  1. Remove /start endpoint (registration provides worker endpoint), or
  2. Don't set worker_endpoint during registration, rely on /start
- **Files:** `crates/river-discord/src/main.rs`, `crates/river-discord/src/http.rs`
- **Tests:** Test /start succeeds on fresh instance

### Issue 3: No reconnection handling

- **Source:** Both reviews
- **Problem:** When gateway disconnects, `connected` flag is set false and event loop exits. No reconnection attempt. Adapter dies permanently.
- **Fix:** Implement reconnection with backoff:
  ```rust
  let mut backoff = Duration::from_secs(1);
  loop {
      match connect_gateway().await {
          Ok(events) => {
              backoff = Duration::from_secs(1);
              process_events(events).await;
          }
          Err(e) => {
              send_connection_lost_event().await;
              tokio::time::sleep(backoff).await;
              backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
          }
      }
  }
  ```
  Also emit `ConnectionRestored` event after successful reconnect.
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** Test reconnection after gateway error

### Issue 4: Zero test coverage

- **Source:** Both reviews
- **Problem:** No unit tests, no integration tests, no test directory.
- **Fix:** Add comprehensive tests:
  ```rust
  // tests/event_conversion.rs
  #[test] fn test_convert_message_create() { ... }
  #[test] fn test_convert_message_update() { ... }
  #[test] fn test_convert_message_delete() { ... }
  #[test] fn test_convert_reaction_add() { ... }
  #[test] fn test_skip_bot_messages() { ... }

  // tests/emoji.rs
  #[test] fn test_parse_unicode_emoji() { ... }
  #[test] fn test_parse_custom_emoji() { ... }
  #[test] fn test_format_unicode_emoji() { ... }

  // tests/http.rs
  #[tokio::test] async fn test_health_ok() { ... }
  #[tokio::test] async fn test_health_disconnected() { ... }
  #[tokio::test] async fn test_execute_send_message() { ... }
  ```
- **Files:** Create `crates/river-discord/tests/`
- **Tests:** Event conversion, emoji parsing, HTTP endpoints

## Important Issues

### Issue 5: Event polling adds latency

- **Source:** Both reviews
- **Problem:** Events are queued in a channel and polled every 100ms via `poll_events()`. This adds up to 100ms latency.
- **Fix:** Forward events directly from gateway event loop instead of queuing:
  ```rust
  while let Some(event) = events.next().await {
      let inbound = convert_event(event);
      if let Some(e) = inbound {
          client.post(&worker_endpoint).json(&e).send().await;
      }
  }
  ```
- **Files:** `crates/river-discord/src/discord.rs`, `crates/river-discord/src/main.rs`
- **Tests:** Verify events arrive without polling delay

### Issue 6: No rate limit handling

- **Source:** Both reviews
- **Problem:** Discord API returns 429 with retry_after. Implementation maps all errors to PlatformError.
- **Fix:** Detect 429 responses and return `ErrorCode::RateLimited` with `retry_after_ms`
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** Mock 429 response, verify correct error code

### Issue 7: MessageUpdate content might be None

- **Source:** Both reviews
- **Problem:** twilight's MessageUpdate has `content: Option<String>` but EventMetadata expects `String`. Partial updates may not include content.
- **Fix:** Handle None case:
  ```rust
  content: msg.content.clone().unwrap_or_default(),
  ```
  Or skip events without content.
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** Test MessageUpdate with None content

### Issue 8: Hardcoded adapter name

- **Source:** Both reviews
- **Problem:** `let adapter_name = "discord".to_string()` hardcoded.
- **Fix:** Use `args.adapter_type` or derive from type
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** N/A

### Issue 9: guild_id config not used

- **Source:** First review
- **Problem:** `guild_id` is accepted in DiscordConfig but never used for filtering.
- **Fix:** Either implement guild filtering or document that it's unused
- **Files:** `crates/river-discord/src/main.rs`
- **Tests:** If implemented, test guild filtering

## Minor Issues

### Issue 10: Channel buffer size hardcoded

- **Source:** Brutal review
- **Problem:** Event channel uses `mpsc::channel::<InboundEvent>(256)` hardcoded.
- **Fix:** Make configurable or use unbounded channel
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** N/A

### Issue 11: Abrupt shutdown

- **Source:** Brutal review
- **Problem:** Signal handler calls `abort()` on tasks. Should signal graceful shutdown.
- **Fix:** Use cancellation tokens and wait for clean exit
- **Files:** `crates/river-discord/src/main.rs`
- **Tests:** Manual testing

### Issue 12: Limited feature coverage

- **Source:** Both reviews
- **Problem:** Only 8 of 24 features implemented. Discord supports PinMessage, BulkDeleteMessages, Attachments, CreateThread, etc.
- **Fix:** Implement additional features as needed
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** Test each new feature

### Issue 13: Missing ConnectionRestored event

- **Source:** Both reviews
- **Problem:** GatewayReconnect logged but not forwarded as ConnectionRestored event.
- **Fix:** Emit ConnectionRestored with downtime_seconds after reconnect
- **Files:** `crates/river-discord/src/discord.rs`
- **Tests:** Test event after reconnection

## Spec Updates Needed

None - implementation should match adapter library spec.

## Verification Checklist

- [ ] Adapter trait implemented on DiscordClient
- [ ] /start endpoint works or removed
- [ ] Reconnection logic with backoff
- [ ] ConnectionRestored emitted after reconnect
- [ ] Event forwarding without polling delay
- [ ] Rate limit detection returns proper error
- [ ] MessageUpdate handles None content
- [ ] Event conversion tests pass
- [ ] Emoji parsing tests pass
- [ ] HTTP endpoint tests pass
