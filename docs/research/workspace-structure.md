# Workspace Structure (Draft)

> To be specced out later

```
workspace/
├── reference.md                 # shared reference material
├── actor/
│   ├── agents.md               # agent definitions
│   ├── identity.md             # actor identity
│   └── rules.md                # actor rules
├── spectator/
│   ├── agents.md
│   ├── identity.md
│   └── rules.md
├── chats/                      # chat history by adapter/channel (see chats.md)
├── embeddings/                 # declarative embedding sync folder (see embedding-architecture.md)
├── memory/                     # memory maintenance instructions
├── notes/                      # personal notes
└── artifacts/                  # agent generated artifacts
```

The worker receives `--workspace` path and operates on this structure. The `river-context` crate assembles context from these files.
