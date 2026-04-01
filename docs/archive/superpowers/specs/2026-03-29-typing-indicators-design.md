# Typing Indicators Design Spec

> Show typing indicators when the agent chooses
>
> Date: 2026-03-29
> Authors: Cass, Claude

---

## 1. Summary

Add a `typing` tool that sends a typing indicator to the current channel. The agent calls this tool when it wants to signal activity. Typing indicators are ephemeral (fire and forget) - they naturally expire after a few seconds.

**Philosophy:** "The agent thinks, then types, then utters."

---

## 2. Tool

### `typing`

Send a typing indicator to the current channel.

```json
{
  "type": "object",
  "properties": {},
  "required": []
}
```

No parameters - uses current channel from `channel_context`.

**Behavior:**
1. Check `channel_context` is set (error if not)
2. Look up adapter in registry
3. Check if adapter supports `TypingIndicator` feature
4. If not supported, return success immediately (silent)
5. POST to adapter's `/typing` endpoint with `channel_id`
6. Return success

**Errors:**
- `"No channel selected. Use switch_channel first."` - no context set

Adapters that don't support typing return silent success - the agent doesn't need to know.

---

## 3. Adapter Protocol

### `POST /typing`

New endpoint on adapters that support `TypingIndicator` feature.

**Request:**
```json
{
  "channel": "789012345678901234"
}
```

**Response:**
```json
{
  "success": true
}
```

**Errors:**
```json
{
  "success": false,
  "error": "Channel not found"
}
```

---

## 4. Discord Implementation

Discord's typing indicator lasts approximately 10 seconds and auto-expires.

**Implementation:**
- Use Twilight's `create_typing_trigger` method
- Target channel from request

```rust
http_client
    .create_typing_trigger(channel_id)
    .await?;
```

Discord adapter already declares `Feature::TypingIndicator` in `discord_adapter_info()`.

---

## 5. Feature Checking

The tool checks adapter capabilities before calling:

```rust
if registry.supports(&adapter_name, Feature::TypingIndicator) {
    // Call /typing endpoint
} else {
    // Return success without calling
}
```

**Required change:** Add `features: HashSet<Feature>` to `AdapterConfig` in `tools/communication.rs`. When adapters register via `/adapters/register`, populate this from `AdapterInfo.features`. Add a `supports()` method to `AdapterRegistry`.

---

## 6. File Structure

| File | Changes |
|------|---------|
| `crates/river-gateway/src/tools/communication.rs` | Add `TypingTool`, add `features` to `AdapterConfig`, add `supports()` to `AdapterRegistry` |
| `crates/river-discord/src/outbound.rs` | Add `/typing` endpoint handler |

---

## 7. Testing

### Unit Tests

- `typing` tool schema validation (no params)
- `typing` without channel selected → error
- `typing` with adapter that doesn't support feature → success

### Integration Tests

- Discord `/typing` endpoint returns success
- End-to-end: switch_channel → typing → verify no error

---

## 8. Summary

| Component | Description |
|-----------|-------------|
| `typing` tool | Send typing indicator to current channel |
| `/typing` endpoint | Adapter endpoint for typing triggers |
| Feature check | Silent success if adapter doesn't support |
| Discord impl | Twilight's `create_typing_trigger` |

The agent thinks, then types, then utters. Typing is deliberate.
