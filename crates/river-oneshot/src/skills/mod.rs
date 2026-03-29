//! Skill system for river-oneshot.
//!
//! Provides:
//! - Skill trait
//! - SkillRegistry for lookup
//! - Built-in skills (shell, http, file_io, summarize)
//!
//! Implemented in Phase 3.

use anyhow::Result;
use async_trait::async_trait;

use crate::types::{ActionResult, SkillContext, ToolDef};

/// Trait for skills.
#[async_trait]
pub trait Skill: Send + Sync {
    /// Get the skill name.
    fn name(&self) -> &str;

    /// Get the skill description.
    fn description(&self) -> &str;

    /// Get the tool definition for LLM.
    fn tool_definition(&self) -> ToolDef;

    /// Execute the skill with parameters.
    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &SkillContext,
    ) -> Result<ActionResult>;
}

/// Registry of available skills.
pub struct SkillRegistry {
    skills: Vec<Box<dyn Skill>>,
}

impl SkillRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self { skills: vec![] }
    }

    /// Register a skill.
    pub fn register(&mut self, skill: Box<dyn Skill>) {
        self.skills.push(skill);
    }

    /// Get a skill by name.
    pub fn get(&self, name: &str) -> Option<&dyn Skill> {
        self.skills.iter().find(|s| s.name() == name).map(|s| s.as_ref())
    }

    /// Get all tool definitions for LLM.
    pub fn tool_definitions(&self) -> Vec<ToolDef> {
        self.skills.iter().map(|s| s.tool_definition()).collect()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// TODO: Implement in Phase 3
// mod builtin;
// mod loader;
//
// pub use builtin::{ShellSkill, HttpSkill, FileIoSkill, SummarizeSkill};
