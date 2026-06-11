# Decisions

Design decisions made where the wall is silent, per the clean-room rule
in CLAUDE.md. Newest at the bottom.

## 2026-06-11 — crate layout

`crates/` workspace, growing as cards land:

- `river-gateway` (bin) — the engine. Pure logic lives in plain modules
  with unit tests (the pure-core/effectful-shell principle is enforced
  by module boundaries, not crate boundaries, until sharing forces a
  split).
- `river-tui` (bin) — the TUI client, when its card lands.
- A shared lib crate appears only when the two binaries need common
  types (likely the websocket protocol); not before.

## 2026-06-11 — gateway CLI shape

The wall specifies the `river` CLI and the birth subcommand but not the
gateway binary's own arguments. Decision: the gateway reads the same
shared config file and extracts its own slice —

```
river-gateway run --config river.json --agent ada [--env-file river.env]
river-gateway birth --workspace <dir> --name <name> [--seed]
```

One config format everywhere; the runner passes `--agent` per spawned
gateway.

## 2026-06-11 — config details the wall delegates

- `$VAR` syntax: `$NAME` where NAME is `[A-Za-z0-9_]+`; `$$` escapes a
  literal dollar; a `$` not followed by a name character passes through.
- .env format: `KEY=value` lines, `#` comments, blank lines skipped,
  one matching pair of surrounding quotes stripped; malformed lines are
  fatal (all reported together) — a secrets file is no place to guess.
- Unknown config fields are rejected (`deny_unknown_fields`): a typoed
  knob should fail loudly, not silently do nothing.
- Omitted `tools` = the eight core tools; omitted `witness_model` = the
  agent's model (per wall ch. 09); `heartbeat_minutes` default 45.

## 2026-06-11 — identity details the wall delegates

- The "workspace-configured timezone" (chs. 03, 08) is an optional
  `timezone` field (IANA name) on the agent's config entry, defaulting
  to the system timezone. The config is the workspace's operational
  description; a dotfile inside the workspace would be engine state
  leaking into the agent's body.
- System prompt separator: each identity file trimmed and joined with
  `\n\n---\n\n`, then `Current time: <zoned timestamp>`.
- Missing identity files are collected and reported together, matching
  the validate-everything-report-together posture of ch. 09.

## 2026-06-11 — dependency policy

Workspace-level dependency table. tokio with `full` features (this is a
binary harness, not a library; compile-time over feature-pruning).
`anyhow` for binary error paths, `thiserror` for typed errors in pure
modules. clap derive. Edition 2024.
