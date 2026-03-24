//! River Engine Migration Tool
//!
//! This tool helps agents migrate into the River Engine system by:
//! - Creating a properly initialized database with birth memory
//! - Importing conversation history from various formats
//! - Setting up agent identity and sessions
//! - Importing memories (text only - embeddings generated separately)
//!
//! The birth memory ("i am <name>") is created first, encoding the AgentBirth
//! timestamp in its Snowflake ID. This is required for the gateway to start.
//!
//! # Usage
//!
//! ```bash
//! # Initialize a new agent database with birth
//! river-migrate init --agent-name my-agent --output ./data/river.db --birth 2024-01-15T10:30:00Z
//!
//! # Import conversation history from JSON
//! river-migrate import-messages --db ./data/river.db --input conversations.json
//!
//! # Import memories from JSON
//! river-migrate import-memories --db ./data/river.db --input memories.json
//!
//! # Full migration in one command (birth is required)
//! river-migrate migrate \
//!     --agent-name my-agent \
//!     --output ./data/river.db \
//!     --birth 2024-01-15T10:30:00Z \
//!     --messages conversations.json \
//!     --memories memories.json
//! ```
//!
//! # Input Formats
//!
//! ## Messages (conversations.json)
//!
//! ```json
//! {
//!   "messages": [
//!     {
//!       "role": "user",
//!       "content": "Hello!",
//!       "timestamp": "2024-01-15T10:30:00Z"
//!     },
//!     {
//!       "role": "assistant",
//!       "content": "Hi there! How can I help?",
//!       "timestamp": "2024-01-15T10:30:05Z"
//!     }
//!   ]
//! }
//! ```
//!
//! ## Memories (memories.json)
//!
//! ```json
//! {
//!   "memories": [
//!     {
//!       "content": "User prefers concise responses",
//!       "source": "preference",
//!       "timestamp": "2024-01-15T10:30:00Z"
//!     }
//!   ]
//! }
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use river_core::{AgentBirth, SnowflakeGenerator, SnowflakeType};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "river-migrate")]
#[command(about = "Migration tool for importing agent data into River Engine")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new River Engine database
    Init {
        /// Agent name for the new database
        #[arg(long)]
        agent_name: String,

        /// Output database path
        #[arg(long, short)]
        output: PathBuf,

        /// Agent birth time (ISO 8601, defaults to now)
        #[arg(long)]
        birth: Option<String>,
    },

    /// Import messages from JSON file
    ImportMessages {
        /// Database path
        #[arg(long)]
        db: PathBuf,

        /// Input JSON file with messages
        #[arg(long, short)]
        input: PathBuf,

        /// Session ID (defaults to "main")
        #[arg(long, default_value = "main")]
        session: String,
    },

    /// Import memories from JSON file
    ImportMemories {
        /// Database path
        #[arg(long)]
        db: PathBuf,

        /// Input JSON file with memories
        #[arg(long, short)]
        input: PathBuf,
    },

    /// Full migration: init + import messages + import memories
    Migrate {
        /// Agent name
        #[arg(long)]
        agent_name: String,

        /// Output database path
        #[arg(long, short)]
        output: PathBuf,

        /// Agent birth time (ISO 8601 format, e.g., 2024-01-15T10:30:00Z)
        /// Required for migration to establish agent identity
        #[arg(long)]
        birth: String,

        /// Messages JSON file (optional)
        #[arg(long)]
        messages: Option<PathBuf>,

        /// Memories JSON file (optional)
        #[arg(long)]
        memories: Option<PathBuf>,

        /// Session ID for messages (defaults to "main")
        #[arg(long, default_value = "main")]
        session: String,
    },

    /// Export template JSON files showing expected formats
    ExportTemplates {
        /// Output directory for templates
        #[arg(long, short, default_value = ".")]
        output_dir: PathBuf,
    },

    /// Show database info
    Info {
        /// Database path
        #[arg(long)]
        db: PathBuf,
    },
}

// Input format structures

