# OpenClaw Detailed Feature Research

> Research conducted 2026-03-21 for river-engine

## Table of Contents
1. [System Prompts & Identity](#system-prompts--identity)
2. [Sandbox & Security](#sandbox--security)
3. [Channel Adapters](#channel-adapters)
4. [Tool System](#tool-system)
5. [Cron & Scheduling](#cron--scheduling)
6. [Model & Providers](#model--providers)

---

## System Prompts & Identity

### Prompt Assembly Architecture

System prompt built from **20+ modular sections** conditionally assembled:

| Section | Purpose | Mode |
|---------|---------|------|
| Tooling | Tool list with descriptions | full, minimal |
| Skills | Available skill scanning | full |
| Memory | memory_search/memory_get guidance | full |
| Safety | Self-preservation constraints | full |
| CLI Reference | Gateway commands | full |
| Self-Update | Config modification | full |
| Model Aliases | Friendly model names | full |
| Workspace | Working directory | full, minimal |
| Documentation | Docs links | full |
| Reply Tags | `[[reply_to_current]]` syntax | full |
| Messaging | Session routing | full |
| Voice/TTS | TTS hints | full |
| Sandbox | Container paths | full, minimal |
| Authorized Senders | Owner allowlist | full |
| Time/Timezone | Current time | full |
| Silent Replies | `__NO_REPLY__` token | full |
| Heartbeats | `HEARTBEAT_OK` response | full |
| Runtime | Agent/model/channel info | full, minimal |
| Project Context | SOUL.md, CONTEXT.md files | full |

### Prompt Modes
- **full**: All sections (main agent)
- **minimal**: Tooling, Workspace, Runtime only (subagents)
- **none**: Just identity line

### Identity Configuration
```typescript
IdentityConfig = {
  name?: string      // Display name
  theme?: string     // UI theme
  emoji?: string     // Agent emoji
  avatar?: string    // Avatar image
}
```

### Context Pruning
- Mode: `"off"` | `"cache-ttl"`
- TTL-based expiration
- `softTrimRatio` / `hardClearRatio` thresholds
- Tool-level filtering

---

## Sandbox & Security

### Docker Hardening (Default)
```
--read-only                    # Read-only root filesystem
--cap-drop ALL                 # Drop all capabilities
--security-opt no-new-privileges
--network none                 # No network by default
tmpfs: ["/tmp", "/var/tmp"]    # Ephemeral temp dirs
```

### Blocked Paths (Cannot Mount)
```
/etc, /proc, /sys, /dev, /root, /boot
/run/docker.sock, /var/run/docker.sock
```

### Tool Policy (Sandbox)
**Allowed by default**:
- `exec`, `process`, `read`, `write`, `edit`, `apply_patch`
- Session tools: `sessions_list`, `sessions_history`, etc.

**Denied by default**:
- `browser`, `canvas`, `nodes`, `cron`, `gateway`
- All channel integrations

### Environment Variable Sanitization
**Blocked patterns**:
- `*_API_KEY`, `*_TOKEN`, `*_PASSWORD`, `*_SECRET`
- AWS, GitHub, Anthropic, OpenAI specific keys

**Allowed patterns**:
- `LANG`, `LC_*`, `PATH`, `HOME`, `USER`, `SHELL`, `TERM`, `TZ`, `NODE_ENV`

### Security Audit Framework
- MITRE ATLAS aligned threat model
- 5 trust boundaries: Channel, Session, Tool, External Content, Supply Chain
- Levels: Critical, Warn, Info

---

## Channel Adapters

### Plugin Interface
```typescript
ChannelPlugin<ResolvedAccount> = {
  id: ChannelId                    // "discord", "slack", etc.
  capabilities: ChannelCapabilities

  // Modular adapters:
  config: ChannelConfigAdapter     // Account resolution
  setup?: ChannelSetupAdapter      // Setup workflow
  groups?: ChannelGroupAdapter     // Group policies
  outbound?: ChannelOutboundAdapter // Sending
  actions?: ChannelMessageActionAdapter // Rich actions
  gateway?: ChannelGatewayAdapter  // Connection lifecycle
  ...
}
```

### Supported Channels
| Channel | Auth | Features |
|---------|------|----------|
| Discord | Bot token | Threads, embeds, reactions, voice, moderation |
| Slack | Bot + User token | Block Kit, interactive components |
| Telegram | Bot API | Topics, polls, document forcing |
| WhatsApp | Web QR | Basic messaging, reactions, polls |

### Message Normalization
Each adapter implements:
- `normalizeTarget()` — Format channel/user IDs
- `normalizeOutboundTarget()` — Validate delivery targets
- Target parsing: `channel:123`, `user:456`, `@group:id`

### Rich Content Support
- **Discord**: Embeds, stickers, components (buttons, modals)
- **Slack**: Block Kit (sections, buttons, images)
- **Telegram**: Markdown, captions, document mode
- **WhatsApp**: Basic text + media URLs

### Typing Indicators
```typescript
createTypingCallbacks({
  start: () => sendTyping(channelId),
  keepaliveIntervalMs: 3000,
  maxDurationMs: 60000,
})
```

---

## Tool System

### Tool Definition
```typescript
AgentTool = {
  name: string
  label?: string
  description: string
  parameters: JSONSchema  // TypeBox
  execute: async (toolCallId, params) => result
  ownerOnly?: boolean
}
```

### 7-Step Policy Pipeline
1. Profile policy (minimal/coding/messaging/full)
2. Provider-specific policy
3. Global tools.allow/deny
4. Global tools.byProvider
5. Agent-specific policy
6. Agent provider policy
7. Group/channel policy

### Tool Profiles
- **minimal**: `session_status` only
- **coding**: ~15 core tools
- **messaging**: Session + message tools
- **full**: All available tools

### Tool Groups
```typescript
CORE_TOOL_GROUPS = {
  "group:fs": ["read", "write", "edit", "apply_patch"],
  "group:runtime": ["exec", "process"],
  "group:sessions": ["sessions_list", "sessions_history", ...],
  "group:memory": ["memory_search", "memory_get"],
  "group:ui": ["browser", "canvas"],
}
```

### Subagent Tool Restrictions
**Always denied**: `gateway`, `agents_list`, `cron`, `memory_search`
**Leaf agents denied**: `subagents`, `sessions_spawn`

### Schema Normalization (Provider-Specific)
- **Gemini**: Strip unsupported keywords
- **OpenAI**: Require `type: "object"`, flatten unions
- **xAI**: Strip validation constraints
- **Anthropic**: Full JSON Schema compliance

---

## Cron & Scheduling

### Schedule Types
```typescript
// One-shot at timestamp
{ kind: "at", at: 1679529600000 }

// Fixed interval
{ kind: "every", everyMs: 3600000 }

// Cron expression
{ kind: "cron", cron: "0 9 * * 1-5", tz: "America/Los_Angeles" }
```

### CronJob Structure
```typescript
CronJob = {
  id, name, enabled: boolean
  schedule: CronSchedule
  payload: { systemEvent: string } | { agentTurn: { prompt } }
  sessionTarget: "main" | "isolated" | "current"
  wakeMode: "now" | "next-heartbeat"
  state: { nextRunAtMs, lastRunAtMs, lastStatus, consecutiveErrors }
}
```

### Error Handling
- Exponential backoff: 30s → 1m → 5m → 15m → 60m
- Transient error detection (rate limit, 5xx, timeout)
- Failure alerts after N consecutive errors (default: 2)

### Heartbeat System
```typescript
// Coalescing timer with priority queue
requestHeartbeatNow(target, { reason, priority })

// Priority levels
RETRY (0) < INTERVAL (1) < DEFAULT (2) < ACTION (3)

// Default interval: 30 minutes per agent
```

### Wake Modes
- **"now"**: Immediate heartbeat, waits up to 2min for busy
- **"next-heartbeat"**: Queue event for next interval

---

## Model & Providers

### Auth Modes
- `api-key`: Direct API key
- `oauth`: OAuth tokens
- `token`: Bearer tokens
- `aws-sdk`: AWS credential chain (Bedrock)

### Auth Resolution Priority
1. Auth profiles (`~/.openclaw/auth-profiles.json`)
2. Environment variables
3. Custom provider config
4. Synthetic local auth (Ollama)
5. AWS SDK defaults

### Model Selection
```typescript
{
  agents: {
    defaults: {
      model: {
        primary: "anthropic/claude-opus-4-6",
        fallbacks: ["openai/gpt-5.4", "google/gemini-2.0-flash"]
      }
    }
  }
}
```

### Thinking Levels
```typescript
type ThinkLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "adaptive"

// Provider support:
// - Anthropic Claude 4.6: "adaptive" (default)
// - Anthropic Claude 4.5: "low" - "high"
// - OpenAI GPT-5.4: "xhigh"
// - Z.ai: "off" | "on"
```

### Context Windows
| Model | Context | Max Output |
|-------|---------|------------|
| Claude Opus 4.6 | 1M tokens | 128K |
| Claude Sonnet 4.6 | 1M tokens | 128K |
| GPT-5.4 | 1.05M tokens | 128K |
| Default | 200K tokens | - |

### Fallback Logic
- Primary failure → try fallback chain in order
- Cooldown tracking (30s minimum)
- Probe throttling (30s between provider probes)
- Context overflow → rethrow (no fallback)
- Abort → skip fallback chain

### Streaming
```typescript
BlockStreamingCoalescing = {
  minChars: 800,
  maxChars: 1200,
  idleMs: 1000,
  joiner: string
}
```

---

## Key Takeaways for River-Engine

### High Priority Adoption
1. **Tool policy pipeline** — Multi-layer filtering with deny-wins semantics
2. **Sandbox tool restrictions** — Separate policy for containerized execution
3. **Heartbeat coalescing** — Priority queue prevents thundering herd
4. **Model fallback chains** — Graceful degradation with cooldowns

### Medium Priority
1. **System prompt sections** — Modular assembly with mode-based inclusion
2. **Channel adapter interface** — Clean separation of concerns
3. **Cron with exponential backoff** — Resilient scheduled execution
4. **Auth profile rotation** — Failed credentials get cooldown

### Consider Later
1. **Full prompt injection prevention** — Unicode sanitization
2. **Block streaming coalescing** — Human-like response pacing
3. **Tool schema normalization** — Provider-specific cleanup
