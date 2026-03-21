# OpenClaw Architecture Research

> Research conducted 2026-03-21 for river-engine module/skill/MCP design

## Repository Structure

```
openclaw/
├── src/                          # Main TypeScript source
│   ├── entry.ts / index.ts        # CLI/library entry points
│   ├── gateway/                   # WebSocket control plane (core runtime loop)
│   ├── agents/                    # Agent loop & runtime (Pi-based)
│   ├── cli/                       # CLI command routing & setup
│   ├── channels/                  # Channel plugins (WhatsApp, Discord, Slack, etc.)
│   ├── plugins/                   # Plugin system & bundling
│   ├── plugin-sdk/                # SDK exports for plugins/extensions
│   ├── extensions/                # Provider plugins (81 extensions)
│   ├── config/                    # Configuration system
│   ├── providers/                 # AI provider setup
│   ├── auto-reply/                # Response generation & hooks
│   ├── memory/                    # Vector search & context
│   ├── web-search/                # Web search providers
│   ├── sessions/                  # Session lifecycle
│   ├── hooks/                     # Plugin hooks system
│   ├── skills/                    # Skill metadata & registration
│   ├── media-understanding/       # Image/video/audio analysis
│   ├── image-generation/          # Image gen providers
│   ├── tts/                       # Text-to-speech providers
│   ├── context-engine/            # Context/memory management
│   ├── infra/                     # Infrastructure (logging, env, etc.)
│   └── ...
├── extensions/                    # 81 pluggable extensions
│   ├── anthropic/                 # Provider plugins
│   ├── openai/
│   ├── discord/                   # Channel plugins
│   ├── slack/
│   ├── telegram/
│   └── ...
├── skills/                        # 50+ bundled skills
│   ├── summarize/
│   ├── apple-notes/
│   ├── github/
│   └── ...
├── packages/                      # NPM packages
│   ├── clawdbot/                  # Discord bot package
│   └── moltbot/                   # Multi-agent bot package
└── apps/                          # Native apps
    ├── macos/                     # macOS menu bar app
    ├── ios/                       # iOS app
    └── android/                   # Android app
```

---

## Skill System

Skills are **CLI tools + metadata files**. Simple and elegant.

### Structure
```
skills/summarize/
├── SKILL.md               # Metadata: triggers, usage, config
└── (external binary)      # Calls external CLI (e.g., `summarize` brew formula)
```

### SKILL.md Declares
- Trigger phrases ("summarize this URL")
- Binary requirements (`requires.bins`)
- Installation methods (`install: [{ kind: "brew" }]`)
- Environment variables needed

### Key Properties
- **Stateless** — no persistent state between invocations
- **Sandboxed** — don't get full API access like plugins
- **External** — wrap existing CLI tools
- **Discoverable** — gateway loads skills snapshot at startup

---

## Plugin/Extension System (Three Layers)

### 1. Skills (CLI Tools)
- External binaries with metadata
- Sandboxed, stateless
- User-installable

### 2. Extensions (TypeScript Modules)
- Full API access via Plugin SDK
- 81 bundled (providers, channels, services)
- Types: Provider, Channel, Service

### 3. Plugin SDK (200+ Subpaths)
```typescript
// Not monolithic — import only what you need
import { definePluginEntry } from "openclaw/plugin-sdk/core";
import { registerProvider } from "openclaw/plugin-sdk/provider-auth";
import { ChannelRuntime } from "openclaw/plugin-sdk/channel-runtime";
import { McpServerConfig } from "openclaw/plugin-sdk/mcp";
```

### Extension Example
```typescript
// extensions/anthropic/index.ts
export default definePluginEntry({
  id: "anthropic",
  setup: ({ registerProvider }) => {
    registerProvider("anthropic", { ... });
  }
});
```

---

## MCP Integration

### Configuration
```yaml
mcp:
  servers:
    context7:
      command: "uvx"
      args: ["context7-mcp"]
```

### How It Works
1. Config defines MCP servers (stdio or HTTP)
2. Agent runtime loads servers via `@modelcontextprotocol/sdk`
3. Tools from MCP servers merge into agent's tool list
4. Agent can call tools, MCP server handles execution

### Key Files
- `src/config/types.mcp.ts` — config schema
- `src/plugins/bundle-mcp.ts` — bundled MCP server discovery
- `src/agents/pi-bundle-mcp-tools.ts` — tool binding
- `src/agents/mcp-stdio.ts` — stdio transport

---

## Configuration System

### Flow
```
~/.openclaw/config.json
    ↓
loadConfig()
    ↓
ConfigFileSnapshot (cached, reactive)
    ↓
OpenClawConfig object
    ↓
applyConfigOverrides() → runtime config
```

### Key Sections
- `identity` — identity/avatar config
- `channels` — per-channel setup (allowFrom, dmPolicy, bindings)
- `providers` — model auth profiles
- `agents` — multi-agent routing (workspace dirs)
- `plugins` — which extensions to enable
- `memory` — vector DB (lancedb)
- `mcp.servers` — MCP server definitions
- `hooks` — module/script paths for lifecycle hooks