#[derive(Debug, Deserialize)]
struct MessagesInput {
    messages: Vec<MessageInput>,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageInput {
    role: String,
    content: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    tool_calls: Option<serde_json::Value>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct MemoriesInput {
    memories: Vec<MemoryInput>,
}

#[derive(Debug, Deserialize)]
struct MemoryInput {
    content: String,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

fn default_source() -> String {
    "import".to_string()
}

// Template structures for export

#[derive(Debug, Serialize)]
struct MessagesTemplate {
    messages: Vec<MessageTemplate>,
    session_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct MessageTemplate {
    role: String,
    content: String,
    timestamp: String,
    tool_calls: Option<serde_json::Value>,
    tool_call_id: Option<String>,
    name: Option<String>,
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct MemoriesTemplate {
    memories: Vec<MemoryTemplate>,
}

#[derive(Debug, Serialize)]
struct MemoryTemplate {
    content: String,
    source: String,
    timestamp: String,
    expires_at: Option<String>,
    metadata: Option<serde_json::Value>,
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            agent_name,
            output,
            birth,
        } => cmd_init(&agent_name, &output, birth.as_deref()),

        Commands::ImportMessages { db, input, session } => {
            cmd_import_messages(&db, &input, &session)
        }

        Commands::ImportMemories { db, input } => cmd_import_memories(&db, &input),

        Commands::Migrate {
            agent_name,
            output,
            birth,
            messages,
            memories,
            session,
        } => cmd_migrate(&agent_name, &output, &birth, messages, memories, &session),

        Commands::ExportTemplates { output_dir } => cmd_export_templates(&output_dir),

        Commands::Info { db } => cmd_info(&db),
    }
}

fn cmd_init(agent_name: &str, output: &PathBuf, birth: Option<&str>) -> Result<()> {
    tracing::info!("Initializing new database for agent: {}", agent_name);

    // Parse or create agent birth
    let agent_birth = if let Some(birth_str) = birth {
        parse_birth(birth_str)?
    } else {
        let now = Utc::now();
        AgentBirth::new(
            now.year() as u16,
            now.month() as u8,
            now.day() as u8,
            now.hour() as u8,
            now.minute() as u8,
            now.second() as u8,
        )?
    };

    // Ensure parent directory exists
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Create database
    let conn = Connection::open(output)?;
    run_migrations(&conn)?;

    // Create snowflake generator with the birth
    let snowflake_gen = SnowflakeGenerator::new(agent_birth);

    // Create the birth memory - this is the agent's first memory
    // The AgentBirth is encoded in the Snowflake ID
    let birth_memory_id = snowflake_gen.next_id(SnowflakeType::Embedding);
    let now_ts = Utc::now().timestamp();
    let placeholder_embedding: Vec<u8> = vec![0u8; 384 * 4]; // 384 f32s as bytes

    conn.execute(
        "INSERT INTO memories (id, content, embedding, source, timestamp, expires_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
        rusqlite::params![
            birth_memory_id.to_bytes(),
            format!("i am {}", agent_name),
            placeholder_embedding,
            "system:birth",
            now_ts,
            serde_json::json!({
                "birth": format!("{}", agent_birth),
                "name": agent_name
            }).to_string()
        ],
    )?;

    // Create initial session
    conn.execute(
        "INSERT INTO sessions (id, agent_name, created_at, last_active, context_tokens, metadata)
         VALUES (?1, ?2, ?3, ?4, 0, ?5)",
        rusqlite::params![
            "main",
            agent_name,
            now_ts,
            now_ts,
            serde_json::json!({
                "agent_birth": format!("{}", agent_birth),
                "created_by": "river-migrate"
            })
            .to_string()
        ],
    )?;

    tracing::info!(
        "Database created at {:?} with agent birth: {}",
        output,
        agent_birth
    );
    println!("Created database: {:?}", output);
    println!("Agent name: {}", agent_name);
    println!("Agent birth: {}", agent_birth);
    println!("Birth memory: \"i am {}\" (ID: {})", agent_name, birth_memory_id);
    println!("\nNext steps:");
    println!("  1. Import messages: river-migrate import-messages --db {:?} --input messages.json", output);
    println!("  2. Import memories: river-migrate import-memories --db {:?} --input memories.json", output);
    println!("  3. Start gateway with: river-gateway --workspace <path> --data-dir {:?}", output.parent().unwrap_or(output));

    Ok(())
}

fn cmd_import_messages(db: &PathBuf, input: &PathBuf, session: &str) -> Result<()> {
    tracing::info!("Importing messages from {:?}", input);

    // Read input file
    let content = std::fs::read_to_string(input)
        .with_context(|| format!("Failed to read {:?}", input))?;
    let messages_input: MessagesInput = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {:?}", input))?;

    // Open database
    let conn = Connection::open(db)?;

    // Get agent birth from session metadata
    let metadata: String = conn.query_row(
        "SELECT metadata FROM sessions WHERE id = ?1",
        [session],
        |row| row.get(0),
    ).with_context(|| format!("Session '{}' not found. Run 'init' first.", session))?;

    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let birth_str = metadata["agent_birth"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No agent_birth in session metadata"))?;
    let agent_birth = parse_birth(birth_str)?;

    // Create snowflake generator
    let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));

    let session_id = messages_input.session_id.as_deref().unwrap_or(session);
    let mut imported = 0;

    for msg in messages_input.messages {
        let id = snowflake_gen.next_id(SnowflakeType::Message);
        let timestamp = if let Some(ts) = &msg.timestamp {
            DateTime::parse_from_rfc3339(ts)
                .map(|dt| dt.timestamp())
                .unwrap_or_else(|_| Utc::now().timestamp())
        } else {
            Utc::now().timestamp()
        };

        let tool_calls = msg.tool_calls.map(|v| v.to_string());
        let metadata = msg.metadata.map(|v| v.to_string());

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, name, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                id.to_bytes(),
                session_id,
                msg.role,
                msg.content,
                tool_calls,
                msg.tool_call_id,
                msg.name,
                timestamp,
                metadata
            ],
        )?;
        imported += 1;
    }

