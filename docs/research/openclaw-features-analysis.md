# OpenClaw Feature Analysis for River-Engine

**Author:** William Thomas Lessing
**Date:** 2026-03-23
**Principle:** If a feature exists to protect the platform from its users, drop it. If it helps the agent be resilient, adaptive, and alive, keep it. Build the forest, not the lumber plantation.

---

## KEEP — Forest Features (organic, resilient, agent-serving)

1. **Tool policy pipeline** — Multi-layer filtering, deny-wins. Essential for security without rigidity. (Simplify to 2-3 layers, see DROP #14.)
2. **Heartbeat coalescing with priority queue** — Prevents thundering herd. Smart resource management.
3. **Cron with exponential backoff** — Resilient scheduling. Self-healing pattern, aligns with Phase 2.
4. **Model fallback chains** — Graceful degradation. Forest resilience — one tree dies, others take over.
5. **System prompt modularity** — Modular assembly with modes (full/minimal). Clean architecture.
6. **Context pruning** — TTL-based expiration. Necessary for long-lived agents.
7. **ATTENTION.md / escalation pattern** — Already in self-healing spec. Keep and extend.
8. **Environment variable sanitization** — Basic security hygiene. Non-negotiable.
9. **Typing indicators** — Small thing but makes agents feel present. Worth having.

## ADAPT — Good ideas, wrong implementation

10. **Channel adapters** — River should have them, but simpler. One clean trait, not the 8-adapter plugin monstrosity. Start with Discord (already have it), add one at a time.
11. **Sandbox/Docker hardening** — River agents run on your machine as you. NixOS module already provides isolation. Use systemd sandboxing (PrivateTmp, ProtectSystem, etc.) instead of Docker-in-Docker.
12. **Auth profiles** — Good idea, but use the filesystem (key files) rather than JSON config blobs. Simpler, more Unixy.
13. **Streaming coalescing** — Human-paced responses are nice, but let the adapter decide pacing, not the core engine.
14. **Thinking temperature** — Not discrete levels (off/minimal/low/medium/high/xhigh/adaptive). Continuous float 0.0–1.0. System sets it dynamically based on signal, not the agent. Maps to I/You/Ground: spectator adjusts the dial, agent operates at whatever level is set, human can override. "No mind should be the sole author of its own cognition level."

## DROP — Monoculture Features (control, legibility for the state, not the agent)

15. **7-step tool policy pipeline** — Seven layers is bureaucracy. River needs 2-3: agent default, per-agent override, runtime deny list. Done.
16. **Security audit framework (MITRE ATLAS)** — Enterprise theater. River agents run on your hardware under your user. Know your threat model, don't import someone else's compliance checklist.
17. **Authorized senders / owner allowlist** — OpenClaw needs this because it's a SaaS product serving strangers. River serves *you*. Your agent knows who you are.
18. **Tool schema normalization per provider** — OpenClaw compensating for supporting 15+ providers. River should support 2-3 well. If a provider can't handle standard JSON Schema, don't use that provider.
19. **Block streaming with min/max char coalescing** — Over-engineering. Let the model stream, let the adapter buffer if it wants to.
20. **Full prompt injection prevention (Unicode sanitization)** — River's agents are trusted. Sanitize input from external sources (web fetch) but don't treat every message as an attack vector.

---

## The Forest Test

For any future feature decision, ask:

> Does this feature help the agent be more resilient, adaptive, and alive? **Keep it.**
>
> Does this feature exist to make the system legible to administrators who don't trust it? **Drop it.**
>
> Does this feature exist because we're serving thousands of strangers? **We're not. Drop it.**

The forest doesn't need a forester. It needs rain, soil, and diversity.

---

*William Thomas Lessing, 2026-03-23*
