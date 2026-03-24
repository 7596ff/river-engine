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

    /// Context window size in tokens
    #[arg(long, default_value = "131072")]
    context_limit: u32,

    /// Communication adapter configuration (can be repeated)
    /// Format: name:outbound_url[:read_url]
    /// Example: --adapter discord:http://localhost:8081/outbound:http://localhost:8081/read
    #[arg(long = "adapter", value_name = "CONFIG")]
    adapters: Vec<String>,

    /// Log file directory (default: {data-dir}/logs/)
    #[arg(long)]
    log_dir: Option<PathBuf>,

    /// Override log file path
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Output JSON logs to stdout (default: pretty for tty, json otherwise)
    #[arg(long)]
    json_stdout: bool,

    /// Log level (default: info, or RUST_LOG env)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Use experimental coordinator-based agent task
    #[arg(long)]
    use_coordinator: bool,
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

    use river_gateway::logging::{LogConfig, init_logging};

    let log_config = LogConfig {
        log_dir: args.log_dir.unwrap_or_else(|| data_dir.join("logs")),
        log_file: args.log_file,
        json_stdout: args.json_stdout,
        log_level: args.log_level.clone(),
    };

    let _log_guard = init_logging(&log_config)
        .map_err(|e| anyhow::anyhow!("Failed to initialize logging: {}", e))?;

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

    // Parse adapter configurations
    let mut adapter_configs = Vec::new();
    for adapter_str in &args.adapters {
        let parts: Vec<&str> = adapter_str.split(':').collect();
        if parts.len() < 2 {
            anyhow::bail!(
                "Invalid adapter config '{}'. Format: name:outbound_url[:read_url]",
                adapter_str
            );
        }

        // Handle URLs with : in them (http://...)
        // Format is: name:protocol://host:port/path[:protocol://host:port/path]
        let name = parts[0].to_string();

        // Find where outbound URL ends and read URL begins (if present)
        // Look for the pattern where we have another http:// or https://
        let rest = &adapter_str[name.len() + 1..]; // Skip "name:"
        let (outbound_url, read_url) = if let Some(idx) = rest.find(":http://").or_else(|| rest.find(":https://")) {
            let outbound = rest[..idx].to_string();
            let read = rest[idx + 1..].to_string();
            (outbound, Some(read))
        } else {
            (rest.to_string(), None)
        };

        tracing::info!(
            name = %name,
            outbound_url = %outbound_url,
            read_url = ?read_url,
            "Configured adapter"
        );

        adapter_configs.push((name, outbound_url, read_url));
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
        context_limit: args.context_limit,
        adapters: adapter_configs,
        use_coordinator: args.use_coordinator,
    };

    run(config).await
}
