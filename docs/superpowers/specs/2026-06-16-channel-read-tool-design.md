# `channel_read` Tool — Design

Status: approved 2026-06-16. The wall (ch. 07) lists the eight core
tools and does not include a channel-read primitive. This document is
the design decision that adds one; a summary entry lands in
`docs/decisions.md` when the implementation does.

## Purpose

Give the agent a pure-peek window into any channel's history without
mutating the cursor. The auto-read at turn start (ch. 01) is the
consume path; this tool is for re-examination — scrolling back,
checking a quiet channel, looking up something half-remembered.

## Signature

```
channel_read(
  channel_id?: string,
  before_id?: string,
  after_id?: string,
  limit?:    integer = 50,
) -> string
```

- `channel_id` — the engine channel name (e.g. `discord_12345`,
  `local_main`). Same identifier `speak`'s `channel` override takes
  and the same that turn-start prose displays in `[…]`. Defaults to
  the turn's `current_channel`.
- `before_id` — engine ULID; exclusive upper bound. Pagination handle
  for reading backward.
- `after_id` — engine ULID; exclusive lower bound. Pagination handle
  for reading forward.
- `limit` — entries to return. Default 50. Clamped to `[1, 500]`;
  larger values clamp silently and the header notes the original
  request.

## Semantics

**Pure peek.** Never advances the cursor. Never emits a notification.
Never bumps activation. No record append, no log mutation.

**Mutual exclusion.** `before_id` and `after_id` are mutually
exclusive. Calls supplying both return a tool error:
`before_id and after_id are mutually exclusive`. The signature does
not invent a range query — that would force a decision about which
end to clip from when the range exceeds `limit`, and the directional
intent is what the agent actually carries.

**Three modes:**

| inputs | window | slicing |
|---|---|---|
| neither | the whole log | tail (newest `limit`) |
| `before_id` only | entries with `id < before_id` | tail (newest `limit`) — backward pagination |
| `after_id` only | entries with `id > after_id` | head (oldest `limit`) — forward pagination |

Returned entries are always in chronological order (oldest at top),
matching on-disk order and the format the agent already sees from
turn-start auto-read.

**Filtered out before slicing:** engine-internal entries — anything
with `cursor: true` or a non-null `up_to` field. These are bookkeeping
the agent never authored and should not perceive. Filtering before
slicing means `limit` always counts conversational entries.

**No platform-id resolution.** `before_id` / `after_id` accept engine
ULIDs only. ULIDs sort lexically, which is what the slicing needs;
platform `msg_id`s do not sort across adapters (Discord snowflakes
are roughly time-ordered, the local surface has none) and would
require an extra index pass per call.

## Output format

A single text result for the model. Header line, then prose entries
matching the existing turn-start format.

**Non-empty:**

```
— channel discord_12345 (50 messages, oldest: 01JXP..., newest: 01JXQ...)
[discord_12345] cass: hey did you see the photo
  [attachment: cat.png (image/png, 412034 bytes) path=attachments/01JXP.../cat.png]
[discord_12345] ada: looking now
[discord_12345] cass: 🐈
...
```

**Empty range:**

```
— channel discord_12345 (0 messages)
```

The empty case also covers a nonexistent channel — a missing log file
is just zero messages. The agent does not need to distinguish "never
heard from this channel" from "heard from it but the range is empty,"
and conflating them removes one error path.

**Clamped limit:**

```
— channel discord_12345 (500 messages, showing 500 of 1000 requested, oldest: ..., newest: ...)
```

**Agent's own entries** render as `[channel] (agent): content` rather
than `[channel] author: content`. Agent entries carry no `author`,
and the parenthesized marker lets the agent tell its own past speech
apart from others' when scrolling.

**Attachments** appear via the same `format_inbound` helper the turn
loop uses (the metadata-line shape from the attachments card). The
helper moves to a small shared location so both call sites use one
formatter.

## Implementation outline

