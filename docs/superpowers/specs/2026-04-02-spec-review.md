# River Engine v4 — Spec Cross-Review

> Generated: 2026-04-02
> Specs reviewed: orchestrator, worker, adapter-library, context-management, snowflake-server, embedding, workspace-structure

## Summary

**Total Issues: 30**
- Critical: 10
- Medium: 20

---

## Critical Issues

### 1. RegistrationInfo Field Name Mismatch
**Specs:** Orchestrator, Worker

Worker spec comment (line 169) says `partner` but the actual field is `partner_endpoint`.

**Fix:** Change worker spec line 169 from `partner` to `partner_endpoint`

---

### 2. ProcessEntry vs Registry Response Mismatch
**Specs:** Orchestrator

`GET /registry` response shows `name`, `partner`, `baton` but `ProcessEntry::Worker` has `dyad`, `side`, `baton`. The response JSON doesn't match the struct.

**Fix:** Align GET /registry response JSON with ProcessEntry types.

---

### 4. ModelConfig Field Type Mismatch
**Specs:** Orchestrator, Worker

- Orchestrator defines `context_limit: Option<usize>`
- Worker expects `context_limit: usize` (required)

**Fix:** Change Orchestrator's ModelConfig to have `context_limit: usize` (required).

---

### 5-6. Missing Baton and Side Type Exports
**Specs:** Orchestrator, Worker, Adapter Library

Both `Baton` and `Side` enums are defined separately in Orchestrator and Worker. Should be shared.

**Fix:** Define `Baton` and `Side` in river-adapter, import in both crates.

---

### 7. Missing Embed Service Registration in Orchestrator
**Specs:** Orchestrator, Embedding Service

Embedding spec describes registration with orchestrator, but:
- Orchestrator `ProcessEntry` has no `EmbedService` variant
- POST /register doesn't handle embed services
- Startup sequence doesn't spawn embed service

**Fix:** Add embed service support to ProcessEntry, registration handler, and startup sequence.

---

### 8. Missing Embed Server Endpoint Discovery
**Specs:** Embedding Service, Worker

Worker needs embed server endpoint for `/index`, `/search`, `/next` but there's no mechanism to discover it.

**Fix:** Either push embed endpoint via registry or provide on worker registration.

---

### 9. Incomplete Adapter Spawn Protocol
**Specs:** Orchestrator, Adapter Library, Worker

Missing details:
- How is config JSON passed to adapter binary?
- When does adapter call POST /start?
- How does adapter discover worker endpoint?

**Fix:** Add explicit adapter startup protocol to orchestrator spec.

---

### 11. Tool Return Value - "remaining" Undefined
**Specs:** Worker, Embedding Service

SearchEmbeddingsTool returns `remaining` but it's never defined what this counts.

**Fix:** Document that `remaining` = number of additional results available.

---

### 13. Missing Worker→Embed Client Connection
**Specs:** Worker, Embedding Service

Worker defines embedding tools but doesn't show:
- How worker gets embed server endpoint
- No embed client initialization
- No HTTP client in crate structure

**Fix:** Add embed server endpoint to worker state and document discovery.

---

### 16. Snowflake Type HTTP API Format
**Specs:** Snowflake Server, Worker

Enum values are integers but HTTP API expects snake_case strings. Conversion not documented.

**Fix:** Document snake_case ↔ enum value mapping.

---

### 19. Missing Model Switch Request Construction
**Specs:** Orchestrator, Worker

RequestModelTool has only `model` field, but orchestrator expects `worker_name`. Worker state has `dyad`, not `name`.

**Fix:** Clarify `worker_name` should be `dyad` or add derivation logic.

---

### 25. Missing Worker→Adapter Integration
**Specs:** Worker, Adapter Library

SpeakTool calls adapter but:
- How does worker get adapter endpoint?
- How does worker know which adapter has which features?
- Direct call or via orchestrator?

**Fix:** Clarify worker uses registry to route adapter calls directly.

---

### 30. Missing Switch Roles Protocol
**Specs:** Worker, Orchestrator

SwitchRolesTool has no defined protocol:
- Who initiates?
- How is orchestrator notified? (no endpoint exists)
- What if both try simultaneously?

**Fix:** Define full switch_roles protocol with orchestrator notification endpoint.

---

## Medium Issues

### 3. Adapter Registration Feature Validation Gap
**Specs:** Orchestrator, Adapter Library

Orchestrator validates features but connection between adapter registration and FeatureId import is unclear.

---

### 10. Ground Type Duplication
**Specs:** Orchestrator, Worker

`Ground` struct defined identically in both specs. Should be shared.

---

### 12. Embed Model Dimensions Hardcoded
**Specs:** Orchestrator, Embedding Service

Storage schema shows `FLOAT[768]` hardcoded but dimensions should come from config.

---

### 14. Tool Result Persistence Format
**Specs:** Worker, Context Management

Worker appends tool results to context but format/conversion not documented.

---

### 15. Adapter Registration Response Missing Info
**Specs:** Orchestrator, Adapter Library

Response only has `{ "accepted": true }` but should include worker endpoint and validated features.

---

### 17. Ground Type Serde Attributes
**Specs:** Orchestrator, Worker

Ground struct needs `#[derive(Serialize, Deserialize)]` but not shown.

---

### 18. ExitStatus Serde Configuration
**Specs:** Orchestrator, Worker

JSON shows `{ "Done": { ... } }` format but serde tag configuration not shown.

---

### 20. Worker Health Response Undefined
**Specs:** Worker

`GET /health` returns `Json<HealthResponse>` but HealthResponse struct not defined.

---

### 21. Ground Channel Field Type
**Specs:** Adapter Library, Worker

Ground has `channel: String` but should probably use `Channel` type for consistency.

---

### 22. Context Assembly for Tool Results
**Specs:** Worker, Context Management

How tool calls/results convert to ContextItem not documented.

---

### 23. context.jsonl Format Undefined
**Specs:** Workspace Structure, Worker, Context Management

Format of context JSONL not specified.

---

### 24. ConversationFile Implementation
**Specs:** Worker

Methods defined as `fn(path)` not `fn(&self)`. Also `mark_read` would require rewriting file.

---

### 26. Embed Service Error Handling
**Specs:** Worker, Embedding Service

How worker handles embed server errors (retry? degrade?) not specified.

---

### 27. Orchestrator Summary Persistence
**Specs:** Orchestrator, Worker

Worker sends summary on exit but orchestrator doesn't show where it's stored for respawn.

---

### 28. Adapter Type Validation
**Specs:** Orchestrator, Adapter Library

No enum of valid adapter types. Comment on ProcessEntry.Adapter.type is wrong.

---

### 29. Tool Error Code Standardization
**Specs:** Worker

Error codes inconsistent across tools. Should be standardized enum.

---

## Priority Order for Fixes

1. **Blocking:** 7, 9, 30 (missing endpoints/protocols)
2. **Type system:** 4, 5, 6, 10 (shared types)
3. **Integration:** 8, 13, 25 (service discovery)
4. **Documentation:** 11, 16, 19, 22, 23 (clarifications)
5. **Polish:** everything else
