# River Engine: Plan 1 - Core Libraries

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the foundational libraries that all River components depend on: Snowflake IDs, shared types, configuration, and error handling.

**Architecture:** A Cargo workspace with a `river-core` crate containing pure, well-tested library code. No I/O, no side effects — just data types and algorithms.

**Tech Stack:** Rust, Cargo workspace, no external dependencies except `serde` for serialization and `thiserror` for error handling.

**Spec Reference:** `/home/cassie/river-engine/docs/superpowers/specs/2026-03-16-river-engine-design.md`

---

## Chunk 1: Project Setup

### Task 1: Initialize Cargo Workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/river-core/Cargo.toml`
- Create: `crates/river-core/src/lib.rs`

- [ ] **Step 1: Create workspace Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/*",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
authors = ["River Engine Contributors"]

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
```

- [ ] **Step 2: Create river-core crate directory**

Run: `mkdir -p crates/river-core/src`

- [ ] **Step 3: Create river-core Cargo.toml**

```toml
[package]
name = "river-core"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Core types and utilities for River Engine"

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true

[dev-dependencies]
```

- [ ] **Step 4: Create lib.rs with module structure**

```rust
//! River Core - foundational types for River Engine

pub mod snowflake;
pub mod types;
pub mod config;
pub mod error;

pub use snowflake::Snowflake;
pub use types::*;
pub use error::RiverError;
```

- [ ] **Step 5: Create empty module files**

Run: `touch crates/river-core/src/{snowflake,types,config,error}.rs`

- [ ] **Step 6: Verify workspace compiles**

Run: `cargo check`
Expected: Compiles with warnings about empty modules

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(core): initialize cargo workspace with river-core crate"
```

---

## Chunk 2: Snowflake ID Implementation

### Task 2: Agent Birth Encoding

Per spec Section 6: Agent birth is 36 bits encoding yyyymmddhhmmss.

**Files:**
- Modify: `crates/river-core/src/snowflake.rs`
- Create: `crates/river-core/src/snowflake/birth.rs`

- [ ] **Step 1: Write failing test for AgentBirth parsing**

In `crates/river-core/src/snowflake.rs`:

```rust
//! Snowflake ID implementation
//!
//! 128-bit sortable unique identifiers:
//! - 64 bits: timestamp (microseconds since agent birth)
//! - 36 bits: agent birth (yyyymmddhhmmss packed)
//! - 8 bits: type
//! - 20 bits: sequence

mod birth;

pub use birth::AgentBirth;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_birth_from_datetime() {
        // March 16, 2026, 14:32:00
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();

        assert_eq!(birth.year(), 2026);
        assert_eq!(birth.month(), 3);
        assert_eq!(birth.day(), 16);
        assert_eq!(birth.hour(), 14);
        assert_eq!(birth.minute(), 32);
        assert_eq!(birth.second(), 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-core test_agent_birth`
Expected: FAIL - `AgentBirth` not found

- [ ] **Step 3: Implement AgentBirth struct**

In `crates/river-core/src/snowflake/birth.rs`:

```rust
//! Agent birth timestamp encoding
//!
//! 36 bits packed as:
//! - Year offset from 2000: 10 bits (0-999)
//! - Month: 4 bits (1-12)
//! - Day: 5 bits (1-31)
//! - Hour: 5 bits (0-23)
//! - Minute: 6 bits (0-59)
//! - Second: 6 bits (0-59)

use serde::{Deserialize, Serialize};

/// Agent birth timestamp, packed into 36 bits
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentBirth(u64); // Only lower 36 bits used

impl AgentBirth {
    /// Create a new AgentBirth from date/time components
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, AgentBirthError> {
        // Validate ranges
        if year < 2000 || year > 2999 {
            return Err(AgentBirthError::InvalidYear(year));
        }
        if month < 1 || month > 12 {
            return Err(AgentBirthError::InvalidMonth(month));
        }
        if day < 1 || day > 31 {
            return Err(AgentBirthError::InvalidDay(day));
        }
        if hour > 23 {
            return Err(AgentBirthError::InvalidHour(hour));
        }
        if minute > 59 {
            return Err(AgentBirthError::InvalidMinute(minute));
        }
        if second > 59 {
            return Err(AgentBirthError::InvalidSecond(second));
        }

        let year_offset = (year - 2000) as u64;

        // Pack into 36 bits:
        // [year:10][month:4][day:5][hour:5][minute:6][second:6]
        let packed = (year_offset << 26)
            | ((month as u64) << 22)
            | ((day as u64) << 17)
            | ((hour as u64) << 12)
            | ((minute as u64) << 6)
            | (second as u64);

        Ok(Self(packed))
    }

    /// Extract year (2000-2999)
    pub fn year(&self) -> u16 {
        ((self.0 >> 26) & 0x3FF) as u16 + 2000
    }

    /// Extract month (1-12)
    pub fn month(&self) -> u8 {
        ((self.0 >> 22) & 0x0F) as u8
    }

    /// Extract day (1-31)
    pub fn day(&self) -> u8 {
        ((self.0 >> 17) & 0x1F) as u8
    }

    /// Extract hour (0-23)
    pub fn hour(&self) -> u8 {
        ((self.0 >> 12) & 0x1F) as u8
    }

    /// Extract minute (0-59)
    pub fn minute(&self) -> u8 {
        ((self.0 >> 6) & 0x3F) as u8
    }

    /// Extract second (0-59)
    pub fn second(&self) -> u8 {
        (self.0 & 0x3F) as u8
    }

    /// Get the raw 36-bit packed value
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Create from raw packed value (lower 36 bits)
    pub fn from_raw(raw: u64) -> Self {
        Self(raw & 0xF_FFFF_FFFF) // Mask to 36 bits
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentBirthError {
    #[error("Invalid year {0}: must be 2000-2999")]
    InvalidYear(u16),
    #[error("Invalid month {0}: must be 1-12")]
    InvalidMonth(u8),
    #[error("Invalid day {0}: must be 1-31")]
    InvalidDay(u8),
    #[error("Invalid hour {0}: must be 0-23")]
    InvalidHour(u8),
    #[error("Invalid minute {0}: must be 0-59")]
    InvalidMinute(u8),
    #[error("Invalid second {0}: must be 0-59")]
    InvalidSecond(u8),
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-core test_agent_birth`
Expected: PASS

- [ ] **Step 5: Add roundtrip test**

Add to `crates/river-core/src/snowflake.rs` tests:

```rust
    #[test]
    fn test_agent_birth_roundtrip() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 45).unwrap();
        let raw = birth.as_u64();
        let restored = AgentBirth::from_raw(raw);

        assert_eq!(birth, restored);
        assert_eq!(restored.year(), 2026);
        assert_eq!(restored.month(), 3);
        assert_eq!(restored.day(), 16);
        assert_eq!(restored.hour(), 14);
        assert_eq!(restored.minute(), 32);
        assert_eq!(restored.second(), 45);
    }

    #[test]
    fn test_agent_birth_validation() {
        assert!(AgentBirth::new(1999, 1, 1, 0, 0, 0).is_err()); // Year too low
        assert!(AgentBirth::new(3000, 1, 1, 0, 0, 0).is_err()); // Year too high
        assert!(AgentBirth::new(2026, 0, 1, 0, 0, 0).is_err());  // Month 0
        assert!(AgentBirth::new(2026, 13, 1, 0, 0, 0).is_err()); // Month 13
        assert!(AgentBirth::new(2026, 1, 0, 0, 0, 0).is_err());  // Day 0
        assert!(AgentBirth::new(2026, 1, 32, 0, 0, 0).is_err()); // Day 32
        assert!(AgentBirth::new(2026, 1, 1, 24, 0, 0).is_err()); // Hour 24
        assert!(AgentBirth::new(2026, 1, 1, 0, 60, 0).is_err()); // Minute 60
        assert!(AgentBirth::new(2026, 1, 1, 0, 0, 60).is_err()); // Second 60
    }
```

- [ ] **Step 6: Run all tests**

Run: `cargo test -p river-core`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(core): implement AgentBirth 36-bit timestamp encoding"
```

---

### Task 3: Snowflake Type Enum

Per spec Section 6: 8-bit type field with defined values.

**Files:**
- Create: `crates/river-core/src/snowflake/types.rs`
- Modify: `crates/river-core/src/snowflake.rs`

- [ ] **Step 1: Write failing test for SnowflakeType**

Add to `crates/river-core/src/snowflake.rs`:

```rust
mod types;
pub use types::SnowflakeType;

// In tests module:
    #[test]
    fn test_snowflake_type_values() {
        assert_eq!(SnowflakeType::Message as u8, 0x01);
        assert_eq!(SnowflakeType::Embedding as u8, 0x02);
        assert_eq!(SnowflakeType::Session as u8, 0x03);
        assert_eq!(SnowflakeType::Subagent as u8, 0x04);
        assert_eq!(SnowflakeType::ToolCall as u8, 0x05);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-core test_snowflake_type`
Expected: FAIL - `SnowflakeType` not found

- [ ] **Step 3: Implement SnowflakeType enum**

In `crates/river-core/src/snowflake/types.rs`:

```rust
//! Snowflake type discriminator

use serde::{Deserialize, Serialize};

/// Type of entity a snowflake ID refers to (8 bits)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SnowflakeType {
    Message = 0x01,
    Embedding = 0x02,
    Session = 0x03,
    Subagent = 0x04,
    ToolCall = 0x05,
}

impl SnowflakeType {
    /// Convert from raw byte value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Message),
            0x02 => Some(Self::Embedding),
            0x03 => Some(Self::Session),
            0x04 => Some(Self::Subagent),
            0x05 => Some(Self::ToolCall),
            _ => None,
        }
    }

    /// Convert to raw byte value
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-core test_snowflake_type`
Expected: PASS

- [ ] **Step 5: Add from_u8 test**

```rust
    #[test]
    fn test_snowflake_type_from_u8() {
        assert_eq!(SnowflakeType::from_u8(0x01), Some(SnowflakeType::Message));
        assert_eq!(SnowflakeType::from_u8(0x02), Some(SnowflakeType::Embedding));
        assert_eq!(SnowflakeType::from_u8(0x00), None); // Invalid
        assert_eq!(SnowflakeType::from_u8(0xFF), None); // Invalid
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p river-core`
Expected: All pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(core): implement SnowflakeType enum"
```

---

### Task 4: Snowflake ID Struct

The main 128-bit snowflake implementation.

**Files:**
- Create: `crates/river-core/src/snowflake/id.rs`
- Modify: `crates/river-core/src/snowflake.rs`

- [ ] **Step 1: Write failing test for Snowflake creation**

Add to `crates/river-core/src/snowflake.rs`:

```rust
mod id;
pub use id::Snowflake;

// In tests:
    #[test]
    fn test_snowflake_creation() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();
        let snowflake = Snowflake::new(
            1000, // 1000 microseconds since birth
            birth,
            SnowflakeType::Message,
            0, // sequence
        );

        assert_eq!(snowflake.timestamp_micros(), 1000);
        assert_eq!(snowflake.agent_birth(), birth);
        assert_eq!(snowflake.snowflake_type(), SnowflakeType::Message);
        assert_eq!(snowflake.sequence(), 0);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-core test_snowflake_creation`
Expected: FAIL

- [ ] **Step 3: Implement Snowflake struct**

In `crates/river-core/src/snowflake/id.rs`:

```rust
//! 128-bit Snowflake ID
//!
//! Structure:
//! - Bits 127-64: timestamp (microseconds since agent birth)
//! - Bits 63-28: agent birth (36 bits)
//! - Bits 27-20: type (8 bits)
//! - Bits 19-0: sequence (20 bits)

use serde::{Deserialize, Serialize};
use super::{AgentBirth, SnowflakeType};

/// 128-bit snowflake ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Snowflake {
    /// Upper 64 bits: timestamp in microseconds since agent birth
    high: u64,
    /// Lower 64 bits: [birth:36][type:8][sequence:20]
    low: u64,
}

impl Snowflake {
    /// Create a new snowflake from components
    pub fn new(
        timestamp_micros: u64,
        agent_birth: AgentBirth,
        snowflake_type: SnowflakeType,
        sequence: u32,
    ) -> Self {
        let sequence = sequence & 0xFFFFF; // Mask to 20 bits

        // Pack lower 64 bits: [birth:36][type:8][sequence:20]
        let low = (agent_birth.as_u64() << 28)
            | ((snowflake_type.as_u8() as u64) << 20)
            | (sequence as u64);

        Self {
            high: timestamp_micros,
            low,
        }
    }

    /// Extract timestamp in microseconds since agent birth
    pub fn timestamp_micros(&self) -> u64 {
        self.high
    }

    /// Extract agent birth
    pub fn agent_birth(&self) -> AgentBirth {
        AgentBirth::from_raw(self.low >> 28)
    }

    /// Extract snowflake type
    pub fn snowflake_type(&self) -> SnowflakeType {
        let type_byte = ((self.low >> 20) & 0xFF) as u8;
        SnowflakeType::from_u8(type_byte).expect("Invalid snowflake type in ID")
    }

    /// Extract sequence number
    pub fn sequence(&self) -> u32 {
        (self.low & 0xFFFFF) as u32
    }

    /// Convert to 128-bit representation
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&self.high.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.low.to_be_bytes());
        bytes
    }

    /// Parse from 128-bit representation
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        let high = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
        let low = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
        Self { high, low }
    }
}

impl PartialOrd for Snowflake {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Snowflake {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by timestamp first (sortable by time)
        match self.high.cmp(&other.high) {
            std::cmp::Ordering::Equal => self.low.cmp(&other.low),
            ord => ord,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-core test_snowflake_creation`
Expected: PASS

- [ ] **Step 5: Add more tests**

```rust
    #[test]
    fn test_snowflake_bytes_roundtrip() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();
        let original = Snowflake::new(
            123456789,
            birth,
            SnowflakeType::Embedding,
            42,
        );

        let bytes = original.to_bytes();
        let restored = Snowflake::from_bytes(bytes);

        assert_eq!(original, restored);
    }

    #[test]
    fn test_snowflake_ordering() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();

        let early = Snowflake::new(100, birth, SnowflakeType::Message, 0);
        let later = Snowflake::new(200, birth, SnowflakeType::Message, 0);

        assert!(early < later);
    }

    #[test]
    fn test_snowflake_sequence_max() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();

        // Max sequence is 2^20 - 1 = 1048575
        let snowflake = Snowflake::new(0, birth, SnowflakeType::Message, 1048575);
        assert_eq!(snowflake.sequence(), 1048575);

        // Overflow wraps
        let snowflake = Snowflake::new(0, birth, SnowflakeType::Message, 1048576);
        assert_eq!(snowflake.sequence(), 0);
    }
```

- [ ] **Step 6: Run all tests**

Run: `cargo test -p river-core`
Expected: All pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(core): implement 128-bit Snowflake ID"
```

---

### Task 5: Snowflake Generator

Thread-safe generator for creating snowflakes.

**Files:**
- Create: `crates/river-core/src/snowflake/generator.rs`
- Modify: `crates/river-core/src/snowflake.rs`

- [ ] **Step 1: Write failing test for generator**

Add to `crates/river-core/src/snowflake.rs`:

```rust
mod generator;
pub use generator::SnowflakeGenerator;

// In tests:
    #[test]
    fn test_snowflake_generator_uniqueness() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let ids: Vec<_> = (0..1000)
            .map(|_| gen.next_id(SnowflakeType::Message))
            .collect();

        // All IDs should be unique
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-core test_snowflake_generator`
Expected: FAIL

- [ ] **Step 3: Implement SnowflakeGenerator**

In `crates/river-core/src/snowflake/generator.rs`:

```rust
//! Thread-safe snowflake ID generator

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use super::{AgentBirth, Snowflake, SnowflakeType};

/// Generator for unique snowflake IDs
pub struct SnowflakeGenerator {
    agent_birth: AgentBirth,
    birth_timestamp_micros: u64,
    last_timestamp: AtomicU64,
    sequence: AtomicU32,
}

impl SnowflakeGenerator {
    /// Create a new generator for an agent
    pub fn new(agent_birth: AgentBirth) -> Self {
        // Calculate birth timestamp in microseconds since UNIX epoch
        let birth_timestamp_micros = Self::birth_to_unix_micros(agent_birth);

        Self {
            agent_birth,
            birth_timestamp_micros,
            last_timestamp: AtomicU64::new(0),
            sequence: AtomicU32::new(0),
        }
    }

    /// Generate the next unique ID
    pub fn next_id(&self, snowflake_type: SnowflakeType) -> Snowflake {
        let now_micros = self.current_micros_since_birth();

        loop {
            let last = self.last_timestamp.load(Ordering::Acquire);

            if now_micros > last {
                // New timestamp, reset sequence
                if self.last_timestamp.compare_exchange(
                    last, now_micros, Ordering::Release, Ordering::Relaxed
                ).is_ok() {
                    self.sequence.store(0, Ordering::Release);
                    return Snowflake::new(now_micros, self.agent_birth, snowflake_type, 0);
                }
            } else {
                // Same or earlier timestamp, increment sequence
                let seq = self.sequence.fetch_add(1, Ordering::AcqRel);
                if seq < 0xFFFFF {
                    return Snowflake::new(last, self.agent_birth, snowflake_type, seq + 1);
                }
                // Sequence exhausted, spin until next microsecond
                std::hint::spin_loop();
            }
        }
    }

    /// Get current time in microseconds since agent birth
    fn current_micros_since_birth(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_micros() as u64;

        now.saturating_sub(self.birth_timestamp_micros)
    }

    /// Convert AgentBirth to microseconds since UNIX epoch
    fn birth_to_unix_micros(birth: AgentBirth) -> u64 {
        // Simple approximation - doesn't account for leap seconds etc.
        // For production, use a proper datetime library
        let year = birth.year() as u64;
        let month = birth.month() as u64;
        let day = birth.day() as u64;
        let hour = birth.hour() as u64;
        let minute = birth.minute() as u64;
        let second = birth.second() as u64;

        // Days since UNIX epoch (1970-01-01)
        // Simplified calculation
        let years_since_1970 = year - 1970;
        let leap_years = (years_since_1970 + 1) / 4; // Rough approximation
        let days = years_since_1970 * 365 + leap_years;

        // Add days for months (simplified, assumes 30-day months as approximation)
        let month_days = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        let days = days + month_days.get(month as usize - 1).copied().unwrap_or(0);
        let days = days + day - 1;

        // Convert to microseconds
        let seconds = days * 86400 + hour * 3600 + minute * 60 + second;
        seconds * 1_000_000
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p river-core test_snowflake_generator`
Expected: PASS

- [ ] **Step 5: Add ordering test**

```rust
    #[test]
    fn test_snowflake_generator_ordering() {
        let birth = AgentBirth::new(2026, 3, 16, 14, 32, 0).unwrap();
        let gen = SnowflakeGenerator::new(birth);

        let mut prev = gen.next_id(SnowflakeType::Message);
        for _ in 0..100 {
            let curr = gen.next_id(SnowflakeType::Message);
            assert!(curr > prev, "IDs should be monotonically increasing");
            prev = curr;
        }
    }
```

- [ ] **Step 6: Run all tests**

Run: `cargo test -p river-core`
Expected: All pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(core): implement SnowflakeGenerator"
```

---

## Chunk 3: Shared Types

### Task 6: Priority Enum

Per spec Section 8.2: Fixed priority tiers.

**Files:**
- Modify: `crates/river-core/src/types.rs`

- [ ] **Step 1: Write failing test**

In `crates/river-core/src/types.rs`:

```rust
//! Shared types for River Engine

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Interactive > Priority::Scheduled);
        assert!(Priority::Scheduled > Priority::Background);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p river-core test_priority`
Expected: FAIL

- [ ] **Step 3: Implement Priority enum**

```rust
/// Request priority level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Lowest priority - subagent work, monitoring
    Background = 0,
    /// Medium priority - heartbeats, timed tasks
    Scheduled = 1,
    /// Highest priority - user-initiated
    Interactive = 2,
}

impl Default for Priority {
    fn default() -> Self {
        Self::Background
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p river-core test_priority`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(core): implement Priority enum"
```

---

### Task 7: SubagentType Enum

Per spec Section 8.3: Task workers and long-running.

**Files:**
- Modify: `crates/river-core/src/types.rs`

- [ ] **Step 1: Write test**

```rust
    #[test]
    fn test_subagent_type() {
        let task: SubagentType = serde_json::from_str("\"task_worker\"").unwrap();
        assert_eq!(task, SubagentType::TaskWorker);

        let long: SubagentType = serde_json::from_str("\"long_running\"").unwrap();
        assert_eq!(long, SubagentType::LongRunning);
    }
```

- [ ] **Step 2: Implement SubagentType**

```rust
/// Type of subagent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentType {
    /// Short-lived, terminates on completion
    TaskWorker,
    /// Runs until explicitly stopped
    LongRunning,
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p river-core test_subagent`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(core): implement SubagentType enum"
```

---

### Task 8: ContextStatus Struct

Per spec Section 3.8: Context status in every tool response.

**Files:**
- Modify: `crates/river-core/src/types.rs`

- [ ] **Step 1: Write test**

```rust
    #[test]
    fn test_context_status() {
        let status = ContextStatus {
            used: 45000,
            limit: 65536,
        };
        assert!((status.percent() - 68.66).abs() < 0.01);
    }
```

- [ ] **Step 2: Implement ContextStatus**

```rust
/// Context window usage status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextStatus {
    /// Tokens used
    pub used: u64,
    /// Token limit
    pub limit: u64,
}

impl ContextStatus {
    /// Calculate percentage used
    pub fn percent(&self) -> f64 {
        if self.limit == 0 {
            0.0
        } else {
            (self.used as f64 / self.limit as f64) * 100.0
        }
    }

    /// Check if approaching limit (90%)
    pub fn is_near_limit(&self) -> bool {
        self.percent() >= 90.0
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p river-core test_context`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(core): implement ContextStatus struct"
```

---

## Chunk 4: Error Types

### Task 9: RiverError Enum

**Files:**
- Modify: `crates/river-core/src/error.rs`

- [ ] **Step 1: Implement error types**

In `crates/river-core/src/error.rs`:

```rust
//! Error types for River Engine

use thiserror::Error;

/// Top-level error type for River operations
#[derive(Debug, Error)]
pub enum RiverError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Tool execution error: {0}")]
    Tool(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Workspace error: {0}")]
    Workspace(String),

    #[error("Communication adapter error: {0}")]
    Adapter(String),

    #[error("Orchestrator error: {0}")]
    Orchestrator(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias for River operations
pub type RiverResult<T> = Result<T, RiverError>;
```

- [ ] **Step 2: Add test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = RiverError::Config("missing field".to_string());
        assert_eq!(err.to_string(), "Configuration error: missing field");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let river_err: RiverError = io_err.into();
        assert!(matches!(river_err, RiverError::Io(_)));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-core`
Expected: All pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(core): implement RiverError types"
```

---

## Chunk 5: Configuration Types

### Task 10: Agent Configuration

Per spec Section 12: NixOS module configuration.

**Files:**
- Modify: `crates/river-core/src/config.rs`

- [ ] **Step 1: Implement config types**

In `crates/river-core/src/config.rs`:

```rust
//! Configuration types for River Engine

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for a single agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name/identifier
    pub name: String,

    /// Workspace directory path
    pub workspace: PathBuf,

    /// Data directory for database, etc.
    pub data_dir: PathBuf,

    /// Primary model name
    pub primary_model: String,

    /// Context window limit in tokens
    pub context_limit: u64,

    /// Gateway port
    pub gateway_port: u16,

    /// Path to auth token file
    pub auth_token_file: Option<PathBuf>,

    /// Heartbeat configuration
    pub heartbeat: HeartbeatConfig,

    /// Embedding configuration
    pub embedding: EmbeddingConfig,
}

/// Heartbeat configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Default interval in minutes
    #[serde(default = "default_heartbeat_minutes")]
    pub default_minutes: u32,
}

fn default_heartbeat_minutes() -> u32 {
    45
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            default_minutes: default_heartbeat_minutes(),
        }
    }
}

/// Embedding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Auto-embed TTL in days
    #[serde(default = "default_ttl_days")]
    pub auto_embed_ttl_days: u32,
}

fn default_ttl_days() -> u32 {
    14
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            auto_embed_ttl_days: default_ttl_days(),
        }
    }
}

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Orchestrator port
    pub port: u16,

    /// Directory containing model files
    pub models_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_deserialize() {
        let json = r#"{
            "name": "thomas",
            "workspace": "/home/thomas/workspace",
            "data_dir": "/var/lib/river/thomas",
            "primary_model": "qwen3-32b",
            "context_limit": 65536,
            "gateway_port": 3000,
            "heartbeat": {},
            "embedding": {}
        }"#;

        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "thomas");
        assert_eq!(config.heartbeat.default_minutes, 45);
        assert_eq!(config.embedding.auto_embed_ttl_days, 14);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-core`
Expected: All pass

- [ ] **Step 3: Update lib.rs exports**

```rust
pub use config::{AgentConfig, HeartbeatConfig, EmbeddingConfig, OrchestratorConfig};
pub use error::{RiverError, RiverResult};
```

- [ ] **Step 4: Final test run**

Run: `cargo test -p river-core`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(core): implement configuration types"
```

---

## Final Verification

- [ ] **Run full test suite**

Run: `cargo test -p river-core`
Expected: All tests pass

- [ ] **Check for warnings**

Run: `cargo clippy -p river-core`
Expected: No warnings

- [ ] **Format code**

Run: `cargo fmt`

- [ ] **Final commit**

```bash
git add -A
git commit -m "chore(core): format and clean up"
```

---

## Summary

This plan implements:

| Component | Description |
|-----------|-------------|
| `AgentBirth` | 36-bit packed timestamp for agent creation |
| `SnowflakeType` | 8-bit type discriminator |
| `Snowflake` | 128-bit unique sortable ID |
| `SnowflakeGenerator` | Thread-safe ID generator |
| `Priority` | Request priority levels |
| `SubagentType` | Task worker vs long-running |
| `ContextStatus` | Context window usage tracking |
| `RiverError` | Error types |
| `AgentConfig` | Agent configuration |

**Next plan:** Plan 2 - Gateway Core (tool loop, sessions, database, file tools)
