use clap::{Parser, Subcommand};
use river_gateway::server::{run, ServerConfig};
use river_gateway::db::{init_db, Memory};
use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};
use chrono::{Datelike, Timelike};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "river-gateway")]
#[command(about = "River Engine Gateway - Agent Runtime")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Workspace directory
    #[arg(short, long)]
    workspace: Option<PathBuf>,

    /// Data directory for database
    #[arg(short, long)]
    data_dir: Option<PathBuf>,

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

    /// Path to file containing bearer token for authentication
    #[arg(long)]
    auth_token_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Birth a new agent (must be run once before first start)
    Birth {
        /// Data directory for database
        #[arg(short, long)]
        data_dir: PathBuf,

        /// Agent name
        #[arg(short, long)]
        name: String,
    },
}

fn birth_agent(data_dir: PathBuf, name: String) -> anyhow::Result<()> {
    println!("Birthing agent '{}'...", name);

    // Initialize database
    let db_path = data_dir.join("river.db");
    std::fs::create_dir_all(&data_dir)?;
    let db = init_db(&db_path)?;

    // Check if already birthed
    if let Some(existing) = db.get_birth_memory()? {
        let birth = existing.id.birth();
        anyhow::bail!(
            "Agent already birthed at {}. Birth memory: \"{}\"",
            birth,
            existing.content
        );
    }

    // Create agent birth from current time
    let now = chrono::Utc::now();
    let agent_birth = AgentBirth::new(
        now.year() as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )?;

    // Create snowflake generator with this birth
    let gen = SnowflakeGenerator::new(agent_birth);

    // Create the first memory - the birth memory
    let birth_memory = Memory {
        id: gen.next_id(SnowflakeType::Embedding),
        content: format!("i am {}", name),
        embedding: vec![0.0; 384], // Placeholder embedding (will be re-embedded if needed)
        source: "system:birth".to_string(),
        timestamp: now.timestamp(),
        expires_at: None,
        metadata: Some(format!("{{\"birth\":\"{}\",\"name\":\"{}\"}}", agent_birth, name)),
    };

    db.insert_memory(&birth_memory)?;

    println!("Agent '{}' born at {}", name, agent_birth);
    println!("Birth memory ID: {}", birth_memory.id);
    println!("Database: {:?}", db_path);

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Handle birth subcommand
    if let Some(Command::Birth { data_dir, name }) = args.command {
        return birth_agent(data_dir, name);
    }

    // Normal startup - requires workspace and data_dir
    let workspace = args.workspace.ok_or_else(|| {
        anyhow::anyhow!("--workspace is required for normal operation")
    })?;
    let data_dir = args.data_dir.ok_or_else(|| {
        anyhow::anyhow!("--data-dir is required for normal operation")
    })?;

    tracing_subscriber::fmt::init();

    tracing::info!("Starting River Gateway");
    tracing::info!("Agent: {}", args.agent_name);
    tracing::info!("Workspace: {:?}", workspace);
    tracing::info!("Data dir: {:?}", data_dir);
    tracing::info!("Port: {}", args.port);

    if args.embedding_url.is_some() {
        tracing::info!("Embedding server: {:?}", args.embedding_url);
    }
    if args.redis_url.is_some() {
        tracing::info!("Redis: {:?}", args.redis_url);
    }

    let config = ServerConfig {
        workspace,
        data_dir,
        port: args.port,
        agent_name: args.agent_name,
        model_url: args.model_url,
        model_name: args.model_name,
        embedding_url: args.embedding_url,
        redis_url: args.redis_url,
        orchestrator_url: args.orchestrator_url,
        auth_token_file: args.auth_token_file,
    };

    run(config).await
}
