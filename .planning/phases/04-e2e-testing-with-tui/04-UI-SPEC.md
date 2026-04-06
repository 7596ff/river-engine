---
phase: 4
slug: e2e-testing-with-tui
status: draft
tool: ratatui-tui
preset: terminal-ui
created: 2026-04-06
---

# Phase 4 — TUI Enhancement & Integration Test Visual Contract

> Terminal UI rendering and interaction contract for integration testing phase. Covers TUI enhancements (baton display, bidirectional backchannel) and test assertion visuals.

**Source:** Pre-populated from 04-CONTEXT.md (locked decisions D-01 through D-20) and 04-RESEARCH.md (established patterns).

---

## Design System

| Property | Value |
|----------|-------|
| Tool | ratatui 0.29 (TUI rendering) |
| Backend | crossterm 0.28 (terminal control) |
| Component library | ratatui widgets (built-in: Block, List, Paragraph, Layout) |
| Color palette | 16-color ANSI terminal (standard xterm) |
| Font | Monospace (terminal default) |

**Preset source:** Not applicable (terminal UI, not web UI)

---

## Terminal Layout & Constraints

### Master Layout (3-section vertical split)

Fixed from **existing TUI architecture** (`crates/river-tui/src/tui.rs`):

| Section | Height | Content | Z-order |
|---------|--------|---------|---------|
| Header | 3 lines (fixed) | Title, dyad, channel, connection status, baton state, line counts | Top |
| Messages | Min 10 lines (flexible) | Interleaved context, backchannel, system messages | Middle |
| Input | 3 lines (fixed) | User prompt + input field | Bottom |

**Total minimum terminal height:** 16 lines (header 3 + messages 10 + input 3)
**Minimum width:** 80 columns (standard terminal width)

### Header Line Specification (3 lines)

**Line 1 (title bar):**
```
 River Mock Adapter  dyad:test-dyad channel:general [connected] L:42 R:38
```

Components (left to right):
- `" River Mock Adapter "` — Title (black text on cyan bg, bold)
- ` dyad:{name}` — Dyad name (yellow text)
- ` channel:{name}` — Channel (green text)
- ` [{status}]` — Connection status: "connected" (cyan) or "waiting..." (dim)
- ` L:{count} R:{count}` — Lines read from left and right context files (dark gray text)

**Line 2 (baton state — NEW D-09):**
```
Actor: left  Spectator: right
```

Components:
- `"Actor: "` — Label (white text)
- `{side}` — Currently "left" or "right" (yellow text when actor, dim gray when spectator)
- `  Spectator: "` — Separator + label (white text)
- `{side}` — Currently "left" or "right" (yellow text when spectator, dim gray when actor)

**Line 3 (metadata):**
```
═══════════════════════════════════════════════════════════════════════════════
```
Visual separator (box border, full width)

**Header box style:** Single border (ratatui `Borders::ALL`), cyan color on non-active state.

---

## Message Display Specification

### Message Line Format

All messages follow this pattern:
```
[HH:MM:SS]  {side} {role}> {content}
```

| Component | Format | Color | Notes |
|-----------|--------|-------|-------|
| Timestamp | `[HH:MM:SS]` | Dark gray | 24-hour format, milliseconds not displayed |
| Side | `L` or `R` | Dark gray | Left or right worker |
| Role | `user`, `asst`, `sys`, `tool` | Role-specific (see below) | 4-char abbreviation |
| Content | Text (wrapped to width) | White (default) | Full message content, wrapped at terminal width |

### Color Scheme by Message Type

| Type | Prefix | Prefix Color | Content Color | Example |
|------|--------|--------------|---------------|---------|
| User input | `you>` | Cyan (bold) | White | `you> what are you thinking?` |
| System | `sys>` | Yellow | Yellow | `sys> Workspace initialized` |
| Assistant | `asst>` | Green | White | `asst> I notice you asked about thinking patterns` |
| Tool call | `tool>` | Blue | Yellow (args), dark gray (metadata) | `tool> call read_history {side: "left"}` |
| Tool result | `tool>` | Blue | White | `tool> [abc123] {"success": true}` |
| Backchannel (outgoing) | `[BC]>` | Magenta | White | `[BC]> {"operation": "commit"}` |
| Backchannel (incoming) | `[BC]<` | Magenta | White | `[BC]< git commit success` |

### Message Wrapping Rules

- Max content width: Terminal width — 12 characters (for timestamp, spacing, role prefix)
- Long lines wrap at word boundaries where possible
- Indentation on wrapped lines: 11 characters (aligns with timestamp width)
- Tool arguments truncated to 30 characters (display limit, `TOOL_ARGS_MAX_LEN`)
- Tool results truncated to 40 characters (display limit, `TOOL_RESULT_MAX_LEN`)

### Backchannel Display (Enhanced D-10)

Backchannel messages appear in the same message list as context messages, prefixed with `[BC]` side indicator:

```
[12:34:56]  [BC]> wrote 5 lines to backchannel.txt
[12:34:58]  [BC]< git commit: abc123def
```

Each backchannel operation creates a timestamped entry. The TUI reads `workspace/conversations/backchannel.txt` and appends entries as they appear.

