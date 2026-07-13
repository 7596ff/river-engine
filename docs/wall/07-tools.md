# 07 — Tools

Tools are how the agent acts. The design has one structural rule that
everything else serves: **the tool surface is per-agent configuration,
not code.** The registry holds everything the engine can do; the
config names what this agent's model is actually offered. A small
model gets a small surface. Capability follows capacity, and changing
an agent's reach is an edit to a config file, not a rebuild.

## Framework

- **Tool trait:** name, description, JSON-schema parameters, execute.
- **Registry:** all tool implementations, constructed at startup.
- **Profile:** the config's per-agent list of tool names (ch. 09).
  Only profiled tools appear in the schemas sent to the model. An
  unprofiled tool is invisible — not advertised, not callable.
- **Executor:** dispatches calls, times them, returns results.

## The core tools

The default profile, fourteen tools:

| tool | what it does |
|---|---|
| `read` | read a file (workspace-rooted paths) |
| `write` | create/overwrite a file |
| `edit` | exact-string replacement in a file |
| `glob` | find files by pattern |
| `grep` | search file contents |
| `bash` | run a shell command in the workspace |
| `speak` | say something on the current channel; attachments on supporting adapters |
| `search` | semantic search over the indexed workspace (ch. 02) |
| `channel_read` | pure-peek window into a channel's history (ch. 05) |
| `reject_candidate` | mark the current digestion candidate as no-go (ch. 04) |
| `create_moment` | author a moment over a turn range (ch. 03) |
| `write_atomic` | birth an atomic note under `knowledge/` with validation (ch. 02) |
| `read_moves` | scan the witness's moves over a turn range (ch. 03) |
| `compact` | force a compaction and leave a handoff for the next session (ch. 03) |

`speak` resolves "the current channel" from the turn's context — the
channel whose notification woke the agent, or the channel it last
spoke on. An explicit channel argument overrides. The outbound path is
ch. 05's: deliver via the adapter, log on acceptance. On adapters that
declare `attachments-send`, an optional list of workspace-relative
paths rides as multipart; the channel-log entry references those
paths directly (no copy).

`search` returns the top-k segments by cosine similarity with file
paths and scores. Each result is an ambient access (ch. 02) for the
notes it touches.

`reject_candidate` is the agent's voice on the divided-authorship
seam (ch. 04). It is only valid inside a `Wake::Digestion` turn —
the engine carries the current candidate's id and text on the tool
context. Signature: `reject_candidate(reason?: string)`. The tool
appends one entry to `workspace/witness/rejections.jsonl` capturing
the candidate id, candidate text, optional reason, the digestion
turn, and a timestamp. The witness reads the last N entries before
its next glean and surfaces them as `{recent_rejections}` in
`on-glean.md`. Calling the tool outside a digestion turn returns a
tool error.

`channel_read` opens a window into any channel's history without
mutating the cursor (ch. 05): `channel_read(channel_id?, before_id?,
after_id?, limit?=50)` — `channel_id` defaults to the current
channel; `before_id` / `after_id` are engine ULIDs and mutually
exclusive (the directional intent picks tail or head slicing inside
the window); `limit` is hard-capped at 500. Returns chronologically
ordered prose in the same shape the agent sees from turn-start
auto-read, headed by a line carrying the count and the boundary
ULIDs for the next call. Engine-internal entries (cursor markers,
`up_to` bookkeeping) are filtered. Empty and nonexistent channels
both render as `(0 messages)`. The tool emits no notifications,
advances no cursor, and bumps no activation — re-examination has its
own surface, separate from the consume path.

`create_moment` is the agent's own compression voice (ch. 03):
`create_moment(turn_start, turn_end, body, links?, tags?)`. The range
is inclusive, must cover at least two turns, and `turn_end` must be
≤ the current turn (no future-dating). The body is the agent's
first-person read of the stretch — what it *meant*, not just what
happened. The engine generates a ULID `id`, writes
`record/moments/{id}.md` atomically with YAML frontmatter, and the
file watcher picks it up: the moment is embedded for retrieval, joins
the typed-link graph (a `links` list becomes `cites` edges to the
named atomic notes), and overrides witness moves for its covered
turns at arc-build time.

`read_moves` is the agent's lookback into the witness's reads:
`read_moves(turn_start, turn_end)`, range size capped at 200 turns.
Returns one line per turn that has a move — `turn N [channel]:
summary` — sorted ascending. Channel attribution is taken from the
turn record. Used to choose what stretch to compress into a moment;
moments stack with each other (overlap is allowed and shows both) so
re-reading a stretch later doesn't overwrite the earlier reading.

`write_atomic` is the agent's dedicated authoring path for the atomic
web (ch. 02): `write_atomic(body, links, tags?, shape?)`. It enforces
the wall's atomic rules that bare `write` leaves unchecked — body ≤
`atomic.max_words` (default 100), at least one typed link — and
auto-populates `id` (ULID) and `created` (RFC3339). Frontmatter is
assembled in a deterministic key order (`id, created, links, tags,
shape`; absent optionals omitted) and the file is written atomically
to `workspace/knowledge/{ulid}.md` (tmp + fsync + rename).
Unresolved link targets return in the result as warnings rather than
blocking the write — forward references are legitimate. The plain
`write` tool remains an escape hatch for the rare exception; bare-
write atomics still get shape-glossed by the sync service (ch. 02).

`compact` is the agent's wind-down tool: `compact(summary)`. It writes
the summary to `workspace/handoff.md` (atomic tmp + fsync + rename) and
raises a flag the turn loop honors at the next turn start as a forced
compaction, even if the threshold has not tripped. The handoff is
consumed once on the next session's startup: it lands as a system-role
record line tagged with `last_turn + 1` on the resume channel, and the
file is deleted. The next live turn picks up after the handoff, with
the message riding in via hot. The handoff persists in the turn record
like any other line; it does not show again on subsequent sessions.

## File tools are memory instruments

This is the reason memory lives inside the engine. When `read` touches
content that is indexed, the memory system records cognitive access —
full activation bump, with propagation. When `write` or `edit` lands
in a watched directory, the sync service re-indexes the file and bumps
it. The agent cannot work with its own knowledge without warming it.
No tool needs to know this; the capture happens in the seam between
the tool layer and the memory system, by construction.

## Execution rules

- Tool calls in a single model response execute sequentially, in the
  order given; all results are appended before the next model call.
- A tool failure returns its error as result text to the model — the
  loop never crashes on a tool. (A *model* failure ends the turn;
  ch. 01.)
- Every call and result is persisted under the current turn
  (persist-once, ch. 01).
- The think/act iteration ceiling (ch. 01) bounds total calls per
  turn.
- `bash` children run with a **scrubbed environment**: the secret
  variables named by the config (ch. 09) are stripped before spawn.
  The agent's shell can do anything the agent's user can do — but it
  does not inherit API keys.

## Contracts

- **Profile gating.** Only tools named in the agent's profile are
  schema-advertised or executable. Calling an unprofiled tool returns
  an error result.
- **Sequential execution** within a response, results batched before
  the next model call.
- **Error-as-result.** Tool failures become result text, never
  panics, never turn aborts.
- **Persist under the turn.** Calls and results carry the turn number
  like every other message.
- **Env scrubbing.** Secret variables never reach tool child
  processes.
- **Capture seam.** Indexed reads → cognitive access; watched writes →
  re-index + bump. Implemented in the dispatch path, not in
  individual tools.
