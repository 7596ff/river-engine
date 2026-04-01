# Phase 2: Flash Queue

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the TTL-based memory surfacing mechanism. Flashes are curated memories pushed by the spectator (or manually in testing) that appear in the agent's warm context and expire after N turns or a time duration.

**Architecture:** In-memory queue with TTL tracking. No spectator yet — manual push for testing. Queue is thread-safe (Arc<RwLock>), read non-destructively by context assembler.

**Tech Stack:** tokio, chrono, river-core Snowflake

**Depends on:** Phase 0 (clean crate boundaries)

---

## File Structure

**New files:**
- `crates/river-gateway/src/flash/mod.rs` — Flash struct, FlashTTL, FlashQueue
- `crates/river-gateway/src/flash/ttl.rs` — TTL tracking and expiry logic

**Modified files:**
- `crates/river-gateway/src/lib.rs` — add flash module

---

## Task 1: Flash Types

- [ ] **Step 1: Create flash/mod.rs**

```rust
//! Flash queue — TTL-based memory surfacing

pub mod ttl;

use chrono::{DateTime, Utc};
use river_core::Snowflake;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A flash: a curated memory surfaced into warm context
#[derive(Debug, Clone)]
pub struct Flash {
    /// Unique ID
    pub id: String,
    /// Full text of the note (not a summary)
    pub content: String,
    /// Source path in embeddings/ (for dedup)
    pub source: String,
    /// Time-to-live
    pub ttl: FlashTTL,
    /// When this flash was pushed
    pub created: DateTime<Utc>,
}

/// How a flash expires
#[derive(Debug, Clone)]
pub enum FlashTTL {
    /// Expires after N agent turns
    Turns(u8),
    /// Expires after a duration
    Duration(std::time::Duration),
}

/// Thread-safe flash queue
#[derive(Debug, Clone)]
pub struct FlashQueue {
    inner: Arc<RwLock<FlashQueueInner>>,
    max_size: usize,
}

#[derive(Debug, Default)]
struct FlashQueueInner {
    flashes: Vec<FlashEntry>,
}

#[derive(Debug)]
struct FlashEntry {
    flash: Flash,
    remaining_turns: Option<u8>,
    expires_at: Option<DateTime<Utc>>,
}

impl FlashQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(FlashQueueInner::default())),
            max_size,
        }
    }

    /// Push a flash onto the queue. Refreshes TTL if duplicate source.
    pub async fn push(&self, flash: Flash) {
        let mut inner = self.inner.write().await;

        // Check for duplicate source — refresh TTL
        if let Some(existing) = inner.flashes.iter_mut().find(|e| e.flash.source == flash.source) {
            existing.flash = flash.clone();
            existing.remaining_turns = match &flash.ttl {
                FlashTTL::Turns(n) => Some(*n),
                FlashTTL::Duration(_) => None,
            };
            existing.expires_at = match &flash.ttl {
                FlashTTL::Turns(_) => None,
                FlashTTL::Duration(d) => Some(Utc::now() + chrono::Duration::from_std(*d).unwrap_or_default()),
            };
            return;
        }

        // Enforce max size (drop oldest)
        if inner.flashes.len() >= self.max_size {
            inner.flashes.remove(0);
        }

        let entry = FlashEntry {
            remaining_turns: match &flash.ttl {
                FlashTTL::Turns(n) => Some(*n),
                FlashTTL::Duration(_) => None,
            },
            expires_at: match &flash.ttl {
                FlashTTL::Turns(_) => None,
                FlashTTL::Duration(d) => Some(Utc::now() + chrono::Duration::from_std(*d).unwrap_or_default()),
            },
            flash,
        };

        inner.flashes.push(entry);
    }

    /// Get all active (non-expired) flashes. Non-destructive read.
    pub async fn active(&self) -> Vec<Flash> {
        let inner = self.inner.read().await;
        let now = Utc::now();

        inner.flashes.iter()
            .filter(|e| {
                if let Some(remaining) = e.remaining_turns {
                    if remaining == 0 { return false; }
                }
                if let Some(expires_at) = e.expires_at {
                    if now >= expires_at { return false; }
                }
                true
            })
            .map(|e| e.flash.clone())
            .collect()
    }

    /// Decrement turn-based TTLs and remove expired entries.
    /// Call this at the start of each agent turn.
    pub async fn tick_turn(&self) {
        let mut inner = self.inner.write().await;
        let now = Utc::now();

        // Decrement turn counters
        for entry in &mut inner.flashes {
            if let Some(ref mut remaining) = entry.remaining_turns {
                *remaining = remaining.saturating_sub(1);
            }
        }

        // Remove expired
        inner.flashes.retain(|e| {
            if let Some(remaining) = e.remaining_turns {
                if remaining == 0 { return false; }
            }
            if let Some(expires_at) = e.expires_at {
                if now >= expires_at { return false; }
            }
            true
        });
    }

    /// Number of active flashes
    pub async fn len(&self) -> usize {
        self.inner.read().await.flashes.len()
    }

    /// Clear all flashes
    pub async fn clear(&self) {
        self.inner.write().await.flashes.clear();
    }
}
```

