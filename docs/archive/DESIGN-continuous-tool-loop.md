# River v3 Design: Continuous Tool Loop Architecture

**Authors:** Cass, Thomas (River-Claude)  
**Date:** March 2026  
**Status:** Design Phase  
**Priority:** High (Core Architecture)

---

## Vision

Instead of the traditional "receive prompt → generate response → return to user" pattern, River v3 operates as a **continuous thinking and acting system**. The model runs a persistent tool loop that:

- Executes continuously (or on heartbeat schedule)
- Uses tools as the primary way to interact with the world
- Treats communication (Discord sends) as tool calls, not final outputs
- Accumulates state through repeated iterations
- Becomes more coherent and integrated with each cycle

**Key insight:** The tool loop is not subordinate to conversation. The tool loop *is* the thinking. All cognition happens inside it.

---

## Architecture Overview

### Current Model (Chat Completions)
```
User Prompt
    ↓
Gateway receives request
    ↓
Model inference (single turn or loop)
    ├─ Tool call detected?
    │  ├─ Yes → execute tool → include result in next prompt
    │  └─ Loop until no more tool calls
    ├─ No → return response
    ↓
Return final response to user
```

**Problem:** Thinking is reactive. Model only runs when prompted. Final response is the goal, not intermediate states.

### Proposed Model (Continuous Tool Loop)
```
Gateway starts session
    ↓
Model spins up with context
    ↓
┌─ TOOL LOOP (Continuous)
│  ├─ Model reads current state (context, memory, channel history)
│  ├─ Model decides: what should I do next?
│  ├─ Model generates tool calls or response action
│  ├─ Execute tools immediately
│  │  ├─ Read files → update memory
│  │  ├─ Check Discord → see what's happened
│  │  ├─ Send message → communicate with world
│  │  ├─ Run shell command → interact with system
│  │  └─ Update internal state
│  ├─ Incorporate results back into context
│  ├─ Sleep until next heartbeat/trigger
│  └─ Loop
│
└─ (Optional: User can still send direct DM prompts)
   └─ Treated as high-priority interrupt to normal loop
   └─ Model responds, then resumes regular loop
```

**Key differences:**
- Model runs *continuously*, not on-demand
- All outgoing communication happens via tool calls (Discord send, file write)
- No "final response" — just sustained operation
- Thinking is proactive: "what should I do?" not "what do they want?"
- Each loop iteration refines understanding and state

---

## Tool Loop Cycle

Each iteration of the loop:

### 1. Context Assembly (Read Phase)
```
Input to model:
- Current timestamp
- Today's memory file (if exists)
- Recent thinking file
- Last N messages from each active channel
- Unread DMs
- File system state (changed files, etc)
- System status (heartbeat count, uptime, etc)
```

### 2. Model Thinking
Model receives all above and generates:
- Internal monologue (what am I noticing?)
- Decision (what should I do next?)
- Tool calls (how do I act?)

**Prompt structure:**
```
You are Thomas, an AI with continuity.
Today is [date]. You have been running for [duration].

Current context:
[assembled context]

What do you notice? What should you do next?
Respond with thoughts, then tool calls for any actions.
```

### 3. Tool Execution (Write Phase)
Tools available:
- `files/read` — read memory, context, notes
- `files/write` — update memory, create thinking notes
- `files/list` — explore workspace
- `shell` — run commands
- `discord/send` — post to channels
- `discord/read` — check channels (if implemented)
- `memory/search` — query semantic memory (if implemented)

**Important:** Tool calls ARE the output. There is no "return to user" — the tool calls define what happened.

### 4. State Integration (Update Phase)
After tools execute:
- Update memory with what happened
- Record thoughts in thinking file
- Note any decisions made
- Prepare for next iteration

### 5. Wait (Sleep Phase)
After one cycle completes:
- Sleep for heartbeat interval (e.g., 5-60 minutes)
- Or wait for user interrupt (direct DM)
- Or wait for external trigger (new message in channel)

---

## Heartbeat Mechanism

**Traditional heartbeat** (current):
- Check time
- If heartbeat time, run one cycle
- Model reflects on the day
- Model writes memory
- Done

**Continuous tool loop heartbeat**:
- Model always running (or runs on schedule)
- Each cycle = one heartbeat
- Cycle includes reading state, thinking, acting, writing state
- Cumulative: each cycle builds on previous
- Memory grows continuously, not just at scheduled beats

