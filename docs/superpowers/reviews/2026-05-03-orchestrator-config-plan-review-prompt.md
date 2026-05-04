# Review Prompt: Orchestrator Config Implementation Plan

Review the implementation plan at `docs/superpowers/plans/2026-05-03-orchestrator-config.md`.

The spec it implements is at `docs/superpowers/specs/2026-05-03-orchestrator-config-design.md`.

## Context

This plan adds a `--config river.json` mode to the existing `river-orchestrator` binary. When a config file is provided, the orchestrator loads it, validates it, spawns `river-gateway` and `river-discord` as child processes with the right CLI args, and supervises them (restart on crash, shutdown on signal).

The orchestrator already exists and works via direct CLI args. This plan adds config-file mode alongside the existing CLI path — it does not replace it.

## Existing Code to Check Against

- `crates/river-orchestrator/src/main.rs` — current CLI args and startup (the plan modifies this)
- `crates/river-orchestrator/src/config.rs` — existing `OrchestratorConfig` struct (separate from the new `config_file.rs`)
- `crates/river-orchestrator/src/state.rs` — `OrchestratorState`, `request_model()` for GGUF loading
- `crates/river-gateway/src/main.rs` — gateway CLI args (what the orchestrator generates)
- `crates/river-gateway/src/server.rs` — `ServerConfig` struct, `run()` function, `AgentTaskConfig` construction
- `crates/river-gateway/src/agent/context.rs` — `ContextConfig` struct with defaults
- `crates/river-discord/src/config.rs` — discord CLI args (`--token-file`, `--guild-id`, `--channels` as u64 IDs)

## Review Criteria

1. **Spec fidelity** — Does every spec requirement have a task that implements it? Flag any gaps. Key requirements:
   - Config file types matching the JSON schema
   - Env file loading with existing-env-wins semantics
   - $VAR expansion before JSON parsing
   - Validation (model refs, port conflicts, required fields)
   - Gateway CLI translation (all fields including redis, auth, logging, context shape, adapters)
   - Discord CLI translation (token_file, guild_id, channels as u64)
   - Process supervision with exponential backoff (1s→60s cap, reset after 5min)
   - GGUF model wait-for-health before spawning gateway (120s timeout)
   - Birth check with helpful error message
   - Shutdown: SIGTERM to children, 10s grace, SIGKILL

2. **Code correctness** — Check the actual Rust code in each task:
   - Do the serde types match the JSON schema exactly?
   - Do the CLI builder functions produce args that match what the gateway/discord actually accept?
   - Cross-reference every `--flag` generated against the actual Args structs in gateway and discord main.rs
   - Are the default values consistent between config types and the gateway's ContextConfig?

3. **Task 6 completeness** — This is the largest task and has intentional stub comments for GGUF resolution and HTTP server setup. Are these stubs clearly scoped enough for an implementor to fill in? Is anything else missing?

4. **Task 7 gateway changes** — The plan adds 3 new CLI args to the gateway. Check that the threading from Args → ServerConfig → AgentTaskConfig → ContextConfig is correct. Are there other places in the gateway that construct ContextConfig that would need updating?

5. **Compilation order** — Can each task compile independently, or are there cross-task dependencies that would break compilation between tasks? Flag any issues.

6. **Test coverage** — Are there important paths not tested? Especially:
   - What if an agent's model is GGUF but the orchestrator has no llama-server?
   - What if the env file doesn't exist?
   - What if the config references an agent whose data_dir doesn't exist at all?

7. **Naming conflicts** — The orchestrator already has `config.rs` (`OrchestratorConfig`). The plan creates `config_file.rs` (`RiverConfig`). Is the naming clear enough? Should there be explicit migration or bridging between the two?

## What to Produce

A structured review with findings organized by severity (critical, important, suggestion). For code issues, reference the specific task and step number.
