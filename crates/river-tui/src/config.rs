//! CLI args and runtime config

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-tui")]
#[command(about = "River Engine TUI Adapter")]
pub struct Args {
    /// River gateway URL
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    pub gateway_url: String,

    /// Port for the TUI's HTTP server (0 = OS-assigned)
    #[arg(long, default_value = "0")]
    pub listen_port: u16,

    /// User display name
    #[arg(long)]
    pub name: Option<String>,

    /// Channel ID for messages
    #[arg(long, default_value = "terminal")]
    pub channel: String,

    /// Path to file containing gateway auth token
    #[arg(long)]
    pub auth_token_file: Option<PathBuf>,

    /// Log file path (default: river-tui.log in current directory)
    #[arg(long)]
    pub log_file: Option<PathBuf>,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub gateway_url: String,
    pub listen_port: u16,
    pub user_name: String,
    pub channel: String,
    pub auth_token: Option<String>,
    pub log_file: PathBuf,
}

impl TuiConfig {
    pub fn from_args(args: Args) -> anyhow::Result<Self> {
        let user_name = args.name.unwrap_or_else(|| {
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "user".to_string())
        });

        let auth_token = if let Some(ref path) = args.auth_token_file {
            let token = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("Failed to read auth token file: {}", e))?
                .trim()
                .to_string();
            Some(token)
        } else {
            None
        };

        let log_file = args.log_file.unwrap_or_else(|| PathBuf::from("river-tui.log"));

        Ok(Self {
            gateway_url: args.gateway_url,
            listen_port: args.listen_port,
            user_name,
            channel: args.channel,
            auth_token,
            log_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_defaults() {
        let args = Args::parse_from(["river-tui"]);
        assert_eq!(args.gateway_url, "http://127.0.0.1:3000");
        assert_eq!(args.listen_port, 0);
        assert_eq!(args.channel, "terminal");
        assert!(args.name.is_none());
    }

    #[test]
    fn test_args_custom() {
        let args = Args::parse_from([
            "river-tui",
            "--name", "cassie",
            "--channel", "dev",
            "--listen-port", "8082",
        ]);
        assert_eq!(args.name, Some("cassie".to_string()));
        assert_eq!(args.channel, "dev");
        assert_eq!(args.listen_port, 8082);
    }
}
