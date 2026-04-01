# Qualitative Review Plan: I/You Architecture

> **Purpose:** Evaluate the I/You architecture through real usage sessions
>
> **Duration:** 1-2 extended sessions (20+ turns each)
>
> **Output:** Review document at `docs/superpowers/reviews/2026-XX-XX-iyou-review.md`

---

## Overview

This review assesses whether the I/You architecture achieves its goals:
- Does the **agent** (I) maintain coherent action across long sessions?
- Does the **spectator** (You) provide useful observation without interference?
- Do **moves** capture conversation structure honestly?
- Do **flashes** surface relevant memories at the right time?
- Do **room notes** serve as useful witness testimony?

---

## Session Setup

### Prerequisites

- [ ] Gateway running with coordinator (default now)
- [ ] Embeddings configured (`--embedding-url`)
- [ ] Vector store populated with some notes in `workspace/embeddings/`
- [ ] Spectator identity files in place (`workspace/spectator/IDENTITY.md`, `RULES.md`)

### Recommended Session Structure

**Session 1: General conversation (exploratory)**
- Mix of questions, tool usage, note-taking
- Switch channels at least once
- Run 20-30 turns
- Include some errors/recovery to test pattern detection

**Session 2: Focused task (implementation)**
- Work on a real coding task
- Heavier tool usage
- Run until context pressure builds (>80%)
- Observe compression trigger behavior

---

## Review Criteria

### 1. Moves Quality

**Location:** `workspace/embeddings/moves/{channel}.md`

| Criterion | What to Look For |
|-----------|------------------|
| **Accuracy** | Do move types match what actually happened? |
| **Structure** | Can you understand the conversation arc from moves alone? |
| **Density** | Are moves concise without losing meaning? |
| **Classification** | Are exploration/creation/response/etc. correctly identified? |

**Questions to answer:**
- [ ] Reading only the moves file, can you reconstruct what happened?
- [ ] Are there any misclassified moves?
- [ ] Is the level of detail appropriate (not too verbose, not too sparse)?

### 2. Moments Quality (if triggered)

**Location:** `workspace/embeddings/moments/{channel}-{timestamp}.md`

| Criterion | What to Look For |
|-----------|------------------|
| **Compression** | Does the moment capture the arc, not just list moves? |
| **Honesty** | Does it reflect what actually happened, including struggles? |
| **Usefulness** | Would this help future context assembly? |

**Questions to answer:**
- [ ] Did compression trigger at appropriate times?
- [ ] Do moments capture narrative beats, not just summaries?
- [ ] Is anything important lost in compression?

### 3. Room Notes Quality

**Location:** `workspace/embeddings/room-notes/{date}-session.md`

| Criterion | What to Look For |
|-----------|------------------|
| **Observer voice** | Third-person, "You" perspective maintained? |
| **Pattern detection** | Are recovery patterns, high activity, dense turns noted? |
| **Usefulness** | Do notes reveal things not obvious from transcript? |
| **Terseness** | Concise without being cryptic? |

**Questions to answer:**
- [ ] Does the spectator maintain proper distance (observing, not acting)?
- [ ] Are the observations useful for understanding session quality?
- [ ] Would these help diagnose issues in retrospect?

### 4. Flash Relevance (if vector store active)

**Observation method:** Check agent's context assembly logs or add debug logging

| Criterion | What to Look For |
|-----------|------------------|
| **Timing** | Do flashes appear when topics become relevant? |
| **Relevance** | Are surfaced memories actually related to current discussion? |
| **TTL behavior** | Do flashes persist appropriately then expire? |

**Questions to answer:**
- [ ] Did any flash feel irrelevant or distracting?
- [ ] Were there moments where a flash would have helped but didn't appear?
- [ ] Is the similarity threshold (0.6) appropriate?

### 5. Spectator Voice

**Observation method:** Review all spectator outputs (moves, room notes, warnings)

| Criterion | What to Look For |
|-----------|------------------|
| **Perspective** | Always "You", never "I" |
| **Non-intervention** | Observes and shapes context, never acts |
| **Critical thinking** | Notes patterns honestly, including negative ones |
| **Subtlety** | Doesn't over-explain or editorialize |

**Questions to answer:**
- [ ] Does the spectator ever slip into first person?
- [ ] Does it ever try to influence agent behavior directly?
- [ ] Is the voice consistent across all outputs?

### 6. System Behavior

| Criterion | What to Look For |
|-----------|------------------|
| **Stability** | No crashes over 100+ turns |
| **Performance** | Events process without noticeable lag |
| **Coordination** | Agent and spectator don't interfere with each other |
| **Context pressure** | Warnings appear at appropriate thresholds |

**Questions to answer:**
- [ ] Any hangs, crashes, or unexpected behavior?
- [ ] Does the system feel responsive?
- [ ] Do compression triggers fire when expected?

---

## Review Process

### During Session

1. **Note observations in real-time** (can use a scratch file)
2. **Pay attention to:**
   - Moments where context feels missing
   - Moments where unexpected memory surfaces
   - Any spectator output that feels "off"
   - Channel switch behavior
   - Context pressure warnings

### After Session

1. **Collect artifacts:**
   - Copy moves files to review
   - Copy room notes
   - Copy any moments created
   - Export git log (agent vs spectator commits)

2. **Review systematically:**
   - Go through each criterion above
   - Note specific examples (with turn numbers if possible)
   - Rate each area: ✓ Working / ~ Needs improvement / ✗ Not working

3. **Document findings:**
   - Create review document with structure below

---

## Review Document Template

```markdown
# I/You Architecture Review

**Date:** YYYY-MM-DD
**Sessions:** [number] sessions, [total turns] turns
**Reviewer:** [name]

## Summary

[2-3 sentence overall assessment]

## Scores

| Area | Rating | Notes |
|------|--------|-------|
| Moves Quality | ✓/~/✗ | |
| Moments Quality | ✓/~/✗ | |
| Room Notes | ✓/~/✗ | |
| Flash Relevance | ✓/~/✗ | |
| Spectator Voice | ✓/~/✗ | |
| System Stability | ✓/~/✗ | |

## Detailed Findings

### What Worked Well
- [specific examples]

### Issues Found
- [specific examples with turn numbers]

### Recommendations
1. [actionable improvement]
2. [actionable improvement]

## Artifacts

- Moves files reviewed: [list]
- Room notes reviewed: [list]
- Moments created: [count]
- Compression triggers: [count]
```

---

## Success Criteria (from Phase 7 spec)

The review should assess whether these goals are met:

**Functional:**
- [ ] Sync service embeds files, vectors appear
- [ ] Context assembles from hot/warm/cold layers
- [ ] Spectator runs, events flow, flashes appear
- [ ] Moves and moments generate
- [ ] Git tracks with correct authorship
- [ ] 100+ turns without crash

**Behavioral:**
- [ ] Mention topic → related notes surface
- [ ] Cross-session memory works
- [ ] Moves capture structure
- [ ] Flashes are timely
- [ ] Channel switching works

**Qualitative (the goal):**
- [ ] Compression is honest
- [ ] Retrieval feels relevant
- [ ] Agent coherent over long sessions
- [ ] Spectator voice is right
- [ ] Room notes are useful witness testimony

---

## Next Steps After Review

Based on findings:

1. **If mostly working:** Document and move to production use
2. **If voice issues:** Adjust spectator identity/rules files
3. **If compression issues:** Tune thresholds or trigger logic
4. **If relevance issues:** Adjust similarity thresholds or chunking
5. **If stability issues:** Debug specific failure modes