---

## Input & Interaction Specification

### Input Field (Bottom Section)

```
> your message here...
```

| Element | Behavior | Notes |
|---------|----------|-------|
| Prompt | `>` (cyan, bold) | Always visible |
| Text | User-typed input | Echoed in white |
| Cursor | Block cursor (terminal default) | Visible during input |
| Backspace | Delete previous character | Standard terminal behavior |
| Enter | Send message | Clears input, triggers `/notify` to worker |

### Keyboard Shortcuts

| Key | Action | Notes |
|-----|--------|-------|
| `Ctrl+C` | Exit TUI | Graceful shutdown, no cleanup needed |
| `Enter` | Send message | Only if input non-empty |
| `PageUp` | Scroll messages up | Increases scroll offset |
| `PageDown` | Scroll messages down | Decreases scroll offset |
| `/bc {text}` | Write backchannel message | Appends to `workspace/conversations/backchannel.txt` |

**Command prefix:** `/bc ` (4 chars) triggers backchannel write mode (not a regular message).

---

## Mock LLM Response Specification (D-15 to D-17)

### Mock Endpoint Contract

**Endpoint:** `POST /v1/chat/completions` (OpenAI-compatible)

**Request format:** Standard OpenAI chat completion request (messages array + model + tools)

**Response format:**

```json
{
  "id": "chatcmpl-{timestamp}",
  "object": "chat.completion",
  "created": 1234567890,
  "model": "mock-gpt-4",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "{role_aware_response}",
        "tool_calls": [
          {
            "id": "call-{uuid}",
            "type": "function",
            "function": {
              "name": "{tool_name}",
              "arguments": "{json_string}"
            }
          }
        ]
      },
      "finish_reason": "tool_calls"
    }
  ],
  "usage": {
    "prompt_tokens": 10,
    "completion_tokens": 8,
    "total_tokens": 18
  }
}
```

### Role-Aware Response Templates (D-16)

| Baton | Response Style | Example | Tool Calls |
|-------|---|---------|-----------|
| **Actor (left)** | Action-oriented, imperative | `"I'll read the latest message from right to understand their perspective."` | Starts with `read_history` or `speak` |
| **Spectator (right)** | Observational, reflective | `"Left seems focused on understanding context first. I notice they're asking about workspace state."` | Contains `observe` or `respond_to_left` |

### Mock Tool Call Sequences (D-17)

**Minimal sequence (actor → actor→spectator → spectator → switch):**

1. **Actor receives user message** → `read_history(side: "actor")` → `speak(...)`
2. **Spectator observes** → `read_history(side: "spectator")` → `speak_observation(...)`
3. **Switch roles** → Both call `switch_roles()`, baton swaps

**Tool definitions exercised in tests:**
- `read_history(side: "left" | "right")` — Read context from specified worker
- `speak(message: string)` — Respond to user
- `switch_roles()` — Request baton switch
- `observe(...)` — Spectator observation (no mutation)

---

## Test Harness Visual Assertion Points

### Assertion Point 1: Process Health (TEST-01)

**Assertion target:** HTTP `/health` endpoints return 200

```
GET http://127.0.0.1:4337/health                 → 200 OK (orchestrator)
GET http://127.0.0.1:8001/health                 → 200 OK (left worker)
GET http://127.0.0.1:8002/health                 → 200 OK (right worker)
GET http://127.0.0.1:9001/health                 → 200 OK (TUI adapter)
```

**Timeout:** 5 seconds per process, 500ms polling interval

### Assertion Point 2: Context File Presence (TEST-02)

**Assertion target:** Context files exist and contain valid JSONL

```
workspace/left/context.jsonl  → exists, readable, ≥1 entry
workspace/right/context.jsonl → exists, readable, ≥1 entry
```

**Entry format:** One JSON object per line (OpenAIMessage schema)

```json
{"role":"system","content":"Workspace initialized","id":"...","timestamp":"..."}
{"role":"assistant","content":"I read the context","tool_calls":[...],"id":"..."}
```

**Timeout:** 3 seconds for first context entry to appear after message send

### Assertion Point 3: Backchannel Appearance (TEST-02)

**Assertion target:** Backchannel entries written to file

```
workspace/conversations/backchannel.txt → readable, ≥2 lines (left + right messages)
```

**Format:** Conversation protocol (JSONL or line-based, per 03-CONTEXT.md)

### Assertion Point 4: Baton State (TEST-03)

**Assertion target:** Baton field in registry reflects role switch

**Registry check:**

```
GET http://127.0.0.1:4337/registry → JSON with ProcessEntry array

Expected before switch:
{
  "dyad": "test-dyad",
  "side": "left",
  "process_type": "Worker",
  "baton": "Actor"  // ← Left is actor
}

Expected after switch (via tool call or manual trigger):
{
  "dyad": "test-dyad",
  "side": "left",
  "process_type": "Worker",
  "baton": "Spectator"  // ← Left is now spectator
}
```

**Timeout:** 2 seconds for baton swap to complete after `switch_roles()` tool call

### Assertion Point 5: TUI Display Update (TEST-01, visual only)

