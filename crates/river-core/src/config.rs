//! The river.json config layer (wall ch. 09).
//!
//! Pure: expansion, parsing, and validation all operate on text and an
//! environment lookup function. Reading the file and the real
//! environment happens in main.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Deserialize;

pub const DEFAULT_TOOLS: [&str; 10] = [
    "read", "write", "edit", "glob", "grep", "bash", "speak", "search", "channel_read",
    "reject_candidate",
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
    /// Per-request timeout. CPU-bound local inference can legitimately
    /// take minutes; remote APIs should keep the tight default.
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
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
    /// Think/act iteration ceiling per turn (wall ch. 01).
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Flat per-turn gleaning probability (wall ch. 04).
    #[serde(default = "default_glean_probability")]
    pub glean_probability: f64,
    /// IANA timezone name for the agent's sense of "now". Defaults to
    /// the system timezone.
    pub timezone: Option<String>,
    /// Workspace directories indexed beyond `knowledge/` (wall
    /// ch. 08). `"."` indexes the whole workspace. Only markdown
    /// files are indexed; hidden paths and the engine-managed
    /// `record/` and `channels/` never are.
    #[serde(default)]
    pub index_dirs: Vec<String>,
    /// Activation knobs (wall ch. 02). Optional; defaults are the
    /// wall's constants. Tuning is edit + restart, never a rebuild.
    #[serde(default)]
    pub activation: ActivationConfig,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
    /// Attachment knobs. Optional; the defaults here are v1's values.
    #[serde(default)]
    pub attachments: AttachmentsConfig,
    /// Witness-side knobs (wall ch. 04). Optional; defaults bind here.
    #[serde(default)]
    pub witness: WitnessConfig,
}

/// Witness-side knobs (wall ch. 04). Every field optional in config;
/// defaults here are the wall's numbers.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WitnessConfig {
    /// Refractory: minimum turns of forward movement required between
    /// queued candidates. Pre-model gate; both wake paths (per-turn
    /// dice and shutdown pass) honor it. Zero disables. Default 12 =
    /// 2 × the 6-turn glean window.
    pub glean_min_new_turns: u64,
    /// Hard ceiling on the extraction queue at enqueue time. The
    /// witness drops candidates beyond this depth with a warning log;
    /// refractory state stays untouched on a drop. Zero disables.
    /// Default 5 — matches "0-1 candidates per quiet stretch" with
    /// headroom for genuinely-productive sessions.
    pub max_queue_depth: u64,
    /// How many recent rejections (from rejections.jsonl) get rendered
    /// into the witness's on-glean.md `{recent_rejections}` slot.
    /// Default 5.
    pub recent_rejections_window: usize,
}

impl Default for WitnessConfig {
    fn default() -> Self {
        Self {
            glean_min_new_turns: 12,
            max_queue_depth: 5,
            recent_rejections_window: 5,
        }
    }
}

/// Attachment-handling knobs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AttachmentsConfig {
    /// Per-file cap on inbound downloads. Over-cap files append with
    /// `path: null, skipped: too_large` so the agent learns they
    /// existed without storing them. Default: 25 MiB (Discord's
    /// free-tier upload ceiling).
    pub max_bytes: u64,
    /// Per-download HTTP timeout. One in-process retry is attempted
    /// before a failure is accepted.
    pub download_timeout_secs: u64,
}

impl Default for AttachmentsConfig {
    fn default() -> Self {
        Self {
            max_bytes: 25 * 1024 * 1024,
            download_timeout_secs: 30,
        }
    }
}

fn default_heartbeat_minutes() -> u64 {
    45
}

fn default_request_timeout() -> u64 {
    120
}

fn default_max_iterations() -> u32 {
    50
}

fn default_glean_probability() -> f64 {
    0.25
}

/// Context knobs (wall ch. 03). Everything optional; defaults bind here.
#[derive(Debug, Clone, Deserialize)]
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
            min_messages: 50,
        }
    }
}

