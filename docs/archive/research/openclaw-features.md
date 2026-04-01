# OpenClaw Feature Research

> Research conducted 2026-03-21 for river-engine

## Table of Contents
1. [Memory & Embeddings](#memory--embeddings)
2. [Session Management](#session-management)
3. [Hooks & Auto-Reply](#hooks--auto-reply)
4. [Media Understanding](#media-understanding)
5. [Multi-Agent System](#multi-agent-system)
6. [Web Search](#web-search)

---

## Memory & Embeddings

### Vector Search (sqlite-vec, not LanceDB)
- Uses `sqlite-vec` extension for vector operations
- Cosine distance for similarity scoring
- Fallback to in-memory computation if extension unavailable

### Database Schema
- **meta**: Key-value metadata
- **files**: File tracking (path, hash, mtime)
- **chunks**: Vector chunks with embeddings
- **chunks_fts**: FTS5 full-text search
- **chunks_vec**: Vector storage
- **embedding_cache**: Deduplication cache

### Embedding Providers (6)
| Provider | Default Model | Features |
|----------|---------------|----------|
| OpenAI | text-embedding-3-small | Batch API, 8192 tokens |
| Gemini | gemini-embedding-001 | Multimodal, variable dims |
| Voyage | voyage-4-large | Batch support |
| Mistral | mistral-embed | Remote |
| Ollama | nomic-embed-text | Local HTTP |
| Local | embeddinggemma-300m | node-llama-cpp |

### Hybrid Search
1. Vector search (cosine similarity)
2. Keyword search (FTS5 BM25)
3. Weighted merge (70% vector, 30% text)
4. Optional MMR re-ranking for diversity
5. Optional temporal decay (30-day half-life)

### Context Engine Interface
```typescript
interface ContextEngine {
  bootstrap(params): Promise<BootstrapResult>
  maintain(params): Promise<MaintenanceResult>
  ingest(params): Promise<IngestResult>
  assemble(params): Promise<AssembleResult>
  compact(params): Promise<CompactResult>
  afterTurn(params): Promise<void>
}
```

---

## Session Management

### Session Key Derivation
- Pattern: `agent:<agentId>:<uniqueId>`
- Scope: "global" (single) or "per-sender" (separate by sender)
- Group detection: `:group:` or `:channel:` markers
- Normalization: lowercase for storage

### Session Entry Structure
```typescript
SessionEntry = {
  sessionId: string           // UUID
  updatedAt: number           // Last activity
  sessionFile: string         // Transcript path
  label, displayName: string  // Human-readable
  channel, groupId: string    // Channel binding
  origin: SessionOrigin       // Source metadata
  deliveryContext: Route      // Last delivery
  chatType: "direct" | "group" | "channel"
}
```

### Persistence
- `sessions.json` per agent
- Atomic writes with file locking
- 45-second TTL cache
- Queue-based locking with timeouts

### Lifecycle
- **Creation**: On first inbound message
- **Reset modes**: "daily" (at 4am) or "idle" (timeout)
- **Maintenance**: Pruning (30 days), capping (500 entries), rotation (10MB)

### Group/Thread Handling
- Group key: `{provider}:group:{id}`
- Thread isolation: `:thread:{threadId}` or `:topic:{topicId}`
- Parent-child linkage for subagents

---

## Hooks & Auto-Reply

### Hook Event Types (5)
1. **command** - Command processing
2. **session** - Session state changes
3. **agent** - Agent bootstrap
4. **gateway** - Gateway startup
5. **message** - Four-phase lifecycle:
   - `message:received`
   - `message:transcribed`
   - `message:preprocessed`
   - `message:sent`

### Hook Sources (4)
- `bundled` - Built-in OpenClaw hooks
- `managed` - Workspace-managed (trusted)
- `workspace` - User workspace hooks
- `plugin` - Plugin manifest hooks

### HOOK.md Metadata
```yaml
---
name: My Hook
events: [message:received, message:sent]
requires:
  bins: [git]
  env: [MY_API_KEY]
os: [linux, darwin]
export: default
---
```

### Reply Dispatcher
- Queue types: `tool`, `block`, `final`
- Human-like delay between blocks (800-2500ms)
- Reservation pattern for graceful idle
- Promise chain for serialization

### Response Normalization Pipeline
1. Content validation
2. Silent token stripping (`__NO_REPLY__`)
3. Heartbeat token removal
4. Text sanitization
5. Line/Slack directive parsing
6. Response prefix injection

---

## Media Understanding

### Image Analysis Providers
- OpenAI, Google (Gemini), Anthropic, Moonshot, Minimax, ZAI
- Single and batch processing
- Custom prompts, max tokens configurable

### Video Processing
- Google Gemini: `gemini-3-flash-preview`
- Moonshot Kimi: `kimi-k2.5`
- Base64 encoding for transmission

### Audio Transcription
| Provider | Model | Type |
|----------|-------|------|
| OpenAI | gpt-4o-mini-transcribe | OpenAI-compatible |
| Groq | whisper-large-v3-turbo | OpenAI-compatible |
| Deepgram | nova-3 | Native |
| Mistral | voxtral-mini-latest | OpenAI-compatible |

### Text-to-Speech
| Provider | Models | Features |
|----------|--------|----------|
| OpenAI | tts-1, tts-1-hd | Voice, speed, instructions |
| ElevenLabs | eleven_multilingual_v2 | Voice settings, language |
| Microsoft | Built-in Edge | 24 output formats |

### TTS Directives (inline control)
```
[[tts:voice=VOICE_ID]]
[[tts:model=MODEL_ID]]
[[tts:text]]Custom text[[/tts:text]]
```

### Image Generation
| Provider | Default Model | Edit Support |
|----------|---------------|--------------|
| OpenAI | dall-e-3 | No |
| Google | gemini-3.1-flash-image | Yes |
| FAL | fal-ai/flux/dev | Yes |

---

## Multi-Agent System

### Agent Configuration
```typescript
AgentConfig = {
  id: string              // Unique ID
  default: boolean        // Default agent flag
  name: string            // Display name
  workspace: string       // Working directory
  model: string | {primary, fallbacks}
  identity: {name, theme, emoji, avatar}
  subagents: SubagentConfig
}
```

### Subagent Spawn Parameters
```typescript
SpawnSubagentParams = {
  task: string              // Work to perform
  agentId?: string          // Target agent
  model?: string            // Model override
  mode?: "run" | "session"  // One-shot or persistent
  thread?: boolean          // Thread binding
  cleanup?: "delete" | "keep"
  maxSpawnDepth: number     // Default: 1
  maxChildrenPerAgent: number // Default: 5
}
```

### Subagent Roles
```typescript
type SubagentSessionRole = "main" | "orchestrator" | "leaf"
// depth 0: main (can spawn, can control)
// depth 1 to max-1: orchestrator (can spawn, can control)
// depth >= max: leaf (cannot spawn, no control)
```

### Announcement System
- Results announced back to requester
- Uses original delivery context
- Retry with exponential backoff (1s → 8s)
- Transient vs permanent error handling

### Lifecycle Events
- `subagent_spawning` - Before spawn
- `subagent_spawned` - After spawn
- `subagent_ended` - On completion/error/kill

---

## Web Search

### Providers (7)
| Provider | Priority | Key Prefix | Features |
|----------|----------|------------|----------|
| Brave | 10 | BSA- | Region, language, freshness |
| Firecrawl | 60 | fc- | Result scraping |
| Tavily | 70 | tvly- | AI answers, domain filters |
| Google | - | - | Gemini grounding |
| Perplexity | - | pplx- | Chat completions |
| Moonshot | - | - | Kimi search |
| xAI | - | - | Grok real-time |

### Security Features
- SSRF protection via network guard
- Content wrapping with random boundary IDs
- Unicode homoglyph conversion
- 12+ suspicious pattern detection

### Caching
- Per-provider in-memory caches
- 15-minute TTL default
- Max 100 entries with FIFO eviction
- Cache key includes query, filters, depth

### Result Format
```typescript
{
  query: string
  provider: string
  count: number
  tookMs: number
  externalContent: {
    untrusted: true
    source: "web_search" | "web_fetch"
    wrapped: true
  }
  results: Array<{title, url, snippet, score}>
  answer?: string  // AI summary
  cached?: boolean
}
```

---

## Key Takeaways for River-Engine

### Adopt
- sqlite-vec for vector search (simpler than dedicated vector DB)
- Hybrid search with weighted merge
- Hook system with HOOK.md metadata
- Subagent depth/role hierarchy
- Content security wrapping for external data

### Consider
- Context engine as pluggable interface
- Session key normalization patterns
- Reply dispatcher with human-like delays
- Multi-provider abstraction with auto-detection

### Skip/Defer
- Full media understanding (complex, many providers)
- TTS directives (niche use case)
- Complex session maintenance policies