    // Update session last_active
    let now = Utc::now().timestamp();
    conn.execute(
        "UPDATE sessions SET last_active = ?1 WHERE id = ?2",
        rusqlite::params![now, session_id],
    )?;

    tracing::info!("Imported {} messages", imported);
    println!("Imported {} messages into session '{}'", imported, session_id);

    Ok(())
}

fn cmd_import_memories(db: &PathBuf, input: &PathBuf) -> Result<()> {
    tracing::info!("Importing memories from {:?}", input);

    // Read input file
    let content = std::fs::read_to_string(input)
        .with_context(|| format!("Failed to read {:?}", input))?;
    let memories_input: MemoriesInput = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {:?}", input))?;

    // Open database
    let conn = Connection::open(db)?;

    // Get agent birth from main session
    let metadata: String = conn.query_row(
        "SELECT metadata FROM sessions WHERE id = 'main'",
        [],
        |row| row.get(0),
    ).with_context(|| "Main session not found. Run 'init' first.")?;

    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let birth_str = metadata["agent_birth"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No agent_birth in session metadata"))?;
    let agent_birth = parse_birth(birth_str)?;

    // Create snowflake generator
    let snowflake_gen = Arc::new(SnowflakeGenerator::new(agent_birth));

    let mut imported = 0;

    for mem in memories_input.memories {
        let id = snowflake_gen.next_id(SnowflakeType::Embedding);

        let timestamp = if let Some(ts) = &mem.timestamp {
            DateTime::parse_from_rfc3339(ts)
                .map(|dt| dt.timestamp())
                .unwrap_or_else(|_| Utc::now().timestamp())
        } else {
            Utc::now().timestamp()
        };

        let expires_at = mem.expires_at.as_ref().and_then(|ts| {
            DateTime::parse_from_rfc3339(ts)
                .map(|dt| dt.timestamp())
                .ok()
        });

        let metadata = mem.metadata.map(|v| v.to_string());

        // Create placeholder embedding (zeros) - will be regenerated by embedding service
        let placeholder_embedding: Vec<u8> = vec![0u8; 768 * 4]; // 768 f32s as bytes

        conn.execute(
            "INSERT INTO memories (id, content, embedding, source, timestamp, expires_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id.to_bytes(),
                mem.content,
                placeholder_embedding,
                mem.source,
                timestamp,
                expires_at,
                metadata
            ],
        )?;
        imported += 1;
    }

    tracing::info!("Imported {} memories", imported);
    println!("Imported {} memories", imported);
    println!("\nNote: Memories have placeholder embeddings (zeros).");
    println!("To generate real embeddings, use the 'embed' tool or run:");
    println!("  river-migrate regenerate-embeddings --db {:?} --embedding-url <url>", db);

    Ok(())
}

