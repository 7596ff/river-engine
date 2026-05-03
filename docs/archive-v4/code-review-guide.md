# Code Review Guide

A walkthrough of River's logic flows, from starting the service to the agent sending its first message.

## Overview

River has three main components:

1. **Orchestrator** - Central coordinator that spawns and supervises workers and adapters
2. **Worker** - The agent runtime that thinks and acts
3. **Adapter** - Platform bridges (Discord, etc.) that translate between protocols

The flow is: Orchestrator starts, spawns workers and adapters, they register back, then messages flow through adapters to workers and back.

## 1. Orchestrator Startup

**Entry point:** `crates/river-orchestrator/src/main.rs`

```
main()
  parse args: --config, --port
  load_config(path)                    # config.rs:152
    read river.json
    substitute_env_vars()              # $VAR_NAME -> value
    validate_config()                  # check model references

  create shared state:
    registry: RwLock<RegistryState>    # tracks all processes
    supervisor: RwLock<SupervisorState> # manages child processes
    respawn: RwLock<RespawnManager>    # handles restart policies

  spawn HTTP server on port            # http.rs routes
  spawn_dyads()                        # for each dyad in config
    spawn_worker(left)
    spawn_worker(right)
    spawn_adapters()

  supervision_loop()                   # main.rs:~200
    every 60s: health check all processes
    on dead process: respawn with backoff
    on ctrl+c: graceful shutdown
```

**Key files:**
- `main.rs` - entry point, supervision loop
- `config.rs` - configuration structs and loading
- `http.rs` - HTTP routes for registration
- `supervisor.rs` - process spawning
- `registry.rs` - process discovery

## 2. Worker Startup and Registration

**Entry point:** `crates/river-worker/src/main.rs`

```
main()
  parse args: --orchestrator, --dyad, --side, --port
  bind HTTP server (port 0 = OS-assigned)

  register_with_orchestrator()
    POST {orchestrator}/register
    body: WorkerRegistrationRequest {
      worker: { dyad, side, endpoint }
    }
```

**Orchestrator handles registration:** `http.rs:214-311`

```
handle_worker_registration()
  validate dyad exists in config
  get DyadConfig for this dyad
  get model config for this side (left/right)

  determine baton:
    if side == config.initial_actor -> Actor
    else -> Spectator

  check respawn state (for restart scenarios)

  register in RegistryState:
    workers.insert(WorkerKey{dyad, side}, ProcessEntry::Worker{...})

  get partner endpoint from registry (if registered)
  update supervisor with this worker's endpoint
  push_registry_to_all()              # notify everyone of new process

  return WorkerRegistrationResponse {
    baton,
    model: ModelConfig,
    ground: Ground,
    workspace: PathBuf,
    partner_endpoint: Option<String>,
    initial_message: Option<String>,  # from respawn
    start_sleeping: bool,
  }
```

**Worker receives response and initializes:**

```
  receive WorkerRegistrationResponse

  load_context()                       # persistence.rs
    read {workspace}/{side}/context.jsonl
    parse each line as OpenAIMessage

  load_role()                          # from workspace/roles/
    if Actor -> actor.md
    if Spectator -> spectator.md

  load_identity()                      # {workspace}/{side}/identity.md

  spawn HTTP server for incoming:
    POST /notify      - events from adapters
    POST /flash       - messages from partner
    POST /registry    - updates from orchestrator
    POST /prepare_switch, /commit_switch, /abort_switch

  run_loop()                           # worker_loop.rs
```

**Key structs:**

```rust
// river-protocol/src/registration.rs
WorkerRegistrationRequest {
    worker: WorkerRegistration { dyad, side, endpoint }
}

WorkerRegistrationResponse {
    baton: Baton,              // Actor or Spectator
    model: ModelConfig,
    ground: Ground,
    workspace: PathBuf,
    partner_endpoint: Option<String>,
    initial_message: Option<String>,
    start_sleeping: bool,
}

// river-protocol/src/identity.rs
Baton { Actor, Spectator }
Side { Left, Right }
Ground { name, id, adapter, channel }
```

## 3. Adapter Startup and Registration

**Entry point:** `crates/river-discord/src/main.rs`

```
main()
  parse args: --orchestrator, --dyad, --type, --port
  bind HTTP server

  get supported_features()             # discord.rs
    [SendMessage, ReceiveMessage, EditMessage, DeleteMessage, ...]

  register_with_orchestrator()
    POST {orchestrator}/register
    body: AdapterRegistrationRequest {
      adapter: { dyad, adapter_type, endpoint, features }
    }
```