/// Activation knobs (wall ch. 02 contracts). Every field optional in
/// config; the defaults here ARE the wall's constants.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ActivationConfig {
    pub cognitive_bump: f64,
    pub ambient_bump: f64,
    pub flash_threshold: f64,
    pub propagation_factor: f64,
    pub propagation_hops: usize,
    pub decay_factor: f64,
    pub semantic_factor: f64,
    pub semantic_top_k: usize,
    pub semantic_threshold: f32,
    pub resonance_factor: f64,
    pub resonance_top_k: usize,
    pub resonance_threshold: f32,
    pub tool_resonance_factor: f64,
    pub search_top_k: usize,
    /// Workspace-relative dir prefixes whose notes may flash. Empty =
    /// everything may. Notes elsewhere still warm, conduct, and
    /// propagate — they just never surface into context.
    pub flash_dirs: Vec<String>,
}

impl Default for ActivationConfig {
    fn default() -> Self {
        Self {
            cognitive_bump: 1.0,
            ambient_bump: 0.5,
            flash_threshold: 1.0,
            propagation_factor: 0.5,
            propagation_hops: 3,
            decay_factor: 0.8,
            semantic_factor: 0.25,
            semantic_top_k: 3,
            semantic_threshold: 0.65,
            resonance_factor: 0.2,
            resonance_top_k: 5,
            resonance_threshold: 0.5,
            tool_resonance_factor: 0.8,
            search_top_k: 8,
            flash_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum AdapterConfig {
    Discord {
        /// Optional: without a guild the adapter is DM-only.
        guild_id: Option<String>,
        #[serde(default)]
        channels: Vec<String>,
        /// Names the environment variable holding the bot token.
        token_env: String,
    },
    Local {
        port: u16,
    },
}

impl Config {
    /// Every secret-bearing variable name in the config — the scrub
    /// list for tool child environments (wall chs. 07, 09).
    pub fn secret_env_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .models
            .values()
            .filter_map(|m| m.api_key_env.clone())
            .collect();
        for agent in self.agents.values() {
            for adapter in &agent.adapters {
                if let AdapterConfig::Discord { token_env, .. } = adapter {
                    names.push(token_env.clone());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }
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

        let a = &agent.activation;
        if a.decay_factor <= 0.0 || a.decay_factor >= 1.0 {
            errors.push(format!(
                "agent {agent_name}: activation.decay_factor must be in (0, 1), got {}",
                a.decay_factor
            ));
        }
        if a.flash_threshold <= 0.0 {
            errors.push(format!(
                "agent {agent_name}: activation.flash_threshold must be positive, got {}",
                a.flash_threshold
            ));
        }
        for (name, value) in [
            ("cognitive_bump", a.cognitive_bump),
            ("ambient_bump", a.ambient_bump),
            ("propagation_factor", a.propagation_factor),
            ("semantic_factor", a.semantic_factor),
            ("resonance_factor", a.resonance_factor),
            ("tool_resonance_factor", a.tool_resonance_factor),
        ] {
            if value < 0.0 {
                errors.push(format!(
                    "agent {agent_name}: activation.{name} must not be negative, got {value}"
                ));
            }
        }
        for (name, value) in [
            ("semantic_threshold", a.semantic_threshold),
            ("resonance_threshold", a.resonance_threshold),
        ] {
            if !(0.0..=1.0).contains(&value) {
                errors.push(format!(
                    "agent {agent_name}: activation.{name} must be in [0, 1], got {value}"
                ));
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
        assert_eq!(ada.context.min_messages, 50);
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
    fn activation_block_parses_partially_and_validates() {
        let text = r#"{
          "models": {
            "m": { "provider": "anthropic", "endpoint": "e", "name": "n" }
          },
          "agents": {
            "ada": {
              "workspace": "/ws", "data_dir": "/d", "model": "m",
              "activation": {
                "tool_resonance_factor": 0.6,
                "flash_dirs": ["knowledge", "embeddings/atomic"]
              }
            }
          }
        }"#;
        let config = parse(text).unwrap();
        validate(&config).unwrap();
        let a = &config.agents["ada"].activation;
        assert_eq!(a.tool_resonance_factor, 0.6, "overridden");
        assert_eq!(a.cognitive_bump, 1.0, "default preserved");
        assert_eq!(a.flash_dirs.len(), 2);

        let bad = text.replace("\"tool_resonance_factor\": 0.6", "\"decay_factor\": 1.5");
        let config = parse(&bad).unwrap();
        let errors = validate(&config).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("decay_factor")), "{errors:?}");
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
