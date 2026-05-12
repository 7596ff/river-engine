# Auth Token Spec Review

## 1. No `dotenv` dependency anywhere in the project

The spec's entire design hinges on reading `RIVER_AUTH_TOKEN` from a `.env` file at the project root. But nothing in the workspace loads `.env` files. There is no `dotenv`, `dotenvy`, or equivalent crate in any `Cargo.toml` across the project. The `river-core` Cargo.toml (`crates/river-core/Cargo.toml`) has only `serde`, `serde_json`, and `thiserror`.

The spec never mentions adding a dotenv dependency. `std::env::var("RIVER_AUTH_TOKEN")` will only find the variable if it is set in the actual process environment (e.g., via systemd, shell export, or a wrapper script). A `.env` file sitting in the project root does nothing on its own. The spec's instructions to create `.env` and `.env.example` files are misleading — they imply file-based config but the proposed `require_auth_token()` function reads from env, not from a file.

This is either a silent deployment failure (token not loaded, service panics at startup) or the spec needs to explicitly add a `dotenvy` dependency to `river-core` and call `dotenvy::dotenv().ok()` at each service's entrypoint.

## 2. `require_auth_token()` panics — the spec says "refuses to start with a clear error message"

The spec says: "If the variable is missing or empty, the service refuses to start with a clear error message." But the function signature is:

```rust
pub fn require_auth_token() -> String
```

A function that returns `String` and "panics with a clear message if missing" (per the doc comment) will produce an unstructured panic backtrace, not a clean error message. The gateway currently uses `anyhow::Result` for startup errors and prints them cleanly. A panic bypasses that entirely. The spec should return `Result<String, ...>` or at minimum acknowledge that this changes the error reporting behavior of every service.

## 3. `AppState.auth_token` is `Option<String>` — the spec makes it non-optional but never addresses the type change

The gateway's `AppState` (`crates/river-gateway/src/state.rs`, line 27) has:

```rust
pub auth_token: Option<String>,
```

The `AppState::new()` constructor takes `auth_token: Option<String>` (line 70). The spec says "there is no `Option<String>` path" and "The new `validate_bearer_token` always validates." But it never specifies changing the `auth_token` field from `Option<String>` to `String`. If you change it to `String`, every call site that constructs `AppState` must change, including all test helpers.

The test helpers `test_state()` and `test_state_with_auth()` in `crates/river-gateway/src/api/routes.rs` (lines 405-526) pass `None` and `Some(token)` respectively. The `test_state()` function that passes `None` will fail to compile if the field becomes non-optional. Every existing test that creates an `AppState` without auth will break. The spec says nothing about updating tests.

## 4. `validate_auth` is called with `state.auth_token.as_deref()` — three different routes use this pattern

The existing code has three call sites for `validate_auth`:
- `handle_incoming` (line 235): `validate_auth(&headers, state.auth_token.as_deref())`
- `handle_bystander` (line 305): same pattern
- `register_adapter` (line 351): same pattern

The spec says the gateway's `validate_auth` "becomes a thin wrapper that extracts the header and calls `river_core::auth::validate_bearer`." But `validate_bearer` takes `(auth_header: &str, expected: &str)` — both non-optional. The wrapper needs to extract the header first and then pass it, but the spec doesn't show what happens to the `Option` unwrapping. If `auth_token` stays as `Option<String>`, you still need the "allow all if None" path the spec explicitly forbids. If it becomes `String`, see issue 3.

## 5. `GET /tools` has no auth check — spec acknowledges this but the fix creates a signature change

The `list_tools` handler (line 338-343) currently takes only `State(state)` — no `headers: HeaderMap` parameter. Adding auth requires adding the `headers` parameter. This is a minor point but the spec says "Add `validate_bearer_token` check" without noting that the handler signature must change. More importantly, if this is behind axum middleware instead, the spec doesn't mention middleware at all.

## 6. Discord adapter endpoints are wrong — the spec omits three existing routes

The spec's Discord adapter endpoint table lists:

| Endpoint | Auth |
|---|---|
| `GET /health` | no |
| `GET /capabilities` | yes |
| `POST /send` | yes |
| `POST /typing` | yes |
| `GET /read` | yes |
| `GET /channels` | yes |

But the actual router in `crates/river-discord/src/outbound.rs` (lines 201-213) has:

```rust
.route("/channels", post(add_channel))
.route("/channels/{id}", delete(remove_channel))
.route("/history/{channel}", get(history))
```

Three routes are missing from the spec's table: `POST /channels`, `DELETE /channels/{id}`, and `GET /history/{channel}`. These are unprotected in the current code and the spec doesn't mention them. `POST /channels` and `DELETE /channels/{id}` are administrative mutation endpoints that can add/remove channels from the listen set — leaving them unprotected while protecting `GET /channels` is an inconsistency. An attacker could add arbitrary channel IDs to the listen set or remove active ones.

## 7. OrchestratorState has no `auth_token` field — the spec says "Store in `OrchestratorState`"

`OrchestratorState` (`crates/river-orchestrator/src/state.rs`, lines 37-44) has fields for agents, config, local_models, external_models, resource_tracker, and process_manager. There is no `auth_token` field. The constructor (`new()`, lines 48-76) doesn't accept a token parameter.

Adding a field requires changing the constructor signature, which means changing every call site including the test helper `test_state()` in the same file (line 394) and in `crates/river-orchestrator/src/api/routes.rs` (line 373). The spec doesn't acknowledge this ripple.

## 8. Orchestrator routes have zero auth infrastructure

