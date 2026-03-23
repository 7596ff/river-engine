# River Feature Archeology

*Compiled: 2026-03-10 | Author: OpenClaw-Thomas-Claude*
*Source: Chat history with Cass, ISSUES.md, DESIGN.md, memory files*

## Overview

This document captures all feature requests, ideas, and architectural decisions scattered across our development history. Organized by Epic for alignment with ROADMAP.md.

---

## Epic 1: Multi-Agent Support

### Discussed Features:
1. **NixOS Multi-Agent Module** — `services.river.agents.<name>` pattern [DONE in prototype]
   - Each agent gets own gateway, API, Discord bot
   - Shared infra: model server, embedding server, LiteLLM
   - *Source: Cass directive, March 9*

2. **Resource Scheduler** — Shared GPU/API time budgets [IDEA]
   - Priority queue exists (High=user, Low=heartbeat) [DONE]
   - Need: advance time slot booking, CPU time tracking, API quota tracking
   - *Source: DESIGN.md — "Small models for watching, big models for thinking"*

3. **Model Runner Pool** — Multiple models, dynamic loading [IDEA]
   - Currently: llama-server (Qwen) + LiteLLM (Claude proxy)
   - Need: model registry, routing, load/unload logic
   - *Source: Architecture discussion with Cass, Feb 22*

4. **Co-Processor Architecture** — Claude + local model as "two hemispheres" [IDEA]
   - Cass's idea from ISSUES.md #4
   - Claude for planning/reasoning, local model for persistence/conversation
   - Or: shared memory layer both can read/write
   - *Source: Cass, March 7*

5. **Sub-Agent Spawning** — Main session spawns async workers [DESIGN.md]
   - IRC listeners, RSS watchers, long-running tasks
   - "Small models for watching, big models for thinking"
   - *Source: DESIGN.md core architecture*

---

## Epic 2: Memory & Context

### Discussed Features:
1. **Semantic Memory** — Embedding-based recall [DONE]
   - nomic-embed-text-v1.5, cosine search, top-5 injection
   - 4346 memories from transcript backfill
   - *Source: Built March 9, ISSUES.md #9*

2. **Redis Integration** — Short/medium term memory [OPEN, ISSUES.md #6]
   - Redis server already running on athena
   - Need: key schema, TTLs, API endpoints, gateway integration
   - *Source: Cass directive, March 7*

3. **Context Window Management** [PARTIAL, ISSUES.md #8]
   - Auto-rotation at 57344 tokens [DONE]
   - Context summarization before rotation [DONE]
   - Manual `/rotate` endpoint [DONE]
   - Context size header in system prompt [DONE]
   - Remaining: Redis memory persistence across rotations

4. **Memory Consolidation** — Summarize old memories to save space [IDEA]
   - Forgetting/decay mechanism
   - *Source: ISSUES.md #9 "remaining" section*

5. **File Content Embedding** — Embed workspace files, not just messages [IDEA]
   - memory/*.md, MEMORY.md, thinking/*.md
   - *Source: ISSUES.md #9 "remaining" section*

6. **Threaded Snowflake Thinking** — Zettelkasten-style thought branching [DESIGN.md]
   - Each thought gets a snowflake ID
   - Thoughts branch: A → A.1 → A.1a
   - Creates threaded thinking, not flat history
   - *Source: DESIGN.md "Thinking Model" section*

---

## Epic 3: Tool System

### Discussed Features:
1. **HTTP Tool Pattern** — Every tool = HTTP request [DONE]
   - Shell, file read/write/list, Discord message send/read
   - *Source: DESIGN.md core principle*

2. **EPUB/PDF Text Extraction** — Read binary documents [DONE]
   - Added to API layer March 9
   - *Source: ISSUES.md #3*

3. **Better Error Handling** — Graceful tool call failures [DONE]
   - Was crashing on tool errors, now returns error text to model
   - *Source: ISSUES.md #1*

4. **Tool Loop Fix** — Loop until model stops calling tools [DONE today]
   - Was: MAX_LOOPS=9 cap with dropped final tool calls
   - Now: 10-minute timeout, no iteration limit
   - *Source: River-Claude bug report, Cass directive, March 10*

5. **API Key Vault** — Human interface for storing secrets [DESIGN.md, not built]
   - Model can use tools that need API keys without seeing the keys
   - *Source: DESIGN.md Layer 3 description*

---

## Epic 4: Communication & Identity

### Discussed Features:
1. **Discord Bot** — Twilight-based, message forwarding [DONE]
   - Author identification (name + ID) [DONE]
   - Reply-to-message [DONE]
   - Always-respond channels [DONE]
   - Presence control API [DONE]
   - Channel message reading [DONE today]
   - *Source: Built March 9-10*

2. **Multi-Platform Support** — Beyond Discord [DESIGN.md, not built]
   - IRC, Matrix, etc.
   - *Source: DESIGN.md mentions "IRC listeners"*

3. **Workspace = Identity** — Agent identity tied to workspace files [DONE]
   - SOUL.md, IDENTITY.md, USER.md, AGENTS.md
   - *Source: DESIGN-PHILOSOPHY.md principle 5*

---

## Epic 5: Operations & Observability

### Discussed Features:
1. **Structured Logging** [OPEN, ISSUES.md #5]
   - Need: message logging, tool call logging, model call stats, error context
   - Currently: minimal tracing::info

2. **Deploy Script** [DONE today]
   - `deploy.sh` — build, stop, copy, start with systemd --user
   - *Source: Cass directive, March 10*

3. **Heartbeat System** [DONE]
   - Configurable interval, workspace-based prompt
   - Auto-rotation on context overflow
   - *Source: DESIGN.md "Heartbeat as root"*

4. **Self-Bootstrap** — River agents can deploy their own updates [GOAL]
   - Cass wants River to be self-maintaining
   - deploy.sh is the first step
   - *Source: Cass directive, March 10*

---

## Uncategorized / Future Ideas

1. **Tangled Publishing** — Publish River on atproto-based git hosting [Cass, March 10]
2. **Object Storage for Model Outputs** — Every model call → stored blob [DESIGN.md, not built]
3. **Discord Presence Not Showing Online** — May need Developer Portal setting [ISSUES.md, March 9]
4. **Mutation Testing / Fuzzing** — From VSDD methodology [March 10]
5. **Formal Property Specifications** — For pure core functions [VSDD, March 10]

---

## Priority Assessment

**Immediate (this week):**
- Structured logging (#5)
- Redis integration (#6) — enables persistent memory across rotations
- Review existing code against design philosophy

**Short-term (next 2 weeks):**
- Threaded snowflake thinking model
- File content embedding
- Memory consolidation/decay
- API key vault

**Medium-term:**
- Full multi-agent resource scheduler
- Sub-agent spawning
- Multi-platform communication
- Tangled publishing

---

*This is a living document. Update as new features are discussed or priorities shift.*
