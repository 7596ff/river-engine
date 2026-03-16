use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-gateway")]
#[command(about = "River Engine Gateway - Agent Runtime")]
struct Args {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Workspace directory
    #[arg(short, long)]
    workspace: PathBuf,

    /// Data directory for database
    #[arg(short, long)]
    data_dir: PathBuf,

    /// Gateway port
    #[arg(short, long, default_value = "3000")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    tracing::info!("Starting River Gateway");
    tracing::info!("Workspace: {:?}", args.workspace);
    tracing::info!("Data dir: {:?}", args.data_dir);
    tracing::info!("Port: {}", args.port);

    Ok(())
}
