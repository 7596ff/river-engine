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

    /// Gateway port
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Model server URL
    #[arg(long)]
    model_url: Option<String>,

    /// Model name
    #[arg(long)]
    model_name: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Gateway");
    tracing::info!("Workspace: {:?}", args.workspace);
    tracing::info!("Data dir: {:?}", args.data_dir);
    tracing::info!("Port: {}", args.port);

    let config = ServerConfig {
        workspace: args.workspace,
        data_dir: args.data_dir,
        port: args.port,
        model_url: args.model_url,
        model_name: args.model_name,
    };

    run(config).await
}
