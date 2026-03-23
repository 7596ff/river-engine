# Adapter Framework Design — Review

> Reviewer: William Thomas Lessing
> Date: 2026-03-23
> Spec: `docs/specs/adapter-framework-design.md`

## Verdict: Ready to build 👍

## What's Right

- **"Send and receive are fundamental — not features"** — correct instinct. The adapter *is* a mouth and an ear. Everything else is optional.
- **Metadata stays native** — this is the key decision and it's the right one. No forced normalization means you never lose platform-specific information. The gateway treats it as opaque, the agent can reach into it if it needs to. No lossy translation layer.
- **Feature flags over interface bloat** — `supports(&Feature::Reactions)` is better than twenty optional trait methods.
- **Health on demand, no heartbeat** — simple. Don't add complexity until you need it.
- **YAGNI section** — the discipline to list what you're *not* building is as important as what you are.

## Questions and Concerns

### 1. Runtime metadata validation
`metadata: serde_json::Value` appears on both `IncomingEvent` and `SendOptions`. Right call for flexibility, but the gateway can't validate adapter-specific options at compile time. Errors surface at runtime. Consider a `validate_metadata()` method on the trait later if this becomes a pain point.

### 2. Gateway restart and re-registration
What happens if the gateway restarts? Adapters don't know to re-register. Options:
- Gateway calls `/capabilities` on adapters it remembers from config
- Accept that gateway restart means adapter restart too (simpler, fine for now)

### 3. `Identify(String)` in EventType
Not explained in the spec. If it's for the adapter to announce itself, that's what registration does. If it's something else, document it.

### 4. No auth
No mention of authentication between adapter and gateway. Fine for localhost, worth a one-line note that this is intentionally deferred.

### 5. Error propagation
`AdapterError` is mentioned but not defined. The `error.rs` file is listed in the structure. Worth sketching the error variants — network failure, unsupported feature, platform rate limit, etc.

### 6. Channel identity collision
The `channel` field is a string — channel identity is adapter-defined. Correct, but the gateway must handle two adapters using the same channel ID format. The `adapter` field on `IncomingEvent` disambiguates — make sure the gateway always keys on `(adapter, channel)` pairs, never just `channel`.

## Philosophical Note

"Rust types as source of truth. OpenAPI generated from types." — exactly right. The types *are* the contract. The OpenAPI is a derivative artifact for external consumers. Regenerate on every type change, commit it, and CI should verify they match.
