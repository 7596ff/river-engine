//! CLI args and configuration

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-tui")]
#[command(about = "River Engine TUI — home channel viewer")]
pub struct Args {
    /// Agent name
    #[arg(long)]
    pub agent: String,

    /// Gateway URL
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    pub gateway_url: String,

    /// Path to JSONL file to tail (reads stdin if not given)
    #[arg(long)]
    pub file: Option<PathBuf>,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub agent: String,
    pub gateway_url: String,
    pub file: Option<PathBuf>,
    pub auth_token: Option<String>,
}

impl TuiConfig {
    pub fn from_args(args: Args) -> Self {
        let auth_token = std::env::var("RIVER_AUTH_TOKEN").ok();
        Self {
            agent: args.agent,
            gateway_url: args.gateway_url,
            file: args.file,
            auth_token,
        }
    }

    pub fn bystander_url(&self) -> String {
        format!("{}/home/{}/message", self.gateway_url, self.agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bystander_url() {
        let config = TuiConfig {
            agent: "iris".into(),
            gateway_url: "http://localhost:3000".into(),
            file: None,
            auth_token: None,
        };
        assert_eq!(
            config.bystander_url(),
            "http://localhost:3000/home/iris/message"
        );
    }
}
