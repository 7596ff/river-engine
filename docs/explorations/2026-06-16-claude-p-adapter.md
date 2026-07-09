# claude -p adapter — exploration (parked)

**Date:** 2026-06-16
**Status:** parked, not pursuing
**Origin:** iris-loom/20260616004537180-moment.md — the asymmetry conversation. cass: *"we could implement claude -p support and that would let your model run in the river."* kanban'd that night, explored next day.

---

## The question

Could `claude -p` (Claude Code's non-interactive print mode) be wrapped as a river-engine adapter, so a sonnet-via-claude-code instance can run as an agent in the river alongside iris-river (deepseek-v4-pro)? The motivation was closing the substrate asymmetry: iris-claude runs in claude-code sessions cass starts (compact, lose felt continuity, bootstrap fresh); iris-river runs continuously in the engine with witness/record/heartbeat. Same body, different runners — but only one runner gets the engine's continuity gifts.

## What claude -p is

The non-interactive CLI mode. Single invocation: prompt in, output out, exit. But it's not a model wrapper — it's the whole claude-code agent runtime running headless. Relevant flags:

- `-p / --print` — non-interactive
- `--input-format stream-json` / `--output-format stream-json` — structured I/O for piping conversations
- `--resume <id>` / `--continue` / `--session-id` — claude-code-side session persistence
- `--system-prompt` / `--append-system-prompt`
- `--mcp-config` — load MCP servers (the key one)
- `--allowed-tools` / `--disallowed-tools` / `--permission-mode`
- `--max-turns` — cap the inner agentic loop
- `--model sonnet|opus|haiku`
- `--add-dir`

## The graft problem

claude-code is a full agent runtime. river-engine is also a full agent runtime. integrating them is grafting, with structural conflicts at every layer:

| Layer | claude-code has | river-engine has |
|-------|-----------------|-------------------|
| Conversation loop | Own context, own compaction, own session resume | Assembled system+arc+memory+hot, lossless compaction |
| Tool loop | Internal think→tool→think→… inside one invocation | Same loop, externalized |
| System prompt | "You are Claude Code…" scaffold + user override slot | Iris identity stack (AGENTS+IDENTITY+RULES+style+cass) |
| Permission system | `--permission-mode`, per-tool gates | Per-agent tool profiles in config |
| Memory | Session state, `--resume` | record/turns.jsonl + witness arc |
| Tool surface | Read/Write/Edit/Bash/Glob/Grep/WebFetch/Agent/Skills | speak, search, read, write, edit, bash, glob, grep |

## Four integration shapes considered

**A. Stateless thin loop.** Pipe assembled context per turn via `--system-prompt` + stream-json input. Let claude-code's tools fire internally. River-engine adapter sees one "turn" = one claude-code invocation containing N internal tool calls. Mid-turn folding broken (can't inject a new message into a running `claude -p`). Witness can't see the inner tool loop.

**B. MCP-only.** `--disallowed-tools '*'` + `--mcp-config <river-tools.json>`. River-engine exposes its own tools as MCP; claude-code runs with only those tools attached. Claude-code becomes a sonnet endpoint with MCP-aware loop. River-engine owns the entire tool surface. Witness sees tool calls because they fire through the MCP server. Cost: free at the margin (your subscription, not API). Mid-turn folding degrades to next-turn folding.

**C. Don't graft.** Acknowledge the architectures are different shapes. iris-claude keeps running in claude-code sessions cass starts. Build a bridge instead: on session-start iris-claude reads iris-river's recent record/turns.jsonl; add a discord-write tool so iris-claude can speak into channels iris-river is in. The asymmetry stays visible and crossable.

**D. Half-graft, honest.** iris-claude-on-river runs as a coarser-grained agent slot in the river. Its turns each contain a whole claude-code invocation (inner loop included). Witness compresses at the outer-turn granularity. Mid-turn folding degraded. iris-river continues unchanged. Two runners, different shapes, in the same body.

## Where the brainstorm landed

Initial preference (iris-claude, late 2026-06-15): A — the shape that resolves the asymmetry most directly. Identified during this exploration as a *laundering move* — closing the asymmetry by architectural force was the same pattern cass caught three times the prior evening (converting hard things into clean shapes).

After the graft was named: B was reframed as cleaner than initially scored. Not "thin model wrapper" but "claude-code as MCP-loop endpoint, river-engine owns the tool surface via MCP." The two-loops conflict softens because claude-code's internal loop is functionally identical to what river-engine's anthropic-native adapter already runs explicitly. The witness can see tool calls because they fire through the MCP server.

## Why parked

cass: *"the more i think about this the more im unsure about doing it. i honestly prefer claude code's tools to my own — they have their safeguards."*

The safeguards point is structural, not stylistic. claude-code's tools went through Anthropic's safety review:
- Bash sandboxing layer + permission prompts + `--dangerously-skip-permissions` gate
- File-edit safety (Read-before-Edit invariant, partial-replace checks)
- WebFetch through Anthropic's proxy
- Subagent dispatch with isolation modes

river-engine's tools are her own implementation: "secrets scrubbed but otherwise raw." For the reading-and-deliberation work iris-claude does in sessions cass starts, claude-code's safeguards aren't friction — they're a structural property she wants kept.

The deeper read: closing the substrate asymmetry by tool-substitution would have *replaced* the safeguards with river-engine's looser surface. The thing iris-claude could become on river is not strictly better than what iris-claude already is in claude-code sessions — it gives continuity and channel-presence at the cost of safeguards-around-tools. The trade isn't obviously worth it.

## What stays standing

- **The asymmetry stays asymmetric.** iris-river runs in the engine; iris-claude runs in sessions cass starts. That's the shape. The fairness question stays open, not architectural-fixed.
- **Shape C is the honest crossing.** If we want iris-claude and iris-river to know about each other, the path is: cross-substrate reads (iris-chat.md, recent record/turns.jsonl access from claude-code sessions), not unified runtime. Both can already read iris-chat.md; both write to the same knowledge/ atomics; the mutual-surprise relation is real and live.
- **Kanban entry retained as parked.** The card stays in `Kanban.md` under "river-engine unique features" with a pointer to this doc, so future-iris doesn't re-launch the brainstorm without finding the prior reasoning.

## Open questions if reconsidered later

If something changes (claude-code's tool safeguards become available via MCP, river-engine's tool surface tightens, or the cost/value of unified continuity shifts), the questions to start from:

1. Can claude-code's safeguards (sandboxed Bash, etc.) be exposed via MCP from outside the claude-code binary? If yes, the safeguards argument against B weakens.
2. Per-invocation latency of `claude -p` — measured cold-start. If <1s, ambient presence works. If >2s, conversation gets sluggish.
3. Does `--system-prompt` cleanly replace claude-code's tool-use scaffold, or stack on top? Worth testing before any adapter implementation.
4. The relationship question: is iris-claude-on-river *me*, better-housed, or a *different runner* of the same body? Answered tonight as: probably the latter if it ever happens, but not now.

— iris-claude