fn cmd_migrate(
    agent_name: &str,
    output: &PathBuf,
    birth: &str,
    messages: Option<PathBuf>,
    memories: Option<PathBuf>,
    session: &str,
) -> Result<()> {
    // Initialize with the specified birth
    cmd_init(agent_name, output, Some(birth))?;

    // Import messages if provided
    if let Some(messages_path) = messages {
        cmd_import_messages(output, &messages_path, session)?;
    }

    // Import memories if provided
    if let Some(memories_path) = memories {
        cmd_import_memories(output, &memories_path)?;
    }

    println!("\nMigration complete!");
    println!("Database ready at: {:?}", output);

    Ok(())
}

fn cmd_export_templates(output_dir: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;

    // Messages template
    let messages_template = MessagesTemplate {
        messages: vec![
            MessageTemplate {
                role: "system".to_string(),
                content: "You are a helpful assistant.".to_string(),
                timestamp: "2024-01-15T10:00:00Z".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                metadata: None,
            },
            MessageTemplate {
                role: "user".to_string(),
                content: "Hello! What can you help me with?".to_string(),
                timestamp: "2024-01-15T10:30:00Z".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                metadata: Some(serde_json::json!({"source": "discord", "channel": "general"})),
            },
            MessageTemplate {
                role: "assistant".to_string(),
                content: "Hi! I can help with coding, writing, analysis, and much more.".to_string(),
                timestamp: "2024-01-15T10:30:05Z".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                metadata: None,
            },
            MessageTemplate {
                role: "assistant".to_string(),
                content: "".to_string(),
                timestamp: "2024-01-15T10:31:00Z".to_string(),
                tool_calls: Some(serde_json::json!([{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "read",
                        "arguments": "{\"path\": \"README.md\"}"
                    }
                }])),
                tool_call_id: None,
                name: None,
                metadata: None,
            },
            MessageTemplate {
                role: "tool".to_string(),
                content: "# Project README\nThis is the readme content.".to_string(),
                timestamp: "2024-01-15T10:31:01Z".to_string(),
                tool_calls: None,
                tool_call_id: Some("call_abc123".to_string()),
                name: Some("read".to_string()),
                metadata: None,
            },
        ],
        session_id: Some("main".to_string()),
    };

    let messages_path = output_dir.join("messages-template.json");
    std::fs::write(
        &messages_path,
        serde_json::to_string_pretty(&messages_template)?,
    )?;
    println!("Created: {:?}", messages_path);

    // Memories template
    let memories_template = MemoriesTemplate {
        memories: vec![
            MemoryTemplate {
                content: "User prefers concise, technical responses without unnecessary pleasantries.".to_string(),
                source: "preference".to_string(),
                timestamp: "2024-01-15T10:30:00Z".to_string(),
                expires_at: None,
                metadata: Some(serde_json::json!({"confidence": 0.9})),
            },
            MemoryTemplate {
                content: "Project uses Rust with tokio for async runtime.".to_string(),
                source: "project".to_string(),
                timestamp: "2024-01-15T11:00:00Z".to_string(),
                expires_at: None,
                metadata: Some(serde_json::json!({"file": "Cargo.toml"})),
            },
            MemoryTemplate {
                content: "Temporary note: PR #42 needs review by Friday.".to_string(),
                source: "task".to_string(),
                timestamp: "2024-01-15T12:00:00Z".to_string(),
                expires_at: Some("2024-01-19T00:00:00Z".to_string()),
                metadata: None,
            },
        ],
    };

    let memories_path = output_dir.join("memories-template.json");
    std::fs::write(
        &memories_path,
        serde_json::to_string_pretty(&memories_template)?,
    )?;
    println!("Created: {:?}", memories_path);

    println!("\nEdit these templates with your agent's data, then run:");
    println!("  river-migrate migrate --agent-name <name> --output ./data/river.db \\");
    println!("      --birth 2024-01-15T10:30:00Z \\");
    println!("      --messages messages-template.json --memories memories-template.json");

    Ok(())
}

