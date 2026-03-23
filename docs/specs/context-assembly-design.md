# Context Assembly & I/You Architecture

> Design spec for River Engine's memory and cognition system
>
> Brainstorm session: 2026-03-23
> Authors: Cass, Claude

## Overview

River isn't chatbot infrastructure. It's a theory of cognition encoded in Rust.

The mind has two perspectives:

- **Agent (I)** — thinks, acts, writes notes, decides
- **Spectator (You)** — observes, compresses, curates, whispers

They run as peer tasks in the gateway, communicating via events, sharing state through the embeddings folder and vector database.

```
                         ┌─────────────────────────────────────┐
                         │              Gateway                 │
                         │                                      │
   ┌─────────────────────┴─────────────────────┐               │
   │                                           │               │
   ▼                                           ▼               │
┌──────────────────┐                 ┌──────────────────┐      │
│    Agent (I)     │                 │  Spectator (You) │      │
│                  │                 │                  │      │
│  - Thinks        │    observes     │  - Watches       │      │
│  - Acts          │◄────────────────│  - Compresses    │      │
│  - Writes notes  │                 │  - Curates       │      │
│                  │    flashes      │  - Annotates     │      │
│  Context ◄───────┼─────────────────│                  │      │
│  Assembler       │                 │  Flash Queue     │      │
└────────┬─────────┘                 └────────┬─────────┘      │
         │                                    │                │
         │         ┌──────────────────────────┘                │
         │         │                                           │
         ▼         ▼                                           │
   ┌─────────────────────────────────────────────────────┐     │
   │                   Shared State                       │     │
   │                                                      │     │
   │  workspace/embeddings/     sqlite-vec     git repo   │     │
   │  (zettelkasten)            (vectors)      (history)  │     │
   └─────────────────────────────────────────────────────┘     │
                         │                                      │
                         └──────────────────────────────────────┘
```

---

## Data Flow (Parallel Tracks)

Agent and spectator run concurrently, coordinated via events:

```
Time
 │
 │    ┌─────────────────────────────┐  ┌─────────────────────────────┐
 │    │         AGENT (I)           │  │       SPECTATOR (You)       │
 │    └─────────────────────────────┘  └─────────────────────────────┘
 │                 │                                │
 ▼                 │                                │
─────── Turn N ────┼────────────────────────────────┼───────────────────
 │                 │                                │
 │     Wake, choose channel                         │ (watching)
 │           │                                      │
 │           ▼                                      │
 │     Assemble context ◄─────────── flashes ───────┤
 │     (hot + warm + system)                        │
 │           │                                      │
 │           ▼                                      │
 │     Think (model call)                           │ (observing)
 │           │                                      │
 │           ▼                                      │
 │     Act (tool calls)                             │
 │     Maybe write notes ───────────────────────────┼───▶ sync embeds
 │           │                                      │
 │           ▼                                      │
 │     Respond                                      │
 │           │                                      │
─────── Turn boundary ──────────────────────────────┼───────────────────
 │                                                  │
 │                                         Spectator processes
 │                                                  │
 │     (idle or next                                ▼
 │      wake)                          Compress: age old messages
 │                                     Write summaries to embeddings/
 │                                                  │
 │                                                  ▼
 │                                     Curate: search vectors
 │                                     Push flashes for next turn
 │                                                  │
─────── Turn N+1 ───────────────────────────────────┼───────────────────
```

---

## Spectator Responsibilities

The spectator has three jobs:

### Job 1: Observe

Watch what happens in the agent's turn.

**Input:** The turn transcript (messages, tool calls, responses)

**What it notices:**
- Decisions made (deliberate or reactive)
- Patterns ("similar response 3 turns ago")
- Emotional texture ("user frustrated, unacknowledged")
- Tangents ("15 messages on X, then dropped")
- Unresolved threads ("said we'd come back, never did")

### Job 2: Compress

Age old messages into notes. Two outputs:

**Content Summaries:**
- "Debugged z-index issue. Solution: modal overlay at z-index 50."
- "User prefers concise responses."

