# Review Prompt: Spectator Compression Spec

Paste this prompt into Gemini with access to the river-engine codebase.

---

You are reviewing a design spec for reworking the spectator compression pipeline in a Rust workspace called river-engine. The spec is at `docs/specs/2026-04-29-spectator-compression-design.md`. Read it in full.

Your job is adversarial review. You are not here to be helpful. You are here to find every gap, contradiction, omission, and unstated assumption. The authors think they're done. Prove them wrong.

## Instructions

### 1. Verify against the actual code

Read every file in `crates/river-gateway/src/spectator/`, `crates/river-db/src/`, and `crates/river-gateway/src/coordinator/`. For every struct, method, field, event, constant, and import that the spec claims to remove, change, or keep:

- Does the thing actually exist where the spec says it does?
- Does the spec accurately describe what it currently does?
- Are there callers or dependents the spec doesn't mention? Grep for every function name being removed — who calls it? What breaks?

List every discrepancy. File path, line number, what the spec says, what the code says.

### 2. Trace the data flow end-to-end

The spec describes two flows. Trace each one through the existing code and the proposed changes. Find where the wiring is missing.

**Move generation flow:**
- `TurnComplete` event is emitted — by what code, in what file? Does the event currently contain everything the spectator needs (`transcript_summary`, `tool_calls`)? Are these fields populated or are they stubs?
- The spectator calls `observe()` → `update_moves()`. What is the current call signature? What does the spec change it to? Does every caller get updated?
- The spectator needs a `Database` handle. Where does it come from? Who creates the `SpectatorTask`? Trace the construction site — does it currently have access to the DB? If not, what has to change in `server.rs` or wherever the gateway wires things together?
- The spectator needs the model client to actually work. Read `ModelClient` — does it currently make real LLM calls? What format does it expect? What does it return? Will the move generation prompt produce a response the code can use?

**Moment generation flow:**
- The spec says "when count_moves exceeds 50." Where is this check triggered? The current code checks in `should_compress()` and in `observe()` after each TurnComplete. The spec changes the threshold but does it change the trigger location? What if the model is slow and the check fires again before the first moment call returns?
- The response parsing: `split on first ---`, parse `turns: N-M`. What if the model puts the `---` in YAML frontmatter style? What if it writes `Turns: 12-34` with a capital T? What if it writes `turns: 12 - 34` with spaces around the dash? How robust is this parsing?
- Moment files are written to `embeddings/moments/`. Who creates this directory? The current `Compressor::create_moment()` calls `create_dir_all`. Does the new version?
- The spec says "moments live in embeddings/ so they are available for vector indexing by the sync service." Read the sync service code in `crates/river-gateway/src/embeddings/sync.rs`. Does it actually watch `embeddings/moments/`? What file patterns does it scan? Will it pick up these moment files or silently ignore them?

### 3. Find internal contradictions

Read the spec's own claims against each other:

- The spec says `classify_move()` is "removed" in the "What Gets Removed" section and "retained as private fallback method" in the "Changes by Crate" section. Which is it?
- The spec says moves "stay in the DB" and are "never deleted" by moment creation. It also adds `delete_moves(channel, up_to_turn)` to the Database API. Why does this method exist if it's never called? Is this a time bomb or an honest utility? The spec should pick one.
- The spec says the `Compressor` takes a `Database` handle "via `Arc<Database>` or similar." The current `Database` struct wraps a `rusqlite::Connection`, which is `!Send` and `!Sync`. Can you actually share it across async tasks via `Arc`? Read the `Database` implementation. Is this a threading problem the spec ignores?
- The spec says prompt files are "loaded once at startup." The spectator's `run()` method is an async loop. Where exactly in the startup sequence does the loading happen? Before or after the event subscription? What if the files don't exist at startup but are created later?

### 4. Find things the spec doesn't address

- **Concurrency**: the spectator runs as an async task. Move insertion and moment creation both hit the database. The agent loop may also be hitting the database (messages, contexts). Is there a locking strategy? Does rusqlite handle concurrent writes from multiple async tasks?
- **Model client contract**: the spec says "calls the model client" repeatedly but never specifies what the call looks like. What method on `ModelClient`? What parameters? What does it return — a string, a structured response, a stream? Read the `ModelClient` implementation and state whether the spec's assumptions match.
- **Token budget**: LLM calls for move generation happen every turn. With 50 moves before moment creation, that's 50 LLM calls minimum. What model is the spectator using? If it's the same model as the agent, are they competing for inference? If it's a local model via the orchestrator, is there queuing? The spec doesn't discuss cost or latency.
- **Channel identity**: what is a "channel" in this system? Read the coordinator events — `channel` is a `String`. What format? Who sets it? Can it change mid-session? What happens to moves if the channel name changes?
- **Startup with existing moves**: if the gateway restarts, the DB has moves from a previous session. The spectator starts fresh, loads identity, subscribes to events. Does it check existing move count and potentially trigger moment creation immediately? Or does it only trigger on new TurnComplete events?
- **The transcript_summary field**: this is what the spectator sends to the LLM. Where does it come from? Is it the raw LLM output, a summary of the LLM output, or something else? How long can it be? If it's the full transcript, the move generation LLM call might itself be expensive.

### 5. Find scope leaks

The spec explicitly defers context assembly. Verify this is actually clean:

- Does anything in this spec depend on context assembly existing?
- Does anything in context assembly depend on moves being in flat files? If the future context assembly spec assumes it can read `embeddings/moves/`, this spec just broke that assumption.
- The memory system design doc (`stream/engine/memory-system-design.md`) describes a spectator that manages Redis-based short-term memory. This spec doesn't mention Redis at all. Are these two specs describing the same spectator or different ones? If different, do they conflict?

### 6. Grade the spec

- **Completeness** (A-F): Does it account for every code change needed?
- **Consistency** (A-F): Do its own claims agree with each other?
- **Precision** (A-F): Are the changes specific enough to implement without guessing?
- **Honesty** (A-F): Does it acknowledge what it doesn't know or might break?

For each grade below B, explain exactly what would raise it.

## Output format

1. **Code discrepancies** — table of file, line, what the spec says, what the code says
2. **Broken data flows** — step-by-step trace showing where wiring is missing
3. **Internal contradictions** — numbered list with quotes from the spec
4. **Unaddressed concerns** — things the spec should discuss but doesn't
5. **Scope leaks** — where deferred work is not actually cleanly deferred
6. **Grades** — with justification

Be specific. Quote the spec. Quote the code. No hand-waving.
