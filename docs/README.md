# River Engine Documentation

## Overview

- **[roadmap.md](roadmap.md)** - Project roadmap and status (source of truth)
- **[DESIGN-PHILOSOPHY.md](DESIGN-PHILOSOPHY.md)** - Design principles and philosophy
- **[MIGRATION-GUIDE.md](MIGRATION-GUIDE.md)** - Guide for migrating agents into River Engine
- **[snowflake-generation.md](snowflake-generation.md)** - Snowflake ID system reference

## Specifications

Design specifications in `superpowers/specs/`:

| Spec | Description |
|------|-------------|
| [river-engine-design](superpowers/specs/2026-03-16-river-engine-design.md) | Core system design |
| [orchestrator-minimal-design](superpowers/specs/2026-03-16-orchestrator-minimal-design.md) | Basic orchestrator |
| [orchestrator-advanced-design](superpowers/specs/2026-03-16-orchestrator-advanced-design.md) | Advanced orchestrator features |
| [discord-adapter-design](superpowers/specs/2026-03-16-discord-adapter-design.md) | Discord integration |
| [nixos-module-design](superpowers/specs/2026-03-16-nixos-module-design.md) | NixOS deployment |
| [gateway-loop-design](superpowers/specs/2026-03-17-gateway-loop-design.md) | Gateway agent loop |

## Implementation Plans

Implementation plans in `superpowers/plans/`:

| Plan | Description |
|------|-------------|
| [plan-01-core-libraries](superpowers/plans/2026-03-16-plan-01-core-libraries.md) | river-core implementation |
| [plan-02-gateway-core](superpowers/plans/2026-03-16-plan-02-gateway-core.md) | Gateway foundation |
| [plan-03-memory-embeddings](superpowers/plans/2026-03-16-plan-03-memory-embeddings.md) | Memory system |
| [plan-4-orchestrator](superpowers/plans/2026-03-16-plan-4-orchestrator.md) | Basic orchestrator |
| [plan-5-advanced-orchestrator](superpowers/plans/2026-03-16-plan-5-advanced-orchestrator.md) | Advanced orchestrator |
| [plan-6-discord-adapter](superpowers/plans/2026-03-16-plan-6-discord-adapter.md) | Discord adapter |
| [plan-7-nixos-module](superpowers/plans/2026-03-16-plan-7-nixos-module.md) | NixOS module |
| [gateway-loop](superpowers/plans/2026-03-17-gateway-loop.md) | Agent loop implementation |

## Status

- **[roadmap.md](roadmap.md)** - Feature roadmap and status (source of truth)
- **[STATUS.md](superpowers/STATUS.md)** - Implementation details and test counts

## Quick Links

### Crate Documentation

```bash
# Generate and open rustdoc
cargo doc --workspace --open
```

### Key Source Files

| File | Description |
|------|-------------|
| `crates/river-core/src/lib.rs` | Core types |
| `crates/river-gateway/src/loop/mod.rs` | Agent loop |
| `crates/river-gateway/src/tools/mod.rs` | Tool system |
| `crates/river-gateway/src/subagent/mod.rs` | Subagent system |
| `crates/river-orchestrator/src/main.rs` | Orchestrator |
| `crates/river-discord/src/main.rs` | Discord adapter |
