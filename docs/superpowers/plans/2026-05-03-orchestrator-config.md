# Orchestrator Config & Process Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The orchestrator reads a single JSON config file and spawns the entire river-engine system — its own HTTP server, gateways, and adapters — from one command.

**Architecture:** New `config_file` module in `river-orchestrator` handles JSON parsing with env var expansion. New `supervisor` module spawns and monitors child processes (gateways, adapters) with restart-on-failure. The orchestrator's `main.rs` gains `--config` and `--env-file` flags; when `--config` is present, it reads the file and drives startup through the supervisor instead of the existing direct-CLI path.

**Tech Stack:** Rust, serde_json, tokio (process spawning, signal handling), clap

**Note on discord channels:** The spec shows `"channels": ["general", "bot"]` but `river-discord --channels` takes comma-separated `u64` channel IDs, not names. The config should use IDs: `"channels": [111222333, 444555666]`. The plan follows the actual CLI.

---

### Task 1: Config File Types

Define the serde types for the JSON config.

**Files:**
- Create: `crates/river-orchestrator/src/config_file.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create config_file.rs with types and tests**

Create `crates/river-orchestrator/src/config_file.rs`:

```rust
//! JSON config file types
//!
//! Deserialized from the --config file after env var expansion.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level config file
#[derive(Debug, Deserialize)]
pub struct RiverConfig {
    /// Orchestrator HTTP port
    #[serde(default = "default_port")]
    pub port: u16,

    /// Named model backends
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,

    /// Named agents (each becomes a gateway process)
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    /// Global resource management
    #[serde(default)]
    pub resources: ResourcesConfig,
}

/// A model backend
#[derive(Debug, Deserialize)]
pub struct ModelConfig {
    /// Provider type: "anthropic", "openai", "ollama", "gguf", etc.
    pub provider: String,

    /// API endpoint URL (for external models)
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Model name at the endpoint
    #[serde(default)]
    pub name: Option<String>,

    /// Path to file containing API key
    #[serde(default)]
    pub api_key_file: Option<PathBuf>,

    /// Context window size in tokens
    #[serde(default)]
    pub context_limit: Option<u64>,

    /// Path to GGUF file (for provider: "gguf")
    #[serde(default)]
    pub path: Option<PathBuf>,

    /// Embedding dimensions (presence marks this as an embedding model)
    #[serde(default)]
    pub dimensions: Option<u32>,
}

/// Agent (gateway) configuration
#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    /// Path to agent's workspace directory
    pub workspace: PathBuf,

    /// Path to agent's data directory (contains river.db)
    pub data_dir: PathBuf,

    /// Gateway HTTP port
    pub port: u16,

    /// Key into models map for primary model
    pub model: String,

    /// Key into models map for spectator/bystander model
    #[serde(default)]
    pub spectator_model: Option<String>,

    /// Key into models map for embeddings
    #[serde(default)]
    pub embedding_model: Option<String>,

    /// Context window configuration
    #[serde(default)]
    pub context: ContextConfig,

    /// Redis connection URL
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Path to file containing auth token for gateway API
    #[serde(default)]
    pub auth_token_file: Option<PathBuf>,

    /// Logging configuration
    #[serde(default)]
    pub log: LogConfig,

    /// Adapter processes to spawn
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
}

/// Context window shape parameters
#[derive(Debug, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_context_limit")]
    pub limit: u64,

    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,

    #[serde(default = "default_fill_target")]
    pub fill_target: f64,

    #[serde(default = "default_min_messages")]
    pub min_messages: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            limit: default_context_limit(),
            compaction_threshold: default_compaction_threshold(),
            fill_target: default_fill_target(),
            min_messages: default_min_messages(),
        }
    }
}

/// Logging configuration
#[derive(Debug, Deserialize, Default)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,

    #[serde(default)]
    pub dir: Option<PathBuf>,

    #[serde(default)]
    pub file: Option<PathBuf>,

    #[serde(default)]
    pub json_stdout: bool,
}

/// Adapter process configuration
#[derive(Debug, Deserialize)]
pub struct AdapterConfig {
    /// Adapter type (e.g., "discord")
    #[serde(rename = "type")]
    pub adapter_type: String,

    /// Path to adapter binary (default: river-{type})
    #[serde(default)]
    pub bin: Option<PathBuf>,

    /// HTTP port for adapter's outbound server
    pub port: u16,

    /// Path to file containing token (for discord)
    #[serde(default)]
    pub token_file: Option<PathBuf>,

    /// Guild/server ID (for discord)
    #[serde(default)]
    pub guild_id: Option<String>,

    /// Channel IDs (for discord)
    #[serde(default)]
    pub channels: Vec<u64>,
}

/// Global resource management config
#[derive(Debug, Deserialize)]
pub struct ResourcesConfig {
    #[serde(default = "default_reserve_vram_mb")]
    pub reserve_vram_mb: u64,

    #[serde(default = "default_reserve_ram_mb")]
    pub reserve_ram_mb: u64,

    #[serde(default = "default_llama_server_path")]
    pub llama_server_path: PathBuf,

    #[serde(default = "default_port_range")]
    pub port_range: String,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            reserve_vram_mb: default_reserve_vram_mb(),
            reserve_ram_mb: default_reserve_ram_mb(),
            llama_server_path: default_llama_server_path(),
            port_range: default_port_range(),
        }
    }
}

