//! Redis integration for working/medium-term memory

mod client;
mod working;
// mod medium_term; // TODO: uncomment in Task 9
// mod coordination; // TODO: uncomment in Task 9
// mod cache; // TODO: uncomment in Task 9

pub use client::{RedisClient, RedisConfig};
pub use working::{WorkingMemorySetTool, WorkingMemoryGetTool, WorkingMemoryDeleteTool};
// pub use medium_term::{MediumTermSetTool, MediumTermGetTool};
// pub use coordination::{ResourceLockTool, CounterIncrementTool, CounterGetTool};
// pub use cache::{CacheSetTool, CacheGetTool};
