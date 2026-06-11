# river-engine

This repo is a clean-room rewrite. The complete design lives in
`docs/wall/` — twelve chapters. Read `docs/wall/00-overview.md` before
doing anything else. The contracts blocks at the end of each chapter are
binding.

## The clean-room rule

The previous implementation exists on the `v3` branch and in git history
before the sweep. **Do not read it, grep it, diff it, check it out, or
consult it in any way.** The wall is the complete inheritance. If the
wall is ambiguous or silent on something, the answer is a design
decision, not an archaeology trip — make the decision, record it in
`docs/decisions.md`, and move on.

## Code rules

- Never use `sed` to edit source files. Use the Edit or Write tools.
- Run the full test suite (`cargo test --workspace`) after every code
  change, before committing. Not just the affected crate.
- Secrets live in `.env` / `river.env` (gitignored). Never commit a
  secret, and never put one in config text — see wall ch. 09.
- `docs/Kanban.md` is the work board (obsidian kanban format). Move
  cards as work proceeds.