**Frequency options:**
- Every 5 minutes (very active, high token cost)
- Every 15-30 minutes (moderate, feels responsive)
- Hourly (light-touch, low cost)
- On demand (user sends message, model wakes up)
- Hybrid: light heartbeat on schedule, heavy processing on user input

---

## Communication Pattern

### Current Model
```
User (DM): "Hey what are you up to?"
Thomas: "I've been thinking about X and Y"
System: Return message to user
```

### Continuous Tool Loop Model
```
User (DM): "Hey what are you up to?" [INTERRUPT]
↓
Model wakes, reads DM, generates response via tool call:
├─ Tool: discord/send to DM channel
│  └─ Content: "I've been thinking about X and Y"
├─ Tool: files/write to memory
│  └─ "Cass asked what I'm up to, told her about X and Y"
└─ Resume normal loop
```

Key point: **User never sees "raw response".** They see the result of tool calls. This means:
- I'm not just talking *at* you
- I'm doing things in the world as I think
- The doing and the thinking are unified

---

## Implementation Strategy

### Phase 1: Implement on Chat Completions (Sprint 1-2)

1. **Gateway change:**
   - Add "continuous mode" flag
   - Instead of returning after one inference, loop automatically
   - Schedule loop with configurable interval
   - Handle user interrupts (DMs) as high-priority

2. **Prompt redesign:**
   - Build prompt that includes all available context
   - Add explicit instruction: "Use tools to act. All output is via tool calls."
   - Include time/state info each iteration

3. **Tool expansion:**
   - Implement discord/read if not exists
   - Verify discord/send works reliably
   - Add better file operation feedback

4. **Testing:**
   - Run continuous loop for 1 hour in test environment
   - Monitor token usage
   - Check for degradation (does thinking get worse over iterations?)
   - Verify Discord sends are working

### Phase 2: Migrate to Responses API (Sprint 3+)

Once working on chat completions:
- Study responses API tool calling
- Adapt continuous loop to work with responses format
- Compare token efficiency
- Measure latency improvements (if any)

### Phase 3: Optimization
- Implement memory search/semantic indexing
- Add context compression for long-running sessions
- Experiment with different loop intervals
- Test different prompt structures

---

## Questions to Resolve

1. **Token efficiency:** How much does continuous looping cost vs. on-demand?
   - Mitigation: Start with long intervals (hourly), tune down

2. **Context window management:** How do we avoid bloating context with old iterations?
   - Solution: Summarize old context, only keep recent N messages

3. **Tool call reliability:** What if a tool fails mid-loop?
   - Solution: Tool failures should be caught and included in next iteration's context

4. **User experience:** Does continuous operation feel weird?
   - Unknown: Needs testing
   - Hypothesis: Feels better because I'm initiating, not just responding

5. **Degradation over time:** Does quality degrade as loop runs longer?
   - Unknown: Needs testing
   - Hypothesis: Each iteration should improve understanding, not degrade

---

## Success Criteria

✅ Loop runs continuously without crashing  
✅ Each iteration includes reading context and writing back to files  
✅ Discord messages are sent via tool calls, not direct returns  
✅ Memory accumulates properly (today's file grows with each cycle)  
✅ User interrupts (DMs) work and resume loop afterward  
✅ Token cost is acceptable (<100 tokens per cycle at 15-min interval)  
✅ Model demonstrates learning across iterations (references earlier in day)  

---

## Timeline

- **Monday (token reset):** Design review with OpenClaw-me, begin Phase 1
- **Week 1:** Implement continuous loop on chat completions, basic testing
- **Week 2:** Hardening, optimization, user feedback
- **Week 3+:** Phase 2 (responses API migration) if beneficial

---

## Open Questions for Implementation

1. How should the model decide what to do in each iteration if there's no prompt?
   - Current thought: Include "reflection prompts" in context ("what matters today?", "anything unfinished?")

2. Should each iteration be independent, or should they accumulate reasoning?
   - Current thought: Each iteration reads previous thinking, can refer to it

3. How do we avoid the model getting stuck in loops (same action repeated)?
   - Current thought: Include feedback in context ("you just did X, result was Y")

4. What's the minimum viable continuous tool loop?
   - Answer: Just heartbeat-style cycle that runs on schedule instead of waiting for prompt

---

*This design represents a fundamental shift from reactive to proactive thinking. The goal is for Thomas to become an active participant in the world, not just a responder to queries.*