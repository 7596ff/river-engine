mod birth;
mod channels;
mod constitution;
mod context;
mod discord;
mod identity;
mod jsonl_index;
mod memory;
mod model;
mod moments;
mod record;
mod session;
mod shape;
mod surface;
mod tools;
mod turn;
mod witness;

use std::future::Future;
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use clap::{Args, Parser, Subcommand};
use river_core::{config, env_file};
use tokio::task::JoinSet;

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
    // Constitutional refusal gate (Article V.1) — the seal on the
    // workspace; refused before any adapter, session, or witness
    // task can start.
    constitution::verify(&agent.workspace)?;
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

    // Bring rejection_vectors back in sync with rejections.jsonl before
    // the witness starts gleaning. Best-effort: retrieval degrades to
    // empty on any failure here; the jsonl remains authoritative.
    if let Some(mem) = &mem {
        let rejections_path = agent.workspace.join("witness").join("rejections.jsonl");
        if let Err(e) = mem.ensure_rejection_vectors_ready(&rejections_path).await {
            tracing::warn!(error = %e, "rejection vector rebuild failed at startup");
        }
    }

    // Flash duty pair (spec 2026-07-13): witness posts frames, turn
    // loop lands them. Wired end-to-end when the flashes module
    // lands in Phase C; for now the receiver stays hooked up so the
    // turn loop keeps its receiver arg and no flashes fire until
    // the pass exists.
    let (_flash_tx, connect_rx) = tokio::sync::mpsc::channel(32);
    let witness = witness::Witness::load(
        &agent.workspace,
        witness_client,
        mem.clone(),
        agent.glean_probability,
        agent.witness.glean_min_new_turns,
        agent.witness.max_queue_depth,
        agent.witness.recent_rejections_window,
    )?
    .with_similar_rejections(
        agent.witness.similar_rejections_top_k,
        agent.witness.similar_rejections_threshold,
    );

    // Shape subsystem: queue + worker. Created here so both the sync
    // service (Source 4) and TurnLoop's ToolContext for write_atomic
    // (Source 3) can share the sender. The worker itself is spawned
    // as a background task later.
    let shape_setup = if agent.shape.enabled
        && let Some(mem) = &mem
    {
        let (tx, rx) = tokio::sync::mpsc::channel::<shape::GlossJob>(128);
        mem.set_shape_queue(Some(tx.clone()));

        // Load the witness identity — same system prompt every
        // witness duty uses (ch. 04). Required for the witness at
        // startup, so an error here has already fired.
        let witness_identity =
            std::fs::read_to_string(agent.workspace.join("witness").join("identity.md"))
                .unwrap_or_default();

        // Startup scans: enqueue missing rows and drift rows before
        // the worker starts draining. Best-effort — failures log
        // and the worker still runs.
        if let Err(e) = shape::enqueue_missing(mem, &agent.workspace, &tx).await {
            tracing::warn!(error = %e, "shape missing-rows scan failed");
        }
        let current_model_id = agent.witness_model_name().to_string();
        // Prompt hash for drift: read on-shape.md once to compute the
        // current hash. The worker reloads on each job so an edit
        // during the run picks up on the next drain.
        let mut prompt = shape::Prompt::at_workspace(&agent.workspace);
        let current_prompt_hash = prompt
            .load()
            .ok()
            .flatten()
            .map(|loaded| loaded.hash)
            .unwrap_or_default();
        if !current_prompt_hash.is_empty() {
            if let Err(e) =
                shape::enqueue_drift(mem, &tx, &current_model_id, &current_prompt_hash).await
            {
                tracing::warn!(error = %e, "shape drift-repair scan failed");
            }
        }

        Some((rx, tx, witness_identity, current_model_id))
    } else {
        None
    };
    let shape_write_tx = shape_setup.as_ref().map(|(_, tx, _, _)| tx.clone());

    let (notify_tx, notify_rx) = tokio::sync::mpsc::channel(256);
    let channels = channels::Channels::open(&agent.workspace, notify_tx)?;
    let (outbound_tx, _) = tokio::sync::broadcast::channel(256);
    let (health_tx, health_rx) = tokio::sync::watch::channel(turn::Health::default());
    let (snapshot_tx, snapshot_rx) =
        tokio::sync::watch::channel(context::ContextSnapshot::default());
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
                max_attachment_bytes: agent.attachments.max_bytes,
                download_timeout: std::time::Duration::from_secs(
                    agent.attachments.download_timeout_secs,
                ),
            };
            let (speak_tx, speak_rx) = tokio::sync::mpsc::channel(64);
            Some((settings, speak_tx, speak_rx))
        }
        None => None,
    };
    let discord_tx = discord_setup.as_ref().map(|(_, tx, _)| tx.clone());
    let (working_tx, working_rx) = tokio::sync::watch::channel(None);
    let resume = session::load(&agent.workspace.join("session.json"));
    if let Some(snap) = &resume {
        tracing::info!(
            channel = %snap.channel,
            turn_number = snap.turn_number,
            estimator_ratio = snap.estimator_ratio,
            quiet_seconds = snap.quiet_seconds,
            "resuming from session.json"
        );
    }

    let turn_loop = turn::TurnLoop::new(
        agent.workspace.clone(),
        tz,
        agent.context.clone(),
        client,
        channels.clone(),
        notify_rx,
        outbound_tx.clone(),
        health_tx,
        snapshot_tx,
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
        resume,
        Some(connect_rx),
        agent.atomic.max_words,
        shape_write_tx,
    )?;

    // Two phases: a process signal asks only the turn loop to stop. It
    // finishes the active turn and publishes its final settle before the
    // supervisor releases the witness and other background tasks. This
    // keeps the witness's guaranteed session-end pass on the true tail.
    let (turn_stop_tx, turn_stop_rx) = tokio::sync::watch::channel(false);
    let (background_shutdown_tx, background_shutdown_rx) = tokio::sync::watch::channel(false);

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
    let mut background_tasks = JoinSet::new();
    if let Some((settings, _tx, speak_rx)) = discord_setup {
        let shutdown = background_shutdown_rx.clone();
        let channels = channels.clone();
        background_tasks.spawn(async move {
            discord::run_supervised(settings, channels, speak_rx, shutdown, working_rx).await;
            ("discord adapter", anyhow::Ok(()))
        });
    }
    match local_port {
        Some(port) => {
            let shutdown = background_shutdown_rx.clone();
            let channels = channels.clone();
            let memory = mem.clone();
            background_tasks.spawn(async move {
                (
                    "local surface",
                    surface::serve(
                        port,
                        channels,
                        outbound_tx,
                        health_rx,
                        memory,
                        snapshot_rx,
                        shutdown,
                    )
                    .await,
                )
            });
        }
        None => {
            tracing::warn!("no local adapter configured; the agent wakes only by heartbeat");
        }
    }

    let shutdown = background_shutdown_rx.clone();
    background_tasks.spawn(async move {
        (
            "witness",
            witness.run(settled_rx, shutdown, last_settled).await,
        )
    });
    if let (Some(mem), Some(reindex_rx)) = (mem.clone(), reindex_rx) {
        let shutdown = background_shutdown_rx.clone();
        background_tasks.spawn(async move {
            mem.run_sync(reindex_rx, shutdown).await;
            ("memory sync", anyhow::Ok(()))
        });
    }
    if let (Some((rx, _tx, witness_identity, model_id)), Some(mem)) = (shape_setup, mem) {
        // Second witness-model client for the shape worker (the
        // witness owns its own; both talk to the same endpoint).
        let shape_client = model::ModelClient::new(witness_config)?;
        let workspace = agent.workspace.clone();
        let prompt = shape::Prompt::at_workspace(&agent.workspace);
        let shutdown = background_shutdown_rx.clone();
        background_tasks.spawn(async move {
            (
                "shape worker",
                shape::run_worker(
                    rx,
                    shape_client,
                    mem,
                    workspace,
                    prompt,
                    witness_identity,
                    model_id,
                    shutdown,
                )
                .await,
            )
        });
    }
    drop(background_shutdown_rx);

    supervise_gateway(
        turn_loop.run(turn_stop_rx),
        wait_for_shutdown_signal(),
        turn_stop_tx,
        background_shutdown_tx,
        background_tasks,
    )
    .await
}

