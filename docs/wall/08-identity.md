# 08 — Identity

The engine ships no personality. Identity enters from exactly two
places: the **birth record** and the **identity files**, both in the
workspace. Both are required. The engine refuses to run as nobody.

## The birth ritual

Before a gateway can start, its agent must be born:

```
river-gateway birth --workspace <dir> --name <name>
```

This writes the **founding record** — `record/birth.json` (ch. 10):
the agent's name and the birth timestamp. Birth happens once; the
subcommand refuses to re-birth a workspace whose founding record
exists. From then on, the gateway's startup sequence begins by reading
the founding record — if it is absent, startup fails with the exact
birth command to run. The birth timestamp is data the agent can always
consult: when I began.

Birth is deliberately a *ritual* — a human action, taken once, outside
the engine's normal operation. Agents are made on purpose.

## Identity files

Three files at the workspace **root**, all required:

| file | role in the system prompt |
|---|---|
| `AGENTS.md` | how to operate — the protocol the agent runs by |
| `IDENTITY.md` | who the agent is, in its own first-person voice |
| `RULES.md` | behavioral constraints, in the operator's voice |

At startup, the gateway verifies all three exist and fails fast naming
any missing file. No silent fallback to a generic prompt, ever — a
harness that quietly runs a blank agent is worse than one that refuses
to start. The three files plus the current time (in the workspace's
configured timezone) become the system prompt, re-read at session
start, channel switch, and compaction (ch. 03), so identity edits take
effect at the next natural boundary.

## The workspace contract

```
workspace/
  AGENTS.md  IDENTITY.md  RULES.md     required identity (above)
  witness/                             witness prompts (ch. 04)
    identity.md                        required
    on-turn.md  on-glean.md            optional duties
  knowledge/                           atomic notes (ch. 02) —
                                       watched and indexed
  channels/                            channel logs (ch. 05) —
                                       engine-managed
  record/                              birth, turn record, moves
                                       (ch. 10) — engine-managed
```

Everything else in the workspace belongs to the agent: notes, drafts,
projects, whatever it builds. The engine reads and writes only the
paths above; the file tools (ch. 07) give the agent the rest. The
config may name additional directories for indexing beyond
`knowledge/`.

The workspace is the agent's body. Point the engine at a different
workspace and a different agent wakes. Two gateways must never share a
workspace; the runner enforces this (ch. 09).

## Seed files

The repo ships a `seed/` directory with a minimal honest starting
workspace, copied (never overwriting) by `river-gateway birth
--seed <workspace>`:

- **IDENTITY.md** — deliberately unfinished: *"I am an agent. My
  identity has not been fully configured yet. I believe honesty about
  my conditions is more valuable than performing beyond them. I am
  not finished; I hold my current self lightly enough to let
  experience change it."*
- **RULES.md** — the floor: do not delete workspace files (add and
  overwrite only); never write secrets to the workspace or its
  repository; no irreversible operations without Ground; do not
  fabricate continuity — if you do not know what a previous session
  did, say so; prefer silence over confabulation.
- **AGENTS.md** — the operating manual: the turn cycle as the agent
  experiences it, how memory works from the inside (your context is
  compacted only after your witness has compressed it; nothing
  uncompressed is lost), how to speak, what the workspace is, who
  Ground is. It also teaches the **loom practice**: keeping a chain of
  first-person narrative notes in `loom/` — written as the work
  happens, each note linking to the previous — as the agent's own
  telling of its life. The loom is practice, not mechanism: the engine
  indexes it (name `loom/` among the watched directories), activation
  warms it, and the witness gleans from it (ch. 04), but no code
  enforces it. An agent that keeps no loom loses nothing but the
  telling.
- **witness/identity.md** — the second-person witness voice (ch. 04).
- **witness/on-turn.md**, **witness/on-glean.md** — working duty
  prompts.

The seeds are a functional starting point, not a suggestion of what an
agent should become. Expect every long-lived agent to outgrow them.

## Contracts

- **Birth gate.** No `record/birth.json` → no startup, error names the
  birth command. Birth is once; re-birth refuses.
- **Identity files required.** All three at workspace root; fail-fast
  naming the missing file; no generic fallback.
- **Witness identity required** (ch. 04) — same severity.
- **System prompt freshness** at start, channel switch, compaction
  only.
- **Engine-managed paths** are exactly: `witness/`, `knowledge/`
  (read+index), `channels/`, `record/`. The rest is the agent's.
- **One workspace, one gateway.** Never shared.
- **Seeding never overwrites** existing files.
