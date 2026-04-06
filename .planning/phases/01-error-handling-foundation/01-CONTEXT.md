# Phase 1: Error Handling Foundation - Context

**Gathered:** 2026-04-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Replace panics with Result types across three crates: river-discord, river-protocol, river-context. All critical code paths should return errors instead of crashing.

</domain>

<decisions>
## Implementation Decisions

### Error handling approach
- **D-01:** Use `thiserror` for custom error types in each crate
- **D-02:** Use `anyhow` for context wrapping at application boundaries
- **D-03:** Preserve existing error information — don't lose context when converting

### Claude's Discretion
- Exact error type hierarchy design
- Which errors are recoverable vs fatal
- Error message wording
- Whether to add new error variants or extend existing ones

</decisions>

<canonical_refs>
## Canonical References

No external specs — requirements fully captured in decisions above. The existing error types in `river-adapter` (`ToolError`, `AdapterError`) provide patterns to follow.

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `river-adapter::ToolError` — existing error enum pattern
- `river-orchestrator::SupervisorError` — existing thiserror usage

### Established Patterns
- Result-based error propagation already used in most crates
- `anyhow::Result` for application-level errors

### Integration Points
- Workers catch errors from context assembly
- Adapters catch errors from protocol parsing
- Orchestrator handles errors from all spawned processes

</code_context>

<specifics>
## Specific Ideas

No specific requirements — follow existing patterns in the codebase.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 01-error-handling-foundation*
*Context gathered: 2026-04-06*
