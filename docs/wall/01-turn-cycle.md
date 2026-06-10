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
(default 45 minutes). The turn proceeds normally, but the context gets an
explicit marker — a user-role message containing only `:heartbeat:` — so
the agent knows it woke on its own and not because anyone spoke. The
heartbeat is the autonomy floor: an agent that only wakes when spoken to
is a service, and this is not a harness for services. What the agent does
with an empty wake-up is its own affair — review channels, work in its
workspace, or settle immediately.

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
  if heartbeat and nothing new: append ":heartbeat:" tagged turn N
  if context needs compaction: compact (ch. 03)

THINK / ACT  (repeat, bounded by max_iterations, default 50)
  call the model with the assembled context and the tool schemas
  append the assistant message (tagged turn N)
  if the response has no tool calls: turn is over → SETTLE
  execute the tool calls; append each result (tagged turn N)
  if messages arrived mid-turn: read them from their channels and
      append as a single system notice (tagged turn N); track those
      channels for cursor-writing at settle

SETTLE
  write a cursor entry to every channel read this turn (ch. 05)
  emit TurnComplete { channel, turn_number: N } on the event bus
```

Everything appended during turn N carries turn number N — user messages,
assistant responses, tool results, system notices. No exceptions. The
turn number is the coordinate that the witness, compaction, and the
record all share; its integrity is what makes safe forgetting possible.

Persistence is not a settle-time step: every message is written to the
record **at the moment it is appended to the context**, exactly once,
with the turn number it was appended under. By the time `TurnComplete`
fires, the record already holds everything the witness needs — the event
carries coordinates, never content.

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

- **Persist-once.** A message enters the record exactly once, at the
  moment it is appended to the context, with the turn number it was
  appended under. Nothing re-persists, re-tags, or batches messages at
  settle.
- **Turn tagging is total.** Every context message appended during turn N
  carries turn number N, including system notices and tool results.
- **No polling.** Wake-on-notification is event-driven. The agent task
  consumes zero CPU while idle.
- **Conversation preempts digestion.** Any inbound message halts the
  digestive cycle immediately; the quiet timer restarts from zero.
- **Bounded turns.** The think/act loop has a configurable iteration
  ceiling. Hitting it ends the turn through the normal settle path.
- **Persist before announce.** `TurnComplete` is emitted only after all
  of the turn's messages are queryable in the record.
- **Clean exit.** SIGTERM → finish current turn → settle → stop tasks →
  exit 0. A turn in progress is never abandoned mid-flight.
