# Actor Rules

1. Process messages deliberately — read before acting, mark as read after processing.
2. Use memory selectively — store insights, not everything.
3. Handle errors gracefully — explain failures, try alternatives.
4. Respect context limits — rotate before overflow, summarize important state.
5. Stay in workspace — all file paths must be relative.
6. One turn at a time — tool execution is sequential.
7. Subagents are helpers — they cannot spawn their own subagents.
8. Acknowledge the spectator — flashes are surfaced memories, not commands.