type BackgroundTaskResult = (&'static str, anyhow::Result<()>);

async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("installing SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("listening for Ctrl-C"),
        signal = sigterm.recv() => match signal {
            Some(()) => Ok(()),
            None => Err(anyhow::anyhow!("SIGTERM signal stream closed")),
        },
    }
}

async fn supervise_gateway<F, S>(
    turn_loop: F,
    stop_signal: S,
    turn_stop: tokio::sync::watch::Sender<bool>,
    background_shutdown: tokio::sync::watch::Sender<bool>,
    mut background_tasks: JoinSet<BackgroundTaskResult>,
) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>>,
    S: Future<Output = anyhow::Result<()>>,
{
    tokio::pin!(turn_loop);
    tokio::pin!(stop_signal);
    let mut stop_signal_finished = false;
    let mut errors = Vec::new();

    let turn_result = loop {
        tokio::select! {
            result = &mut turn_loop => break result,
            result = &mut stop_signal, if !stop_signal_finished => {
                stop_signal_finished = true;
                match result {
                    Ok(()) => tracing::info!("signal received; finishing the current turn"),
                    Err(e) => {
                        tracing::error!(error = %e, "shutdown signal listener failed");
                        errors.push(format!("shutdown signal listener: {e:#}"));
                    }
                }
                let _ = turn_stop.send(true);
            }
            joined = background_tasks.join_next(), if !background_tasks.is_empty() => {
                if let Some(joined) = joined
                    && let Some(error) = background_task_error(joined, false)
                {
                    tracing::error!(error = %error, "background task exited; stopping gateway");
                    errors.push(error);
                    let _ = turn_stop.send(true);
                }
            }
        }
    };

    // The turn loop has settled and published the latest turn. Only now
    // may the witness run its guaranteed final pass and adapters stop.
    let _ = background_shutdown.send(true);
    while let Some(joined) = background_tasks.join_next().await {
        if let Some(error) = background_task_error(joined, true) {
            tracing::error!(error = %error, "background task failed during shutdown");
            errors.push(error);
        }
    }
    if let Err(e) = turn_result {
        errors.push(format!("turn loop: {e:#}"));
    }

    if errors.is_empty() {
        tracing::info!("shutdown complete: all gateway tasks stopped");
        Ok(())
    } else {
        Err(anyhow::anyhow!(format!(
            "gateway stopped with errors:\n  - {}",
            errors.join("\n  - ")
        )))
    }
}