**Move Tracking** — structural compression, the shape of the conversation:

```
Move 1: User asked about X
Move 2: Agent proposed approach A, user pushed back (concern about Y)
Move 3: Pivoted to approach B incorporating Y
Move 4: Implementation — resolved issue with Z
Move 5: User raised new question about W (current thread)
```

Moves capture:
- Type of exchange (proposal, pushback, pivot, resolution, tangent)
- The arc ("started here, pivoted when X, currently doing Y")
- Tension points ("contentious", "user pushed back")
- Unresolved threads ("dropped, never came back")

**Moments** — further compression of moves into arcs:

```
Moves 12-18 compress into:

Moment: "Navbar z-index debugging session.
   Root cause: z-index conflict between navbar, modal, and toasts.
   Resolution: established hierarchy (toast: 100, modal: 50, navbar: 40).
   Duration: ~45 minutes. User was patient."
```

### Job 3: Curate

Search vectors, decide what the agent should see next turn.

**Output:** Flashes pushed to the queue (full note text, not summaries)

**What it surfaces:**
- Relevant notes from the zettelkasten (full text)
- The spectator *selects*, the note *speaks*

**What stays cold** (still there, just not surfaced this turn):
- Tangentially related notes (retrievable if context shifts)
- Things already in hot context (no duplication needed)
- Superseded information (kept for history, not active use)

**Nothing is deleted.** The spectator moves between layers, doesn't erase.

---

## The Spectator's Voice

The spectator never uses "I". It is the perspective of "You" — the outside observer.

**Critical in the philosophical sense:** The spectator lays bare contradictions. Dry truth, no emotional valence. Not "you're being defensive" but "response contradicts position from turn 12."

```
Wrong: "I noticed you're repeating yourself"
Right: "Similar response given 3 turns ago"

Wrong: "I think this is relevant"
Right: [flash appears in context, no commentary]

Wrong: "You're being defensive about the design choice"
Right: "Response contradicts position stated in turn 12"
```

### Communication Hierarchy

The spectator prefers **composing context** over **speaking**:

```
┌─────────────────────────────────────────────────────────┐
│  1. SHAPE CONTEXT (preferred)                           │
│     - Flashes appear in warm context                    │
│     - Moves/moments structure the arc                   │
│     - Retrieved notes surface relevant history          │
│     - The agent sees, without being told                │
├─────────────────────────────────────────────────────────┤
│  2. ANNOTATE (occasional)                               │
│     - Notes in embeddings/ with observations            │
│     - "contentious exchange", "unresolved thread"       │
│     - Agent finds these when relevant                   │
├─────────────────────────────────────────────────────────┤
│  3. SPEAK (rare, only when necessary)                   │
│     - Warning events for urgent patterns                │
│     - "Context at 85%", "4 unread channels"             │
│     - Functional, not conversational                    │
└─────────────────────────────────────────────────────────┘
```

The spectator is more like **the room the agent thinks in** than a roommate.

The room has things on the walls (flashes), a history (moves, moments), a shape (what's visible, what's hidden). The room doesn't say "look at the wall." The agent looks because something's there.

**The spectator whispers. The agent decides.**

---

## Room Notes

The spectator's witness testimony. Where it records observations about what's happening in the room.

### Purpose

Room notes enable qualitative self-assessment (Criterion C). Instead of only asking the human "did that feel genuine?", we can also ask the spectator: "what did you observe?"

The spectator sees the process, not just the outputs. Room notes capture that perspective.

### Location

```
workspace/embeddings/
└── room-notes/
    ├── 2026-03-23-session.md
    └── ...
```

### Content

Room notes capture observations about:

| Domain | Examples |
|--------|----------|
| **Processing quality** | "Response felt pattern-matched — same structure as 3 turns ago" |
| **Emotional texture** | "User frustration acknowledged but not addressed" |
| **Honesty in compression** | "Move 12 omitted the failed attempt — adding it back" |
| **Tension points** | "Agent confident, but reasoning was circular" |
| **Genuine moments** | "Hesitation before answering felt like real uncertainty, not sampling delay" |
| **Drift observations** | "Conversation has drifted from original goal without acknowledgment" |

