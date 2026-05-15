use chrono::{Datelike, Timelike};
use clap::{Parser, Subcommand};
use river_core::{AgentBirth, Snowflake, SnowflakeGenerator, SnowflakeType};
use river_gateway::server::{run, ServerConfig};
use river_gateway::BirthRecord;
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

    /// Embedding model name
    #[arg(long)]
    embedding_model_name: Option<String>,

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

    /// Compaction threshold (fraction of context limit, e.g., 0.80)
    #[arg(long, default_value = "0.80")]
    compaction_threshold: f64,

    /// Post-compaction fill target (fraction of context limit, e.g., 0.40)
    #[arg(long, default_value = "0.40")]
    fill_target: f64,

    /// Minimum messages always kept in context
    #[arg(long, default_value = "20")]
    min_messages: u32,

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

    /// Spectator model server URL (defaults to same as agent)
    #[arg(long)]
    spectator_model_url: Option<String>,

    /// Spectator model name (defaults to same as agent)
    #[arg(long)]
    spectator_model_name: Option<String>,

    /// Env var name for agent model API key (e.g. DEEPSEEK_API_KEY)
    #[arg(long)]
    model_api_key_env: Option<String>,

    /// Env var name for spectator model API key
    #[arg(long)]
    spectator_api_key_env: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Birth a new agent (must be run once before first start)
    Birth {
        /// Data directory
        #[arg(short, long)]
        data_dir: PathBuf,

        /// Workspace directory
        #[arg(short, long)]
        workspace: PathBuf,

        /// Agent name
        #[arg(short, long)]
        name: String,
    },
}

fn birth_agent(data_dir: PathBuf, workspace: PathBuf, name: String) -> anyhow::Result<()> {
    println!("Birthing agent '{}'...", name);

    std::fs::create_dir_all(&data_dir)?;

    let birth_path = data_dir.join("birth.json");

    // Check if already birthed
    if birth_path.exists() {
        let existing: BirthRecord =
            serde_json::from_str(&std::fs::read_to_string(&birth_path)?)?;
        let birth = existing.id.birth();
        anyhow::bail!(
            "Agent already birthed at {}. Name: \"{}\"",
            birth,
            existing.name
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

    // Create snowflake generator and first ID
    let gen = SnowflakeGenerator::new(agent_birth);
    let birth_id = gen.next_id(SnowflakeType::Embedding);
    let record = BirthRecord {
        id: birth_id,
        name: name.clone(),
    };

    // Write birth file
    let json = serde_json::to_string_pretty(&record)?;
    std::fs::write(&birth_path, &json)?;

    // Write first home channel entry
    let home_dir = workspace.join("channels/home").join(&name);
    std::fs::create_dir_all(home_dir.join("moves"))?;
    std::fs::create_dir_all(home_dir.join("tool-results"))?;

    let home_channel_path = workspace
        .join("channels/home")
        .join(format!("{}.jsonl", name));

    let birth_entry = river_core::HomeChannelEntry::Message(
        river_core::MessageEntry::system_msg(
            birth_id,
            format!("agent '{}' born at {}", name, agent_birth),
        ),
    );
    let mut entry_json = serde_json::to_string(&birth_entry)?;
    entry_json.push('\n');
    std::fs::write(&home_channel_path, &entry_json)?;

    println!("Agent '{}' born at {}", name, agent_birth);
    println!("Birth ID: {}", birth_id);
    println!("Birth file: {:?}", birth_path);
    println!("Home channel: {:?}", home_channel_path);

    Ok(())
}

/// Load birth record from {data_dir}/birth.json
pub fn load_birth(data_dir: &std::path::Path) -> anyhow::Result<BirthRecord> {
    let birth_path = data_dir.join("birth.json");
    if !birth_path.exists() {
        anyhow::bail!(
            "Agent not birthed. Run `river-gateway birth --data-dir {:?} --name <name>` first.",
            data_dir
        );
    }
    let content = std::fs::read_to_string(&birth_path)?;
    let record: BirthRecord = serde_json::from_str(&content)?;
    Ok(record)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();

    // Handle birth subcommand
    if let Some(Command::Birth {
        data_dir,
        workspace,
        name,
    }) = args.command
    {
        return birth_agent(data_dir, workspace, name);
    }

    // Normal startup - requires workspace and data_dir
    let workspace = args
        .workspace
        .ok_or_else(|| anyhow::anyhow!("--workspace is required for normal operation"))?;
    let data_dir = args
        .data_dir
        .ok_or_else(|| anyhow::anyhow!("--data-dir is required for normal operation"))?;

    use river_gateway::logging::{init_logging, LogConfig};

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
        let (outbound_url, read_url) =
            if let Some(idx) = rest.find(":http://").or_else(|| rest.find(":https://")) {
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
        embedding_model_name: args.embedding_model_name,
        redis_url: args.redis_url,
        orchestrator_url: args.orchestrator_url,
        auth_token_file: args.auth_token_file,
        context_limit: args.context_limit,
        compaction_threshold: args.compaction_threshold,
        fill_target: args.fill_target,
        min_messages: args.min_messages as usize,
        adapters: adapter_configs,
        spectator_model_url: args.spectator_model_url,
        spectator_model_name: args.spectator_model_name,
        model_api_key_env: args.model_api_key_env,
        spectator_api_key_env: args.spectator_api_key_env,
    };

    run(config).await
}
