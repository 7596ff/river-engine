# River Engine v4 — Spec Cross-Review

> Generated: 2026-04-02
> Last updated: 2026-04-02
> Specs reviewed: orchestrator, worker, adapter-library, context-management, snowflake-server, embedding, workspace-structure

## Summary

**Total Issues: 30** — All resolved ✓

---

## Resolved Issues

### Critical (all fixed)

| # | Issue | Resolution |
|---|-------|------------|
| 1 | RegistrationInfo field name mismatch | Fixed comment in worker spec |
| 2 | ProcessEntry vs registry response | Aligned GET /registry JSON with ProcessEntry |
| 4 | ModelConfig context_limit type | Made context_limit required in orchestrator |
| 5-6 | Baton/Side type duplication | Consolidated in river-adapter |
| 7 | Missing embed service registration | Added EmbedService to ProcessEntry, registration handler |
| 8 | Missing embed endpoint discovery | Worker receives via registry push |
| 9 | Incomplete adapter spawn protocol | Defined registration-based config delivery |
| 11 | "remaining" undefined | Documented as count of additional results |
| 13 | Missing worker→embed client | Documented endpoint discovery via registry |
| 16 | Snowflake type HTTP format | Documented snake_case serde config |
| 19 | Model switch request construction | Clarified dyad + side derivation |
| 25 | Missing worker→adapter integration | Documented registry-based routing |
| 30 | Missing switch_roles protocol | Defined orchestrator-mediated two-phase commit |

### Medium (all fixed)

| # | Issue | Resolution |
|---|-------|------------|
| 3 | Adapter feature validation gap | Documented FeatureId import in orchestrator |
| 10 | Ground type duplication | Consolidated in river-adapter |
| 12 | Embed dimensions hardcoded | Schema now uses config value |
| 14 | Tool result persistence format | Documented OpenAI format storage |
| 15 | Adapter registration response | Added worker_endpoint, validated_features, config |
| 17 | Ground serde attributes | Added derive macros |
| 18 | ExitStatus serde configuration | Documented tag configuration |
| 20 | Worker health response | Defined HealthResponse struct |
| 21 | Ground.channel field type | Changed to Channel type |
| 22 | Context assembly for tool results | Documented OpenAI format pass-through |
| 23 | context.jsonl format | Defined as OpenAI message format |
| 24 | ConversationFile implementation | Redesigned as hybrid append-only with compaction |
| 26 | Embed service error handling | Documented ToolError::EmbedServerUnreachable |
| 27 | Orchestrator summary persistence | Documented storage in Ground for respawn |
| 28 | Adapter type validation | Type is freeform string, comment fixed |
| 29 | Tool error code standardization | Added ToolError enum |

---

## Second Review Pass (2026-04-02)

**6 additional issues found and fixed:**

| # | Issue | Resolution |
|---|-------|------------|
| 31 | worker_name field doesn't exist | Changed to dyad + side in orchestrator endpoints |
| 32 | Flash target identification unclear | Changed to target_dyad + target_side fields |
| 33 | Worker initial state says "role" not "baton" | Fixed to "baton" |
| 34 | Channel ID collision in moves/moments | Changed to `{adapter}_{channel_id}.jsonl` |
| 35 | Embed service HTTP API table misleading | Clarified /register is orchestrator's endpoint |
| 36 | Undescribed directories (memory/, artifacts/) | Added brief descriptions |

---

## Key Design Decisions Made During Review

1. **Type ownership:** river-adapter owns shared types (Baton, Side, Ground, Channel, Author)
2. **Context storage:** Uses OpenAI message format directly (zero-conversion LLM calls)
3. **Conversation files:** Hybrid append-only with periodic compaction
4. **Adapter config:** Registration-based delivery (no CLI secrets)
5. **Switch roles:** Orchestrator-mediated two-phase commit with dyad lock
6. **Service discovery:** Registry push for embed/adapter endpoints
7. **Worker identification:** Always via (dyad, side) pair, never by name