**Assertion target:** TUI header shows baton state change

**Before test:** Header line 2:
```
Actor: left  Spectator: right
```

**After role switch (manual inspection or screenshotable):** Header line 2:
```
Actor: right  Spectator: left
```

---

## Color Palette (16-color ANSI)

Terminal colors follow xterm 256-color scheme, degraded to 16 for wide compatibility:

| Color | ANSI Code | Usage | RGB (if 256-color) |
|-------|-----------|-------|---------------------|
| Black | 0 | Text background, unused | #000000 |
| Red | 1 | Not used currently | #800000 |
| Green | 2 | Assistant messages (`asst>`) | #00AA00 |
| Yellow | 3 | Dyad/channel labels, tool calls, system messages | #AAAA00 |
| Blue | 4 | Tool result prefix | #0000AA |
| Magenta | 5 | Backchannel prefix | #AA00AA |
| Cyan | 6 | User messages (`you>`), status indicator, title background | #00AAAA |
| White | 7 | Default content, borders | #FFFFFF |
| Dark Gray | 8 | Timestamps, metadata, truncated content | #555555 |

**Title bar background:** Cyan (Color::Cyan)
**Title bar text:** Black (Color::Black)

---

## Typography & Text Rendering

Terminal UI has no fonts — all text is monospace (terminal default).

| Element | Weight | Modifier | Notes |
|---------|--------|----------|-------|
| Title bar | N/A | Bold | `BOLD` modifier set in ratatui Style |
| Headers/labels | N/A | None | Plain text |
| Active interactive | N/A | Underline | Not used (terminal default doesn't highlight input) |
| Emphasis | N/A | Bold | Used for role prefixes in tool calls |

**Line height:** 1 (no line spacing in terminal)
**Tab width:** 4 spaces (standard)

---

## Spacing Rules (Terminal Grid)

Terminal UI is grid-based (character cells). No pixel-based spacing.

| Spacing | Value | Usage |
|---------|-------|-------|
| Margin (header) | 1 char left/right | Border spacing |
| Margin (messages) | 1 char left/right | List padding |
| Margin (input) | 1 char left/right | Input field margin |
| Gap (header components) | 1 space | Between title, dyad, channel, status |
| Indent (wrapped messages) | 11 chars | Aligns with timestamp width |
| Padding (tool args) | Truncate to 30 chars | Inline display limit |

**Borders:** Single-line box (ratatui `Borders::ALL`), no multi-line borders

---

## Copywriting Contract

| Element | Copy | Notes |
|---------|------|-------|
| Title bar | `" River Mock Adapter "` | Static, cyan background |
| Status (connected) | `[connected]` | When worker endpoint registered |
| Status (waiting) | `[waiting...]` | During startup before registration |
| System prefix | `[sys]` | Alerts, errors, lifecycle events |
| User prompt | `>` | Single char, cyan color |
| Backchannel command | `/bc {text}` | Prefix to write to backchannel |
| No worker message | `"No worker endpoint configured"` | If worker registration fails |
| Invalid command | Not applicable | TUI accepts any text; `/` prefix only for backchannel |

---

## Registry Safety

| Registry | Blocks Used | Safety Gate |
|----------|-------------|-------------|
| ratatui official | Block, List, Paragraph, Layout, Style, Color | Built-in library, no external registry |
| crossterm official | Terminal control, event handling | Built-in library, no external registry |

No third-party blocks or registries. TUI uses only workspace dependencies (ratatui, crossterm) verified in Cargo.toml.

---

## Checker Sign-Off

- [ ] Dimension 1 Copywriting: PASS
- [ ] Dimension 2 Visuals: PASS (terminal layout, baton header, color scheme)
- [ ] Dimension 3 Color: PASS (ANSI 16-color palette defined)
- [ ] Dimension 4 Typography: PASS (monospace, weights, emphasis)
- [ ] Dimension 5 Spacing: PASS (grid-based, character cell rules)
- [ ] Dimension 6 Registry Safety: PASS (no third-party blocks)

**Approval:** pending (awaiting gsd-ui-checker verification)

---

## Pre-Population Sources

| Source | Decisions Used | Count |
|--------|----------------|-------|
| 04-CONTEXT.md (locked) | D-09 (baton header), D-10 (backchannel bidirectional), D-15–D-17 (mock LLM response templates) | 6 |
| 04-RESEARCH.md | TUI architecture patterns, color scheme, layout constraints | 3 |
| crates/river-tui/src/tui.rs | Existing header format, message coloring, layout structure | Existing code reviewed |
| User input (Claude's discretion) | Mock response templates, assertion point timeouts, backchannel display rules | 5 |

**All pre-locked decisions preserved.** No contradictions detected.

---

## Notes

- TUI enhancement (D-09, D-10) is a code change to `crates/river-tui/src/tui.rs` (rendered by executor phase)
- Mock LLM endpoint (D-15–D-17) is test-only fixture, not production code
- Integration tests assert on rendered output via context.jsonl polling, not terminal screenshot parsing
- Terminal emulation in tests: stdout/stderr capture sufficient; no xterm simulation needed