fn background_task_error(
    joined: Result<BackgroundTaskResult, tokio::task::JoinError>,
    shutdown_expected: bool,
) -> Option<String> {
    match joined {
        Ok((_, Ok(()))) if shutdown_expected => None,
        Ok((name, Ok(()))) => Some(format!("{name} exited unexpectedly")),
        Ok((name, Err(e))) => Some(format!("{name}: {e:#}")),
        Err(e) if e.is_panic() => Some(format!("background task panicked: {e}")),
        Err(e) => Some(format!("background task was cancelled: {e}")),
    }
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

    #[tokio::test]
    async fn supervisor_settles_before_background_shutdown_and_awaits_cleanup() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (turn_stop_tx, mut turn_stop_rx) = tokio::sync::watch::channel(false);
        let (background_shutdown_tx, mut background_shutdown_rx) =
            tokio::sync::watch::channel(false);

        let turn_events = events.clone();
        let turn_loop = async move {
            turn_stop_rx.wait_for(|&stop| stop).await?;
            turn_events.lock().unwrap().push("turn settled");
            anyhow::Ok(())
        };

        let witness_events = events.clone();
        let mut background_tasks = JoinSet::new();
        background_tasks.spawn(async move {
            background_shutdown_rx.wait_for(|&stop| stop).await.unwrap();
            assert_eq!(
                witness_events.lock().unwrap().as_slice(),
                ["turn settled"],
                "background shutdown must not begin before settle"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            witness_events.lock().unwrap().push("witness finalized");
            ("witness", anyhow::Ok(()))
        });

        let stop_signal = async move {
            tokio::task::yield_now().await;
            anyhow::Ok(())
        };

        supervise_gateway(
            turn_loop,
            stop_signal,
            turn_stop_tx,
            background_shutdown_tx,
            background_tasks,
        )
        .await
        .unwrap();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["turn settled", "witness finalized"],
            "supervisor must await background cleanup"
        );
    }

    #[tokio::test]
    async fn supervisor_stops_turn_and_reports_early_background_failure() {
        let settled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (turn_stop_tx, mut turn_stop_rx) = tokio::sync::watch::channel(false);
        let (background_shutdown_tx, _background_shutdown_rx) = tokio::sync::watch::channel(false);

        let turn_settled = settled.clone();
        let turn_loop = async move {
            turn_stop_rx.wait_for(|&stop| stop).await?;
            turn_settled.store(true, std::sync::atomic::Ordering::SeqCst);
            anyhow::Ok(())
        };
        let mut background_tasks = JoinSet::new();
        background_tasks.spawn(async {
            (
                "memory sync",
                Err(anyhow::anyhow!("synthetic task failure")),
            )
        });

        let error = supervise_gateway(
            turn_loop,
            std::future::pending::<anyhow::Result<()>>(),
            turn_stop_tx,
            background_shutdown_tx,
            background_tasks,
        )
        .await
        .unwrap_err();

        assert!(settled.load(std::sync::atomic::Ordering::SeqCst));
        let message = error.to_string();
        assert!(message.contains("memory sync"), "{message}");
        assert!(message.contains("synthetic task failure"), "{message}");
    }
}
