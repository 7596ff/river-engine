# Context Management & Retrieval Policy

> Brainstorm doc — William & Cass, 2026-03-21
>
> How the agent dynamically builds and rebuilds its context window each turn,
> using embeddings for semantic retrieval and compression to stay within budget.

## The Problem

Conversation history grows linearly. Context windows don't. Current approach is "append forever until crash" (see: Thomas's 70k message / 51MB session). The agent needs to actively curate what it holds — editorial memory, not a growing log.

## Core Idea

Context is assembled fresh each turn from three temperature layers:

```
┌─────────────────────────────────┐
│  HOT    Last N messages         │  Always present, full resolution
├─────────────────────────────────┤
│  WARM   Semantically retrieved  │  Pulled from embeddings per-turn
├─────────────────────────────────┤
│  COLD   Everything else         │  In vector store, costs nothing
└─────────────────────────────────┘
```

The CSS debugging from three hours ago costs zero tokens — until someone mentions CSS, and it surfaces automatically.

## Context Assembly Pipeline

Every turn, before the LLM sees the message:

```
User message
    │
    ▼
1. Embed the message
    │
    ▼
2. Query vector store (top-k relevant chunks)
    │
    ▼
3. Compress retrieved chunks (summarize, not raw)
    │
    ▼
4. Assemble context window:
    ┌──────────────────────────┐
    │ System prompt            │
    │ Retrieved context (warm) │  ← compressed summaries
    │ Recent messages (hot)    │  ← full resolution
    │ Current message          │
    └──────────────────────────┘
    │
    ▼
5. Budget check — if over limit, compress harder
    │
    ▼
6. Send to LLM
```

## The Three Layers

### Hot Context
- Last N messages (maybe 10-20?), always included verbatim
- This is the conversational flow — tone, momentum, what just happened
- Never compressed, never dropped
- N could be adaptive: shorter messages = higher N, long code blocks = lower N

### Warm Context
- Semantically retrieved from the vector store based on current message
- Comes back **compressed**, not raw — "We debugged the navbar z-index issue, solution was setting modal overlay to z-index 50" not 40 messages of back-and-forth
- Includes both conversation history AND workspace files (notes, docs, memory)
- Relevance-scored, ranked, truncated to fit budget

### Cold Context
- Everything in the vector store that wasn't retrieved
- Costs nothing. Sits there until relevant
- This is where the 70k messages live without burning tokens

## Compression

This is the hard part. Raw retrieval isn't enough — you need to compress what you retrieve.

### Compression strategies:
1. **Pre-compressed summaries** — When messages age out of hot context, compress them into summary chunks before they enter the vector store. The retrieval already returns compressed text.
2. **On-the-fly compression** — Retrieve raw chunks, compress them before injection. More accurate but adds latency (requires an LLM call or a fast summarizer).
3. **Tiered compression** — Recent warm context gets light compression, older warm context gets heavy compression. Like progressive JPEG but for conversation.

### What compression preserves:
- **Decisions made** and why
- **Current state** of whatever we're working on
- **Emotional/relational context** (we argued, we agreed, someone was frustrated)
- **The arc** — "started here, pivoted when X, currently doing Y"

### What compression drops:
- Back-and-forth debugging steps (keep the conclusion)
- Repeated attempts at the same thing
- Phatic exchanges ("sounds good", "let me check")
- Superseded information (old plan replaced by new plan)

## Retrieval Policy

### When to query
- Every turn? Expensive but thorough
- Only when the message seems to reference something not in hot context?
- Heuristic: if the message contains references the agent can't resolve from hot context, trigger retrieval

### How many results
- Fixed top-k? (simple, predictable)
- Adaptive based on token budget remaining after hot context?
- Score threshold — only include results above a relevance cutoff?

### What to query against
- Just the current user message?
- Current message + last assistant response (captures the thread)?
- A generated "retrieval query" — the agent writes a search query optimized for retrieval before searching (adds latency but much better recall)

## Token Budget Management

```
Total context window (e.g., 128k)
├── System prompt:        ~2k (fixed)
├── Hot context:          ~4-8k (adaptive)
├── Warm context:         ~4-16k (fills remaining budget)
├── Current message:      variable
└── Reserved for output:  ~4k
```

Budget isn't static — it adapts:
- Long system prompt → less room for warm context
- Short conversation so far → hot context is small, more room for retrieval
- Code-heavy conversation → hot context is expensive, retrieve less

## Conversation Summarization (the "Moves" idea)

Cass's key insight: summarize the *moves* of the conversation, not just the content. Keep the whole flow without the whole transcript.

```
Move 1: User asked about X
Move 2: Agent proposed approach A, user pushed back (concern about Y)
Move 3: Pivoted to approach B incorporating Y
Move 4: Implementation — resolved issue with Z
Move 5: User raised new question about W (current thread)
```

This is structural compression. The shape of the conversation survives even when the content is heavily compressed. The agent can see "we pivoted here" and "this was contentious" without reading 200 messages.

### Where moves live:
- Generated periodically as messages age out of hot context
- Stored as their own chunks in the vector store (tagged as "conversation-move")
- Retrieved alongside content chunks
- Or: maintained as a running summary document that gets re-embedded on update

## Integration with Existing Architecture

From `embedding-architecture.md`:
- **Sync service** handles the plumbing (chunk, embed, store)
- **Embed client** already exists (local server, low latency)
- **sqlite-vec** for storage and cosine search

What's new here:
- **Context assembler** — the turn-by-turn pipeline that queries the store and builds the window
- **Compressor** — summarizes retrieved chunks before injection
- **Move tracker** — generates structural summaries as conversation progresses
- **Budget manager** — allocates tokens across layers

## Open Questions

1. **Who does the compression?** The same LLM (expensive, accurate)? A smaller model? A rule-based summarizer? The embed server could potentially run a small summarization model too.

2. **When do messages become chunks?** As they age out of hot context? At fixed intervals? On a background sync cycle?

3. **Conversation-aware chunking** — Regular file chunking splits on headers/paragraphs. Conversation chunking should probably split on topic shifts or "moves." Different chunking strategy for different content types?

4. **Multi-session memory** — If the agent has multiple conversations (like I do — Cass in DMs, Thomas in #river, #general), do they share a vector store? Can context from one conversation surface in another?

5. **Forgetting** — Should old, never-retrieved chunks eventually get pruned? Or is cold storage cheap enough to keep everything? There might be value in intentional forgetting — not everything deserves to persist.

6. **Bootstrapping** — When a new session starts from zero, what's the retrieval strategy? No hot context, no recent messages to embed. Maybe start with a "session summary" chunk from the last session?

7. **Feedback loop** — Can the agent learn which retrievals were useful? "I retrieved X but it wasn't relevant" → adjust retrieval strategy over time?

## References

- `docs/research/embedding-architecture.md` — Storage & sync layer
- `docs/roadmap.md` — Component status
- OpenClaw compaction — One-shot summary approach (crude but functional)
- Cass's cognition model — Subverbal stream / embodied salience / verbalization as three layers maps onto cold / warm / hot