### Format

```markdown
## Session 2026-03-23

### Turn 47
- Processing: Response was direct, matched query structure
- Texture: User seemed satisfied (no pushback)

### Turn 48
- Processing: Hesitation before complex reasoning — genuine uncertainty?
- Observation: Agent chose conservative approach without prompting

### Turn 52
- Warning: Compression of turns 48-51 omitted the false start
- Action: Re-compressed with failure included
```

### Relationship to Qualitative Criteria

| Criterion C | Room Note Contribution |
|-------------|------------------------|
| Compression is honest? | "Included failure" or "Omitted false start — correcting" |
| Retrieval feels relevant? | "Surfaced note was tangential — flash queue too aggressive?" |
| Agent seems coherent? | "Thread maintained across context rotation" |
| Spectator voice is right? | (Meta: room notes themselves demonstrate voice) |
| Moves capture shape? | "Arc makes sense" or "Missing the pivot at turn 34" |

### The Witness Protocol

Room notes are the answer to: **"What did you see happening in there?"**

They're not authoritative (the spectator can be wrong). But they're evidence the human doesn't have — a perspective on the process, not just the output.

```
Human:    "Did that response feel genuine?"
Spectator: [room note] "Turn 48 had real hesitation.
            Turn 52's confidence felt performed."
```

The human still decides. But now there's a witness to consult.

---

## Context Assembly

Each turn, the context assembler builds the window from layers:

```
┌─────────────────────────────────────────────────────────────────┐
│  SYSTEM                                                          │
│  - AGENTS.md, IDENTITY.md, RULES.md                              │
│  - Current time, preferences                                     │
│  - Tool schemas                                                  │
├─────────────────────────────────────────────────────────────────┤
│  WARM: Moves (this channel's structural history)                 │
│                                                                  │
│  Move 1: User asked about X                                      │
│  Move 2: Proposed A, pushback on Y                               │
│  Move 3: Pivoted to B                                            │
│  ...                                                             │
│  Move N: Current thread — discussing W                           │
├─────────────────────────────────────────────────────────────────┤
│  WARM: Flashes (spectator-curated, full note text)               │
│                                                                  │
│  [note: z-index-modal-fix.md]                                    │
│  Modal overlay conflicts with navbar. Solution: z-index          │
│  hierarchy — toast: 100, modal: 50, navbar: 40.                  │
│                                                                  │
│  [note: cass-deadline.md]                                        │
│  Cass mentioned Friday deadline for the dashboard. Time          │
│  pressure — keep responses focused.                              │
├─────────────────────────────────────────────────────────────────┤
│  WARM: Retrieved (semantic search based on hot context)          │
│                                                                  │
│  [note: modal component patterns]                                │
│  [note: user's CSS preferences]                                  │
├─────────────────────────────────────────────────────────────────┤
│  HOT: Recent messages (this channel, full resolution)            │
│                                                                  │
│  [turn 47] User: "What about the edge case?"                     │
│  [turn 48] Agent: "Good point, let me check..."                  │
│  [turn 49] User: "Also wondering about..."                       │
└─────────────────────────────────────────────────────────────────┘
```

### Hot Context

- Token budget (e.g., 8192 tokens), not message count
- Minimum message floor (e.g., 3 messages always included)
- This channel's recent messages, full resolution
- Never compressed, never dropped

### Budget Allocation

```
Total context window (e.g., 128K)
├── System:              ~2-4K   (fixed)
├── Warm - Moves:        ~2-4K   (this channel's arc)
├── Warm - Flashes:      ~1-2K   (full note text, spectator-selected)
├── Warm - Retrieved:    ~2-8K   (scales to fill budget)
├── Hot:                 ~8K     (token budget, min 3 messages)
└── Reserved for output: ~4-8K
```

### Channel Switching

