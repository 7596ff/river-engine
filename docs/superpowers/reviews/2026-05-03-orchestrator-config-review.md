# Review: Orchestrator Config Implementation Plan

Review of `docs/superpowers/plans/2026-05-03-orchestrator-config.md` against `docs/superpowers/specs/2026-05-03-orchestrator-config-design.md`.

## Summary

The plan is comprehensive and covers all major requirements of the spec. It correctly identifies the need for new CLI args in the gateway and provides a solid supervisor implementation. However, there are some gaps in the "Task 6" integration logic, particularly around how the existing `OrchestratorState` is initialized and how GGUF models are resolved.

## Critical Findings

### 1. OrchestratorState Initialization (Task 6)
**Severity: Critical**
**Description:** Task 6 is vague about how `OrchestratorState` is initialized in `--config` mode. The orchestrator's current `main.rs` populates state by scanning `model_dirs` and reading an `external_models` JSON file.
**Issue:** In `--config` mode, the `models` defined in the JSON file should be the **sole source of truth**. 
**Correction:** 
- For `provider: "gguf"` models, the plan must explicitly state that the orchestrator should parse the GGUF metadata from the provided `path` (using `river_orchestrator::discovery::gguf::parse_gguf`) to create `LocalModel` entries for the state.
- For other providers, it should create `ExternalModel` entries.
- The `OrchestratorConfig` struct used to initialize the state should be derived from `RiverConfig`.

### 2. GGUF Model Resolution & Waiting (Task 6, Step 2)
**Severity: Critical**
**Description:** The plan stubs GGUF resolution with `// ... model loading via orchestrator state ...`.
**Issue:** The spec requires the orchestrator to wait for the model to be healthy before spawning gateways.
**Correction:** The implementation MUST use `state.request_model(model_id, 120).await`. This function already handles spawning `llama-server` and polling for health. The `ResolvedModel` for the gateway's `--model-url` should be populated using the endpoint returned by `request_model`.

---

## Important Findings

### 3. Missing `OrchestratorConfig` Fields in `RiverConfig` (Task 1 & 6)
**Severity: Important**
**Description:** `RiverConfig` (Task 1) is missing some fields that `OrchestratorConfig` (and `OrchestratorState`) currently requires, such as `health_threshold_seconds` and `idle_timeout_seconds`.
**Correction:** Either add these to `RiverConfig` or ensure they are explicitly set to defaults when building the `OrchestratorConfig` in Task 6.

### 4. Discord `guild_id` Type Mismatch (Task 1 & Task 4)
**Severity: Important**
**Description:** `river-discord` expects `--guild-id` to be a `u64`. `RiverConfig` (Task 1) defines it as `Option<String>`.
**Observation:** The use of `String` is correct to support `$VAR` expansion. However, Task 4 (`cli_builder.rs`) should probably ensure it's a valid number if possible, or at least acknowledge that `river-discord`'s clap parser will handle the string-to-u64 conversion.
**Action:** Ensure the example config uses a string even for numeric IDs to maintain consistency with expansion.

### 5. Port Range Parsing Logic (Task 1 & 6)
**Severity: Important**
**Description:** `RiverConfig` uses a string `port_range`. The existing `main.rs` has a `parse_port_range` helper.
**Correction:** Task 6 should use the existing helper or `ResourcesConfig` should store `port_range_start` and `port_range_end` directly to match `OrchestratorConfig`.

---

## Suggestions

### 6. Gateway Argument Defaults (Task 7)
**Severity: Suggestion**
**Description:** The plan adds 3 new args to the gateway.
**Suggestion:** Ensure these args have the same defaults in the gateway's `Args` struct as defined in the orchestrator's `config_file.rs` (0.80, 0.40, 20) to maintain consistency when running the gateway manually. (Note: Step 1 already does this, which is good).

### 7. Supervisor Log Target (Task 5)
**Severity: Suggestion**
**Description:** Task 5 uses `tracing::info!(target: "child", ...)`.
**Suggestion:** Ensure the orchestrator's subscriber is configured to handle the `child` target if special formatting is desired, or just use the default target for simplicity if it's going to the same log.

### 8. Signal Handling Grace Period (Task 6)
**Severity: Suggestion**
**Description:** The spec mentions a 10s grace period and SIGKILL. 
**Correction:** The `supervisor.rs` `supervise` function uses `child.kill().await` on shutdown, which sends SIGKILL immediately on Linux. To support graceful shutdown as per spec, it should send SIGTERM first, wait, then kill. However, `tokio::process::Child::kill` is SIGKILL. Using `nix` crate or `libc` to send SIGTERM might be needed if grace is required.

## Conclusion

The plan is strong but requires more "surgical" detail in Task 6 to bridge the new config types with the existing state management system. Specifically, the "source of truth" for models must be shifted to the config file when `--config` is provided.
