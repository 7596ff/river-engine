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

The default profile, ten tools:

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