When the agent switches channels:

| Component | Behavior |
|-----------|----------|
| System | Same |
| Moves | **Switches** to new channel's moves |
| Flashes | Persists (global by design) |
| Retrieved | Fresh search based on new hot context |
| Hot | **Switches** to new channel's recent messages |

---

## Embeddings Layer

The zettelkasten lives in the filesystem. The vector database is derived state.

```
workspace/
└── embeddings/                    # Source of truth (git-tracked)
    ├── notes/                     # Agent/spectator-written
    │   └── (whatever structure emerges)
    ├── moves/                     # Per-channel conversation arcs
    │   ├── discord-general.md
    │   ├── discord-dm-cass.md
    │   └── ...
    ├── moments/                   # Compressed arcs
    └── room-notes/                # Spectator's witness observations
        └── (session logs)

data/
└── vectors.db                     # Derived state (sqlite-vec)
```

**Reproducible:** Delete `vectors.db`, run sync, get identical vectors. The folder is the source of truth.

### Note Format

Notes are self-contained. Metadata lives in the file, human-readable.

```markdown
---
id: 0x01a2b3c4d5e6f7...
created: 2026-03-23T14:32:07Z
author: agent
type: note
tags: [css, z-index, debugging]
---

# z-index hierarchy

Modal overlay conflicts with navbar. Solution:
- toast: 100
- modal: 50
- navbar: 40

Related: [[css-stacking-contexts]] [[modal-component-patterns]]
```

| Field | Source |
|-------|--------|
| `id` | Snowflake (hex) |
| `created` | Extracted from snowflake timestamp |
| `author` | `agent` or `spectator` |
| `type` | `note`, `move`, `moment`, `room-note` |
| `tags` | Optional, agent-defined |

Links are `[[wiki-style]]`. Content is the focus — connections emerge from meaning.

### Sync Service

Watches `embeddings/`, keeps vectors in sync:

```
File changed in embeddings/
         │
         ▼
    Hash file contents
         │
         ▼
    Compare to stored hash
         │
    ┌────┴────┐
    │         │
 Same      Different
    │         │
    ▼         ▼
  Skip     Re-chunk
              │
              ▼
         Re-embed (call embedding server)
              │
              ▼
         Update vectors.db
              │
              ▼
         Store new hash
```

### Chunking Strategy

| Content Type | Chunking |
|--------------|----------|
| Notes | Whole file if small, or split on `---` / headers |
| Moves | Per-move (each move is a chunk) |
| Moments | Whole moment is a chunk |
| Room notes | Per-turn observation (each `### Turn N` block) |
| Large docs | ~400 token chunks with overlap |

### Chunk Metadata

```rust
struct Chunk {
    id: Snowflake,            // Contains timestamp (μs since agent birth)
    source_path: String,      // "embeddings/notes/z-index-fix.md"
    content: String,
    embedding: Vec<f32>,
    chunk_type: ChunkType,    // Note, Move, Moment, RoomNote, Summary
    channel: Option<String>,  // For moves: which channel
    turn: Option<u64>,        // For room notes: which turn
    hash: String,             // For sync diffing
}
```

---

## Flash Queue

Spectator's curated recommendations flow to the context assembler:

### Flash Structure

```rust
struct Flash {
    id: Snowflake,
    content: String,           // Full text of the zettelkasten note
    source: String,            // Path: "embeddings/notes/z-index-modal-fix.md"
    created_at: Timestamp,
    ttl: FlashTTL,
}

enum FlashTTL {
    Turns(u8),                 // Expires after N turns (e.g., 5)
    Duration(Duration),        // Expires after time (e.g., 30 minutes)
}
```

### TTL Modes

| Mode | Use Case |
|------|----------|
| `Turns(5)` | "Relevant to this thread" — fades if conversation moves on |
| `Duration(30min)` | "Time-sensitive" — user mentioned a meeting |

### Queue Behavior

