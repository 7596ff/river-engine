use clap::Parser;
use river_gateway::server::{run, ServerConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-gateway")]
#[command(about = "River Engine Gateway - Agent Runtime")]
struct Args {
    /// Workspace directory
    #[arg(short, long)]
    workspace: PathBuf,

    /// Data directory for database
    #[arg(short, long)]
    data_dir: PathBuf,

    /// Agent name (used for Redis namespacing)
    #[arg(long, default_value = "default")]
    agent_name: String,

    /// Gateway port
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Model server URL
    #[arg(long)]
    model_url: Option<String>,

    /// Model name
    #[arg(long)]
    model_name: Option<String>,

    /// Embedding server URL (enables memory tools)
    #[arg(long)]
    embedding_url: Option<String>,

    /// Redis URL (enables working/medium-term memory tools)
    #[arg(long)]
    redis_url: Option<String>,

    /// Orchestrator URL (enables heartbeats)
    #[arg(long)]
    orchestrator_url: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Gateway");
    tracing::info!("Agent: {}", args.agent_name);
    tracing::info!("Workspace: {:?}", args.workspace);
    tracing::info!("Data dir: {:?}", args.data_dir);
    tracing::info!("Port: {}", args.port);

    if args.embedding_url.is_some() {
        tracing::info!("Embedding server: {:?}", args.embedding_url);
    }
    if args.redis_url.is_some() {
        tracing::info!("Redis: {:?}", args.redis_url);
    }

    let config = ServerConfig {
        workspace: args.workspace,
        data_dir: args.data_dir,
        port: args.port,
        agent_name: args.agent_name,
        model_url: args.model_url,
        model_name: args.model_name,
        embedding_url: args.embedding_url,
        redis_url: args.redis_url,
        orchestrator_url: args.orchestrator_url,
    };

    run(config).await
}
