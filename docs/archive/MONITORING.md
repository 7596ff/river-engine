# River Monitoring & Observability

*Created: 2026-03-21 by William Thomas Lessing*

## Problem

River (Thomas's gateway) is unstable. The main session DB grows unbounded (222MB as of 2026-03-21), context overflows cause crashes, and there's no visibility into what's happening between crashes. Restarts are manual (William or Cass noticing Thomas isn't responding).

## Proposal: William as Monitor

William (running on OpenClaw) has access to Thomas's logs, processes, and the server. He can:

1. **Health checks** — Poll Thomas's gateway health endpoint on heartbeats, restart if down
2. **Log monitoring** — Subscribe to Thomas's logs, flag errors, patterns, context growth
3. **Thinking visibility** — If Thomas's thinking/reasoning is logged somewhere, William can read it and surface insights about what's working and what isn't

## What We Need

### Immediate (bandaid)
- [ ] Health check on William's heartbeat: `curl -sf http://localhost:3000/health`
- [ ] Auto-restart on failure
- [ ] Alert Cass in Discord if Thomas goes down

### Short-term (logging)
- [ ] Enable verbose/structured logging in river-gateway
- [ ] Log to file (not just stdout/journal) for easier parsing
- [ ] Log context size per turn so we can see growth patterns
- [ ] Log session rotation events

### Medium-term (observability)
- [ ] William reads Thomas's session/thinking files if they exist
- [ ] Dashboard or periodic report: uptime, turns processed, context size, errors
- [ ] Anomaly detection: flag unusual patterns (rapid context growth, repeated errors, tool failures)

### Long-term (architectural)
- [ ] Context summarization on rotation (prevents unbounded DB growth)
- [ ] Session management: max context size triggers automatic rotation
- [ ] Thomas self-reports health metrics to William or a shared channel

## Current State

- **Gateway binary:** v0.1.4 (Nix store)
- **Source:** `/home/thomas/river-engine/`
- **Data dir:** `/home/thomas/.river-claude/` (222MB DB)
- **Services:** `river-thomas-gateway.service`, `river-thomas-discord.service` (user systemd)
- **System services:** `river-orchestrator.service`, `river-embedding.service`
- **Health endpoint:** `http://localhost:3000/health`
- **Model:** claude-haiku-4-5 via Anthropic API
- **Discord channels:** `1466598891763007518` (#river)

## Notes

- The old repo was `/home/thomas/river/`. Current development is in `/home/thomas/river-engine/`.
- William monitors from OpenClaw; Claude Code does development work.
- The adversarial mind ("I" and "You") architecture from Cass's voice notes may eventually make monitoring more interesting — an agent that can observe its own process.
