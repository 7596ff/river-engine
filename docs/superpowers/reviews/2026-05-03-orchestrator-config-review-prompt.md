# Review Prompt: Orchestrator Config & Process Management Spec

Review the spec at `docs/superpowers/specs/2026-05-03-orchestrator-config-design.md`.

## Context

This is a Rust workspace (`river-engine`) with multiple crates. The orchestrator (`river-orchestrator`) currently accepts only CLI args. This spec adds a JSON config file that describes the full system topology — models, agents, adapters — so the orchestrator can spawn and manage all processes from a single command.

Key crates:
- `river-orchestrator` — model resource management, agent heartbeats, will gain config loading and process spawning
- `river-gateway` — agent runtime (wake/think/act/settle loop, tools, HTTP API)
- `river-discord` — Discord adapter, connects to gateway via HTTP

The orchestrator does NOT proxy model requests. It ensures models are available and tells agents where to find them. Agents talk directly to model endpoints.

## Existing Code to Check Against

- `crates/river-orchestrator/src/main.rs` — current CLI args (should remain as fallback)
- `crates/river-orchestrator/src/config.rs` — existing OrchestratorConfig struct
- `crates/river-gateway/src/main.rs` — gateway CLI args (what the orchestrator needs to generate)
- `crates/river-gateway/src/agent/context.rs` — ContextConfig struct (context window shape params)
- `crates/river-discord/src/main.rs` — discord adapter CLI args
- `deploy/river-orchestrator.service` — systemd unit already expects `--config`
- `deploy/river.example.json` — old config format (stale, being replaced by this spec)

## Review Criteria

1. **Config completeness** — does the JSON schema capture everything the gateway and discord adapter need? Compare every CLI arg in `river-gateway` and `river-discord` main.rs against the config fields. Flag anything the config can't express.

2. **Config ergonomics** — are the defaults reasonable? Are there fields that should be optional but are marked required, or vice versa? Would a user find this config intuitive?

3. **Env var expansion** — the spec says expand `$VAR` on the raw string before JSON parsing. What are the edge cases? What if a value legitimately contains `$`? What about `${VAR}` syntax? What about undefined optional vars vs undefined required vars?

4. **Process management** — is the startup sequence correct? Are there race conditions (e.g., gateway starts before its model is loaded for GGUF)? Is the shutdown sequence safe?

5. **Error handling** — are all failure modes covered? What happens if a port is already in use? What if the discord adapter can't connect? What if a GGUF model path doesn't exist?

6. **Security** — API keys appear in the config (via env var expansion). Are there concerns about the expanded config being logged, appearing in /proc/cmdline, or being readable by other processes?

7. **Missing pieces** — the spec says context shape params need new gateway CLI args. Are there other gateway features not expressible via CLI that the config would need to cover? Check `ServerConfig` in `server.rs`.

8. **Compatibility** — the spec says existing CLI args remain as fallback. Is there a clear precedence story? What if someone passes both `--config` and `--port`?

## What to produce

A structured review with findings organized by severity (critical, important, suggestion). Flag any config fields that don't map to existing CLI args. Note any race conditions in process management.
