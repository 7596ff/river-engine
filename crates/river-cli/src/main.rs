//! The `river` runner (wall ch. 09): one config, however many
//! gateways. Parse and validate everything before spawning anything;
//! supervise with exponential backoff (1s→60s, reset after 5 healthy
//! minutes); forward child output with `[name]` prefixes; on
//! Ctrl-C/SIGTERM cascade gracefully — SIGTERM each gateway, a grace
//! period long enough to finish a turn, then SIGKILL stragglers.
//! Unbirthed agents are reported (with the birth command) and
//! skipped; the rest start.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use river_core::{config, env_file};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::watch;

const GRACE: Duration = Duration::from_secs(30);
const HEALTHY_RESET: Duration = Duration::from_secs(300);

#[derive(Parser)]
#[command(name = "river", version, about = "Run river gateways from one config file.")]
struct Cli {
    /// Path to the river.json config file.
    #[arg(long)]
    config: PathBuf,

    /// Path to a .env file with secrets.
    #[arg(long)]
    env_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Health of each agent, from its local surface.
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(env_path) = &cli.env_file {
        let text = std::fs::read_to_string(env_path)
            .with_context(|| format!("reading env file {}", env_path.display()))?;
        let pairs = env_file::parse(&text).map_err(|errors| {
            anyhow::anyhow!(config::render_errors("invalid env file:", &errors))
        })?;
        env_file::apply(pairs);
    }

    let raw = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("reading config {}", cli.config.display()))?;
    let expanded = config::expand_vars(&raw, |name| std::env::var(name).ok())
        .map_err(|errors| anyhow::anyhow!(config::render_errors("config expansion:", &errors)))?;
    let cfg = config::parse(&expanded).map_err(|e| anyhow::anyhow!(e))?;
    config::validate(&cfg)
        .map_err(|errors| anyhow::anyhow!(config::render_errors("invalid config:", &errors)))?;

    match cli.command {
        Some(Command::Status) => status(&cfg).await,
        None => run(&cfg, &cli).await,
    }
}

fn gateway_path() -> PathBuf {
    // Sibling of this binary first (the workspace build), PATH second.
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent()
    {
        let sibling = dir.join("river-gateway");
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("river-gateway")
}

async fn run(cfg: &config::Config, cli: &Cli) -> anyhow::Result<()> {
    let gateway = gateway_path();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    tokio::spawn(async move {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("installing SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
        eprintln!("[river] stopping: cascading SIGTERM to gateways");
        let _ = shutdown_tx.send(true);
    });

    let mut supervisors = Vec::new();
    for (name, agent) in &cfg.agents {
        let birth = agent.workspace.join("record").join("birth.json");
        if !birth.exists() {
            eprintln!(
                "[river] {name} is unbirthed — skipping. Birth it first:\n  \
                 river-gateway birth --workspace {} --name {name}",
                agent.workspace.display()
            );
            continue;
        }
        supervisors.push(tokio::spawn(supervise(
            name.clone(),
            gateway.clone(),
            cli.config.clone(),
            cli.env_file.clone(),
            shutdown_rx.clone(),
        )));
    }
    if supervisors.is_empty() {
        anyhow::bail!("no agents to run");
    }
    for supervisor in supervisors {
        let _ = supervisor.await;
    }
    eprintln!("[river] all gateways stopped");
    Ok(())
}

async fn supervise(
    name: String,
    gateway: PathBuf,
    config_path: PathBuf,
    env_file: Option<PathBuf>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if *shutdown.borrow() {
            return;
        }
        let started = std::time::Instant::now();
        let mut command = tokio::process::Command::new(&gateway);
        command
            .arg("run")
            .arg("--config")
            .arg(&config_path)
            .arg("--agent")
            .arg(&name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(env_path) = &env_file {
            command.arg("--env-file").arg(env_path);
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                eprintln!("[river] {name}: failed to spawn gateway: {e}");
                return;
            }
        };
        eprintln!("[river] {name}: started (pid {:?})", child.id());

        forward(child.stdout.take(), name.clone());
        forward(child.stderr.take(), name.clone());

        let status = tokio::select! {
            status = child.wait() => status,
            _ = async { let _ = shutdown.wait_for(|&s| s).await; } => {
                // Graceful cascade: SIGTERM, a turn-sized grace, SIGKILL.
                if let Some(pid) = child.id() {
                    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                }
                match tokio::time::timeout(GRACE, child.wait()).await {
                    Ok(status) => status,
                    Err(_) => {
                        eprintln!("[river] {name}: grace expired; SIGKILL");
                        let _ = child.kill().await;
                        child.wait().await
                    }
                }
            }
        };

        if *shutdown.borrow() {
            eprintln!("[river] {name}: stopped ({status:?})");
            return;
        }
        eprintln!(
            "[river] {name}: exited ({status:?}); restarting in {backoff:?}"
        );
        if started.elapsed() >= HEALTHY_RESET {
            backoff = Duration::from_secs(1);
        }
        tokio::select! {
            _ = async { let _ = shutdown.wait_for(|&s| s).await; } => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

fn forward(pipe: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>, name: String) {
    if let Some(pipe) = pipe {
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                println!("[{name}] {line}");
            }
        });
    }
}

async fn status(cfg: &config::Config) -> anyhow::Result<()> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    for (name, agent) in &cfg.agents {
        let port = agent.adapters.iter().find_map(|adapter| match adapter {
            config::AdapterConfig::Local { port } => Some(*port),
            _ => None,
        });
        let Some(port) = port else {
            println!("{name}: no local surface configured");
            continue;
        };
        match http
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
        {
            Ok(response) => {
                let health: serde_json::Value = response.json().await.unwrap_or_default();
                println!(
                    "{name}: turn {} · context {}% · witness lag {} · queue {} · settled {}",
                    health["turn_number"],
                    health["context_percent"],
                    health["witness_lag"],
                    health["queue_depth"],
                    health["last_settle"].as_str().unwrap_or("never"),
                );
            }
            Err(e) => println!("{name}: unreachable ({e})"),
        }
    }
    Ok(())
}
