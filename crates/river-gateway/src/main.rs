mod config;
mod env_file;

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "river-gateway",
    version,
    about = "One agent's harness: turn loop, witness, memory, adapters."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the gateway for one agent.
    Run(RunArgs),
}

#[derive(Args)]
struct RunArgs {
    /// Path to the river.json config file.
    #[arg(long)]
    config: PathBuf,

    /// Name of the agent entry in the config to run.
    #[arg(long)]
    agent: String,

    /// Path to a .env file with secrets. Already-set environment
    /// variables win over the file.
    #[arg(long)]
    env_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => run(args).await,
    }
}

async fn run(args: RunArgs) -> anyhow::Result<()> {
    if let Some(env_path) = &args.env_file {
        let text = std::fs::read_to_string(env_path)
            .with_context(|| format!("reading env file {}", env_path.display()))?;
        let pairs = env_file::parse(&text).map_err(|errors| {
            anyhow::anyhow!(config::render_errors(
                &format!("invalid env file {}:", env_path.display()),
                &errors
            ))
        })?;
        env_file::apply(pairs);
    }

    let raw = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading config {}", args.config.display()))?;
    let expanded = config::expand_vars(&raw, |name| std::env::var(name).ok())
        .map_err(|errors| anyhow::anyhow!(config::render_errors("config expansion:", &errors)))?;
    let cfg = config::parse(&expanded).map_err(|e| anyhow::anyhow!(e))?;
    config::validate(&cfg)
        .map_err(|errors| anyhow::anyhow!(config::render_errors("invalid config:", &errors)))?;

    let Some(agent) = cfg.agents.get(&args.agent) else {
        bail!(
            "agent {:?} not found in {} (configured: {})",
            args.agent,
            args.config.display(),
            cfg.agents.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    };

    tracing::info!(
        agent = %args.agent,
        workspace = %agent.workspace.display(),
        model = %agent.model,
        "river-gateway starting"
    );
    bail!("nothing to run yet: the gateway is a skeleton")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn run_args_parse() {
        let cli = Cli::parse_from([
            "river-gateway",
            "run",
            "--config",
            "river.json",
            "--agent",
            "ada",
        ]);
        let Command::Run(args) = cli.command;
        assert_eq!(args.agent, "ada");
        assert_eq!(args.config, PathBuf::from("river.json"));
        assert!(args.env_file.is_none());
    }
}
