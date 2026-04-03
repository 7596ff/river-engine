# Workspace Template

This is a template workspace for a River Engine dyad. Two workers (left and right) share this workspace, switching between actor and spectator roles.

## Structure

- `roles/` — Role definitions loaded when a worker holds a baton
  - `actor.md` — Behavioral guidance for the actor role
  - `spectator.md` — Behavioral guidance for the spectator role
- `left/` — Left worker's files
  - `identity.md` — Who the left worker is (personality)
- `right/` — Right worker's files
  - `identity.md` — Who the right worker is (personality)
- `shared/` — Reference material for both workers
  - `reference.md` — Tools, workspace structure, file formats

## Setup

The following directories are created at runtime:

- `conversations/` — Chat history by adapter/channel
- `moves/` — Per-turn summaries (spectator creates)
- `moments/` — Arc summaries (spectator creates)
- `embeddings/` — Files indexed for semantic search
- `memory/` — Long-term memory
- `notes/` — Working notes
- `artifacts/` — Generated files

TODO: Add a setup script that creates these directories and initializes the workspace.

## Customization

1. Edit `left/identity.md` and `right/identity.md` to define worker personalities
2. Edit `shared/reference.md` to add project-specific context
3. Role files (`actor.md`, `spectator.md`) are generally shared across workspaces
