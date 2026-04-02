# Workspace Structure

> Shared workspace for a dyad (actor + spectator pair)

```
workspace/
├── roles/
│   ├── actor.md              # actor role behavior/instructions
│   └── spectator.md          # spectator role behavior/instructions
├── left/
│   ├── identity.md           # left worker's fixed identity
│   └── context.jsonl         # left worker's context history
├── right/
│   ├── identity.md           # right worker's fixed identity
│   └── context.jsonl         # right worker's context history
├── shared/
│   └── reference.md          # shared reference material
├── conversations/            # chat history by adapter/channel
│   └── {adapter}/
│       ├── {guild_id}-{guild_name}/
│       │   └── {channel_id}-{channel_name}.txt
│       └── dm/
│           └── {user_id}-{user_name}.txt
├── moves/                    # message summaries by channel
│   └── {channel_id}.jsonl
├── moments/                  # move summaries by channel
│   └── {channel_id}.jsonl
├── embeddings/               # files to embed (watched by worker)
├── memory/                   # memory maintenance instructions
├── notes/                    # personal notes
└── artifacts/                # agent generated artifacts
```

## Dyad Model

A dyad is a pair of workers (left and right) that share a workspace. Each worker:
- Has a fixed identity (`left/identity.md` or `right/identity.md`)
- Maintains its own context (`left/context.jsonl` or `right/context.jsonl`)
- Has a fixed model assignment (from orchestrator config)
- Can be in either role (actor or spectator)

## Role Switching

Workers can switch roles via the `switch_roles` tool:
- Left loads `roles/spectator.md` instead of `roles/actor.md` (or vice versa)
- Right loads the opposite
- Identities, contexts, and models stay with their workers
- Only the role behavior changes

## File Ownership

| Directory | Owned by |
|-----------|----------|
| `roles/` | Both (read-only during runtime) |
| `left/` | Left worker |
| `right/` | Right worker |
| `shared/` | Both (coordinate via git) |
| `conversations/` | Actor (whoever is currently actor) |
| `moves/`, `moments/` | Spectator (whoever is currently spectator) |
| `embeddings/` | Both (push to embed server on write) |

The `river-context` crate assembles context from these files.
