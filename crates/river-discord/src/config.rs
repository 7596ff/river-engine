//! Configuration types

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-discord")]
#[command(about = "River Engine Discord Adapter")]
pub struct Args {
    /// Discord bot token file
    #[arg(long)]
    pub token_file: PathBuf,

    /// River gateway URL
    #[arg(long, default_value = "http://localhost:3000")]
    pub gateway_url: String,

    /// Port for adapter HTTP server
    #[arg(long, default_value = "3002")]
    pub listen_port: u16,

    /// Initial channel IDs (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub channels: Vec<u64>,

    /// State file for channel persistence
    #[arg(long)]
    pub state_file: Option<PathBuf>,

    /// Guild ID for slash command registration
    #[arg(long)]
    pub guild_id: u64,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    pub token: String,
    pub gateway_url: String,
    pub listen_port: u16,
    pub initial_channels: Vec<u64>,
    pub state_file: Option<PathBuf>,
    pub guild_id: u64,
}

impl DiscordConfig {
    /// Load configuration from CLI args
    pub fn from_args(args: Args) -> anyhow::Result<Self> {
        let token = std::fs::read_to_string(&args.token_file)
            .map_err(|e| anyhow::anyhow!("Failed to read token file: {}", e))?
            .trim()
            .to_string();

        if token.is_empty() {
            anyhow::bail!("Token file is empty");
        }

        Ok(Self {
            token,
            gateway_url: args.gateway_url,
            listen_port: args.listen_port,
            initial_channels: args.channels,
            state_file: args.state_file,
            guild_id: args.guild_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_defaults() {
        // Verify clap parsing works with minimal args
        let args = Args::parse_from([
            "river-discord",
            "--token-file", "/tmp/token",
            "--guild-id", "123456",
        ]);
        assert_eq!(args.gateway_url, "http://localhost:3000");
        assert_eq!(args.listen_port, 3002);
        assert!(args.channels.is_empty());
    }

    #[test]
    fn test_args_with_channels() {
        let args = Args::parse_from([
            "river-discord",
            "--token-file", "/tmp/token",
            "--guild-id", "123456",
            "--channels", "111,222,333",
        ]);
        assert_eq!(args.channels, vec![111, 222, 333]);
    }
}
