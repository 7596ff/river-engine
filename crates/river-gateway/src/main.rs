mod birth;
mod channels;
mod context;
mod discord;
mod identity;
mod memory;
mod model;
mod record;
mod surface;
mod tools;
mod turn;
mod witness;

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use clap::{Args, Parser, Subcommand};
use river_core::{config, env_file};

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
    /// The birth ritual: write the founding record for a new agent.
    Birth(BirthArgs),
}

#[derive(Args)]
struct BirthArgs {
    /// The agent's workspace directory.
    #[arg(long)]
    workspace: PathBuf,

    /// The agent's name.
    #[arg(long)]
    name: String,
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
        Command::Birth(args) => {
            let record = birth::perform_birth(&args.workspace, &args.name)?;
            println!(
                "born: {} ({}) at {} in {}",
                record.name,
                record.id,
                record.born_at,
                args.workspace.display()
            );
            Ok(())
        }
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

    let founding = birth::read_birth(&agent.workspace)?;
    // Fail-fast startup invariant (wall ch. 08); the turn loop
    // re-reads at every boundary thereafter.
    identity::load(&agent.workspace)?;
    let tz = identity::timezone(agent.timezone.as_deref())?;

    let model_config = cfg
        .models
        .get(&agent.model)
        .expect("validated: model reference resolves");
    let client = model::ModelClient::new(model_config)?;

    // The witness: its own client (often a cheaper model), its own
    // startup invariant — no witness identity, no gateway.
    let witness_config = cfg
        .models
        .get(agent.witness_model_name())
        .expect("validated: witness model reference resolves");
    let witness_client = model::ModelClient::new(witness_config)?;

    // The memory body, when an embedding model is configured.
    let (mem, reindex_tx, reindex_rx) = match &agent.embedding_model {
        Some(embed_ref) => {
            let embed_config = cfg
                .models
                .get(embed_ref)
                .expect("validated: embedding model reference resolves");
            let embedder = memory::EmbeddingClient::new(embed_config)?;
            let mem = memory::Memory::open_with(
                &agent.data_dir,
                &agent.workspace,
                &agent.index_dirs,
                std::sync::Arc::new(embedder),
                agent.activation.clone(),
            )?;
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            (Some(mem), Some(tx), Some(rx))
        }
        None => {
            tracing::info!("no embedding model configured; memory features disabled");
            (None, None, None)
        }
    };

    let witness = witness::Witness::load(
        &agent.workspace,
        witness_client,
        mem.clone(),
        agent.glean_probability,
    )?;

    let (notify_tx, notify_rx) = tokio::sync::mpsc::channel(256);
    let channels = channels::Channels::open(&agent.workspace, notify_tx)?;
    let (outbound_tx, _) = tokio::sync::broadcast::channel(256);
    let (health_tx, health_rx) = tokio::sync::watch::channel(turn::Health::default());
    let last_settled = record::last_turn(&agent.workspace.join("record").join("turns.jsonl"))?;
    let (settled_tx, settled_rx) = tokio::sync::watch::channel(last_settled);

    // Discord, when configured: token from the environment only.
    let discord_setup = match agent.adapters.iter().find_map(|adapter| match adapter {
        config::AdapterConfig::Discord {
            guild_id,
            channels,
            token_env,
        } => Some((guild_id.clone(), channels.clone(), token_env.clone())),
        _ => None,
    }) {
        Some((guild_id, listen_names, token_env)) => {
            let token = std::env::var(&token_env)
                .map_err(|_| anyhow::anyhow!("token_env {token_env} is not set"))?;
            let guild_id = match guild_id {
                Some(text) => Some(
                    text.parse()
                        .map_err(|_| anyhow::anyhow!("bad guild_id {text:?}"))?,
                ),
                None => None,
            };
            let settings = discord::DiscordSettings {
                guild_id,
                listen_names,
                token,
            };
            let (speak_tx, speak_rx) = tokio::sync::mpsc::channel(64);
            Some((settings, speak_tx, speak_rx))
        }
        None => None,
    };
    let discord_tx = discord_setup.as_ref().map(|(_, tx, _)| tx.clone());
    let (working_tx, working_rx) = tokio::sync::watch::channel(None);

    let turn_loop = turn::TurnLoop::new(
        agent.workspace.clone(),
        tz,
        agent.context.clone(),
        client,
        channels.clone(),
        notify_rx,
        outbound_tx.clone(),
        health_tx,
        settled_tx,
        std::time::Duration::from_secs(agent.heartbeat_minutes * 60),
        tools::Registry::core(),
        agent.tool_profile(),
        cfg.secret_env_names(),
        agent.max_iterations,
        mem.clone(),
        reindex_tx,
        discord_tx,
        working_tx,
    )?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("installing SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
        tracing::info!("signal received; finishing the current turn");
        let _ = shutdown_tx.send(true);
    });

    tracing::info!(
        agent = %args.agent,
        name = %founding.name,
        born_at = %founding.born_at,
        workspace = %agent.workspace.display(),
        model = %agent.model,
        "river-gateway running"
    );

    let local_port = agent.adapters.iter().find_map(|adapter| match adapter {
        config::AdapterConfig::Local { port } => Some(*port),
        _ => None,
    });
    if let Some((settings, _tx, speak_rx)) = discord_setup {
        tokio::spawn(discord::run_supervised(
            settings,
            channels.clone(),
            speak_rx,
            shutdown_rx.clone(),
            working_rx,
        ));
    }
    match local_port {
        Some(port) => {
            tokio::spawn(surface::serve(
                port,
                channels.clone(),
                outbound_tx,
                health_rx,
                shutdown_rx.clone(),
            ));
        }
        None => {
            tracing::warn!("no local adapter configured; the agent wakes only by heartbeat");
        }
    }

    tokio::spawn(witness.run(settled_rx, shutdown_rx.clone()));
    if let (Some(mem), Some(reindex_rx)) = (mem, reindex_rx) {
        tokio::spawn(mem.run_sync(reindex_rx, shutdown_rx.clone()));
    }

    turn_loop.run(shutdown_rx).await
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
        let Command::Run(args) = cli.command else {
            panic!("expected run subcommand");
        };
        assert_eq!(args.agent, "ada");
        assert_eq!(args.config, PathBuf::from("river.json"));
        assert!(args.env_file.is_none());
    }
}