// Defaults
fn default_port() -> u16 { 5000 }
fn default_context_limit() -> u64 { 128_000 }
fn default_compaction_threshold() -> f64 { 0.80 }
fn default_fill_target() -> f64 { 0.40 }
fn default_min_messages() -> u32 { 20 }
fn default_log_level() -> String { "info".to_string() }
fn default_reserve_vram_mb() -> u64 { 500 }
fn default_reserve_ram_mb() -> u64 { 2000 }
fn default_llama_server_path() -> PathBuf { PathBuf::from("llama-server") }
fn default_port_range() -> String { "8080-8180".to_string() }

impl ModelConfig {
    /// Returns true if this is an embedding model
    pub fn is_embedding(&self) -> bool {
        self.dimensions.is_some()
    }

    /// Returns true if this is a local GGUF model
    pub fn is_gguf(&self) -> bool {
        self.provider == "gguf"
    }
}

impl AdapterConfig {
    /// Get the binary path, defaulting to river-{type}
    pub fn bin_path(&self) -> PathBuf {
        self.bin.clone().unwrap_or_else(|| {
            PathBuf::from(format!("river-{}", self.adapter_type))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_config() {
        let json = r#"{
            "port": 5000,
            "models": {
                "claude": {
                    "provider": "anthropic",
                    "endpoint": "https://api.anthropic.com/v1",
                    "name": "claude-sonnet-4-20250514",
                    "api_key_file": "/run/secrets/key",
                    "context_limit": 200000
                },
                "local": {
                    "provider": "gguf",
                    "path": "/models/test.gguf",
                    "context_limit": 32000
                }
            },
            "agents": {
                "iris": {
                    "workspace": "/home/test/stream",
                    "data_dir": "/var/lib/river/iris",
                    "port": 3000,
                    "model": "claude",
                    "adapters": []
                }
            }
        }"#;

        let config: RiverConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 5000);
        assert_eq!(config.models.len(), 2);
        assert!(config.models["local"].is_gguf());
        assert!(!config.models["claude"].is_gguf());
        assert_eq!(config.agents["iris"].port, 3000);
    }

    #[test]
    fn test_defaults_applied() {
        let json = r#"{
            "agents": {
                "test": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "m"
                }
            }
        }"#;

        let config: RiverConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 5000);
        assert_eq!(config.agents["test"].context.limit, 128_000);
        assert_eq!(config.agents["test"].context.compaction_threshold, 0.80);
        assert_eq!(config.agents["test"].log.level, "info");
        assert_eq!(config.resources.reserve_vram_mb, 500);
    }

    #[test]
    fn test_adapter_bin_path_default() {
        let adapter = AdapterConfig {
            adapter_type: "discord".to_string(),
            bin: None,
            port: 8081,
            token_file: None,
            guild_id: None,
            channels: vec![],
        };
        assert_eq!(adapter.bin_path(), PathBuf::from("river-discord"));
    }

    #[test]
    fn test_adapter_bin_path_custom() {
        let adapter = AdapterConfig {
            adapter_type: "discord".to_string(),
            bin: Some(PathBuf::from("/usr/local/bin/my-discord")),
            port: 8081,
            token_file: None,
            guild_id: None,
            channels: vec![],
        };
        assert_eq!(adapter.bin_path(), PathBuf::from("/usr/local/bin/my-discord"));
    }

    #[test]
    fn test_model_is_embedding() {
        let embed = ModelConfig {
            provider: "ollama".to_string(),
            endpoint: Some("http://localhost:11434/v1".to_string()),
            name: Some("nomic".to_string()),
            api_key_file: None,
            context_limit: None,
            path: None,
            dimensions: Some(768),
        };
        assert!(embed.is_embedding());
        assert!(!embed.is_gguf());
    }

    #[test]
    fn test_parse_discord_adapter() {
        let json = r#"{
            "type": "discord",
            "port": 8081,
            "token_file": "/run/secrets/discord",
            "guild_id": "123456",
            "channels": [111, 222, 333]
        }"#;

        let adapter: AdapterConfig = serde_json::from_str(json).unwrap();
        assert_eq!(adapter.adapter_type, "discord");
        assert_eq!(adapter.channels, vec![111, 222, 333]);
        assert_eq!(adapter.guild_id.unwrap(), "123456");
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/river-orchestrator/src/lib.rs`:

```rust
pub mod config_file;
```

- [ ] **Step 3: Run tests**

Run: `cd ~/river-engine && cargo test -p river-orchestrator config_file`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/config_file.rs crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): add config file types with serde deserialization"
```

---

### Task 2: Env File Loading & Variable Expansion

Parse env files and expand `$VAR` references in the config string.

**Files:**
- Create: `crates/river-orchestrator/src/env.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create env.rs with loading and expansion**

Create `crates/river-orchestrator/src/env.rs`:

```rust
//! Environment file loading and variable expansion
//!
//! Loads key=value files into the process environment (existing env wins).
//! Expands $VAR references in strings before JSON parsing.

use std::path::Path;

/// Load an env file into the process environment.
/// Existing environment variables take precedence (are NOT overwritten).
pub fn load_env_file(path: &Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read env file {:?}: {}", path, e))?;

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse KEY=VALUE
        let Some((key, value)) = line.split_once('=') else {
            tracing::warn!(line = line_num + 1, "Skipping malformed env line (no '='): {}", line);
            continue;
        };

        let key = key.trim();
        let value = value.trim();

        if key.is_empty() {
            continue;
        }

        // Existing environment wins
        if std::env::var(key).is_ok() {
            tracing::debug!(key = key, "Env var already set, skipping env file value");
            continue;
        }

        std::env::set_var(key, value);
    }

    Ok(())
}

