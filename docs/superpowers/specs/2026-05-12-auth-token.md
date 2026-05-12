# Auth Token — Design Spec

One bearer token. One env file. Every service. Auth is always required.

## Token Source

A `.env` file at the project root contains:

```
RIVER_AUTH_TOKEN=<token>
```

Every service calls `dotenvy::dotenv().ok()` at the top of `main()` to load the `.env` file into the process environment, then reads `RIVER_AUTH_TOKEN` via `std::env::var`. If the variable is missing or empty, the service exits with a clear error message (not a panic — a proper `Result` error through the service's existing error handling).

A `.env.example` is checked into the repo:

```
RIVER_AUTH_TOKEN=your-secret-token-here
```

The `.env` file itself is gitignored.

**Dependency:** Each service crate (`river-gateway`, `river-orchestrator`, `river-discord`) adds `dotenvy` to its `Cargo.toml`. `river-core` does not depend on `dotenvy` — it only provides the auth validation logic using `std::env`.

**CLI migration:** The gateway currently accepts `--auth-token-file`. This is kept as a fallback — if `RIVER_AUTH_TOKEN` is set in the environment, it takes precedence. If not, the gateway reads from `--auth-token-file` if provided. If neither is set, the service refuses to start. This avoids breaking existing deployments.

## Shared Auth Module

`river-core` gets an `auth` module with two public functions:

```rust
/// Read RIVER_AUTH_TOKEN from environment.
/// Returns Err if missing or empty.
pub fn require_auth_token() -> Result<String, RiverError>

/// Validate a bearer token from an Authorization header value.
/// `auth_header` is the raw value of the Authorization header (e.g. "Bearer abc123").
/// Returns true if it matches "Bearer <expected>".
pub fn validate_bearer(auth_header: &str, expected: &str) -> bool
```

Each service extracts the `Authorization` header using its own HTTP framework (axum `HeaderMap`) and passes the raw header value to `validate_bearer`. This keeps `river-core` free of HTTP framework dependencies.

Each service implements a thin `validate_auth` wrapper that extracts the header and calls `river_core::auth::validate_bearer`. The gateway already has this pattern — the orchestrator and Discord adapter copy it.

## State Changes

Every service that holds auth state changes its token field from `Option<String>` to `String`:

- **Gateway:** `AppState.auth_token` changes from `Option<String>` to `String`. The `AppState::new()` constructor changes accordingly. Test helpers that previously passed `None` for auth must pass a test token instead.
- **Orchestrator:** `OrchestratorState` gains an `auth_token: String` field. Constructor updated. Test helpers updated.
- **Discord adapter:** `AppState` gains an `auth_token: String` field. Constructor updated. Test helpers updated.

## Gateway Changes

**Read token:** Try `RIVER_AUTH_TOKEN` env var first, fall back to `--auth-token-file` if provided, error if neither.

**Add auth to `GET /tools`:** The `list_tools` handler gains a `headers: HeaderMap` parameter and calls `validate_auth`.

**All endpoints after change:**

| Endpoint | Auth |
|---|---|
| `GET /health` | ❌ |
| `POST /incoming` | ✅ |
| `POST /home/{agent}/message` | ✅ |
| `GET /tools` | ✅ |
| `POST /adapters/register` | ✅ |

## Gateway → Adapter Auth (Outbound Calls)

The gateway is a client of adapter endpoints — it calls `/send` on adapters via `AdapterRegistry::send_to_adapter`. If adapters require auth, the gateway must send the bearer token on these outbound requests.

The gateway builds a `reqwest::Client` with a default `Authorization: Bearer <token>` header (via `reqwest::ClientBuilder::default_headers`). This client is used for all outbound HTTP calls — to adapters and to the orchestrator. This replaces any inline `reqwest::Client::new()` calls in `send_to_adapter` and the heartbeat client.

## Orchestrator Changes

**Read token:** Call `dotenvy::dotenv().ok()` in `main()`, then `river_core::auth::require_auth_token()`. Store in `OrchestratorState`.

**Add auth to all non-health endpoints.** Every handler gains a `headers: HeaderMap` parameter and calls `validate_auth`. Import `HeaderMap` and `AUTHORIZATION` from `axum::http`.

| Endpoint | Auth |
|---|---|
| `GET /health` | ❌ |
| `POST /heartbeat` | ✅ |
| `GET /agents/status` | ✅ |
| `GET /models/available` | ✅ |
| `POST /model/request` | ✅ |
| `POST /model/release` | ✅ |
| `GET /resources` | ✅ |

## Gateway → Orchestrator (Heartbeat Client)

The `HeartbeatClient` accepts an auth token in its constructor and includes `Authorization: Bearer <token>` on every request to the orchestrator.

The heartbeat client currently swallows all HTTP errors (returns `Ok(())` on 401, 500, etc). This changes: 401 responses are logged at `error` level with a clear message ("orchestrator rejected heartbeat — auth token mismatch"), not silently swallowed as warnings. Other HTTP errors remain warnings.

## Adapter → Gateway Auth (Outbound Calls)

Adapters are clients of the gateway — the Discord adapter calls `POST /adapters/register` to self-register and forwards incoming events to `POST /incoming` via `GatewayClient`. Both endpoints require auth after this change.

The Discord adapter reads the auth token at startup and builds a `reqwest::Client` with a default `Authorization: Bearer <token>` header, same pattern as the gateway. This client is used for all outbound calls to the gateway — registration, event forwarding, and health checks.

## Discord Adapter Changes

**Read token:** Call `dotenvy::dotenv().ok()` in `main()`, then `river_core::auth::require_auth_token()`. Store in `AppState`.

**Add auth to all non-health endpoints:**

| Endpoint | Auth |
|---|---|
| `GET /health` | ❌ |
| `GET /capabilities` | ✅ |
| `POST /send` | ✅ |
| `POST /typing` | ✅ |
| `GET /read` | ✅ |
| `GET /channels` | ✅ |
| `POST /channels` | ✅ |
| `DELETE /channels/{id}` | ✅ |
| `GET /history/{channel}` | ✅ |

## Out of Scope

- TUI auth (deferred to TUI redesign for home channel tail)
- Per-service tokens or role-based access
- Token rotation
- HTTPS/TLS (assumed localhost or VPN for now)
