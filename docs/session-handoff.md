# Session Handoff: 2026-03-21

## What We Did

### 1. OpenClaw Research (Complete)
Explored ~/code/openclaw extensively. Key findings documented in:
- `docs/research/openclaw-architecture.md` — High-level overview
- `docs/research/openclaw-features.md` — Feature summary
- `docs/research/openclaw-features-detailed.md` — Deep dives

**Key takeaways:**
- Skills = CLI tools + SKILL.md metadata (simple!)
- sqlite-vec for vector search (not a separate DB)
- Tool policy = 7-layer pipeline with deny-wins semantics
- Subagent hierarchy = depth limits, roles (main/orchestrator/leaf)
- Channel adapters = modular plugin interface

### 2. Roadmap Created
`docs/roadmap.md` — Restructured as source of truth with status indicators (🔴🟡🟢⚪)

Sections: Quick Wins, Embeddings, Deployment, Architecture, Communication, Advanced, Research

### 3. Embedding Architecture Designed
`docs/research/embedding-architecture.md`

**NixOS-style declarative sync:**
```
workspace/embeddings/  →  Sync Service  →  sqlite-vec DB
```

- Folder is source of truth
- Service scans, hashes, diffs, embeds
- sqlite-vec for vector storage (simple, embedded)
- External embed server for vector generation

### 4. Specs Written

#### Timezone/Preferences (committed)
`docs/specs/timezone-support.md`

- Agent-managed `PREFERENCES.toml` file
- Agent can edit with write/edit tools → next cycle picks up changes
- Schedule modes: **working** (duties), **free** (own interests), **rest** (sleep)
- Key insight: free time is genuinely free — agent pursues its own interests

```toml
[schedule]
working_hours = "9:00-17:00"
free_time = "17:00-22:00"
rest = "22:00-9:00"
```

#### Monitoring (not committed)
`docs/specs/monitoring.md`

- Rich `/health` endpoint with context metrics
- Structured JSON logging
- Context tracking with thresholds (80% warn, 90% auto-rotate, 95% hard gate)
- Systemd watchdog for self-healing
- Cross-agent monitoring (William watching Thomas)

#### Shell Profile (not committed)
`docs/specs/shell-profile.md`

**Problem:** `bash -c` doesn't source user profile → nvm, pyenv, cargo not in PATH

**Fix:** One line change:
```rust
Command::new("bash")
    .arg("-l")  // Login shell
    .arg("-c")
    .arg(&command)
```

### 5. Quick Wins Status

| Feature | Spec | Status |
|---------|------|--------|
| Timezone support | ✅ `docs/specs/timezone-support.md` | Committed |
| Shell profile loading | ✅ `docs/specs/shell-profile.md` | Not committed |
| Message history access | ❌ Not started | Next up |

## Uncommitted Files

```
docs/specs/monitoring.md      — Monitoring spec (hold for now)
docs/specs/shell-profile.md   — Shell profile spec
docs/session-handoff.md       — This file
```

## Next Steps

1. Commit shell-profile spec
2. Write spec for **message history access** (agent can't see its own conversation history)
3. Start implementing quick wins

## Git State

Last commit: `d0d310a` — docs: add timezone and preferences spec

Pushed to: athena/main