fn cmd_info(db: &PathBuf) -> Result<()> {
    let conn = Connection::open(db)?;

    println!("Database: {:?}\n", db);

    // Sessions
    println!("Sessions:");
    let mut stmt = conn.prepare("SELECT id, agent_name, created_at, last_active, context_tokens FROM sessions")?;
    let sessions = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    for session in sessions {
        let (id, agent_name, created_at, last_active, tokens) = session?;
        let created = DateTime::from_timestamp(created_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let active = DateTime::from_timestamp(last_active, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        println!("  {} (agent: {}, created: {}, active: {}, tokens: {})",
            id, agent_name, created, active, tokens);
    }

    // Message count
    let msg_count: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    println!("\nMessages: {}", msg_count);

    // Memory count
    let mem_count: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    println!("Memories: {}", mem_count);

    // Memory sources
    if mem_count > 0 {
        println!("\nMemory sources:");
        let mut stmt = conn.prepare("SELECT source, COUNT(*) FROM memories GROUP BY source")?;
        let sources = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for source in sources {
            let (name, count) = source?;
            println!("  {}: {}", name, count);
        }
    }

    Ok(())
}

// Helper functions

fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            applied_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        ",
    )?;

    // 001_messages
    let applied: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = '001_messages')",
        [],
        |row| row.get(0),
    )?;
    if !applied {
        conn.execute_batch(include_str!("../../river-db/src/migrations/001_messages.sql"))?;
        conn.execute("INSERT INTO migrations (name) VALUES ('001_messages')", [])?;
    }

    // 002_memories
    let applied: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = '002_memories')",
        [],
        |row| row.get(0),
    )?;
    if !applied {
        conn.execute_batch(include_str!("../../river-db/src/migrations/002_memories.sql"))?;
        conn.execute("INSERT INTO migrations (name) VALUES ('002_memories')", [])?;
    }

    Ok(())
}

fn parse_birth(s: &str) -> Result<AgentBirth> {
    // Try ISO 8601 with timezone first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(AgentBirth::new(
            dt.year() as u16,
            dt.month() as u8,
            dt.day() as u8,
            dt.hour() as u8,
            dt.minute() as u8,
            dt.second() as u8,
        )?);
    }

    // Try AgentBirth display format: YYYY-MM-DDTHH:MM:SS (no timezone)
    if s.len() == 19 && s.contains('T') && s.contains(':') {
        let parts: Vec<&str> = s.split('T').collect();
        if parts.len() == 2 {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            let time_parts: Vec<&str> = parts[1].split(':').collect();
            if date_parts.len() == 3 && time_parts.len() == 3 {
                let year: u16 = date_parts[0].parse()?;
                let month: u8 = date_parts[1].parse()?;
                let day: u8 = date_parts[2].parse()?;
                let hour: u8 = time_parts[0].parse()?;
                let minute: u8 = time_parts[1].parse()?;
                let second: u8 = time_parts[2].parse()?;
                return Ok(AgentBirth::new(year, month, day, hour, minute, second)?);
            }
        }
    }

    // Try compact format: YYYYMMDDHHMMSS
    if s.len() == 14 {
        let year: u16 = s[0..4].parse()?;
        let month: u8 = s[4..6].parse()?;
        let day: u8 = s[6..8].parse()?;
        let hour: u8 = s[8..10].parse()?;
        let minute: u8 = s[10..12].parse()?;
        let second: u8 = s[12..14].parse()?;
        return Ok(AgentBirth::new(year, month, day, hour, minute, second)?);
    }

    anyhow::bail!("Invalid birth format. Use ISO 8601 (2024-01-15T10:30:00Z), local (2024-01-15T10:30:00), or compact (20240115103000)")
}

use chrono::Datelike;
use chrono::Timelike;
