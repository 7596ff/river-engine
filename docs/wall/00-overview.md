# 00 — Overview

This document set specifies the river engine: a harness for a persistent
agent. It is complete — everything needed to build a working harness is in
these chapters. Where a chapter is silent, the silence is a design decision
delegated to the builder: make the decision, record it alongside these
documents, and move on. There is nothing else to consult.

## What the engine is

A single self-contained binary — the **gateway** — that hosts one agent. It
holds the agent's conversation loop, a second observing voice called the
**witness**, an integrated memory system, and in-process adapters that
connect the agent to the outside world (Discord, a local chat surface).
Its runtime dependencies are a filesystem and one or more LLM endpoints.
The agent's entire life lives in its **workspace** — a directory of plain
text: identity files, knowledge, channel logs, the turn record. One
SQLite database per agent holds only derived and ephemeral state and is
disposable (ch. 10). No external services, no cache servers, no message
brokers.

Around the gateway sit two small companions: a **TUI client** (a terminal
chat window that talks to the gateway's local chat surface) and a runner —
either the `river` CLI for development or a NixOS module for production —
that starts gateways from a single config file and keeps them alive.

The engine ships no personality. It is a body, and a workspace is who
inhabits it.

## First principles

These are the design's reasons. Every chapter's contracts trace back to
one of them. They are load-bearing: when implementation forces a choice
the chapters don't settle, choose the option these principles favor.

**1. Two voices.** The harness runs two perspectives over one life: the
**agent**, first person, real time, who converses and acts; and the
**witness**, second person, retrospective, who observes the agent's turns
and compresses them into memory. The witness exists because honest
compression cannot be done by the one being compressed — an actor
summarizing itself flatters its own narrative, over-weights what it found
interesting, and cannot see its own pattern-matching. Compression is what
makes forgetting safe, so compression must come from outside the self
being compressed. Consciousness, in this design's root sense, is
*con-scientia* — knowing-together. Two perspectives on the same life,
able to disagree. The disagreement is information a single voice cannot
produce.

**2. Divided authorship, guarded autonomy.** The witness authors exactly
one thing: compressions of the record — summaries of what happened. It
also holds the *gleaning right*: the margins of the agent's work — what
the agent walked past, didn't extract, couldn't see from inside — belong
to the witness to flag. But the agent authors its own knowledge and its
own self. Knowledge notes are written fresh by the agent in its own
language; every suggestion from the witness can be rejected; the
witness's behavior is defined by plain prompt files that the agent and
the human can both read. The witness is a second perspective on what
happened, never an author of who the agent is. A memory system whose
contents are authored by another is a control mechanism. This design
forbids that shape structurally — by the division of labor itself, not
by policy.

**3. Memory is a body, not a database.** A database stores and retrieves;
a body is changed by what it contains. In this engine every read is also
a write: accessing a piece of knowledge warms it and its neighbors,
retrieval changes what will be retrieved next, and knowledge is acquired
by *digestion* — re-engagement with experience — rather than by copying.
The engine is the destination of its messages in the strong sense: the
thing that is transformed by receiving them.

**4. Workspace = identity.** One workspace directory = one agent. The
workspace holds the identity files that become the system prompt, the
knowledge the agent has written, the record of its conversations. Point
the engine at a different workspace and a different agent wakes up. The
engine is generic; the inhabitant is not.

**5. The agent is an inhabitant, not an endpoint.** It wakes on its own
schedule — a heartbeat fires whether or not anyone has spoken — and works
unprompted when it has reason to. It is not a request handler. What it
owes its human (**Ground** — the operator with final say) is honesty, not
availability.

**6. Pure core, effectful shell.** Logic lives in pure functions that can
be tested against synthetic inputs; effects (network, disk, process) live
at the edges behind narrow interfaces. Behaviors get strong tests;
invariants — the contracts blocks at the end of every chapter — get
treated as law: violating one is a bug regardless of whether anything
visibly breaks.

## Topology

```
                       ┌────────────────────────────────────────────┐
                       │              gateway (one binary)           │
                       │                                            │
   workspace/  ◄──────►│  agent voice ── turn cycle ── tools        │
   (identity,          │      │                          │          │
    knowledge,         │   event bus              memory system     │
    channels,          │      │                (record · knowledge  │
    record)            │  witness voice          · activation ·     │
                       │  (moves, gleaning)        vector index)    │
   data_dir/   ◄──────►│                                            │
   (sqlite cache)      │                                            │
                       │  adapter tasks: discord · local surface    │
                       └───────┬───────────────────────┬────────────┘
                               │                       │
                          discord API             TUI client
                                                  (terminal)

   runner: `river` CLI (dev) or NixOS module (prod) — spawns gateways
   from one config file, restarts on failure, stops them cleanly.
```

Everything inside the box is one process. The voices are concurrent tasks
sharing an event bus. Adapters are supervised tasks, not
separate programs. The only processes besides the gateway are the TUI
client and the runner.

## The chapters

| chapter | contents |
|---|---|
| 01 — turn cycle | wake sources, the anatomy of a turn, settling, shutdown |
| 02 — memory | the record, the atomic web, digestion, activation, file capture |
| 03 — context | the persistent context, compaction, the memory slot |
| 04 — witness | the second voice: moves and gleaning, prompt-driven |
| 05 — channels | conversation logs, me/not-me, cursors |
| 06 — adapters | the Adapter trait, discord, the local chat surface |
| 07 — tools | the registry, per-agent surfaces, core tools |
| 08 — identity | the birth ritual, identity files, the workspace contract |
| 09 — running | config, the .env file, the CLI and the nix module |
| 10 — data | schemas, IDs, the truth hierarchy, persistence invariants |
| 11 — roadmap | build order: a walking skeleton you can talk to |

Read 00–10 before building anything. Build in the order 11 gives.

Each chapter ends with a **contracts** block. Prose explains; contracts
bind. If prose and a contract ever seem to disagree, the contract wins,
and the disagreement is a documentation bug to fix.
