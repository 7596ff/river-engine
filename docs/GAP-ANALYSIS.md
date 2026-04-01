# River Engine — Gap Analysis (Updated)

## Status: Most gaps resolved. Remaining items are deferred or open questions.

---

## Resolved

| # | Gap | Resolution |
|---|-----|------------|
| 1 | Worker intro says "three things" | Removed. Philosophy section only. |
| 2 | Conversation file format undefined | Deferred to context-building crate design. |
| 3 | Flash documented in two places | Deferred to context-building crate as source of truth. |
| 4 | speak tool table says "payload only" | Fixed: says "feature + payload" now. |
| 5 | Worker doesn't know orchestrator endpoint | Added `orchestrator_endpoint` to `WorkerInput`. |
| 6 | Worker doesn't know its own name | Added `worker_name` to `WorkerInput`. |
| 7 | Worker has no HTTP API documented | Added: `/notify`, `/registry`, `/flash`, `/health`. |
| 8 | First LLM call has no token count | Accepted: first response establishes baseline. |
| 9 | Retry logic underspecified | 1m, 2m, 5m backoff. System message on each retry. Fail after 3. |
| 10 | sleep + summary in same batch | Summary always wins. |
| 11 | Context persistence timing | Persist after every mutation (model response AND each tool result). |
| 14 | Parallel vs sequential tool calls | All tool calls in a single response are parallel. Sequential only across turns. |
| 15 | speak without switch_channel | `ground` concept: default channel is human operator's DM. |
| 16 | How orchestrator passes config to children | CLI args (orchestrator endpoint). |
| 17 | Health check frequency/timeout | Every 60s. Dead after 3 failures (3 minutes). |
| 18 | First notification contents | No special message. First real notification from adapter or ground starts the loop. |
| 19 | Stale adapter endpoints after restart | Worker requests context from orchestrator on startup. Fresh registry pushed. |
| 20 | Respawn policy | Done → permanent sleep. Exhausted → respawn with summary. Error → restart from JSONL. |
| 21 | Graceful shutdown timeout | 5 minutes. Local models may be slow. |
| 22 | Registry includes model? | Yes, model tracked in Worker registry entries. |
| 23 | Feature negotiation | Resolved: adapter features in registration, orchestrator includes in system prompt. |
| 24 | Adapter registration missing features | Added features list to adapter registration payload. |
| 25 | Two Discord adapters / one token | Use different bot tokens per adapter process. |
| 26 | Event types unstandardized | Standard event types defined in `river-adapter` crate. |
| 27 | Error format unstandardized | Standard: `{ok, data?, error?: {code, message}}`. |
| 28 | Schema from Rust to system prompt | Orchestrator reads features from registration, uses `AdapterFeature::schema()` from shared crate. |
| 29 | No auth between processes | v0: no auth, localhost only. Documented. |
| 30 | No TLS | v0: localhost only. Documented. |
| 31 | Orchestrator SPOF | v0: accepted. Workers persist JSONL. Manual recovery. |
| 32 | No logging | Deferred to after initial prototype. |
| 33 | Workspace race conditions | File read/write routed through orchestrator for locking. |
| 34 | Model needs feature names | System prompt includes names and ints: "SendMessage(0), EditMessage(10)". Documentation deferred to after engine is built. |
| 37 | No timestamps for the model | Current time injected in status messages after text responses and sleep wake-ups. |

## Remaining Open Questions

| # | Gap | Status |
|---|-----|--------|
| 12 | write insert past end of file | **Minor.** Decide during implementation. Likely: append + warning. |
| 13 | read on huge files | **Minor.** Add optional `max_lines` param during implementation. |
| 35 | Max tool calls per cycle | **Deferred.** Need more information from practice. Add after initial prototype if needed. |
| 36 | Flash during streaming LLM response | **Open.** Can the Worker cancel a streaming response? Currently: flash queued until stream completes. May want stream cancellation for responsiveness. |
| - | Multiple actor/spectator pairs | **Open.** Config supports it. Edge cases unknown. |
| - | Flash overflow / backpressure | **Open.** Defer to context-building crate design. |
