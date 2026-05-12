# Home Channel Design Review Prompt

Paste the spec below into Gemini or another reviewer, followed by this prompt:

---

You are reviewing a design spec for **home channel**, a fundamental architectural change to a multi-agent orchestration system called river-engine. The change replaces the agent's invisible internal context with a visible, append-only log (the "home channel") that becomes the single source of truth for all agent activity.

This is a critical review. Your job is not to validate — it is to **lay bare contradictions**. Find the places where the spec promises two incompatible things, where the architecture implies behaviors the text doesn't address, where the removal of existing systems creates gaps the new system doesn't fill.

## 1. The Source of Truth Contradiction

The spec says the home channel is "the single source of truth" AND that "per-adapter logs remain." These are two sources of truth. Interrogate this:
- What happens when they diverge? (Network failure during dual-write, crash between writing to adapter log and home channel)
- Which one wins in a conflict?
- If the home channel is truly the source of truth, why does the adapter log exist? What does it provide that a filtered view of the home channel couldn't?
- Is "catching up on channel context" (the stated reason for keeping adapter logs) actually easier with a separate log than with a tagged query on the home channel?

## 2. The Compression Paradox

The spec says context is a "derived compressed view" of the home channel, and that "old entries get compressed in place." But:
- If entries are compressed *in place* in the log, the log is no longer append-only. It's mutable. This contradicts the append-only claim.
- If compression produces a *new* entry and marks the old one, the log grows. What's the retention policy?
- The spectator "drives compression as before" — but before, it operated on `PersistentContext` which was an in-memory structure. Now it operates on a file. What are the performance implications of reading/writing JSONL for every context build?
- Rolling compression means the home channel is both archive AND working memory. Can these two functions coexist in one structure without one degrading the other?

## 3. The Batching Timing Problem

Messages queue up and get injected "after tool results, before the next model completion call." But:
- What if no tool calls happen? The agent responds with pure text (no tool calls). Where do batched messages go? They arrived during the completion call. The spec says they get injected between tool results and the next completion — but there are no tool results.
- What if the agent makes 20 tool calls in sequence? Do batched messages get injected after EVERY tool result, or only after the last one before the next completion?
- What about messages that arrive during the model completion call itself (not during tool execution)? The model is thinking. A message arrives. When does the agent see it?

## 4. The Channel Switching Removal

The spec removes channel switching entirely. But:
- The agent still needs to know which adapter/channel to respond to. How does it decide? Does it respond to the most recent `[user]` tag? What if two users from different adapters are talking simultaneously?
- The current `send_message` tool requires an explicit adapter and channel. Does this remain unchanged? If so, the agent has to parse its own `[user:adapter:channel_id/channel_name]` tags to figure out where to send. Is this reliable?
- What happens to conversations that span multiple channels? (A user mentions something in Discord that relates to something in the TUI.) Is the home channel the implicit "merge point" for cross-channel conversations?

## 5. The Bystander Endpoint Security

The bystander endpoint accepts anonymous messages with no authentication and no author identity. But:
- The gateway has auth (`validate_auth` with bearer tokens). Does the bystander endpoint skip auth?
- If it uses auth, it's not truly open. If it skips auth, it's an unauthenticated write to the agent's source of truth.
- Anonymous messages that influence the agent's behavior are an attack surface. The spec says "anonymous by design" — is this a feature or a vulnerability?
- Can a bystander flood the home channel and trigger infinite turns?

## 6. The Tool Result File Pattern

Large tool results get written to files and linked. But:
- What's the size threshold?
- Who cleans up these files? The home channel log references them forever. If they're deleted, the log has dangling references.
- When the context builder encounters a file link, does it read the file? If so, the "derived view" requires filesystem access beyond just reading the JSONL. What if the file is missing?

## 7. The Heartbeat as Home Channel Entry

Heartbeats get written to the home channel. But:
- Heartbeats fire every 45 minutes by default. Each one creates a home channel entry and triggers a turn. The agent wakes, builds context from the entire home channel, calls the model, and... does what? There's no message to respond to. What does a heartbeat turn look like?
- Over time, heartbeat entries accumulate in the log. Are they compressed? They have no content to compress.
- If the agent is actively working (in the middle of a turn), does a heartbeat still write to the home channel? Does it trigger a nested turn?

## 8. What's Missing

- **Migration path.** How do existing agents with `PersistentContext` data transition to home channels? Is existing context lost?
- **Concurrency.** Multiple processes writing to the same JSONL file (agent writing responses, HTTP handler writing incoming messages). What's the locking strategy?
- **Performance.** Building context by reading a JSONL file on every turn instead of keeping it in memory. At what log size does this become a bottleneck?
- **Ordering guarantees.** The home channel is the source of truth, but writes come from different sources (agent task, HTTP handlers, heartbeat timer). How is ordering guaranteed in the JSONL?

Be specific. Cite the spec when pointing out issues. Suggest concrete fixes, not vague concerns.
