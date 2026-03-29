//! Configuration loading for river-oneshot.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Working directory.
    #[serde(default = "default_workspace")]
    pub workspace: PathBuf,

    /// SQLite database location.
    #[serde(default = "default_database_path")]
    pub database_path: PathBuf,

    /// LLM provider.
    #[serde(default)]
    pub provider: Provider,

    /// Model name.
    #[serde(default = "default_model")]
    pub model: String,

    /// API key (can also be set via environment).
    pub api_key: Option<String>,

    /// API base URL (for ollama or proxies).
    pub api_base_url: Option<String>,

    /// Custom system prompt file.
    pub system_prompt_path: Option<PathBuf>,

    /// Additional skills directory.
    pub skills_dir: Option<PathBuf>,

    /// LLM retry attempts.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_workspace() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".river").join("workspace"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_database_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".river").join("oneshot.db"))
        .unwrap_or_else(|| PathBuf::from("oneshot.db"))
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_max_retries() -> u32 {
    3
}

impl Default for Config {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            database_path: default_database_path(),
            provider: Provider::default(),
            model: default_model(),
            api_key: None,
            api_base_url: None,
            system_prompt_path: None,
            skills_dir: None,
            max_retries: default_max_retries(),
        }
    }
}

impl Config {
    /// Load configuration from file, with CLI overrides.
    pub fn load(path: Option<&PathBuf>, cli: &CliOverrides) -> Result<Self> {
        let mut config = if let Some(path) = path {
            if path.exists() {
                let contents = std::fs::read_to_string(path)
                    .with_context(|| format!("Failed to read config file: {}", path.display()))?;
                toml::from_str(&contents)
                    .with_context(|| format!("Failed to parse config file: {}", path.display()))?
            } else {
                Config::default()
            }
        } else {
            // Try default location
            let default_path = dirs::home_dir()
                .map(|h| h.join(".river").join("oneshot.toml"));

            if let Some(path) = default_path.filter(|p| p.exists()) {
                let contents = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config file: {}", path.display()))?;
                toml::from_str(&contents)
                    .with_context(|| format!("Failed to parse config file: {}", path.display()))?
            } else {
                Config::default()
            }
        };

        // Apply CLI overrides
        if let Some(workspace) = &cli.workspace {
            config.workspace = workspace.clone();
        }
        if let Some(model) = &cli.model {
            config.model = model.clone();
        }
        if let Some(provider) = &cli.provider {
            config.provider = *provider;
        }

        // Try to get API key from environment if not in config
        if config.api_key.is_none() {
            config.api_key = std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .map(|s| s.trim().to_string());
        }

        Ok(config)
    }

    /// Get the system prompt, either from file or default.
    pub fn system_prompt(&self) -> Result<String> {
        if let Some(path) = &self.system_prompt_path {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read system prompt: {}", path.display()))
        } else {
            Ok(DEFAULT_SYSTEM_PROMPT.to_string())
        }
    }
}

/// CLI overrides for configuration.
#[derive(Debug, Default)]
pub struct CliOverrides {
    pub workspace: Option<PathBuf>,
    pub model: Option<String>,
    pub provider: Option<Provider>,
}

/// LLM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Claude,
    OpenAi,
    Ollama,
}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" | "anthropic" => Ok(Provider::Claude),
            "openai" | "gpt" => Ok(Provider::OpenAi),
            "ollama" | "local" => Ok(Provider::Ollama),
            _ => Err(format!("Unknown provider: {}", s)),
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Claude => write!(f, "claude"),
            Provider::OpenAi => write!(f, "openai"),
            Provider::Ollama => write!(f, "ollama"),
        }
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are a helpful assistant with access to tools.

When you want to take an action, use a tool. When you have information to share, respond directly.

Available tools will be provided. Use them when appropriate. You can request multiple tools in a single response — they will be queued and executed one per turn.

Be concise. Focus on completing the user's request."#;
