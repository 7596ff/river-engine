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

    // Check model-level constraints
    for (model_id, model) in &config.models {
        if model.is_gguf() && model.path.is_none() {
            errors.push(format!(
                "Model '{}': provider is 'gguf' but no 'path' specified",
                model_id
            ));
        }
        if !model.is_gguf() && !model.is_embedding() && model.endpoint.is_none() {
            errors.push(format!(
                "Model '{}': external provider '{}' requires 'endpoint'",
                model_id, model.provider
            ));
        }
    }

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

    #[test]
    fn test_gguf_missing_path() {
        let config = parse_config(r#"{
            "models": {
                "local": { "provider": "gguf" }
            },
            "agents": {
                "iris": {
                    "workspace": "/tmp/ws",
                    "data_dir": "/tmp/data",
                    "port": 3000,
                    "model": "local"
                }
            }
        }"#);
        let errors = validate(&config);
        assert!(errors.iter().any(|e| e.contains("gguf") && e.contains("path")));
    }

    #[test]
    fn test_external_model_missing_endpoint() {
        let config = parse_config(r#"{
            "models": {
                "m": { "provider": "anthropic", "name": "claude" }
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
        assert!(errors.iter().any(|e| e.contains("endpoint")));
    }
}
