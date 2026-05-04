//! Build CLI argument vectors for child processes
//!
//! Translates config structs into the exact args that river-gateway
//! and river-discord expect.

use crate::config_file::{AgentConfig, AdapterConfig, ModelConfig};
use std::collections::HashMap;

/// Resolved model endpoint info
pub struct ResolvedModel {
    pub endpoint: String,
    pub name: Option<String>,
}

/// Build CLI args for a river-gateway process
pub fn gateway_args(
    agent_name: &str,
    agent: &AgentConfig,
    _models: &HashMap<String, ModelConfig>,
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

    // Context shape params
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
        assert!(args.contains(&"https://api.anthropic.com/v1".to_string()));
        assert!(args.contains(&"--redis-url".to_string()));
        assert!(args.contains(&"--auth-token-file".to_string()));
        assert!(args.contains(&"--embedding-url".to_string()));
        assert!(args.contains(&"--compaction-threshold".to_string()));
        assert!(args.contains(&"--spectator-model-url".to_string()));
    }

    #[test]
    fn test_gateway_args_adapter_format() {
        let agent = test_agent();
        let resolved = test_resolved();
        let args = gateway_args("iris", &agent, &HashMap::new(), 5000, &resolved);

        assert!(args.contains(&"--adapter".to_string()));
        assert!(args.contains(&"discord:http://127.0.0.1:8081/send:http://127.0.0.1:8081/read".to_string()));
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

    #[test]
    fn test_discord_args_minimal() {
        let adapter: AdapterConfig = serde_json::from_str(r#"{
            "type": "discord",
            "port": 8081
        }"#).unwrap();

        let args = discord_args(&adapter, 3000);
        assert!(args.contains(&"--listen-port".to_string()));
        assert!(!args.contains(&"--token-file".to_string()));
        assert!(!args.contains(&"--channels".to_string()));
    }
}