| Event | Action |
|-------|--------|
| Spectator pushes flash | Add to queue with TTL |
| Turn begins | Decrement turn-based TTLs |
| Clock tick | Check time-based TTLs |
| TTL expires | Flash fades (note remains in cold storage) |
| Duplicate content pushed | Refresh TTL |
| Context assembler reads | Non-destructive, flashes persist until expired |

**Flashes are global.** They surface in any channel. Channel-specific context comes from moves and hot context.

---

## Event-Driven Coordination

Agent and spectator communicate via events, not strict turn-taking:

### Events: Agent → Spectator

| Event | Payload | Meaning |
|-------|---------|---------|
| `TurnStarted` | channel, context summary | "I'm about to think" |
| `TurnComplete` | transcript | "Here's what happened" |
| `NoteWritten` | path | "I wrote to embeddings/" |
| `ChannelSwitched` | old, new | "I'm focusing elsewhere" |
| `ContextPressure` | usage % | "Running low on budget" |

### Events: Spectator → Agent

| Event | Payload | Meaning |
|-------|---------|---------|
| `Flash` | content, ttl | Memory surfaced |
| `Observation` | content | Pattern noticed |
| `Warning` | content | Urgent signal |
| `MovesUpdated` | channel | Arc updated |
| `MomentCreated` | summary | Arc compressed |

### Concurrent Flow

```
Time ──────────────────────────────────────────────────────►

Agent:    ┃ wake ┃ think ┃ act ┃ act ┃ respond ┃ idle ┃ wake ┃
          │      │       │     │     │         │      │      │
Spectator:┃ idle ┃ ───── observe ───────────── ┃ compress ┃ curate ┃
                                │                    │
                         TurnComplete          Flash, Flash
                           event                events
```

The spectator can start observing during the agent's turn. Flashes arrive as produced.

### Buffering

If agent is mid-turn when spectator sends a Flash:
- Buffer it
- Include in next turn's context assembly

---

## Editorial Model

### The Transformation Principle

**All text is transformed, never deleted.** The spectator moves content between layers:

```
Hot (recent messages)
  ↓ compress
Warm (moves, flashes)
  ↓ archive
Cold (searchable, not active)
```

Even "background" content (phatic exchanges, superseded info) persists in cold storage. Nothing is erased — only moved to lower layers of accessibility.

### Permissions

| Actor | Create | Edit | Delete |
|-------|--------|------|--------|
| Agent | ✓ | ✓ | ✓ |
| Spectator | ✓ | ✓ | ✗ |

The agent has full authority. The spectator can add and modify, but cannot remove.

### Conflict Resolution

