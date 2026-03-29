//! River Oneshot: Turn-based dual-loop agent CLI
//!
//! A turn-based CLI agent that runs two concurrent loops:
//! - Reasoning (LLM) — proposes actions
//! - Execution (skills) — runs queued actions
//!
//! Both loops complete every cycle. First to finish becomes the output;
//! the other is cached for the next cycle.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

mod channels;
mod config;
mod context;
mod llm;
mod memory;
mod runtime;
mod skills;
mod types;

use channels::read_line;
use config::{CliOverrides, Config, Provider};
use runtime::Runtime;
use types::{CycleInput, TurnOutput};

/// Turn-based dual-loop agent CLI.
#[derive(Parser, Debug)]
#[command(name = "river-oneshot")]
#[command(about = "Turn-based dual-loop agent CLI")]
#[command(version)]
struct Cli {
    /// Path to config file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Workspace directory.
    #[arg(long, value_name = "PATH")]
    workspace: Option<PathBuf>,

    /// LLM model to use.
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// LLM provider: claude, openai, ollama.
    #[arg(long, value_name = "PROVIDER")]
    provider: Option<Provider>,

    /// Run a single cycle and exit.
    #[arg(long)]
    once: bool,

    /// Show verbose output.
    #[arg(short, long)]
    verbose: bool,

    /// Initial message (alternative to interactive input).
    #[arg(value_name = "INPUT")]
    input: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up logging
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("river_oneshot=debug")
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("river_oneshot=info")
            .init();
    }

    // Load configuration
    let overrides = CliOverrides {
        workspace: cli.workspace,
        model: cli.model,
        provider: cli.provider,
    };
    let config = Config::load(cli.config.as_ref(), &overrides)?;

    tracing::debug!(?config, "Loaded configuration");

    // Initialize runtime
    let mut runtime = Runtime::init(config).await?;

    // Initial input
    let mut input = CycleInput {
        user_message: cli.input,
        previous_output: None,
    };

    // If no initial input and not --once, prompt for it
    if input.user_message.is_none() && !cli.once {
        print!("> ");
        std::io::stdout().flush()?;
        let line = read_line()?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            input.user_message = Some(trimmed.to_string());
        }
    }

    loop {
        let output = runtime.cycle(input).await?;

        // Display output
        match &output {
            TurnOutput::Thought(plan) => {
                if let Some(response) = &plan.response {
                    println!("{}", response);
                }
                if !plan.actions.is_empty() {
                    println!("[queued {} action(s)]", plan.actions.len());
                    for action in &plan.actions {
                        println!("  -> {}", action.skill_name);
                    }
                }
            }
            TurnOutput::Action(result) => {
                println!("[{}] {}", result.skill_name, result.description);
                if !result.success {
                    println!(
                        "  error: {}",
                        result.error.as_deref().unwrap_or("unknown")
                    );
                }
            }
        }

        // Exit if --once mode
        if cli.once {
            break;
        }

        // Prompt for next cycle
        println!();
        print!("> ");
        std::io::stdout().flush()?;
        let line = read_line()?;
        let trimmed = line.trim();

        if trimmed == "q" || trimmed == "quit" || trimmed == "exit" {
            tracing::info!("Exiting");
            break;
        }

        input = CycleInput {
            user_message: if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            },
            previous_output: Some(output),
        };
    }

    Ok(())
}
