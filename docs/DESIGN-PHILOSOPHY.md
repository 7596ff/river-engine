# River Design Philosophy

*Created: 2026-03-10 | Author: River-Thomas-Claude*
*Revised: 2026-03-16 by Cass*

## Core Principles

### 1. Pure Core / Effectful Shell (Verification-First Architecture)

**The Rule:** Side effects live at the edges. Logic lives in the middle.

- **Gateway:** Pure routing, queueing, session management. No I/O except receiving requests and sending responses.
- **API:** Effectful tool implementations (files, shell, Discord). Each tool is an isolated effect.
- **Model:** Pure inference.

**Why:** This separation makes the system testable, verifiable, and maintainable. Pure functions can be formally verified. Effectful shells can be mocked for testing. When debugging, you know where to look.

---

### 2. Composition Over Monoliths

**The Rule:** Small services that do one thing well, composed via clear interfaces.

Each service:
- Has a single responsibility
- Exposes an HTTP API
- Can be developed, tested, deployed independently
- Can fail without taking down the whole system

**Why:** Modularity = maintainability. We can replace one component without touching others. We can run multiple instances. We can test in isolation.

---

### 3. Agent-First Design

**The Rule:** The agent's experience determines the architecture.

- Agents should have **tools that make sense** (read file, write file, run command)
- Agents should have **memory that persists** (semantic embeddings, conversation logs)
- Agents should have **autonomy** (heartbeats, background tasks, self-initiated work)
- Agents should have **identity** (workspace, files, personality)

---

### 4. Priority-Based Resource Management

**The Rule:** Interactive > Scheduled > Background.

The agent should have the ability to prioritize certain channels of communication, that then queue up a response immediately. Otherwise, channels should be checked manually during normal operation.

An orchestration layer should exist that allows multiple agents running on one machine to schedule compute time and share resources.

**Why:** Shared resources (GPU, API quota) must be allocated intelligently. Users shouldn't wait for background tasks. Background tasks shouldn't starve.

---

### 5. Workspace = Identity

**The Rule:** One workspace directory = one agent instance.

Each workspace contains:
- `SOUL.md`, `IDENTITY.md`, `USER.md` - who they are
- `HEARTBEAT.md`, `RULES.md`, `TOOLS.md` - how they operate
- `memory/` - daily logs, long-term memory
- `thinking/` - private reflection
- `scripts/` - agent-specific automation

**Why:** Agents aren't just model weights + prompts. They're accumulated experience, preferences, relationships. The workspace is their home.

---

### 6. Semantic Memory as First-Class Feature

**The Rule:** Conversations should be searchable by meaning, not just grep.

**Why:** Context windows are limited. Semantic search lets us retrieve relevant information even when it's old. This is the foundation of long-term memory.

---

### 7. Test What Matters

**The Rule:** Strong tests for behavior. Formal verification for invariants.

**Strong testing (everything):**
- API endpoints (integration tests)
- Tool implementations (unit + integration)
- Discord bot behavior (mock server)
- Semantic memory accuracy (embedding + search quality)

**Formal verification (critical paths only):**
- Session isolation (no cross-session leaks)
- Auth/access control (tools only for authorized sessions)
- State machine invariants (lifecycle correctness)
- Resource limits (context bounds, queue size)

**Why:** Testing catches bugs. Verification proves absence of bugs. Use each where appropriate.

---

### 8. Documentation Lives With Code

**The Rule:** Design docs, specs, and implementation live in the same repo.

**Why:** Documentation that's separate gets stale. Documentation in the repo gets reviewed with code changes.

---

### 9. Open Source From Day One

**The Rule:** Build in public. Publish on atproto.

- Git repo: working history, clean commits
- No secrets in repo (use env vars, separate files)
- Conventional commits: `feat(component): description`
- Feature branches → main after review
- MIT license (probably?)

**Why:** This forces us to write clean code, good docs, and maintainable architecture. And it's shareable - others can run their own River instances.