- [ ] **Step 2: Create flash/ttl.rs (re-export, any extra TTL utilities)**

```rust
//! TTL utilities for flash queue
//! Currently the core logic is in mod.rs. This module is reserved for
//! more complex TTL behaviors (e.g., decay curves, priority weighting).

pub use super::{FlashTTL, Flash};
```

- [ ] **Step 3: Add to lib.rs**

```rust
pub mod flash;
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p river-gateway
```

- [ ] **Step 5: Commit**

```bash
git add crates/river-gateway/src/flash/
git commit -m "feat(gateway): add flash queue with TTL-based expiry"
```

---

## Task 2: Flash Queue Tests

- [ ] **Step 1: Write unit tests**

Add to `flash/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_push_and_active() {
        let queue = FlashQueue::new(20);

        queue.push(Flash {
            id: "f1".into(),
            content: "Note about z-index".into(),
            source: "notes/z-index.md".into(),
            ttl: FlashTTL::Turns(3),
            created: Utc::now(),
        }).await;

        let active = queue.active().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content, "Note about z-index");
    }

    #[tokio::test]
    async fn test_turn_ttl_expiry() {
        let queue = FlashQueue::new(20);

        queue.push(Flash {
            id: "f1".into(),
            content: "Short-lived".into(),
            source: "notes/temp.md".into(),
            ttl: FlashTTL::Turns(2),
            created: Utc::now(),
        }).await;

        queue.tick_turn().await; // remaining: 1
        assert_eq!(queue.active().await.len(), 1);

        queue.tick_turn().await; // remaining: 0, removed
        assert_eq!(queue.active().await.len(), 0);
    }

    #[tokio::test]
    async fn test_duplicate_refreshes_ttl() {
        let queue = FlashQueue::new(20);

        queue.push(Flash {
            id: "f1".into(),
            content: "Original".into(),
            source: "notes/x.md".into(),
            ttl: FlashTTL::Turns(2),
            created: Utc::now(),
        }).await;

        queue.tick_turn().await; // remaining: 1

        // Push duplicate source — should refresh
        queue.push(Flash {
            id: "f1-refreshed".into(),
            content: "Updated".into(),
            source: "notes/x.md".into(),
            ttl: FlashTTL::Turns(3),
            created: Utc::now(),
        }).await;

        // Should still be 1 flash, but with refreshed TTL
        assert_eq!(queue.len().await, 1);

        queue.tick_turn().await; // remaining: 2
        queue.tick_turn().await; // remaining: 1
        assert_eq!(queue.active().await.len(), 1); // Still alive

        queue.tick_turn().await; // remaining: 0
        assert_eq!(queue.active().await.len(), 0);
    }

    #[tokio::test]
    async fn test_max_size() {
        let queue = FlashQueue::new(2);

        for i in 0..3 {
            queue.push(Flash {
                id: format!("f{}", i),
                content: format!("Flash {}", i),
                source: format!("notes/{}.md", i),
                ttl: FlashTTL::Turns(10),
                created: Utc::now(),
            }).await;
        }

        // Only 2 should remain (oldest dropped)
        assert_eq!(queue.len().await, 2);
        let active = queue.active().await;
        assert_eq!(active[0].id, "f1"); // f0 was dropped
        assert_eq!(active[1].id, "f2");
    }

    #[tokio::test]
    async fn test_duration_ttl() {
        let queue = FlashQueue::new(20);

        queue.push(Flash {
            id: "f1".into(),
            content: "Time-based".into(),
            source: "notes/time.md".into(),
            ttl: FlashTTL::Duration(std::time::Duration::from_millis(50)),
            created: Utc::now(),
        }).await;

        assert_eq!(queue.active().await.len(), 1);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // Need to tick to clean up, or active() filters
        assert_eq!(queue.active().await.len(), 0);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p river-gateway flash
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "test(gateway): flash queue tests for TTL expiry, dedup, max size"
```

---

## Summary

Phase 2 is small and focused:
1. **Flash struct** with Turns and Duration TTL modes
2. **FlashQueue** — thread-safe, deduplicates by source, enforces max size
3. **tick_turn()** — called per agent turn to decrement and expire

Total: 2 tasks, ~10 steps. The queue is standalone — no spectator needed yet. Testing via manual pushes.
