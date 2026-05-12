# Auth Token Spec Review -- Pass 2

Review 1 found 16 issues including 6 blockers. The spec has been updated. This review verifies the fixes and identifies what remains broken.

## Fixed Issues (Verified)

The following issues from review 1 are now addressed in the updated spec:

- **Issue 1 (dotenvy dependency):** Fixed. The spec now explicitly says each service crate adds `dotenvy` and calls `dotenvy::dotenv().ok()` in `main()`. It correctly keeps `dotenvy` out of `river-core`, which only uses `std::env::var`. Properly fixed.

- **Issue 2 (require_auth_token panics):** Fixed. The function signature is now `Result<String, RiverError>`. The spec says "not a panic -- a proper Result error through the service's existing error handling." Clean.

- **Issue 3 (Option<String> to String):** Fixed. The spec explicitly says `auth_token` changes from `Option<String>` to `String` for gateway, orchestrator, and Discord. States that test helpers must pass a test token instead of `None`.

- **Issue 5 (GET /tools no auth):** Fixed. The spec adds `headers: HeaderMap` to `list_tools` and puts it in the auth-required table.

- **Issue 6 (missing Discord routes):** Fixed. The Discord table now includes `POST /channels`, `DELETE /channels/{id}`, and `GET /history/{channel}`.

- **Issue 7/8 (OrchestratorState missing field/infrastructure):** Fixed. The spec says `OrchestratorState` gains `auth_token: String`, constructor updated, test helpers updated. All handlers gain `headers: HeaderMap`.

- **Issue 9 (HeartbeatClient no token):** Fixed. The spec says HeartbeatClient accepts an auth token in its constructor and includes the Authorization header.

- **Issue 10 (heartbeat errors swallowed):** Fixed. The spec says 401 responses are logged at `error` level with a clear message, not silently swallowed.

- **Issue 11 (gateway-to-adapter auth):** Fixed. The spec adds a "Gateway -> Adapter Auth" section: `AdapterRegistry` stores the auth token and includes `Authorization: Bearer <token>` on every request to adapter endpoints.

- **Issue 12 (Discord AppState missing field):** Fixed. The spec says Discord's `AppState` gains `auth_token: String`.

- **Issue 14 (CLI migration):** Fixed. The spec keeps `--auth-token-file` as a fallback: env var takes precedence, then file, then refuse to start.

- **Issue 15 (constant-time comparison):** Fixed. The `validate_bearer` doc comment now says "Uses constant-time comparison to prevent timing side-channels."

- **Issue 16 (naming inconsistency):** Fixed. The spec consistently uses `validate_bearer` everywhere.

## Remaining and New Issues

### BLOCKER 1: `validate_bearer` constant-time claim but no `subtle` dependency

The spec says `validate_bearer` "uses constant-time comparison to prevent timing side-channels" but `river-core`'s `Cargo.toml` has only `serde`, `serde_json`, and `thiserror`. There is no `subtle` crate (or any other constant-time comparison crate) in the dependency list, and the spec does not mention adding one.

Rust's `==` on `&str`/`String` is **not** constant-time -- it short-circuits on the first differing byte. To deliver on the spec's promise, `river-core` needs `subtle` (or equivalent) added to its `Cargo.toml`, and `validate_bearer` must use `subtle::ConstantTimeEq` to compare the token bytes. Without this, the spec upgrades a documented limitation into an undocumented lie. Either add the dependency or drop the constant-time claim.

### BLOCKER 2: `AdapterRegistry` auth token storage is underspecified and creates a wiring problem

The spec says "The `AdapterRegistry` (or the HTTP client it uses for outbound calls) stores the auth token." But the actual `AdapterRegistry` struct (`crates/river-gateway/src/tools/adapters.rs`, line 27-29) is:

```rust
pub struct AdapterRegistry {
    adapters: HashMap<String, AdapterConfig>,
}
```

It has no token field, no HTTP client, and the `send_to_adapter` function (line 62) takes an external `http_client: &reqwest::Client` -- the registry does not own the client. The spec says "or the HTTP client it uses" but this ambiguity means an implementer has to make a design decision the spec should be making:

1. Add `auth_token: String` to `AdapterRegistry` and thread it into `send_to_adapter`.
2. Add `auth_token: String` to `AdapterConfig` (per-adapter tokens, but the spec says one token).
3. Add the token to the `reqwest::Client` as a default header via `reqwest::ClientBuilder::default_headers`.
4. Modify `send_to_adapter` to take an additional `auth_token: &str` parameter.

Each option has different ripple effects on call sites. The spec should pick one. Option 3 is the cleanest (a `reqwest::Client` built with a default `Authorization` header covers all outbound calls), but it means the client must be constructed after the token is loaded. The current code constructs `reqwest::Client` inline or uses defaults -- none of the outbound call paths currently have access to a pre-configured client with auth headers.

### BLOCKER 3: Discord adapter self-registration with gateway will break

The Discord adapter's `register_with_gateway` function (`crates/river-discord/src/adapter.rs`, line 29-49) calls `POST /adapters/register` on the gateway. After this spec, that endpoint requires auth. But the `register_with_gateway` function does a bare `.post(&url).json(&RegisterRequest { adapter: info }).send()` with no Authorization header.

The spec's "Gateway -> Adapter Auth" section only covers the gateway calling adapter endpoints. It never addresses the reverse: adapters calling gateway endpoints. The Discord adapter needs the token to register itself with the gateway. This means `register_with_gateway` must accept the token and include the Authorization header. The `DiscordConfig` or `Args` must source the token. The Discord `main.rs` must load it. None of this is specified.