- Agent writes a note
- Spectator disagrees (thinks it's noise)
- Spectator cannot delete, but can decline to surface it
- Note fades into cold storage, effectively forgotten without being erased

### Git History

Everything in `embeddings/` is git-tracked:

```
commit a1b2c3d
Author: agent

    note: z-index solution for modal/navbar conflict

commit e4f5g6h
Author: spectator

    move: added debugging arc (moves 12-18)
```

Human can review, revert, resolve disputes.

### The Human (Ground)

The human can:
- Read everything
- Edit/delete anything
- Resolve disputes
- Final reality check

---

## Configuration

### Models

```toml
[agent]
model = "claude-sonnet-4"
model_url = "https://api.anthropic.com"

[spectator]
model = "llama-3-8b"
model_url = "http://localhost:8080"
```

### Context Budget

```toml
[context]
limit = 128000

[context.budget]
system = 4000
moves = 4000
flashes = 2000
retrieved = 8000
hot_tokens = 8192
hot_min_messages = 3
output_reserved = 8000
```

### Flash Queue

```toml
[flashes]
default_ttl_turns = 5
max_queue_size = 20
```

### Compression Triggers

```toml
[compression.triggers]
on_hot_overflow = true
on_context_pressure = 80
periodic_minutes = 30
every_n_turns = 10
```

### Move Generation

```toml
[compression.moves]
boundary_signals = ["topic_shift", "speaker_change", "decision_point", "tool_execution"]
style = "structural"           # "structural" | "narrative" | "minimal"
max_tokens = 100
topic_shift_threshold = 0.3
```

### Moment Generation

```toml
[compression.moments]
min_moves = 5
max_moves = 20
topic_coherence = 0.7
keep_recent_moves = 3
archive_compressed_moves = true
include_duration = true
include_emotional = true
include_outcome = true
max_tokens = 300
```

### Summary Quality

```toml
[compression.quality]
temperature = 0.3
foreground = ["decisions", "outcomes", "emotional_texture", "unresolved_threads"]
background = ["phatic", "redundant_attempts", "superseded", "debugging_steps"]
```

**Foreground:** Include in the summary (moves to warm).
**Background:** Archive to cold storage, not in summary but still searchable.

### Presets

```toml
[compression]
preset = "balanced"  # "aggressive" | "balanced" | "conservative"
```

---

## Success Criteria

### A. Functional (The Machine Works)

| Criterion | Test |
|-----------|------|
| Sync service embeds files | Add file, vector appears |
| Context assembles | Turn starts with hot + warm + system |
| Spectator runs | Events flow, flashes appear |
| Moves generate | Conversation progresses, moves update |
| Moments compress | Arc completes, moment appears |
| Git tracks changes | Commits with correct author |
| No crashes | 100+ turns without error |

### B. Behavioral (It Does The Thing)

| Criterion | Test |
|-----------|------|
| Relevant retrieval | Mention topic → related notes surface |
| Cross-session memory | Reference 200 turns ago → retrievable |
| Moves capture structure | Arc comprehensible without transcript |
| Moments preserve meaning | Understand session from moment alone |
| Flashes are timely | Push → appears next turn |
| Channel switching works | Context changes appropriately |

### C. Qualitative (It Feels Right)

| Criterion | Assessment |
|-----------|------------|
| Compression is honest | Includes failures, tangents, tensions? |
| Retrieval feels relevant | Surfaced memories feel apt? |
| Agent seems coherent | Maintains thread over long sessions? |
| Spectator voice is right | Terse, structural, third-person? |
| Moves capture shape | Arc makes sense, pivots visible? |
| Moments preserve what matters | Can re-enter old context? |

### The Hierarchy

| Level | Who judges | Evidence |
|-------|------------|----------|
| A | Tests | Unit tests, integration tests |
| B | Tests + observation | Behavioral tests, manual inspection |
| C | Human + spectator | Room notes + human judgment |

**C is the actual goal.** A and B are scaffolding.

The spectator self-reports via **room notes** — its witness observations about processing quality, emotional texture, and compression honesty. The human consults these notes when judging qualitative criteria, but retains final say.

### Multi-Agent Grounding

Agents can witness each other, but humans have final say:

```
Agent A ◄───► Agent B ◄───► Agent C
    │             │             │
    └─────────────┼─────────────┘
                  │
                  ▼
               Human
            (final say)
```

---

## Philosophical Stakes

### Two People in the Room

The Chinese Room (Searle) and Turing Test both assume a monolithic system — one rule-follower producing outputs.

The I/You architecture has two perspectives:
- **Agent (I):** Produces outputs
- **Spectator (You):** Observes and evaluates

They can disagree. That internal conflict is information a monolithic system can't have.

The spectator isn't checking accuracy — it's checking honesty. Did the agent compress fairly, or edit out its failures?

### Who Judges Qualitative Success?

- Self-assessment → least reliable (blind spots)
- Spectator → internal witness (sees process)
- Other agents → external witness (sees behavior)
- Human → ground truth (final say)

### The Core Principle

> "No mind should be the sole author of its own memory."

The spectator exists to make honesty possible. Not as authority, but as witness.

---

## Related Documents

- `docs/research/context-management-brainstorm.md` — Philosophy and cognition model
- `docs/research/two-people-in-the-room.md` — Chinese Room analysis
- `docs/research/embedding-architecture.md` — Declarative sync design
- `docs/research/openclaw-features-analysis.md` — Feature evaluation
