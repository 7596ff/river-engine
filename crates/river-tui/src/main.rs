use clap::Parser;
use river_tui::config::{Args, TuiConfig};
use river_tui::post::BystanderClient;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args = Args::parse();
    let config = TuiConfig::from_args(args);

    // Log to file — stdout is owned by ratatui
    let log_file = std::fs::File::create("river-tui.log")?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("Starting river-tui for agent: {}", config.agent);

    let client = Arc::new(BystanderClient::new(
        config.bystander_url(),
        config.auth_token.clone(),
    ));

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    let file = config.file.clone();
    tokio::spawn(async move {
        river_tui::input::run_reader(file, tx).await;
    });

    river_tui::render::run(config.agent, rx, client).await?;

    Ok(())
}