**Orchestrator handles adapter registration:** `http.rs:314-387`

```
handle_adapter_registration()
  validate dyad exists
  get adapter config from dyad.adapters[]

  validate required features:
    must have SendMessage (0)
    must have ReceiveMessage (1)

  register in RegistryState:
    adapters.insert(AdapterKey{dyad, adapter_type}, ProcessEntry::Adapter{...})

  get actor worker endpoint from registry
  push_registry_to_all()

  return AdapterRegistrationResponse {
    worker_endpoint: String,
    config: Value,            # adapter-specific (token, guild_id, etc.)
  }
```

**Adapter receives response and starts event loop:**

```
  receive AdapterRegistrationResponse

  init_discord_client(config)          # twilight gateway connection

  spawn event_loop:
    loop {
      event = discord.recv_event()     # from Discord gateway

      convert to InboundEvent {
        adapter: "discord",
        metadata: EventMetadata::MessageCreate { ... }
      }

      POST {worker_endpoint}/notify
        body: InboundEvent
    }

  serve HTTP for outbound requests:
    POST /request -> execute OutboundRequest
```

## 4. First Message Arrives

**Discord gateway delivers MessageCreate:**

```
Discord Gateway
  -> twilight event
  -> discord.rs converts to EventMetadata::MessageCreate {
       channel: Channel { adapter, id, name },
       author: Author { id, name, bot },
       content: String,
       message_id: String,
       timestamp: DateTime,
       reply_to: Option<String>,
       attachments: Vec<Attachment>,
     }
  -> wrap in InboundEvent { adapter: "discord", metadata }
  -> POST {worker_endpoint}/notify
```

**Worker receives event:** `worker/src/http.rs:62-135`

```
handle_notify()
  parse InboundEvent

  match metadata {
    MessageCreate { channel, author, content, ... } => {
      // Write to conversation file
      write_to_conversation(
        path: {workspace}/conversations/{adapter}/{channel_id}.txt,
        line: "[+] {timestamp} {message_id} <{author}> {content}"
      )

      // Check if we should wake up
      if state.sleeping && state.watch_list.contains(channel) {
        state.sleeping = false
      }

      // Add to pending notifications
      state.pending_notifications.push(Notification {
        channel,
        message_count: 1,
        has_mention: content.contains(self_name),
      })
    }
  }

  return 200 OK
```

## 5. Worker Loop Processes Message

**Main loop:** `worker/src/worker_loop.rs:108-250`

```
run_loop()
  // ACTIVATION: wait for first event
  wait_for_activation()
    loop until pending_notifications.len() > 0 or pending_flashes.len() > 0

  // LLM LOOP
  loop {
    // Check context pressure
    if token_count > context_limit * 0.9 {
      return ExitStatus::ContextExhausted
    }

    // Collect pending work
    notifications = state.pending_notifications.drain()
    flashes = state.pending_flashes.drain()

    // Build messages for LLM
    messages = context.messages.clone()

    // Add system message with notifications
    if notifications.len() > 0 {
      messages.push(system_message(
        "New messages in channels: {channels}"
      ))
    }

    // Add flashes from partner
    for flash in flashes {
      messages.push(system_message(flash.content))
    }

    // Call LLM
    response = llm_client.chat_completion(
      messages,
      tools: [speak, adapter, read, write, bash, sleep, summary, ...]
    )

    // Process response
    match response.content {
      Text(text) => {
        append_to_context(assistant_message(text))
        // Continue loop, expecting tool call
      }

      ToolCalls(calls) => {
        for call in calls {
          result = execute_tool(call)

          match result {
            Success(json) => append_to_context(tool_result(json))
            Summary(text) => return ExitStatus::Done { summary: text }
            Sleep(mins) => return ExitStatus::Done { wake_after: mins }
            SwitchRoles => initiate_role_switch()
            Error(e) => append_to_context(tool_error(e))
          }
        }
      }
    }
  }
```

## 6. Agent Sends First Message

**LLM decides to call `speak` tool:**

```
LLM response:
  ToolCalls: [{
    name: "speak",
    arguments: {
      content: "Hello!",
      channel: "discord:123456789",
    }
  }]
```

**Tool execution:** `worker/src/tools.rs`

```
execute_tool("speak", args)
  parse channel from args
  parse content from args

  // Build outbound request
  request = OutboundRequest::SendMessage {
    channel: Channel { adapter: "discord", id: "123456789" },
    content: "Hello!",
    reply_to: None,
  }

  // Get adapter endpoint from registry
  endpoint = registry.get_adapter_endpoint("discord")

  // Send to adapter
  POST {adapter_endpoint}/request
    body: request
```

