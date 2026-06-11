//! The river.json config layer (wall ch. 09).
//!
//! Pure: expansion, parsing, and validation all operate on text and an
//! environment lookup function. Reading the file and the real
//! environment happens in main.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Deserialize;

pub const DEFAULT_TOOLS: [&str; 8] = [
    "read", "write", "edit", "glob", "grep", "bash", "speak", "search",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub models: BTreeMap<String, ModelConfig>,
    pub agents: BTreeMap<String, AgentConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub provider: Provider,
    pub endpoint: String,
    pub name: String,
    /// Names an environment variable holding the API key. The value
    /// never appears in config text.
    pub api_key_env: Option<String>,
    pub context_limit: Option<u64>,
    /// Present on embedding models only.
    pub dimensions: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    Openai,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub model: String,
    /// Defaults to the agent's model.
    pub witness_model: Option<String>,
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub context: ContextConfig,
    /// The tool profile (wall ch. 07). Defaults to the eight core tools.
    pub tools: Option<Vec<String>>,
    #[serde(default = "default_heartbeat_minutes")]
    pub heartbeat_minutes: u64,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
}

fn default_heartbeat_minutes() -> u64 {
    45
}

/// Context knobs (wall ch. 03). Everything optional; defaults bind here.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ContextConfig {
    pub limit: u64,
    pub compaction_threshold: f64,
    pub fill_target: f64,
    pub min_messages: u64,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            limit: 128_000,
            compaction_threshold: 0.80,
            fill_target: 0.40,
            min_messages: 20,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum AdapterConfig {
    Discord {
        guild_id: String,
        channels: Vec<String>,
        /// Names the environment variable holding the bot token.
        token_env: String,
    },
    Local {
        port: u16,
    },
}

