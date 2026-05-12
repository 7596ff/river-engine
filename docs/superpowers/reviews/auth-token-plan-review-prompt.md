# Auth Token Plan Review Prompt

Paste the plan below into Gemini or another reviewer, followed by this prompt. Also paste the spec (`docs/superpowers/specs/2026-05-12-auth-token.md`) for reference.

---

You are reviewing an **implementation plan** for adding bearer token authentication across all services in a multi-agent orchestration system called river-engine. The plan implements a spec that has been through two review passes. Your job is not to re-review the spec — it is to verify that the plan faithfully implements it and that the code will actually compile and work.

This is a critical review. Your job is to **lay bare contradictions** between the plan and the spec, find code that won't compile, catch type mismatches, and identify gaps where the plan promises something but doesn't deliver the code.

## 1. Spec Fidelity

Walk through each section of the spec and verify the plan has a task that implements it:
- Shared auth module in river-core with `require_auth_token` and `validate_bearer`
- `.env` file loaded via `dotenvy` in every service
- Gateway: env var with `--auth-token-file` fallback, non-optional token, auth on all non-health endpoints
- Gateway outbound: authed client for adapter calls and heartbeat
- Heartbeat client: 401 logged as error, not warning
- Orchestrator: auth on all non-health endpoints, `auth_token` added to state
- Discord adapter: auth on ALL endpoints (the spec lists 9 routes — does the plan cover all 9?)
- Discord outbound: `GatewayClient` and `register_with_gateway` use authed client
- `AppState.auth_token` changes from `Option<String>` to `String` everywhere

## 2. Compilation Issues

Read the code in each task as if you were the Rust compiler:
- Does `build_authed_client` in river-core work? `river-core` would need `reqwest` as a dependency. Does the plan add it?
- The plan adds `reqwest` to `river-core/Cargo.toml` with `rustls-tls`. Does `river-core` currently depend on `reqwest`? Will this create duplicate dependency issues with the other crates that also depend on `reqwest`?
- Does `RiverError::config()` exist? The plan says "check if missing, add if needed" — but does it actually show the code to add it?
- The `validate_auth` wrapper is copy-pasted into three services (gateway, orchestrator, discord). Should it be in `river-core` instead? If not, is the duplication intentional?
- Task 3 Step 9 adds `http_client: reqwest::Client` to `AppState`. But `AppState::new` already has many parameters. Does the plan show the updated constructor call in `server.rs`?
- Task 4 says "find every place that passes a `reqwest::Client`" — this is vague. The plan should list the exact files and line numbers. Does it?

## 3. Test Coverage Gaps

- Task 1 has 9 tests for the auth module. Are there edge cases missing? What about a token with spaces, newlines, or unicode?
- Task 3 says "update any tests that call `test_state()` and expect no auth — they now need to include `Authorization: Bearer test-token`." How many tests is that? The plan doesn't list them. An implementer following the plan will have to grep for every test that makes HTTP requests and update each one.
- Tasks 5 and 6 say "update tests" but don't show the updated test code. An implementer has to guess what changes are needed.

## 4. Task Dependencies and Order

- Can Task 1 (river-core auth module) be tested independently? It depends on `reqwest` for `build_authed_client`. Does `cargo test -p river-core` work with `reqwest` added?
- Task 4 depends on Task 3's `http_client` field in `AppState`. But Task 3 Step 9 adds the field at the end of the task. If an implementer stops at Step 8, Task 4 breaks.
- Task 6 (Discord) depends on Task 3 (Gateway) because the gateway now requires auth on `/adapters/register` and `/incoming`. If Task 6 is implemented first, the Discord adapter can't register. Is the ordering correct?

## 5. The `build_authed_client` Problem

The plan puts `build_authed_client` in `river-core`. But:
- `river-core` is a lightweight core crate (`serde`, `serde_json`, `thiserror`). Adding `reqwest` (which pulls in `tokio`, `hyper`, `rustls`, etc.) makes it a heavy dependency for every crate that depends on `river-core`.
- Does every consumer of `river-core` need `reqwest`? The snowflake module, the config module, the error types — none of these need HTTP.
- Would it be better to put `build_authed_client` in each service, or in a separate `river-http` crate?
- If `build_authed_client` stays in `river-core`, does the plan handle the feature flag correctly? Should `reqwest` be an optional dependency behind a feature flag?

## 6. The Communication Tools Problem

Task 4 Step 1 says to update `communication.rs` tools that hold their own `http_client: reqwest::Client`. Looking at the actual code:
- `SendMessageTool` has `http_client: reqwest::Client` initialized with `reqwest::Client::new()`
- `SyncConversationTool` in `sync.rs` has `http_client: reqwest::Client` initialized with `reqwest::Client::new()`
- `ModelRequestTool` in `model.rs` has `http_client: reqwest::Client`

Each of these constructs its own client. The plan says to "pass the authed client from the outside" but doesn't show the constructor changes for each tool, the updated tool registration in `server.rs`, or how `ToolExecutor` threading works.

## 7. What's Missing

- **Token generation.** The plan creates `.env.example` with a placeholder. Task 7 generates a real token with `openssl rand -hex 32`. But there's no documentation or script for an operator setting up a new deployment. Is the README updated?
- **The `on-sweep.md` prompt substitution.** The plan uses `{recent_moves}` and `{entries}` as template variables. Does the `substitute` function use `{key}` or `{{key}}`? If the auth token contains `{`, does it break template substitution?
- **Concurrent env var tests.** Task 1 tests `require_auth_token` by setting/unsetting `RIVER_AUTH_TOKEN`. If tests run in parallel, they will interfere with each other. Is `cargo test` running these serially?

Be specific. Cite task numbers and code when pointing out issues. Focus on things that would cause compilation failures, runtime panics, or silent security holes.
