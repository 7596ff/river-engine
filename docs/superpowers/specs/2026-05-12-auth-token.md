# Auth Token — Design Spec

One bearer token. One env file. Every service. Auth is always required.

## Token Source

A `.env` file at the project root contains:

```
RIVER_AUTH_TOKEN=<token>
```

Every service reads `RIVER_AUTH_TOKEN` from the environment at startup. If the variable is missing or empty, the service refuses to start with a clear error message. No fallback, no "allow all if unconfigured."

A `.env.example` is checked into the repo:

```
RIVER_AUTH_TOKEN=your-secret-token-here
```

The `.env` file itself is gitignored.

## Shared Auth Module

`river-core` gets an `auth` module with two public functions:

```rust
/// Read RIVER_AUTH_TOKEN from environment. Panics with a clear message if missing.
pub fn require_auth_token() -> String

/// Validate a bearer token value against the expected token.
/// `auth_header` is the raw value of the Authorization header.
/// Returns true if it matches "Bearer <expected>".
pub fn validate_bearer(auth_header: &str, expected: &str) -> bool
```

Each service extracts the `Authorization` header using its own HTTP framework (axum `HeaderMap`) and passes the raw header value to `validate_bearer`. This keeps `river-core` free of HTTP framework dependencies.

The gateway's existing `validate_auth` function in `api/routes.rs` becomes a thin wrapper that extracts the header and calls `river_core::auth::validate_bearer`. The other services copy this same thin wrapper pattern.

## Gateway Changes

**Read token from env:** Replace `--auth-token-file` CLI arg and file-based loading with `river_core::auth::require_auth_token()` at startup. Store the token in `AppState`.

**Remove `auth_token_file`** from `ServerConfig` and CLI args. Remove the file-reading logic in `server.rs`.

**Add auth to `GET /tools`:** Currently unprotected. Add `validate_bearer_token` check.

**Make token required:** The existing `validate_auth` allowed all requests when no token was configured. The new `validate_bearer_token` always validates — there is no `Option<String>` path.

**Endpoints after change:**

| Endpoint | Auth |
|---|---|
| `GET /health` | ❌ |
| `POST /incoming` | ✅ |
| `POST /home/{agent}/message` | ✅ |
| `GET /tools` | ✅ |
| `POST /adapters/register` | ✅ |

## Orchestrator Changes

**Read token from env:** Call `river_core::auth::require_auth_token()` at startup. Store in `OrchestratorState`.

**Add auth to all non-health endpoints:**

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

The `HeartbeatClient` reads the token from the environment and sends `Authorization: Bearer <token>` on every request to the orchestrator. The token is passed to `HeartbeatClient::new()` and stored alongside the other fields.

## Discord Adapter Changes

**Read token from env:** Call `river_core::auth::require_auth_token()` at startup. Store in `AppState`.

**Add auth to all non-health endpoints:**

| Endpoint | Auth |
|---|---|
| `GET /health` | ❌ |
| `GET /capabilities` | ✅ |
| `POST /send` | ✅ |
| `POST /typing` | ✅ |
| `GET /read` | ✅ |
| `GET /channels` | ✅ |

## Out of Scope

- TUI auth (deferred to TUI redesign for home channel tail)
- Per-service tokens or role-based access
- Token rotation
- HTTPS/TLS (assumed localhost or VPN for now)