impl AgentConfig {
    pub fn tool_profile(&self) -> Vec<String> {
        match &self.tools {
            Some(tools) => tools.clone(),
            None => DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn witness_model_name(&self) -> &str {
        self.witness_model.as_deref().unwrap_or(&self.model)
    }
}

/// Expand `$VAR` references in raw config text against `lookup`,
/// before parsing. For non-secrets only by convention: `*_env` fields
/// name variables instead of referencing them, so secrets never pass
/// through here. `$$` escapes a literal dollar. Unresolvable variables
/// are fatal, all reported together with line numbers.
pub fn expand_vars(
    text: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, Vec<String>> {
    let mut out = String::with_capacity(text.len());
    let mut errors = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let mut chars = line.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            if let Some(&(_, '$')) = chars.peek() {
                chars.next();
                out.push('$');
                continue;
            }
            let name: String = line[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                out.push('$');
                continue;
            }
            for _ in 0..name.len() {
                chars.next();
            }
            match lookup(&name) {
                Some(value) => out.push_str(&value),
                None => errors.push(format!(
                    "line {}: unresolvable $${} in config",
                    line_no + 1,
                    name
                )),
            }
        }
        out.push('\n');
    }

    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

/// Parse expanded config text.
pub fn parse(expanded: &str) -> Result<Config, String> {
    serde_json::from_str(expanded).map_err(|e| format!("config parse error: {e}"))
}

/// Validate everything before spawning anything; report all errors
/// together (wall ch. 09).
pub fn validate(config: &Config) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    let mut workspaces: BTreeMap<&PathBuf, &str> = BTreeMap::new();
    let mut ports: BTreeMap<u16, String> = BTreeMap::new();

    for (agent_name, agent) in &config.agents {
        for (label, model_ref) in [
            ("model", Some(&agent.model)),
            ("witness_model", agent.witness_model.as_ref()),
            ("embedding_model", agent.embedding_model.as_ref()),
        ] {
            if let Some(model_ref) = model_ref
                && !config.models.contains_key(model_ref)
            {
                errors.push(format!(
                    "agent {agent_name}: {label} \"{model_ref}\" is not a configured model"
                ));
            }
        }

        if let Some(embed_ref) = &agent.embedding_model
            && let Some(model) = config.models.get(embed_ref)
            && model.dimensions.is_none()
        {
            errors.push(format!(
                "agent {agent_name}: embedding_model \"{embed_ref}\" has no dimensions field"
            ));
        }

        if let Some(prior) = workspaces.insert(&agent.workspace, agent_name) {
            errors.push(format!(
                "agents {prior} and {agent_name} share workspace {}",
                agent.workspace.display()
            ));
        }

        for adapter in &agent.adapters {
            if let AdapterConfig::Local { port } = adapter {
                if let Some(prior) = ports.insert(*port, agent_name.to_string()) {
                    errors.push(format!(
                        "agents {prior} and {agent_name} both bind local port {port}"
                    ));
                }
            }
        }
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

/// Render a multi-error list as one fatal message.
pub fn render_errors(header: &str, errors: &[String]) -> String {
    let mut msg = String::from(header);
    for e in errors {
        let _ = write!(msg, "\n  - {e}");
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
      "models": {
        "sonnet": {
          "provider": "anthropic",
          "endpoint": "https://api.anthropic.com/v1",
          "name": "claude-sonnet-4",
          "api_key_env": "ANTHROPIC_KEY",
          "context_limit": 200000
        },
        "embed": {
          "provider": "openai",
          "endpoint": "http://localhost:11434/v1",
          "name": "nomic-embed-text",
          "dimensions": 768
        }
      },
      "agents": {
        "ada": {
          "workspace": "/home/ada/ws",
          "data_dir": "/home/ada/.local/state/river/ada",
          "model": "sonnet",
          "embedding_model": "embed",
          "adapters": [ { "type": "local", "port": 7700 } ]
        }
      }
    }"#;

    #[test]
    fn parses_and_validates_good_config() {
        let config = parse(GOOD).unwrap();
        validate(&config).unwrap();
        let ada = &config.agents["ada"];
        assert_eq!(ada.heartbeat_minutes, 45);
        assert_eq!(ada.witness_model_name(), "sonnet");
        assert_eq!(ada.tool_profile(), DEFAULT_TOOLS.to_vec());
        assert_eq!(ada.context.limit, 128_000);
        assert_eq!(ada.context.min_messages, 20);
    }

    #[test]
    fn expansion_substitutes_and_escapes() {
        let lookup = |name: &str| match name {
            "HOME" => Some("/home/ada".to_string()),
            "GUILD" => Some("123".to_string()),
            _ => None,
        };
        let out = expand_vars("path: $HOME/ws guild: $GUILD cash: $$5", lookup).unwrap();
        assert_eq!(out, "path: /home/ada/ws guild: 123 cash: $5\n");
    }

    #[test]
    fn expansion_reports_all_unresolved_with_lines() {
        let errors = expand_vars("a: $NOPE\nb: $ALSO_NOPE", |_| None).unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("line 1"));
        assert!(errors[0].contains("NOPE"));
        assert!(errors[1].contains("line 2"));
        assert!(errors[1].contains("ALSO_NOPE"));
    }

    #[test]
    fn lone_dollar_passes_through() {
        let out = expand_vars("price is $ alone", |_| None).unwrap();
        assert_eq!(out, "price is $ alone\n");
    }

    #[test]
    fn validation_collects_all_errors() {
        let text = r#"{
          "models": {
            "embed": { "provider": "openai", "endpoint": "e", "name": "n" }
          },
          "agents": {
            "ada": {
              "workspace": "/ws", "data_dir": "/d1", "model": "missing",
              "embedding_model": "embed",
              "adapters": [ { "type": "local", "port": 7700 } ]
            },
            "bee": {
              "workspace": "/ws", "data_dir": "/d2", "model": "missing",
              "adapters": [ { "type": "local", "port": 7700 } ]
            }
          }
        }"#;
        let config = parse(text).unwrap();
        let errors = validate(&config).unwrap_err();
        // ada: bad model ref + embed without dimensions; bee: bad model
        // ref; shared workspace; shared port.
        assert_eq!(errors.len(), 5, "{errors:?}");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = r#"{ "models": {}, "agents": {}, "surprise": true }"#;
        assert!(parse(text).is_err());
    }

    #[test]
    fn discord_adapter_parses() {
        let text = r#"{
          "models": {
            "m": { "provider": "anthropic", "endpoint": "e", "name": "n" }
          },
          "agents": {
            "ada": {
              "workspace": "/ws", "data_dir": "/d", "model": "m",
              "adapters": [
                { "type": "discord", "guild_id": "1", "channels": ["general"],
                  "token_env": "DISCORD_TOKEN_ADA" }
              ]
            }
          }
        }"#;
        let config = parse(text).unwrap();
        validate(&config).unwrap();
        match &config.agents["ada"].adapters[0] {
            AdapterConfig::Discord { token_env, .. } => {
                assert_eq!(token_env, "DISCORD_TOKEN_ADA");
            }
            other => panic!("expected discord adapter, got {other:?}"),
        }
    }
}
