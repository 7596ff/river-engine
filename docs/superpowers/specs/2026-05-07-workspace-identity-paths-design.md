# Workspace Identity File Paths

## Goal

Move agent identity files from `{workspace}/actor/` to the workspace root. Require all three files to exist at startup.

## Changes

### Agent identity files

The gateway reads `AGENTS.md`, `IDENTITY.md`, and `RULES.md` from the workspace root instead of `{workspace}/actor/`.

All three are required. If any file is missing, the gateway logs an error naming the missing file(s) and exits. No silent fallback to a generic prompt.

### Spectator files

No change. The spectator reads from `{workspace}/spectator/` as before:
- `identity.md` — required (already enforced)
- `on-turn-complete.md` — optional
- `on-compress.md` — optional
- `on-pressure.md` — optional

## Code changes

Two functions in `crates/river-gateway/src/agent/task.rs`:

- `build_system_prompt` (async) — change `workspace.join("actor").join(filename)` to `workspace.join(filename)`. Add validation that all three files exist before reading. Return an error if any are missing.
- `build_system_prompt_sync` (sync) — same changes.

Both functions currently return `String`. They need to return `Result<String>` (or the caller needs to handle the missing-file case) so the gateway can exit cleanly instead of falling back to a generic prompt.

## Tests

Update `test_build_system_prompt_default` and `test_build_system_prompt_with_identity` in `agent/task.rs` to write files at the workspace root instead of `actor/`. Add a test that verifies the gateway fails when a required file is missing.