Without this fix, adapter self-registration silently fails (the registration spawn catches errors and logs warnings but continues), and the gateway never learns about the adapter. Message delivery breaks because `AdapterRegistry` is empty unless adapters are also configured via CLI `--adapter` flags.

### BLOCKER 4: `river_core::auth` module does not exist

The spec says `river-core` gets an `auth` module. `river-core/src/lib.rs` currently declares four modules: `config`, `error`, `snowflake`, `types`. There is no `auth` module, no `auth.rs` file, and no re-export. The spec should explicitly state that `auth.rs` must be created in `crates/river-core/src/` and that `lib.rs` must add `pub mod auth;` and the appropriate re-exports. This is not a "the implementer will figure it out" situation -- the spec should name the file.

This is marginal as a blocker since an implementer could infer it, but combined with the missing `subtle` dependency it means the entire auth validation layer is unspecified at the implementation level.

### Issue 5: Gateway `validate_auth` wrapper still uses `Option` pattern

The spec says each service implements a thin `validate_auth` wrapper. But the existing `validate_auth` in `crates/river-gateway/src/api/routes.rs` (line 18) takes `Option<&str>` and short-circuits to allow-all on `None`:

```rust
fn validate_auth(headers: &HeaderMap, expected_token: Option<&str>) -> Result<(), StatusCode> {
    let Some(expected) = expected_token else {
        return Ok(());
    };
    // ...
}
```

The spec changes `auth_token` from `Option<String>` to `String`, which means `state.auth_token.as_deref()` (used at lines 235, 305, 351) returns `&str` not `Option<&str>`. The wrapper signature must change to accept `&str` instead of `Option<&str>`, and the "allow all if None" path must be removed. The spec says this implicitly by making auth mandatory, but the existing code has three call sites using `.as_deref()` that will need updating. This is straightforward but unacknowledged.

### Issue 6: `send_to_adapter` has no typing indicator or other outbound call paths

The spec says the auth token applies to `SendMessageTool`, `ReadChannelTool`, "and any other tool that calls adapter HTTP endpoints." But the actual outbound calls to adapters happen in `send_to_adapter` (in `adapters.rs`). There is no `ReadChannelTool` in the codebase -- the read tool is `SyncConversation` in `crates/river-gateway/src/tools/sync.rs` (disabled, per the comment in server.rs). The spec references a tool that does not exist under that name.

More critically, `send_to_adapter` is the only function that actually makes HTTP calls to adapter URLs. The typing indicator is never called from the gateway tools -- it is only reachable when something calls the adapter's `/typing` endpoint directly. The spec should clarify which code paths actually need the auth header added, rather than referencing nonexistent tools.

### Issue 7: `dotenvy::dotenv().ok()` path resolution is fragile

The spec says every service calls `dotenvy::dotenv().ok()` at the top of `main()`. `dotenvy::dotenv()` searches for `.env` starting from the current working directory and walking up. In production, the gateway runs as a systemd service where the working directory is typically `/` or the service's `WorkingDirectory=` setting. The `.env` file at the project root will not be found unless the systemd unit explicitly sets `WorkingDirectory` to the project root, or the `.env` file is placed at `/`, or `EnvironmentFile=` is used in the unit file instead.

The spec should acknowledge this deployment constraint. The `--auth-token-file` fallback partially mitigates this, but only for the gateway -- the orchestrator and Discord adapter have no file fallback, so they rely entirely on `dotenvy` finding the `.env` or the variable being set in the process environment.

### Issue 8: No test coverage specified for the new `river_core::auth` module

The spec adds two public functions to `river-core` but says nothing about testing them. At minimum:
- `require_auth_token()` with var set, with var missing, with var empty
- `validate_bearer()` with valid token, wrong token, missing "Bearer " prefix, empty header, case sensitivity of "Bearer"

These are security-critical functions. The spec should require unit tests.

### Issue 9: `GatewayClient` in Discord adapter also calls gateway without auth

The Discord adapter creates a `GatewayClient` (`crates/river-discord/src/main.rs`, line 35) that does health checks and forwards incoming Discord events to the gateway's `/incoming` endpoint. The `GatewayClient::new` takes only a URL. After this spec, `/incoming` requires auth. The `GatewayClient` must send the bearer token on its calls to the gateway.

The spec's Discord adapter section only covers protecting the Discord adapter's own endpoints. It never mentions that the Discord adapter is also a *client* of the gateway and must authenticate its outbound calls to the gateway. This is the same class of bug as the original review's issue 11, but in the opposite direction (adapter -> gateway instead of gateway -> adapter).

## Summary

**4 blockers remain:**
1. Constant-time comparison claimed but no `subtle` dependency added
2. `AdapterRegistry` auth token storage is ambiguous -- spec must pick a design
3. Discord adapter self-registration with gateway will get 401 (no auth on outbound call)
4. `river_core::auth` module does not exist yet and the spec does not say to create the file

**5 non-blocking issues:**
5. Gateway `validate_auth` wrapper must change signature (implicit but unacknowledged)
6. Spec references nonexistent `ReadChannelTool`
7. `dotenvy` path resolution fragile in production deployments
8. No test coverage specified for security-critical auth module
9. Discord `GatewayClient` also calls gateway endpoints without auth

The spec fixed most of review 1's problems substantively rather than papering over them. The remaining blockers fall into two categories: the outbound-auth story has gaps (blocker 2, 3, and issue 9 -- the spec addressed gateway-to-adapter but missed adapter-to-gateway and the implementation details), and the `river-core` auth module exists only as prose (blocker 1, 4). The first category is the more dangerous one: it is easy to implement the spec as written and have a system where adapters cannot register and events cannot be forwarded.