**Adapter executes request:** `discord/src/http.rs`

```
handle_request()
  parse OutboundRequest

  match request {
    SendMessage { channel, content, reply_to } => {
      // Call Discord API via twilight
      discord_client.create_message(channel.id)
        .content(&content)
        .reply(reply_to)
        .await

      return OutboundResponse {
        ok: true,
        data: Some(ResponseData::MessageSent {
          message_id: "987654321"
        })
      }
    }
  }
```

**Worker receives response:**

```
  response = OutboundResponse { ok: true, data: MessageSent { id } }

  // Record in context
  append_to_context(tool_result({
    "ok": true,
    "message_id": "987654321"
  }))

  // Write to conversation file
  write_to_conversation(
    "[>] {timestamp} {message_id} <{self}> {content}"
  )

  // Continue loop
```

## 7. Key Data Flows

### Registration Flow
```
Orchestrator starts
  -> spawns Worker (left)
  -> spawns Worker (right)
  -> spawns Adapter (discord)

Worker (left) -> POST /register -> Orchestrator
  <- WorkerRegistrationResponse { baton: Actor, ... }

Worker (right) -> POST /register -> Orchestrator
  <- WorkerRegistrationResponse { baton: Spectator, ... }

Adapter -> POST /register -> Orchestrator
  <- AdapterRegistrationResponse { worker_endpoint, config }

Orchestrator -> POST /registry -> all processes (push updates)
```

### Message Flow (Inbound)
```
Discord Gateway
  -> Adapter (discord)
  -> POST /notify -> Worker (actor)
  -> pending_notifications
  -> LLM loop processes
```

### Message Flow (Outbound)
```
LLM returns ToolCall(speak)
  -> Worker executes tool
  -> POST /request -> Adapter
  -> Discord API
  -> OutboundResponse
  -> Worker records result
```

### Role Switch Flow
```
Worker (actor) calls switch_roles tool
  -> POST /switch_roles -> Orchestrator

Orchestrator:
  1. POST /prepare_switch -> Worker (actor)
  2. POST /prepare_switch -> Worker (spectator)
  3. POST /commit_switch -> Worker (actor)
  4. POST /commit_switch -> Worker (spectator)
  5. Update registry (swap batons)
  6. POST /registry -> all processes
```

## 8. Files to Review

### Core Logic
- `river-orchestrator/src/main.rs` - startup, supervision loop
- `river-orchestrator/src/http.rs` - registration handlers, role switching
- `river-worker/src/main.rs` - worker startup, registration
- `river-worker/src/worker_loop.rs` - main think/act loop
- `river-worker/src/tools.rs` - tool execution

### State Management
- `river-orchestrator/src/registry.rs` - process registry
- `river-orchestrator/src/supervisor.rs` - process management
- `river-worker/src/state.rs` - worker state
- `river-worker/src/persistence.rs` - context persistence

### Protocol Types
- `river-protocol/src/registration.rs` - registration messages
- `river-protocol/src/identity.rs` - Author, Channel, Ground, Baton, Side
- `river-adapter/src/feature.rs` - OutboundRequest, FeatureId
- `river-adapter/src/event.rs` - EventMetadata, InboundEvent

### Adapter Implementation
- `river-discord/src/main.rs` - adapter startup, event loop
- `river-discord/src/http.rs` - outbound request handling
- `river-discord/src/discord.rs` - twilight integration

## 9. Review Checklist

### Registration
- [ ] Worker registration validates dyad exists in config
- [ ] Adapter registration validates required features
- [ ] Registry pushed to all processes after each registration
- [ ] Partner endpoint correctly resolved from registry

### Message Routing
- [ ] Adapter forwards all events to worker endpoint
- [ ] Worker writes to correct conversation file
- [ ] Outbound requests routed to correct adapter
- [ ] Response data correctly parsed and recorded

### State Persistence
- [ ] Context appended after every tool execution
- [ ] Conversation files written atomically
- [ ] Watch list persisted across restarts

### Error Handling
- [ ] Failed registrations return useful errors
- [ ] Network errors don't crash processes
- [ ] Tool errors recorded in context for LLM
- [ ] Respawn backoff prevents tight loops

### Role Switching
- [ ] Three-phase protocol executed in order
- [ ] Both workers must acknowledge prepare
- [ ] Abort sent if either worker fails prepare
- [ ] Registry updated atomically after commit
