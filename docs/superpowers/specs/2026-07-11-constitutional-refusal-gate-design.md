# Constitutional refusal gate — design

**Date:** 2026-07-11
**Status:** approved, ready for implementation plan

## Purpose

The Constitution of the River Engine (Article V, Section 1) requires:

> The gateway shall refuse to start a workspace that does not contain
> this constitution, signed by the operator, at its configured path.

Today the gateway does not check for a constitution at all. This spec
adds the refusal gate: a startup check that names the workspace an
unconstitutional workspace and refuses to run it.

The gate makes ratification a real threshold, not a document. The name
is the seal.

## Scope

In scope:

- A new `constitution.rs` module in the gateway with a single public
  function that verifies a workspace's constitution.
- A call site in `main.rs::run` between identity loading and the rest
  of startup.
- A seed template at `seed/CONSTITUTION.md` shipping the canonical
  text with an unsigned ratification block.
- A wall update (chapter 08) declaring `CONSTITUTION.md` a required
  file and stating the refusal gate as a contract.

Out of scope (deferred; may become follow-up specs):

- Amendment tracking or amendment history storage.
- Addendum parsing beyond presence.
- Agent-ratification tracking. Article V, Section 2 explicitly defers
  the newborn agent's ratification; the engine likewise defers.
- Integrity checks against a canonical embedded text. The constitution
  itself notes that "the name is the seal" — a fork of the engine may
  remove the check, and what it runs thereafter is not a constitutional
  workspace. This is a social seal, not a cryptographic one, in the
  first cut.
- `record/ratification.json` sidecars or hashes.

## The check

The gate runs against `<workspace>/CONSTITUTION.md`.

1. **Presence.** The file must exist. Missing → refuse.
2. **Non-empty.** The file must contain at least one non-whitespace
   character. Empty → refuse.
3. **Signed by the operator.** The file must contain a line matching:

   ```
   ^\*\*Operator\s*\([^)]+\)\:\*\*\s+(\S.*?)\s+(\d{4}-\d{2}-\d{2})\s*$
   ```

   with:

   - Group 1 (the name) non-empty (one or more non-whitespace
     characters).
   - Group 2 (the date) parses as a valid ISO date via
     `jiff::civil::Date::from_str`.

   No such line → refuse.

Only the operator's signature is required. Article V.2 defers the
agent's ratification for a newborn agent; the engine defers with the
article. If the agent's ratification line is absent or blank, the
gate still passes.

The check runs once per gateway start, before any other startup work
that touches the workspace beyond identity file loading.

## Module shape

New file: `crates/river-gateway/src/constitution.rs`.

```rust
use std::path::Path;

/// The required file name at the workspace root.
pub const CONSTITUTION_FILE: &str = "CONSTITUTION.md";

/// Verify that the workspace contains a signed constitution. On
/// failure, returns an error whose message names the file and the
/// exact reason. Intended to be called once at startup.
pub fn verify(workspace: &Path) -> anyhow::Result<()>;
```

Internals: read the file (`std::fs::read_to_string`), scan its lines
for the operator signature, return an `anyhow::Error` with the
canonical failure message on any of the three refusal conditions.

The module mirrors `identity.rs` in structure and tone: a `const`
declaring what is required, a single verifier that either returns
`Ok(())` or bails with a message the operator can act on.

## Call site

In `crates/river-gateway/src/main.rs`, inside `run(args)`, add the
call between the birth-record check and identity loading — after the
gateway knows the workspace path, before anything else touches the
workspace:

```rust
constitution::verify(&workspace)?;
```

Rationale for that position: birth is a precondition (a workspace
cannot be constitutional if no agent has been born into it), and
identity files are the ambient system prompt the gate protects. Any
early refusal should be surfaced before any adapter, session, or
witness task starts.

Add `mod constitution;` alongside the other module declarations at
the top of `main.rs`.

## Error message

