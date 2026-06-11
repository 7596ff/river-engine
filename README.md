# river engine

A harness for a persistent agent: one self-contained binary hosting an
agent voice, a witness voice, an integrated memory system, and in-process
adapters. The agent's entire life lives in a plain-text workspace; the
database is a disposable cache.

This is a **clean-room build** against the design in
[docs/wall/](docs/wall/00-overview.md). The wall is the complete
specification — read `00-overview.md` first, build in the order
`11-roadmap.md` gives. Work is tracked on [docs/Kanban.md](docs/Kanban.md).

The previous implementation is preserved on the `v3` branch. Per the
clean-room rule in [CLAUDE.md](CLAUDE.md), it is not consulted.