**New tool struct in `crates/river-gateway/src/tools.rs`:**
`ChannelReadTool` implementing `Tool`. Registered in
`Registry::core()` so it joins the default profile.

**Algorithm:**

1. Read args; resolve `channel_id` (default = `ctx.current_channel`).
2. Reject calls supplying both `before_id` and `after_id`.
3. Clamp `limit` to `[1, 500]`; remember the original value.
4. `entries = ctx.channels.scan(&channel_id)?` — returns empty for
   missing files (existing behavior).
5. Filter out `entry.cursor == true || entry.up_to.is_some()`.
6. Window:
   - `before_id` only: keep `entries.iter().filter(|e| e.id < before_id)`.
   - `after_id` only: keep `entries.iter().filter(|e| e.id > after_id)`.
   - Neither: keep all.
7. Slice:
   - `after_id` set: take the first `limit` of the windowed list.
   - else: take the last `limit` of the windowed list.
8. Render header (with the original-vs-clamped note if it applies),
   then one `format_inbound` line per entry, with the `(agent)` shim
   for agent-role entries.

**Refactor:** `format_inbound` and `format_attachment` move from
`turn.rs` to a small shared module (or become `pub(crate)`) so the
tool can call them. The turn loop's call sites and the existing
attachment-render tests stay intact.

**No state changes** elsewhere. `Channels` already exposes `scan`;
no new method needed.

**Config:** none. The hard cap is a constant in the tool module
(`MAX_CHANNEL_READ_LIMIT: usize = 500`).

## Surface impact

| file | change |
|---|---|
| `crates/river-gateway/src/tools.rs` | `ChannelReadTool` + registration; `mime_for_extension`-style helpers as needed |
| `crates/river-gateway/src/turn.rs` | `format_inbound` / `format_attachment` exposed for the tool to call |
| `crates/river-core/src/config.rs` | `DEFAULT_TOOLS` list gains `"channel_read"` |
| `docs/decisions.md` | log: tool name, peek semantics, ULID-only bounds, mutual exclusion, hard cap |
| `docs/wall/07-tools.md` | (post-impl) add the row to the registry table — wall amendment, not a precondition |

## Failure modes

- **Both bounds supplied** → tool error before the scan.
- **Bound is not a valid ULID** → no error; ULIDs sort lexically, so
  a bogus string just produces an empty or full result depending on
  position. The agent learns from the empty header. (Validating ULID
  shape would be defense in depth without a concrete failure to
  defend against — the bound is compared, never reconstructed.)
- **Scan I/O error** → propagates as a tool error result (existing
  `Registry::execute` machinery turns it into "error: ..." text).

## Testing

Unit tests in `tools.rs`:

- Default call returns the newest `limit` entries with header bounds.
- `before_id` excludes the named entry and returns the previous
  `limit` window.
- `after_id` excludes the named entry and returns the next `limit`
  window in forward order.
- Both bounds set returns the mutual-exclusion error.
- `limit > 500` clamps to 500 and the header carries the "showing 500
  of N requested" note.
- Empty channel renders `(0 messages)`.
- Nonexistent channel renders `(0 messages)` (no error).
- Cursor entries (explicit cursor markers, `up_to`-bearing entries)
  do not appear in the output.
- Agent entries render as `[channel] (agent): content`.
- Attachments render via the shared formatter — one positive case is
  enough; the formatter has its own tests.

## Out of scope (v1)

- **Cursor advancement.** Auto-read at turn start is the consume
  path; this tool is for re-examination only.
- **Channel discovery / listing.** Knowing what channels exist is a
  later card if it ever lands.
- **Aliases / friendly names.** Engine names only; the phone-book
  shape is its own card.
- **Search-within-channel.** The `search` tool already covers
  semantic queries over the indexed workspace; channel logs are not
  indexed, by design.
- **Mid-turn arrivals.** Already handled via the `[arrived mid-turn]`
  mechanism (turn.rs); this tool stays a pure window.