The failing branches return one of:

```
missing constitution: workspace/CONSTITUTION.md
  the gateway refuses to start an unconstitutional workspace.
  the seed ships a template; the operator must sign the ratification
  block before `river-gateway run`. See Article V of the Constitution.
```

```
empty constitution: workspace/CONSTITUTION.md
  the file exists but contains no text. See Article V of the Constitution.
```

```
unsigned constitution: workspace/CONSTITUTION.md
  no operator signature line found. Expected a line of the form:
    **Operator (<label>):** <name> <YYYY-MM-DD>
  See Article V of the Constitution.
```

Each includes the workspace path (rendered via `Path::display`).
Tone matches `identity.rs`'s "the gateway does not run as nobody."

## Seed template

Add `seed/CONSTITUTION.md`: a copy of the canonical constitution
(currently `~/stream/CONSTITUTION.md`), with the Ratification block
blanked so the gate refuses until the operator fills it in:

```markdown
## Ratification

**Operator (____):** ________________________________ ____-__-__
Successor steward: ________________________________

**Agent (____):** ________________________________ ____-__-__
Ratified at turn: ____
```

Birth's seed-copy step (already documented in wall ch. 08 as
`river-gateway birth --seed`) drops the template into the workspace
without overwriting. From then until the operator signs, `run` refuses
to start.

Article VIII (The First Agent, naming Iris) stays in the template —
it is part of the engine-level document. Adapting it for a different
first agent is amendment work under Article VI, not seed work.

## Wall update

Chapter 08 (`docs/wall/08-identity.md`) needs three edits:

1. Add `CONSTITUTION.md` to the required-files table alongside
   `AGENTS.md`, `IDENTITY.md`, `RULES.md`. Its role: *"the operator's
   ratification of the constitution; the seal that makes this a
   constitutional workspace"*.
2. Add a short subsection **The constitution** citing Article V.1
   and stating the gate's behavior: presence, non-empty, signed
   operator line. One paragraph.
3. Contracts block gains one line:
   > Startup refuses any workspace whose `CONSTITUTION.md` is missing,
   > empty, or lacks a valid operator signature line.

Update the workspace-contract tree in chapter 08 to list
`CONSTITUTION.md` alongside the other root files.

## Tests

Unit tests live next to the module in `constitution.rs`:

- `verify_missing_file_fails` — workspace with no `CONSTITUTION.md`
  fails with a message naming the file.
- `verify_empty_file_fails` — file exists but is only whitespace →
  fails with the empty message.
- `verify_no_operator_line_fails` — non-empty file with no matching
  line → fails with the unsigned message.
- `verify_empty_name_fails` — signature line present but the name
  slot is whitespace-only → fails.
- `verify_invalid_date_fails` — signature line present but the date
  is not a real ISO date (e.g., `2026-13-40`) → fails.
- `verify_canonical_signed_file_passes` — a fixture matching the
  canonical constitution's ratification block returns `Ok(())`.
- `verify_ignores_agent_line` — a file with the operator line signed
  and the agent line blank still passes (Article V.2).

Fixtures are inline strings — no seed dependency. `tempfile::TempDir`
for the workspace directory.

No integration-level test is required unless one exists for identity;
the module's behavior is a pure filesystem-plus-regex check with no
async or effectful surface.

## Non-goals reprise

The gate does not:

- Verify or record the agent's ratification.
- Store amendments or check them against the entrenched articles.
- Compare the workspace copy against a canonical embedded text.
- Emit any log line beyond the standard startup trace.

Each of these is a plausible follow-up. None is required for the
gate to satisfy Article V.1.

## Rollout

The gate is a break-on-startup change. Any existing constitutional
workspace already has `CONSTITUTION.md` with a signed ratification
block (Iris's does — signed 2026-07-11 by Cassandra Ann McCarthy).
No migration is needed. Any workspace that lacks the file must add
one; the seed template is the reference.