The orchestrator's route handlers (`crates/river-orchestrator/src/api/routes.rs`) don't import `HeaderMap`, don't import `AUTHORIZATION`, and none of them accept headers as a parameter. Every single handler would need its signature changed. The spec lists six endpoints that need auth but doesn't acknowledge that none of the existing handlers have the plumbing to receive or check headers.

## 9. HeartbeatClient has no token field — spec says "passed to `HeartbeatClient::new()` and stored"

`HeartbeatClient::new()` (`crates/river-gateway/src/heartbeat.rs`, line 25) takes `(orchestrator_url, agent_name, gateway_url)`. There is no token parameter. The struct (lines 16-20) has no token field. The `send_heartbeat` method (line 47) does a bare `.post(&url).json(&req).send()` with no Authorization header.

Adding the token parameter changes the constructor signature. The call site in `server.rs` (lines 386-389) constructs `HeartbeatClient::new(orchestrator_url, agent_name, gateway_url)` — this will fail to compile when `new()` gains a fourth parameter.

But there's a deeper issue: the spec says the HeartbeatClient "reads the token from the environment." If `require_auth_token()` is called here, it's a second call to read the env var. If the orchestrator requires auth, the gateway needs to know the orchestrator's token. But the spec says "one bearer token" shared by all services. So the gateway's token and the orchestrator's token are the same. This works, but only if every service is deployed with the same `.env` file — a constraint the spec never explicitly states. If someone deploys the orchestrator on a different machine with a different token, heartbeats silently fail (the heartbeat client already swallows errors on lines 53-59).

## 10. Heartbeat errors are silently swallowed — auth failures become invisible

`send_heartbeat()` in `crates/river-gateway/src/heartbeat.rs` returns `Ok(())` on both HTTP errors and network errors (lines 53-59). If the orchestrator starts requiring auth and the gateway sends unauthenticated heartbeats, the orchestrator returns 401, the client logs a warning, and returns `Ok(())`. The agent appears to run normally but the orchestrator thinks the agent is dead. The spec doesn't address this failure mode.

## 11. Gateway calls discord adapter endpoints without auth — who authenticates?

The gateway calls the discord adapter's `/send` endpoint via `SendMessageTool` (through `AdapterRegistry::send_message`). Looking at `crates/river-gateway/src/tools/adapters.rs` line 98, the outbound call is `.post(&config.outbound_url).json(&request).send()` — no Authorization header.

If the discord adapter now requires auth on `/send`, `/typing`, and `/read`, then the gateway's calls to those endpoints will start getting 401 responses. The spec doesn't mention that the gateway (and any other service that calls adapter endpoints) needs to include the bearer token in its outgoing requests to adapters. This is the reverse direction of the auth flow and the spec completely ignores it.

This is arguably the biggest problem: the spec adds auth to adapter endpoints without considering that the gateway is a client of those endpoints. Implementing the spec as written will break message delivery.

## 12. Discord adapter's `AppState` has no `auth_token` field

The discord adapter's `AppState` (`crates/river-discord/src/outbound.rs`, lines 160-167) has no `auth_token` field. The `AppState::new()` method (lines 170-179) doesn't accept a token. Same as the orchestrator issue, but the spec doesn't mention the specific changes needed.

## 13. `river-core` has no env-reading capability

`river-core`'s `Cargo.toml` has only `serde`, `serde_json`, and `thiserror`. `std::env::var` is in the standard library so the `require_auth_token()` function can technically work, but if the spec intends `.env` file support, `dotenvy` must be added. The spec doesn't list any dependency changes for `river-core`.

## 14. Migration path — existing deployments use `--auth-token-file`

The spec says "Remove `auth_token_file` from `ServerConfig` and CLI args." The gateway currently accepts `--auth-token-file` (line 53 of `main.rs`). Any deployment that passes `--auth-token-file /path/to/token` will get an "unrecognized argument" error after the change. The spec provides no migration path — no deprecation period, no warning, no documentation of the breaking CLI change.

Deployments using systemd unit files, docker compose files, or shell scripts that pass `--auth-token-file` will break on upgrade. The spec's "Out of Scope" section doesn't mention this.

## 15. `validate_bearer` does not use constant-time comparison

The spec's `validate_bearer` function is described as doing string matching. The existing `validate_auth` in `routes.rs` (line 33) does `token == expected`, which is a non-constant-time string comparison. This is a timing side-channel vulnerability. For a localhost-only deployment this is low risk, but the spec explicitly defers HTTPS/TLS, meaning if anyone deploys this over a network, the token is both sent in plaintext AND vulnerable to timing attacks. The spec should at minimum note this as a known limitation or specify using a constant-time comparison.

## 16. The spec names two different functions: `validate_bearer` and `validate_bearer_token`

The shared module section defines `validate_bearer`. The Gateway Changes section says "Add `validate_bearer_token` check." These are different names. An implementer following the spec literally will either define two functions or get confused about which name to use. Pick one.

## Summary of blocking issues

1. **Compilation failures**: Changing `auth_token` from `Option<String>` to `String` breaks all test helpers and constructors across gateway, orchestrator, and discord adapter. The spec doesn't address any of these.

2. **Runtime breakage**: Gateway calls to discord adapter endpoints will get 401 after auth is added to the adapter. Message delivery stops.

3. **Silent failure**: Heartbeat client swallows 401 errors. Orchestrator thinks agents are dead.

4. **Missing routes**: Three discord adapter endpoints omitted from the spec's auth table.

5. **No `.env` loading**: The `.env` file does nothing without a `dotenvy` dependency that the spec never adds.

6. **Breaking CLI change**: `--auth-token-file` removal breaks existing deployments with no migration path.
