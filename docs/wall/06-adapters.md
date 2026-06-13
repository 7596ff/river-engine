# 06 — Adapters

An adapter connects the engine to a place where conversation happens.
Adapters are **in-process**: implementations of an `Adapter` trait,
running as supervised tasks inside the gateway. There is no adapter
protocol, no registration handshake, no authentication between the
engine and its own limbs — those are the costs of process boundaries,
and there is no process boundary here.

## The trait

Conceptually:

```rust
trait Adapter {
    fn name(&self) -> &str;                       // "discord", "local"
    fn features(&self) -> &[Feature];             // what this place can do
    async fn run(self, ctx: AdapterContext);      // own the connection
}
```

- **Inbound:** the adapter's `run` task receives platform events,
  normalizes each into an inbound message (channel id, author name +
  id, content, platform msg_id), and hands it to the channel layer
  (ch. 05) through `AdapterContext`. The adapter forwards everything
  it is configured to listen to; deciding what *matters* is the
  agent's job, not the adapter's.
- **Outbound:** the adapter receives outbound requests (channel,
  content, optional reply-to) on a channel from `AdapterContext`,
  delivers them to the platform, and returns the platform's message id
  (or an error, which becomes tool-result text for the agent).
- **Features:** each implementation declares what its platform
  supports — send, edit, delete, react, typing, threads, history. The
  gateway folds the declarations into the agent's system prompt, so
  the model knows what each place can actually do. Feature lists are
  data, not capabilities: an adapter with a feature the agent never
  uses costs nothing.

## Supervision

Each configured adapter binding runs as its own tokio task under a
small supervisor: panics are caught, the task is restarted with
exponential backoff (1s doubling to a 60s cap, counter reset after 5
minutes of health), and the agent is unaffected throughout — a dying
Discord connection never touches a turn in progress. Inbound messages
during adapter downtime are the platform's problem (Discord redelivers
nothing; the agent reads forward from its cursor when the connection
returns and the platform's history permits).

## The Discord adapter

A trait implementation owning a Discord gateway websocket connection.

- **Listening:** a configured guild and listen-set of channels. Slash
  commands `/listen` and `/unlisten` manage the set at runtime (state
  persisted in the data directory); DMs to the bot always pass.
- **Inbound:** message-create events from listened channels and DMs,
  normalized; the bot's own messages excluded.
- **Outbound:** send (with optional reply-to), typing indicator.
- **Features declared:** send, receive, reply, edit, delete, react,
  typing, history — per what the implementation actually wires.
- **Token:** read from the environment at startup (ch. 09); never in
  config or logs.

## The local surface

The second shipped adapter is the engine's own front door: a small
HTTP + WebSocket server bound to localhost.

- `GET /chat` (WebSocket): bidirectional chat — client sends
  `{author, content}`, receives the agent's messages on the channel.
  Every connected client sees the same channel (`local_main` by
  default); this is Ground's door, not a multi-user system.
- `POST /message`: one-shot inbound message for scripting
  (`{author, content}` → `{ok}`).
- `GET /health`: liveness and basic state — written by the live turn
  loop (current turn number, last settle time, context usage). Health
  data that is not produced by the live path must not exist.
- `GET /graph` + `GET /graph/view`: the activation graph as read-only
  JSON (all indexed notes, cold included; activation scores; typed,
  wiki, and semantic edges; the flash threshold) and a single
  self-contained HTML page rendering it (vendored d3-force, no build
  step; color = warmth, size = score, halo near the flash threshold,
  typed solid / semantic dashed, 5s poll, click for detail).
- `GET /context` + `GET /context/view`: the live context window as
  read-only JSON (per-layer token estimates — system, arc, memory
  slot, hot — turn range, slot contents, estimate vs limit,
  calibration ratio), published by the turn loop at settle, and a
  page drawing the window as a stacked bar against the compaction
  line. **Strictly windows, never hands**: nothing on these routes
  mutates state.

The **TUI client** is a separate small binary: a terminal chat window
(message log, status bar, input line) speaking the WebSocket protocol.
It holds no state, renders what it receives, and dies harmlessly. Any
other client — curl, a script, a future web page — uses the same
surface.

## Contracts

- **In-process only.** Adapters are trait implementations in the
  gateway binary. No inter-process adapter protocol exists in v1; if
  one is ever wanted, it arrives as a new adapter impl that proxies,
  additively.
- **Supervised.** Adapter panic → catch, log, restart with backoff
  (1s→60s, reset after 5 healthy minutes). Never propagates to the
  agent.
- **Forward everything configured; filter nothing by content.**
- **Feature declarations** are folded into the agent's system prompt.
- **Local surface is localhost-only** and is the sole HTTP exposure of
  the gateway.
- **Health is honest.** Every field served by `/health` is written by
  the component it describes, in the live path.
