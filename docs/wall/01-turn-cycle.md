# 01 — The Turn Cycle

The agent's life is a sequence of **turns**. A turn is the unit of
everything: context grows by turns, the witness compresses by turns, the
record is coordinated by turn number. One turn = one wake, one stretch of
thinking and acting, one settle.

## Wake sources

The agent task sleeps until one of three things happens:

**A notification.** An adapter received a message and the channel layer
(ch. 05) pushed a pointer onto the notification queue. The queue wakes the
task via async notification — the loop never polls. Multiple notifications
may be waiting; the turn drains them all.

**The heartbeat.** A timer fires after a configurable interval of silence
(default 45 minutes), measured from the last turn's settle. The turn
proceeds normally, but the context gets an explicit marker — a user-role
message containing only `Read HEARTBEAT.md.` — so the agent knows it woke
on its own and not because anyone spoke, and knows where its standing
instructions for self-directed time live: `HEARTBEAT.md` at the workspace
root, seeded by the engine (ch. 08), owned and editable by the agent and
Ground alike. The heartbeat is the autonomy floor: an agent that only
wakes when spoken to is a service, and this is not a harness for
services. What the agent does with an empty wake-up is its own affair —
the briefing file suggests; it never compels.

**The quiet trigger.** After 5 minutes with no inbound messages, the
digestive cycle (ch. 02) begins draining the extraction queue. This is
memory work, not conversation, and it has strictly lower priority: the
moment a message arrives, digestion halts, the message is handled, and the
quiet timer resets from zero. Conversation always wins.

## Anatomy of a turn

```
WAKE
  increment turn number N
  drain all pending notifications; deduplicate channels
  for each notified channel: read entries since cursor (ch. 05)
  append each new message to the context as a user message,
      tagged turn N, formatted "[channel] author: content"
  if heartbeat and nothing new: append "Read HEARTBEAT.md." tagged turn N
  if context needs compaction: compact (ch. 03)

THINK / ACT  (repeat, bounded by max_iterations, default 50)
  if remaining iterations are in the last 20% (integer ceil):
      append a system notice "[R/M tool calls remaining]" (tagged turn N)
  call the model with the assembled context and the tool schemas
  append the assistant message (tagged turn N)
  if the response has no tool calls: turn is over → SETTLE
  execute the tool calls; append each result (tagged turn N)
  if messages arrived mid-turn: read them from their channels and
      append as a single system notice (tagged turn N); track those
      channels for cursor-writing at settle

SETTLE
  write a cursor entry to every channel read this turn (ch. 05)
  emit TurnComplete { turn_number: N } on the event bus
```

Everything appended during turn N carries turn number N — user messages,
assistant responses, tool results, system notices. No exceptions. The
turn number is the coordinate that the witness, compaction, and the
record all share; its integrity is what makes safe forgetting possible.

Persistence is not a settle-time step: every message is appended to the
turn record (`record/turns.jsonl`, ch. 10) **at the moment it is
appended to the context**, exactly once, with the turn number it was
appended under and the channel it concerns. The record is one stream
for the whole life — a turn that reads three channels is still one
turn, in one place. By the time `TurnComplete` fires, the record file
already holds everything the witness needs — the event carries a
coordinate, never content.

Mid-turn arrivals deserve their note: when messages land while tools are
executing, they are folded into the *current* turn as a system notice
rather than queued for the next one. The agent sees them immediately and
can respond in the same breath. Their channels get cursors at settle like
any other.

## The model call

The model client speaks two protocols, chosen by endpoint: the Anthropic
Messages API and the OpenAI-compatible chat completions API. Tool calls
within a single model response execute in the order given; the results
are appended together before the next model call. A failed tool call
produces an error-text result for the model, never a crashed loop. A
failed *model* call ends the turn (settle still runs); the turn's
messages are already persisted, so nothing is lost.

After every model response, the context's token estimator calibrates
itself against the prompt token count the model reports (ch. 03).

## Shutdown

On SIGTERM or SIGINT the gateway finishes what it is doing and leaves
cleanly: the current turn runs to settle, background tasks (adapters,
sync, decay) stop, and the process exits. The runner never has to kill a
healthy gateway. If a turn is somehow stuck, the runner escalates to
SIGKILL after a grace period (ch. 09) — and because persistence is
append-time, even that loses nothing already said.

## Contracts

- **Turns are serial.** One turn at a time; a new turn begins only after
  the previous turn's settle completes. Nothing in the engine ever
  interleaves turns.
- **Turn numbers are monotonic for life.** Turn numbers never repeat and
  never reset; startup resumes from the highest turn number in the
  record. Restarts are invisible to the numbering.
- **Persist-once.** A message enters the record exactly once, at the
  moment it is appended to the context, with the turn number it was
  appended under and the channel it concerns. Nothing re-persists,
  re-tags, or batches messages at settle.
- **Turn tagging is total.** Every context message appended during turn N
  carries turn number N, including system notices and tool results.
- **No polling.** Wake-on-notification is event-driven. The agent task
  consumes zero CPU while idle.
- **Conversation preempts digestion.** Any inbound message halts the
  digestive cycle immediately; the quiet timer restarts from zero.
- **Bounded turns.** The think/act loop has a configurable iteration
  ceiling. Hitting it ends the turn through the normal settle path.
- **Visible budget.** In the last 20% of the turn's iterations (integer
  ceil, minimum one round), a system frame `[R/M tool calls remaining]`
  is appended before the model call — durable in the record, visible
  in the next prompt. The agent should not be cut off in the dark;
  with the count in hand it can choose to wind down (speak, summarize,
  end) instead of piling on tools whose results it will never see.
- **Every turn settles.** A failed model call, a hit iteration ceiling,
  and a shutdown signal all end the turn through the same settle path;
  what was persisted before the failure is never lost.
- **Heartbeat floor.** After the configured interval with no turns, a
  heartbeat turn begins. A self-wake with nothing new always carries the
  `Read HEARTBEAT.md.` marker — a wake is never ambiguous about why it
  happened.
- **Cursors at settle.** Every channel read during the turn — including
  mid-turn arrivals — receives a cursor entry at settle (ch. 05).
- **Persist before announce.** `TurnComplete` is emitted only after all
  of the turn's messages are durably appended to the record file.
- **Clean exit.** SIGTERM → finish current turn → settle → stop tasks →
  exit 0. A turn in progress is never abandoned mid-flight.
