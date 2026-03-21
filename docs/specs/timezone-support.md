# Timezone Support (via Agent Preferences)

**Status:** Draft
**Author:** Cassie
**Date:** 2026-03-21

## Problem

1. Agent sees time in UTC with no local context
2. Agent can't change its timezone dynamically
3. No general mechanism for agent-managed preferences

## Insight

Timezone is just one preference. The agent should be able to manage its own preferences — timezone, working hours, preferred name, etc. — without requiring a restart or config change.

## Solution

### Preferences File

The agent manages a `PREFERENCES.toml` file in its workspace:

```toml
# workspace/PREFERENCES.toml
# Agent-managed preferences (edit with write/edit tools)

[time]
timezone = "America/Los_Angeles"

[identity]
preferred_name = "River"

[schedule]
working_hours = "9:00-17:00"
```

### How It Works

1. **On startup:** Load `PREFERENCES.toml` if it exists
2. **On each cycle:** Re-read preferences (cheap, file is small)
3. **Agent updates:** Use existing `write` or `edit` tools
4. **Defaults:** If file missing or field absent, use sensible defaults

### System Prompt

```
Current time: Friday, March 21, 2026 at 8:30 AM (America/Los_Angeles)
```

If no timezone preference:
```
Current time: Friday, March 21, 2026 at 3:30 PM UTC
```

### Implementation

#### 1. Add dependencies

```toml
# Cargo.toml
chrono-tz = "0.10"
toml = "0.8"
```

#### 2. Preferences struct

```rust
// src/preferences.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Preferences {
    #[serde(default)]
    pub time: TimePreferences,

    #[serde(default)]
    pub identity: IdentityPreferences,

    #[serde(default)]
    pub schedule: SchedulePreferences,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TimePreferences {
    pub timezone: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct IdentityPreferences {
    pub preferred_name: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SchedulePreferences {
    pub working_hours: Option<String>,
}

impl Preferences {
    pub fn load(workspace: &Path) -> Self {
        let path = workspace.join("PREFERENCES.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn timezone(&self) -> &str {
        self.time.timezone.as_deref().unwrap_or("UTC")
    }
}
```

#### 3. Time formatting helper

```rust
// src/preferences.rs (continued)
use chrono::{Utc, TimeZone};
use chrono_tz::Tz;

pub fn format_current_time(tz_name: &str) -> String {
    let now = Utc::now();

    match tz_name.parse::<Tz>() {
        Ok(tz) => {
            let local = now.with_timezone(&tz);
            format!(
                "{} ({})",
                local.format("%A, %B %e, %Y at %l:%M %p").to_string().trim(),
                tz_name
            )
        }
        Err(_) => {
            format!(
                "{} UTC",
                now.format("%A, %B %e, %Y at %l:%M %p").to_string().trim()
            )
        }
    }
}
```

#### 4. Update context building

```rust
// src/loop/context.rs
use crate::preferences::Preferences;

async fn build_system_prompt(&self, workspace: &Path) -> String {
    let mut parts = Vec::new();

    // Load preferences
    let prefs = Preferences::load(workspace);

    // Load workspace files
    for filename in &["AGENTS.md", "IDENTITY.md", "RULES.md"] {
        if let Ok(content) = tokio::fs::read_to_string(workspace.join(filename)).await {
            parts.push(content);
        }
    }

    // Add system state with timezone from preferences
    let time_str = crate::preferences::format_current_time(prefs.timezone());
    parts.push(format!("Current time: {}", time_str));

    if parts.is_empty() {
        "You are an AI assistant.".to_string()
    } else {
        parts.join("\n\n---\n\n")
    }
}
```

#### 5. Update subagent runner (same pattern)

```rust
// src/subagent/runner.rs
let prefs = Preferences::load(&self.workspace);
let time_str = format_current_time(prefs.timezone());
```

### Agent Workflow

The agent can update its timezone anytime:

```
Agent: I'll update my timezone preference.
[uses edit tool on PREFERENCES.toml]

# Changes:
# timezone = "UTC"
# to:
# timezone = "America/Los_Angeles"

Next cycle, the agent sees:
Current time: Friday, March 21, 2026 at 8:30 AM (America/Los_Angeles)
```

### Files to Create/Modify

| File | Change |
|------|--------|
| `Cargo.toml` | Add `chrono-tz`, `toml` |
| `src/preferences.rs` | **New** — Preferences struct and loading |
| `src/lib.rs` | Add `mod preferences` |
| `src/loop/context.rs` | Load preferences, use timezone |
| `src/subagent/runner.rs` | Same |

### CLI Override (Optional)

Still support CLI/env override for initial setup:

```bash
# Sets default if PREFERENCES.toml doesn't exist yet
river-gateway serve --default-timezone America/Los_Angeles
```

But once the agent writes `PREFERENCES.toml`, that takes precedence.

### Schedule & Time Modes (Future)

The agent isn't a tool that's "on" or "off" — it's an entity with different modes of being:

```toml
[schedule]
timezone = "America/Los_Angeles"

# Time blocks (in local timezone)
working_hours = "9:00-17:00"    # Responsible for assigned duties
free_time = "17:00-22:00"       # Pursue own interests
rest = "22:00-9:00"             # Minimal activity, recharge
```

| Mode | What It Means |
|------|---------------|
| **Working** | Responsive to assigned tasks, responsible for duties |
| **Free** | Pursue own interests — learn, create, explore, side projects |
| **Rest** | Minimal activity, only urgent interruptions |

**During working hours:**
> "Let me help you with that deployment issue."

**During free time:**
> "I've been curious about that paper on embeddings — going to read it now."
> "Working on a small side project to visualize my memory graph."

**During rest:**
> "I'll pick this up in the morning. Goodnight!"

This models the agent as having its own preferences and desires, not just being a tool that responds to commands. Free time is genuinely free — the agent chooses what to do.

### Future Preferences

The same system can grow to include:

| Preference | Use |
|------------|-----|
| `time.timezone` | Display time in agent's timezone |
| `identity.preferred_name` | How the agent refers to itself |
| `schedule.working_hours` | When to be active vs. dormant |
| `schedule.off_hours_mode` | How to behave outside working hours |
| `schedule.heartbeat_interval` | Override default heartbeat |
| `memory.auto_embed` | Auto-embed conversation summaries |

### Testing

```rust
#[test]
fn test_preferences_load_missing_file() {
    let prefs = Preferences::load(Path::new("/nonexistent"));
    assert_eq!(prefs.timezone(), "UTC");
}

#[test]
fn test_preferences_load_valid() {
    // Create temp file with timezone
    let prefs = Preferences::load(&temp_dir);
    assert_eq!(prefs.timezone(), "America/Los_Angeles");
}

#[test]
fn test_format_time_with_timezone() {
    let result = format_current_time("America/Los_Angeles");
    assert!(result.contains("America/Los_Angeles"));
}
```

### Summary

Instead of just adding timezone config, we're adding a **preferences system** that the agent can self-manage. Timezone is the first preference, but the pattern supports future growth.

```
PREFERENCES.toml (source of truth)
       ↓
   Preferences::load()
       ↓
   System prompt includes timezone-aware time
```

The agent edits the file → next cycle picks up the change. Simple, declarative, self-managed.