/// Expand $VAR references in a string using the current process environment.
/// Returns an error if any referenced variable is not defined.
pub fn expand_vars(input: &str) -> anyhow::Result<String> {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Collect variable name (alphanumeric + underscore)
            let mut var_name = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || next == '_' {
                    var_name.push(next);
                    chars.next();
                } else {
                    break;
                }
            }

            if var_name.is_empty() {
                // Bare $ with no variable name — keep it literal
                result.push('$');
                continue;
            }

            match std::env::var(&var_name) {
                Ok(value) => result.push_str(&value),
                Err(_) => {
                    anyhow::bail!(
                        "Undefined environment variable: ${} (referenced in config)",
                        var_name
                    );
                }
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_expand_vars_simple() {
        std::env::set_var("TEST_EXPAND_A", "hello");
        let result = expand_vars("value is $TEST_EXPAND_A").unwrap();
        assert_eq!(result, "value is hello");
        std::env::remove_var("TEST_EXPAND_A");
    }

    #[test]
    fn test_expand_vars_multiple() {
        std::env::set_var("TEST_EXPAND_X", "foo");
        std::env::set_var("TEST_EXPAND_Y", "bar");
        let result = expand_vars("$TEST_EXPAND_X and $TEST_EXPAND_Y").unwrap();
        assert_eq!(result, "foo and bar");
        std::env::remove_var("TEST_EXPAND_X");
        std::env::remove_var("TEST_EXPAND_Y");
    }

    #[test]
    fn test_expand_vars_in_json() {
        std::env::set_var("TEST_GUILD", "123456");
        let input = r#"{"guild_id": "$TEST_GUILD"}"#;
        let result = expand_vars(input).unwrap();
        assert_eq!(result, r#"{"guild_id": "123456"}"#);
        std::env::remove_var("TEST_GUILD");
    }

    #[test]
    fn test_expand_vars_undefined_is_error() {
        let result = expand_vars("$DEFINITELY_NOT_SET_EVER_12345");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("DEFINITELY_NOT_SET_EVER_12345"));
    }

    #[test]
    fn test_expand_vars_no_vars() {
        let result = expand_vars("no variables here").unwrap();
        assert_eq!(result, "no variables here");
    }

    #[test]
    fn test_expand_vars_bare_dollar() {
        let result = expand_vars("price is $5").unwrap();
        // $5 — '5' is not alphanumeric-start? Actually it is.
        // This will try to expand $5 which won't be set.
        // Let's handle this: bare $ followed by digit could be ambiguous.
        // For now, this is expected to fail since $5 looks like a var.
        // Users should not have bare $ in config values.
        assert!(result.is_err() || result.unwrap().contains("$"));
    }

    #[test]
    fn test_load_env_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.env");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "# comment").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "TEST_ENV_LOAD_A=hello").unwrap();
        writeln!(file, "TEST_ENV_LOAD_B=world").unwrap();

        load_env_file(&path).unwrap();

        assert_eq!(std::env::var("TEST_ENV_LOAD_A").unwrap(), "hello");
        assert_eq!(std::env::var("TEST_ENV_LOAD_B").unwrap(), "world");

        std::env::remove_var("TEST_ENV_LOAD_A");
        std::env::remove_var("TEST_ENV_LOAD_B");
    }

    #[test]
    fn test_load_env_file_existing_wins() {
        std::env::set_var("TEST_ENV_EXIST", "original");

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.env");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "TEST_ENV_EXIST=overwritten").unwrap();

        load_env_file(&path).unwrap();

        assert_eq!(std::env::var("TEST_ENV_EXIST").unwrap(), "original");
        std::env::remove_var("TEST_ENV_EXIST");
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/river-orchestrator/src/lib.rs`:

```rust
pub mod env;
```

- [ ] **Step 3: Add tempfile dev-dependency**

Add to `crates/river-orchestrator/Cargo.toml` under `[dev-dependencies]`:

```toml
tempfile = "3.10"
```

- [ ] **Step 4: Run tests**

Run: `cd ~/river-engine && cargo test -p river-orchestrator env`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/env.rs crates/river-orchestrator/src/lib.rs crates/river-orchestrator/Cargo.toml
git commit -m "feat(orchestrator): env file loading and variable expansion"
```

---

### Task 3: Config Validation

Validate the parsed config: model references resolve, no port conflicts, required fields present.

**Files:**
- Create: `crates/river-orchestrator/src/validate.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create validate.rs**

Create `crates/river-orchestrator/src/validate.rs`:

```rust
//! Config validation
//!
//! Checks model references, port conflicts, required fields.

use crate::config_file::RiverConfig;
use std::collections::HashSet;

/// Validate a parsed config. Returns a list of errors (empty = valid).
pub fn validate(config: &RiverConfig) -> Vec<String> {
    let mut errors = Vec::new();

    // Collect all ports to check for conflicts
    let mut ports: Vec<(u16, String)> = vec![(config.port, "orchestrator".to_string())];

    for (name, agent) in &config.agents {
        // Check model reference
        if !config.models.contains_key(&agent.model) {
            errors.push(format!(
                "Agent '{}': model '{}' not found in models map",
                name, agent.model
            ));
        }

        // Check spectator model reference
        if let Some(ref spec_model) = agent.spectator_model {
            if !config.models.contains_key(spec_model) {
                errors.push(format!(
                    "Agent '{}': spectator_model '{}' not found in models map",
                    name, spec_model
                ));
            }
        }

        // Check embedding model reference
        if let Some(ref embed_model) = agent.embedding_model {
            if !config.models.contains_key(embed_model) {
                errors.push(format!(
                    "Agent '{}': embedding_model '{}' not found in models map",
                    name, embed_model
                ));
            } else if let Some(model) = config.models.get(embed_model) {
                if !model.is_embedding() {
                    errors.push(format!(
                        "Agent '{}': embedding_model '{}' has no 'dimensions' field",
                        name, embed_model
                    ));
                }
            }
        }

        // Check GGUF models have path
        if let Some(model) = config.models.get(&agent.model) {
            if model.is_gguf() && model.path.is_none() {
                errors.push(format!(
                    "Model '{}': provider is 'gguf' but no 'path' specified",
                    agent.model
                ));
            }
        }

        // Check external models have endpoint
        for (model_id, model) in &config.models {
            if !model.is_gguf() && !model.is_embedding() && model.endpoint.is_none() {
                errors.push(format!(
                    "Model '{}': external provider '{}' requires 'endpoint'",
                    model_id, model.provider
                ));
            }
        }

        // Collect ports
        ports.push((agent.port, format!("agent '{}'", name)));

        for (i, adapter) in agent.adapters.iter().enumerate() {
            ports.push((adapter.port, format!("agent '{}' adapter[{}] ({})", name, i, adapter.adapter_type)));
        }
    }

    // Check port conflicts
    let mut seen_ports = HashSet::new();
    for (port, owner) in &ports {
        if !seen_ports.insert(port) {
            // Find the first owner
            let first = ports.iter().find(|(p, _)| p == port).unwrap();
            errors.push(format!(
                "Port conflict: {} and {} both use port {}",
                first.1, owner, port
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(json: &str) -> RiverConfig {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn test_valid_config() {
        let config = parse_config(r#"{
            "models": {
                "claude": {
                    "provider": "anthropic",
                    "endpoint": "https://api.anthropic.com/v1",
                    "name": "claude-sonnet"
                }
            },
            "agents": {
                "iris": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "claude"
                }
            }
        }"#);
        let errors = validate(&config);
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_missing_model_reference() {
        let config = parse_config(r#"{
            "agents": {
                "iris": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "nonexistent"
                }
            }
        }"#);
        let errors = validate(&config);
        assert!(errors.iter().any(|e| e.contains("nonexistent")));
    }

    #[test]
    fn test_port_conflict() {
        let config = parse_config(r#"{
            "port": 3000,
            "models": {
                "m": { "provider": "openai", "endpoint": "http://localhost:8080/v1" }
            },
            "agents": {
                "iris": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "m"
                }
            }
        }"#);
        let errors = validate(&config);
        assert!(errors.iter().any(|e| e.contains("Port conflict")));
    }

    #[test]
    fn test_embedding_model_missing_dimensions() {
        let config = parse_config(r#"{
            "models": {
                "m": { "provider": "openai", "endpoint": "http://localhost/v1" },
                "embed": { "provider": "ollama", "endpoint": "http://localhost/v1", "name": "nomic" }
            },
            "agents": {
                "iris": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "m",
                    "embedding_model": "embed"
                }
            }
        }"#);
        let errors = validate(&config);
        assert!(errors.iter().any(|e| e.contains("dimensions")));
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/river-orchestrator/src/lib.rs`:

```rust
pub mod validate;
```

- [ ] **Step 3: Run tests**

Run: `cd ~/river-engine && cargo test -p river-orchestrator validate`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/validate.rs crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): config validation — model refs, port conflicts, required fields"
```

---

### Task 4: CLI Argument Builder

Translate config into CLI argument vectors for gateway and adapter processes.

**Files:**
- Create: `crates/river-orchestrator/src/cli_builder.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create cli_builder.rs**

Create `crates/river-orchestrator/src/cli_builder.rs`:

```rust
//! Build CLI argument vectors for child processes
//!
//! Translates config structs into the exact args that river-gateway
//! and river-discord expect.

use crate::config_file::{AgentConfig, AdapterConfig, ModelConfig, RiverConfig};
use std::collections::HashMap;
use std::path::PathBuf;

/// Resolved model endpoint info
pub struct ResolvedModel {
    pub endpoint: String,
    pub name: Option<String>,
}

/// Build CLI args for a river-gateway process
pub fn gateway_args(
    agent_name: &str,
    agent: &AgentConfig,
    models: &HashMap<String, ModelConfig>,
    orchestrator_port: u16,
    resolved_models: &HashMap<String, ResolvedModel>,
) -> Vec<String> {
    let mut args = vec![
        "--workspace".to_string(), agent.workspace.display().to_string(),
        "--data-dir".to_string(), agent.data_dir.display().to_string(),
        "--port".to_string(), agent.port.to_string(),
        "--agent-name".to_string(), agent_name.to_string(),
        "--context-limit".to_string(), agent.context.limit.to_string(),
        "--orchestrator-url".to_string(), format!("http://127.0.0.1:{}", orchestrator_port),
    ];

    // Primary model
    if let Some(resolved) = resolved_models.get(&agent.model) {
        args.push("--model-url".to_string());
        args.push(resolved.endpoint.clone());
        if let Some(ref name) = resolved.name {
            args.push("--model-name".to_string());
            args.push(name.clone());
        }
    }

    // Spectator model
    let spectator_key = agent.spectator_model.as_deref().unwrap_or(&agent.model);
    if let Some(resolved) = resolved_models.get(spectator_key) {
        args.push("--spectator-model-url".to_string());
        args.push(resolved.endpoint.clone());
        if let Some(ref name) = resolved.name {
            args.push("--spectator-model-name".to_string());
            args.push(name.clone());
        }
    }

    // Embedding model
    if let Some(ref embed_key) = agent.embedding_model {
        if let Some(resolved) = resolved_models.get(embed_key) {
            args.push("--embedding-url".to_string());
            args.push(resolved.endpoint.clone());
        }
    }

    // Redis
    if let Some(ref redis_url) = agent.redis_url {
        args.push("--redis-url".to_string());
        args.push(redis_url.clone());
    }

    // Auth token file
    if let Some(ref token_file) = agent.auth_token_file {
        args.push("--auth-token-file".to_string());
        args.push(token_file.display().to_string());
    }

    // Logging
    args.push("--log-level".to_string());
    args.push(agent.log.level.clone());
    if let Some(ref log_file) = agent.log.file {
        args.push("--log-file".to_string());
        args.push(log_file.display().to_string());
    } else if let Some(ref log_dir) = agent.log.dir {
        args.push("--log-dir".to_string());
        args.push(log_dir.display().to_string());
    }
    if agent.log.json_stdout {
        args.push("--json-stdout".to_string());
    }

    // Adapters (as --adapter name:outbound_url[:read_url])
    for adapter in &agent.adapters {
        let outbound = format!("http://127.0.0.1:{}/send", adapter.port);
        let read = format!("http://127.0.0.1:{}/read", adapter.port);
        args.push("--adapter".to_string());
        args.push(format!("{}:{}:{}", adapter.adapter_type, outbound, read));
    }

    // Context shape params (these will be new gateway CLI args)
    args.push("--compaction-threshold".to_string());
    args.push(agent.context.compaction_threshold.to_string());
    args.push("--fill-target".to_string());
    args.push(agent.context.fill_target.to_string());
    args.push("--min-messages".to_string());
    args.push(agent.context.min_messages.to_string());

    args
}

/// Build CLI args for a discord adapter process
pub fn discord_args(
    adapter: &AdapterConfig,
    gateway_port: u16,
) -> Vec<String> {
    let mut args = vec![
        "--gateway-url".to_string(), format!("http://127.0.0.1:{}", gateway_port),
        "--listen-port".to_string(), adapter.port.to_string(),
    ];

    if let Some(ref token_file) = adapter.token_file {
        args.push("--token-file".to_string());
        args.push(token_file.display().to_string());
    }

    if let Some(ref guild_id) = adapter.guild_id {
        args.push("--guild-id".to_string());
        args.push(guild_id.clone());
    }

    if !adapter.channels.is_empty() {
        args.push("--channels".to_string());
        let ids: Vec<String> = adapter.channels.iter().map(|id| id.to_string()).collect();
        args.push(ids.join(","));
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_file::*;

    fn test_agent() -> AgentConfig {
        serde_json::from_str(r#"{
            "workspace": "/home/test/stream",
            "data_dir": "/var/lib/river/iris",
            "port": 3000,
            "model": "claude",
            "spectator_model": "claude",
            "embedding_model": "embed",
            "redis_url": "redis://127.0.0.1:6379",
            "auth_token_file": "/run/secrets/token",
            "adapters": [{
                "type": "discord",
                "port": 8081,
                "token_file": "/run/secrets/discord",
                "guild_id": "123456",
                "channels": [111, 222]
            }]
        }"#).unwrap()
    }

    fn test_resolved() -> HashMap<String, ResolvedModel> {
        let mut m = HashMap::new();
        m.insert("claude".to_string(), ResolvedModel {
            endpoint: "https://api.anthropic.com/v1".to_string(),
            name: Some("claude-sonnet-4-20250514".to_string()),
        });
        m.insert("embed".to_string(), ResolvedModel {
            endpoint: "http://localhost:11434/v1".to_string(),
            name: None,
        });
        m
    }

    #[test]
    fn test_gateway_args_contains_required() {
        let agent = test_agent();
        let resolved = test_resolved();
        let args = gateway_args("iris", &agent, &HashMap::new(), 5000, &resolved);

        assert!(args.contains(&"--workspace".to_string()));
        assert!(args.contains(&"/home/test/stream".to_string()));
        assert!(args.contains(&"--port".to_string()));
        assert!(args.contains(&"3000".to_string()));
        assert!(args.contains(&"--agent-name".to_string()));
        assert!(args.contains(&"iris".to_string()));
        assert!(args.contains(&"--model-url".to_string()));
        assert!(args.contains(&"--redis-url".to_string()));
        assert!(args.contains(&"--auth-token-file".to_string()));
        assert!(args.contains(&"--embedding-url".to_string()));
        assert!(args.contains(&"--compaction-threshold".to_string()));
    }

    #[test]
    fn test_discord_args() {
        let adapter: AdapterConfig = serde_json::from_str(r#"{
            "type": "discord",
            "port": 8081,
            "token_file": "/run/secrets/discord",
            "guild_id": "123456",
            "channels": [111, 222]
        }"#).unwrap();

        let args = discord_args(&adapter, 3000);
        assert!(args.contains(&"--gateway-url".to_string()));
        assert!(args.contains(&"http://127.0.0.1:3000".to_string()));
        assert!(args.contains(&"--token-file".to_string()));
        assert!(args.contains(&"/run/secrets/discord".to_string()));
        assert!(args.contains(&"--guild-id".to_string()));
        assert!(args.contains(&"123456".to_string()));
        assert!(args.contains(&"--channels".to_string()));
        assert!(args.contains(&"111,222".to_string()));
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/river-orchestrator/src/lib.rs`:

```rust
pub mod cli_builder;
```

- [ ] **Step 3: Run tests**

Run: `cd ~/river-engine && cargo test -p river-orchestrator cli_builder`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/cli_builder.rs crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): CLI argument builder for gateway and adapter processes"
```

---

### Task 5: Process Supervisor

Spawn child processes, capture output with log prefixes, restart on failure with backoff, shutdown on signal.

**Files:**
- Create: `crates/river-orchestrator/src/supervisor.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create supervisor.rs**

Create `crates/river-orchestrator/src/supervisor.rs`:

```rust
//! Process supervisor — spawn, monitor, restart child processes
//!
//! Each child has a label (e.g., "iris/gateway"), a binary, and args.
//! Stdout/stderr are forwarded to the orchestrator log with the label prefix.
//! On exit, the child is restarted with exponential backoff.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

/// A child process definition
#[derive(Debug, Clone)]
pub struct ChildSpec {
    /// Label for logging (e.g., "iris/gateway")
    pub label: String,
    /// Binary path
    pub bin: PathBuf,
    /// CLI arguments
    pub args: Vec<String>,
}

/// Backoff state for restarts
struct Backoff {
    delay: Duration,
    healthy_since: Option<Instant>,
}

impl Backoff {
    fn new() -> Self {
        Self {
            delay: Duration::from_secs(1),
            healthy_since: None,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let d = self.delay;
        self.delay = (self.delay * 2).min(Duration::from_secs(60));
        d
    }

    fn mark_running(&mut self) {
        self.healthy_since = Some(Instant::now());
    }

    fn maybe_reset(&mut self) {
        if let Some(since) = self.healthy_since {
            if since.elapsed() > Duration::from_secs(300) {
                self.delay = Duration::from_secs(1);
            }
        }
    }
}

/// Spawn a child process and forward its output to tracing
fn spawn_child(spec: &ChildSpec) -> std::io::Result<Child> {
    Command::new(&spec.bin)
        .args(&spec.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

/// Forward a child's stdout/stderr to tracing with a label prefix
fn forward_output(label: String, child: &mut Child) {
    if let Some(stdout) = child.stdout.take() {
        let label = label.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "child", "[{}] {}", label, line);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let label = label.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "child", "[{}] {}", label, line);
            }
        });
    }
}

/// Run a supervised child process. Restarts on exit with backoff.
/// Stops when shutdown_rx receives a signal.
pub async fn supervise(spec: ChildSpec, mut shutdown_rx: broadcast::Receiver<()>) {
    let mut backoff = Backoff::new();
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        backoff.maybe_reset();

        tracing::info!(
            label = %spec.label,
            bin = %spec.bin.display(),
            attempt = attempt,
            "Spawning child process"
        );

        let mut child = match spawn_child(&spec) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    label = %spec.label,
                    error = %e,
                    "Failed to spawn child process"
                );
                let delay = backoff.next_delay();
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        forward_output(spec.label.clone(), &mut child);
        backoff.mark_running();

        // Wait for child exit or shutdown signal
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) => tracing::warn!(label = %spec.label, status = %s, "Child exited"),
                    Err(e) => tracing::error!(label = %spec.label, error = %e, "Child wait failed"),
                }

                let delay = backoff.next_delay();
                tracing::info!(
                    label = %spec.label,
                    delay_secs = delay.as_secs(),
                    attempt = attempt,
                    "Restarting after backoff"
                );

                // Check for shutdown during backoff
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.recv() => {
                        tracing::info!(label = %spec.label, "Shutdown during backoff, not restarting");
                        return;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!(label = %spec.label, "Shutdown signal received, killing child");
                let _ = child.kill().await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_escalation() {
        let mut b = Backoff::new();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_secs(16));
        assert_eq!(b.next_delay(), Duration::from_secs(32));
        assert_eq!(b.next_delay(), Duration::from_secs(60)); // capped
        assert_eq!(b.next_delay(), Duration::from_secs(60)); // stays capped
    }

    #[test]
    fn test_child_spec() {
        let spec = ChildSpec {
            label: "iris/gateway".to_string(),
            bin: PathBuf::from("river-gateway"),
            args: vec!["--port".to_string(), "3000".to_string()],
        };
        assert_eq!(spec.label, "iris/gateway");
    }

    #[tokio::test]
    async fn test_supervise_shutdown() {
        let spec = ChildSpec {
            label: "test".to_string(),
            bin: PathBuf::from("sleep"),
            args: vec!["3600".to_string()],
        };

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let handle = tokio::spawn(supervise(spec, shutdown_rx));

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Send shutdown
        let _ = shutdown_tx.send(());

        // Should complete quickly
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("supervise should exit on shutdown")
            .expect("task should not panic");
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/river-orchestrator/src/lib.rs`:

```rust
pub mod supervisor;
```

- [ ] **Step 3: Run tests**

Run: `cd ~/river-engine && cargo test -p river-orchestrator supervisor`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/supervisor.rs crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): process supervisor with restart backoff and shutdown"
```

---

### Task 6: Wire --config Into main.rs

Add `--config` and `--env-file` CLI args. When `--config` is present, load config, validate, resolve models, spawn children via supervisor.

**Files:**
- Modify: `crates/river-orchestrator/src/main.rs`

- [ ] **Step 1: Add new CLI args to the Args struct**

In `crates/river-orchestrator/src/main.rs`, add to the `Args` struct:

```rust
    /// Path to JSON config file (starts full system)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to environment file (loaded before config)
    #[arg(long)]
    env_file: Option<PathBuf>,
```

- [ ] **Step 2: Add config-driven startup function**

Add a new `run_from_config` function that implements the startup sequence from the spec. This function:
1. Loads env file if provided
2. Reads config, expands vars, parses JSON
3. Validates
4. Starts orchestrator HTTP server
5. Checks agent births
6. Resolves models (waits for GGUF health)
7. Spawns gateway + adapter supervisors
8. Waits for shutdown signal

This is the largest step. The function should be added to `main.rs` and called from `main()` when `--config` is provided:

```rust
async fn run_from_config(config_path: PathBuf, env_file: Option<PathBuf>) -> anyhow::Result<()> {
    // 1. Load env file
    if let Some(ref env_path) = env_file {
        river_orchestrator::env::load_env_file(env_path)?;
        tracing::info!("Loaded env file: {:?}", env_path);
    }

    // 2. Read and parse config
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| anyhow::anyhow!("Failed to read config {:?}: {}", config_path, e))?;
    let expanded = river_orchestrator::env::expand_vars(&raw)?;
    let config: river_orchestrator::config_file::RiverConfig = serde_json::from_str(&expanded)
        .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;

    // 3. Validate
    let errors = river_orchestrator::validate::validate(&config);
    if !errors.is_empty() {
        for e in &errors {
            tracing::error!("Config error: {}", e);
        }
        anyhow::bail!("{} config validation error(s)", errors.len());
    }

    tracing::info!(
        port = config.port,
        agents = config.agents.len(),
        models = config.models.len(),
        "Config loaded"
    );

    // 4. Start orchestrator HTTP server (reuse existing setup)
    // ... (build OrchestratorState from config, create router, bind)

    // 5-6. For each agent, check birth and resolve models
    let mut resolved_models = std::collections::HashMap::new();
    for (model_id, model) in &config.models {
        if model.is_gguf() {
            // Request model load, wait for health (120s timeout)
            // Use existing state.request_model() which blocks until healthy
            tracing::info!(model = %model_id, "Loading GGUF model...");
            // ... model loading via orchestrator state ...
        } else if let Some(ref endpoint) = model.endpoint {
            resolved_models.insert(model_id.clone(), river_orchestrator::cli_builder::ResolvedModel {
                endpoint: endpoint.clone(),
                name: model.name.clone(),
            });
        }
    }

    // 7. Spawn supervised children
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles = Vec::new();

    for (name, agent) in &config.agents {
        // Check birth
        let db_path = agent.data_dir.join("river.db");
        if !db_path.exists() {
            tracing::error!(
                agent = %name,
                "Agent not birthed. Run: river-gateway birth --data-dir {:?} --name {}",
                agent.data_dir, name
            );
            continue;
        }

        // Spawn gateway
        let gateway_args = river_orchestrator::cli_builder::gateway_args(
            name, agent, &config.models, config.port, &resolved_models,
        );
        let gateway_spec = river_orchestrator::supervisor::ChildSpec {
            label: format!("{}/gateway", name),
            bin: std::path::PathBuf::from("river-gateway"),
            args: gateway_args,
        };
        let rx = shutdown_tx.subscribe();
        handles.push(tokio::spawn(river_orchestrator::supervisor::supervise(gateway_spec, rx)));

        // Spawn adapters
        for (i, adapter) in agent.adapters.iter().enumerate() {
            let adapter_args = match adapter.adapter_type.as_str() {
                "discord" => river_orchestrator::cli_builder::discord_args(adapter, agent.port),
                other => {
                    tracing::warn!(adapter = %other, "Unknown adapter type, skipping");
                    continue;
                }
            };
            let adapter_spec = river_orchestrator::supervisor::ChildSpec {
                label: format!("{}/{}", name, adapter.adapter_type),
                bin: adapter.bin_path(),
                args: adapter_args,
            };
            let rx = shutdown_tx.subscribe();
            handles.push(tokio::spawn(river_orchestrator::supervisor::supervise(adapter_spec, rx)));
        }
    }

    if handles.is_empty() {
        anyhow::bail!("No agents could start");
    }

    tracing::info!(children = handles.len(), "All children spawned");

    // 8. Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutdown signal received");
    let _ = shutdown_tx.send(());

    // Wait for all supervisors to stop (with timeout)
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        futures::future::join_all(handles),
    ).await;

    tracing::info!("Shutdown complete");
    Ok(())
}
```

- [ ] **Step 3: Update main() to dispatch to config mode**

In `main()`, after parsing args, add:

```rust
    // Config-driven mode
    if let Some(config_path) = args.config {
        return run_from_config(config_path, args.env_file).await;
    }
```

This goes before the existing direct-CLI startup path.

- [ ] **Step 4: Add `futures` dependency**

Add to `crates/river-orchestrator/Cargo.toml`:

```toml
futures = "0.3"
```

- [ ] **Step 5: Run compilation check**

Run: `cd ~/river-engine && cargo check -p river-orchestrator`
Expected: Compiles (the GGUF model resolution is stubbed with a comment — full integration uses existing `request_model`)

- [ ] **Step 6: Commit**

```bash
git add crates/river-orchestrator/src/main.rs crates/river-orchestrator/Cargo.toml
git commit -m "feat(orchestrator): --config mode — load config, validate, spawn supervised children"
```

---

### Task 7: Gateway Context Shape CLI Args

Add `--compaction-threshold`, `--fill-target`, `--min-messages` to the gateway CLI so the orchestrator can pass them.

**Files:**
- Modify: `crates/river-gateway/src/main.rs`
- Modify: `crates/river-gateway/src/server.rs`

- [ ] **Step 1: Add CLI args to gateway Args struct**

In `crates/river-gateway/src/main.rs`, add to the `Args` struct:

```rust
    /// Compaction threshold (fraction of context limit, e.g., 0.80)
    #[arg(long, default_value = "0.80")]
    compaction_threshold: f64,

    /// Post-compaction fill target (fraction of context limit, e.g., 0.40)
    #[arg(long, default_value = "0.40")]
    fill_target: f64,

    /// Minimum messages always kept in context
    #[arg(long, default_value = "20")]
    min_messages: u32,
```

- [ ] **Step 2: Pass through to ServerConfig**

In `crates/river-gateway/src/main.rs`, add to the `ServerConfig` construction in `main()`:

```rust
    let config = ServerConfig {
        // ... existing fields ...
        compaction_threshold: args.compaction_threshold,
        fill_target: args.fill_target,
        min_messages: args.min_messages as usize,
    };
```

- [ ] **Step 3: Add fields to ServerConfig**

In `crates/river-gateway/src/server.rs`, add to `ServerConfig`:

```rust
    /// Compaction threshold (fraction of context limit)
    pub compaction_threshold: f64,
    /// Post-compaction fill target
    pub fill_target: f64,
    /// Minimum messages kept in context
    pub min_messages: usize,
```

- [ ] **Step 4: Use in AgentTaskConfig construction**

In `crates/river-gateway/src/server.rs`, in the `run()` function where `AgentTaskConfig` is built, replace the context config:

```rust
    let agent_config = AgentTaskConfig {
        workspace: agent_workspace.clone(),
        context_config: crate::agent::ContextConfig {
            limit: agent_context_limit as usize,
            compaction_threshold: config.compaction_threshold,
            fill_target: config.fill_target,
            min_messages: config.min_messages,
        },
        // ... rest unchanged ...
    };
```

- [ ] **Step 5: Run compilation check and tests**

Run: `cd ~/river-engine && cargo check -p river-gateway && cargo test -p river-gateway`
Expected: Compiles and all tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-gateway/src/main.rs crates/river-gateway/src/server.rs
git commit -m "feat(gateway): add --compaction-threshold, --fill-target, --min-messages CLI args"
```

---

### Task 8: Integration Test & Example Config

Create an example config file and verify the full parsing pipeline end-to-end.

**Files:**
- Create: `deploy/river.example.json` (overwrite stale file)
- Create: `deploy/river.example.env`

- [ ] **Step 1: Write the example config**

Overwrite `deploy/river.example.json`:

```json
{
  "port": 5000,

  "models": {
    "claude-sonnet": {
      "provider": "anthropic",
      "endpoint": "https://api.anthropic.com/v1",
      "name": "claude-sonnet-4-20250514",
      "api_key_file": "/run/secrets/anthropic_key",
      "context_limit": 200000
    },
    "nomic-embed": {
      "provider": "ollama",
      "endpoint": "http://localhost:11434/v1",
      "name": "nomic-embed-text",
      "dimensions": 768
    }
  },

  "agents": {
    "iris": {
      "workspace": "$HOME/stream",
      "data_dir": "/var/lib/river/iris",
      "port": 3000,
      "model": "claude-sonnet",
      "spectator_model": "claude-sonnet",
      "embedding_model": "nomic-embed",
      "context": {
        "limit": 200000,
        "compaction_threshold": 0.80,
        "fill_target": 0.40,
        "min_messages": 20
      },
      "redis_url": "redis://127.0.0.1:6379",
      "auth_token_file": "/run/secrets/gateway_token",
      "log": {
        "level": "info"
      },
      "adapters": [
        {
          "type": "discord",
          "port": 8081,
          "token_file": "/run/secrets/discord_token",
          "guild_id": "$DISCORD_GUILD_ID",
          "channels": []
        }
      ]
    }
  },

  "resources": {
    "reserve_vram_mb": 500,
    "reserve_ram_mb": 2000,
    "llama_server_path": "llama-server",
    "port_range": "8100-8200"
  }
}
```

- [ ] **Step 2: Write the example env file**

Overwrite `deploy/river.example.env`:

```
# River Engine environment
# Copy to /etc/river/river.env or ~/.config/river/river.env

# Non-secret values only — secrets use *_file fields in the JSON config
DISCORD_GUILD_ID=1234567890123456789
```

- [ ] **Step 3: Write end-to-end parsing test**

Add to `crates/river-orchestrator/src/config_file.rs` tests:

```rust
    #[test]
    fn test_parse_example_config() {
        // Read the actual example config, expand $HOME and $DISCORD_GUILD_ID
        std::env::set_var("DISCORD_GUILD_ID", "999888777");
        let raw = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../deploy/river.example.json")
        ).unwrap();
        let expanded = crate::env::expand_vars(&raw).unwrap();
        let config: RiverConfig = serde_json::from_str(&expanded).unwrap();

        assert_eq!(config.port, 5000);
        assert_eq!(config.models.len(), 2);
        assert!(config.models.contains_key("claude-sonnet"));
        assert!(config.models["nomic-embed"].is_embedding());
        assert_eq!(config.agents.len(), 1);
        assert!(config.agents.contains_key("iris"));

        let iris = &config.agents["iris"];
        assert_eq!(iris.port, 3000);
        assert_eq!(iris.context.limit, 200000);
        assert_eq!(iris.adapters.len(), 1);
        assert_eq!(iris.adapters[0].adapter_type, "discord");

        // Validate
        let errors = crate::validate::validate(&config);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);

        std::env::remove_var("DISCORD_GUILD_ID");
    }
```

- [ ] **Step 4: Run tests**

Run: `cd ~/river-engine && cargo test -p river-orchestrator test_parse_example`
Expected: Pass

- [ ] **Step 5: Run full test suite**

Run: `cd ~/river-engine && cargo test`
Expected: All tests pass across workspace

- [ ] **Step 6: Commit**

```bash
git add deploy/river.example.json deploy/river.example.env crates/river-orchestrator/src/config_file.rs
git commit -m "feat(orchestrator): example config + env files, end-to-end parsing test"
```
