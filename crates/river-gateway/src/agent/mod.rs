//! Agent (I) — the acting self
//!
//! Currently contains context assembly. The full agent task comes in Phase 5.

pub mod context;

pub use context::{ContextAssembler, ContextBudget, AssembledContext, LayerStats};
