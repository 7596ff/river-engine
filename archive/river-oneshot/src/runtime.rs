//! Runtime and cycle execution for river-oneshot.

use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::config::{Config, Provider};
use crate::context::{build_reasoning_context, build_system_prompt, parse_plan};
use crate::llm::{ClaudeProvider, LlmProvider};
use crate::memory::Memory;
use crate::skills::SkillRegistry;
use crate::types::{CycleInput, TurnOutput};

/// The main runtime that manages cycles.
pub struct Runtime {
    pub config: Config,
    pub llm: Arc<dyn LlmProvider>,
    pub skills: Arc<SkillRegistry>,
    pub memory: Memory,
    default_prompt: String,
}

impl Runtime {
    /// Initialize the runtime with configuration.
    pub async fn init(config: Config) -> Result<Self> {
        // Ensure workspace exists
        if !config.workspace.exists() {
            std::fs::create_dir_all(&config.workspace)?;
        }

        // Load default system prompt (used if no workspace files)
        let default_prompt = config.system_prompt()?;

        // Create LLM provider
        let llm: Arc<dyn LlmProvider> = match config.provider {
            Provider::Claude => {
                let api_key = config
                    .api_key
                    .clone()
                    .ok_or_else(|| anyhow!("ANTHROPIC_API_KEY not set"))?;
                Arc::new(ClaudeProvider::new(
                    api_key,
                    config.model.clone(),
                    config.api_base_url.clone(),
                ))
            }
            Provider::OpenAi => {
                return Err(anyhow!("OpenAI provider not yet implemented"));
            }
            Provider::Ollama => {
                return Err(anyhow!("Ollama provider not yet implemented"));
            }
        };

        // Create skill registry (empty for now, Phase 3)
        let skills = Arc::new(SkillRegistry::new());

        // Load or create memory
        let memory_path = config.workspace.join("memory.json");
        let memory = Memory::load(&memory_path).await.unwrap_or_default();

        Ok(Self {
            config,
            llm,
            skills,
            memory,
            default_prompt,
        })
    }

    /// Run a single cycle.
    pub async fn cycle(&mut self, input: CycleInput) -> Result<TurnOutput> {
        // 1. Check if previous cycle has a cached result
        if let Some(deferred) = self.memory.deferred_output.take() {
            // Still incorporate any new user input into memory
            if let Some(msg) = &input.user_message {
                self.memory.record_user(msg);
            }
            return Ok(deferred);
        }

        // 2. Build system prompt from workspace files
        let system_prompt = build_system_prompt(&self.config.workspace, &self.default_prompt).await;

        // 3. Build reasoning context
        let tools = self.skills.tool_definitions();
        let ctx = build_reasoning_context(&input, &self.memory, &system_prompt, tools);

        // 4. Record user input in memory (after context build to avoid duplication)
        if let Some(msg) = &input.user_message {
            self.memory.record_user(msg);
        }

        // 5. Call LLM
        tracing::debug!("Calling LLM with {} messages", ctx.messages.len());
        let response = self.llm.complete(&ctx.messages, &ctx.tools).await?;

        // 6. Parse response into plan
        let plan = parse_plan(response);

        // 7. Record in memory and queue actions
        let output = TurnOutput::Thought(plan.clone());
        self.memory.record(&output);
        self.memory.pending_actions.extend(plan.actions.clone());

        // 8. Tick flash TTLs
        self.memory.tick_flashes();

        // 9. Keep conversation bounded
        self.memory.truncate(50); // Keep last 50 turns

        // 10. Persist memory
        let memory_path = self.config.workspace.join("memory.json");
        self.memory.save(&memory_path).await?;

        Ok(output)
    }
}