### Features
- JSON5 format (comments allowed)
- Schema validation at startup
- Hot reload support
- Plugin config schema merging

---

## Gateway (Control Plane)

### Purpose
WebSocket server that orchestrates all subsystems.

### Responsibilities
- Plugin registry & loading
- Channel manager
- Config reloader
- Session lifecycle handlers
- Model catalog
- Maintenance timers
- Cron service
- Health monitor
- Auto-reply dispatcher

### Key File
`src/gateway/server.impl.ts` — 200+ lines of subsystem initialization

---

## Agent Loop (Execution Plane)

### Core Components

**Request Scope** (`gateway-request-scope.ts`)
- Creates isolated context per request
- Holds: session, user, channels, config snapshot, plugin runtime

**Plugin Runtime** (`plugins/runtime/index.ts`)
- Agent methods: `runCliAgent()`, stream responses
- Config access: read-only config snapshot
- Channel runtime: send messages, update presence
- Media: image generation, transcription
- Web search: tavily + other providers
- Memory: vector search (lancedb)
- Subagent: call other agents

**Tool Policy Pipeline** (`agents/tool-policy-pipeline.ts`)
- Filters available tools based on:
  - Channel capabilities
  - User allowlists
  - Agent configuration
  - Tool-specific policies

**Lanes** (`gateway/server-lanes.ts`)
- Groups agents by workspace
- Applies per-lane concurrency limits
- Prevents resource overload

### Execution Flow
1. User sends message → Channel receives
2. Channel adapter converts to `ChannelSessionMessage`
3. Gateway routes to session
4. Config determines which agent processes it
5. Agent spawned with prompt, system prompt, tools, context
6. Pi agent processes via Claude (reasoning + streaming)
7. Tools invoked → skill CLIs, APIs, shell commands
8. Responses streamed back
9. Final response sent to all bound channels
10. Session updated, memory indexed

---

## Architecture Diagram

```
┌─────────────────────────────────────────┐
│    Gateway (WebSocket Control Plane)    │
├─────────────────────────────────────────┤
│  ┌──────────────────┐                  │
│  │  Config Loader   │                  │
│  │   + Validator    │                  │
│  └────────┬─────────┘                  │
│           │                            │
│  ┌────────▼──────────────┐             │
│  │  Plugin Registry      │             │
│  │  - Extensions         │             │
│  │  - Channels           │             │
│  │  - Providers          │             │
│  └────────┬──────────────┘             │
│           │                            │
│  ┌────────▼──────────────┐             │
│  │  Channel Manager      │             │
│  │  - Discord            │             │
│  │  - Slack              │             │
│  │  - WhatsApp, etc.     │             │
│  └─────────────┬─────────┘             │
│                │                       │
│     ┌──────────▼──────────┐            │
│     │ Session Registry    │            │
│     │ - Conversations     │            │
│     │ - Binding targets   │            │
│     └─────────────────────┘            │
└────────────────────┬───────────────────┘
                     │
         ┌───────────▼────────────┐
         │  Agent Execution Loop  │
         │  - System Prompt Setup │
         │  - Pi Agent (Claude)   │
         │  - Tool Execution      │
         └────────┬───────────────┘
                  │
         ┌────────▼──────────────┐
         │  Response → Channels  │
         └───────────────────────┘
```

---

## Design Patterns for River-Engine

| Pattern | OpenClaw Approach | Implication |
|---------|-------------------|-------------|
| **Skill System** | CLI tools + metadata | Define clear skill boundary |
| **Plugin SDK** | Subpath exports | Modular SDK, not monolithic |
| **MCP Integration** | Config-driven discovery | Support stdio + HTTP |
| **Channel Architecture** | Plugin per channel | Abstract channel specifics early |
| **Config System** | JSON5 + validation | Schema validation, hot reload |
| **Hook System** | Pre/post lifecycle hooks | Extension points for custom logic |
| **Concurrency Control** | Lanes per workspace | Bounded concurrency from day 1 |
| **Request Scope** | Isolated context | Dependency injection or context objects |

---

## Subsystem Reference

| Subsystem | Purpose | Key Files |
|-----------|---------|-----------|
| Gateway | Control plane, orchestration | `server.impl.ts` |
| Agent | Claude execution, tool binding | `cli-runner.ts`, `pi-embedded-runner.ts` |
| Channels | Message I/O | `channels/plugins/`, `extensions/*/` |
| Skills | CLI tool wrapping | `skills/`, `SKILL.md` |
| Plugins | TypeScript modules | `extensions/`, `plugin-sdk/` |
| Config | File-based configuration | `config/config.ts`, `config/io.ts` |
| Sessions | Conversation state | `sessions/` |
| Memory | Vector DB, context search | `memory/`, `context-engine/` |
| Auto-Reply | Response generation | `auto-reply/`, `hooks/` |
| MCP | Model Context Protocol | `config/types.mcp.ts`, `plugins/bundle-mcp.ts` |
