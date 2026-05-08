use clap::Parser;
use river_tui::config::{Args, TuiConfig};
use river_tui::gateway::GatewayClient;
use river_tui::server::create_router;
use river_tui::state::SharedState;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = TuiConfig::from_args(args)?;

    // Log to file, not stdout — stdout is owned by ratatui
    let log_file = std::fs::File::create(&config.log_file)?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("Starting River TUI Adapter");
    tracing::info!("Gateway: {}", config.gateway_url);
    tracing::info!("User: {}", config.user_name);
    tracing::info!("Channel: {}", config.channel);

    let state = SharedState::new();
    let gateway = Arc::new(GatewayClient::new(
        config.gateway_url.clone(),
        config.auth_token.clone(),
    ));

    // Spawn HTTP server — bind to configured port (0 = OS-assigned)
    let http_state = state.clone();
    let listener = tokio::net::TcpListener::bind(
        format!("127.0.0.1:{}", config.listen_port)
    ).await?;
    let actual_port = listener.local_addr()?.port();
    tracing::info!("HTTP server listening on 127.0.0.1:{}", actual_port);

    let server_state = state.clone();
    tokio::spawn(async move {
        let app = create_router(http_state);
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "HTTP server failed");
            server_state.set_server_healthy(false);
        }
    });

    // Register with gateway (non-blocking, non-fatal)
    let gw_for_register = gateway.clone();
    tokio::spawn(async move {
        for attempt in 1..=3 {
            match gw_for_register.register(actual_port).await {
                Ok(()) => {
                    tracing::info!("Registered with gateway on attempt {}", attempt);
                    return;
                }
                Err(e) => {
                    tracing::warn!("Failed to register with gateway (attempt {}): {}", attempt, e);
                    tokio::time::sleep(std::time::Duration::from_secs(5 * attempt as u64)).await;
                }
            }
        }
        tracing::warn!("Failed to register with gateway after 3 attempts (continuing anyway)");
    });

    // Spawn gateway health check loop
    let gw_for_health = gateway.clone();
    let state_for_health = state.clone();
    tokio::spawn(async move {
        loop {
            let reachable = gw_for_health.health_check().await;
            state_for_health.set_gateway_connected(reachable);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    // Run TUI (blocks until Ctrl-C, terminal cleanup is guaranteed)
    river_tui::tui::run(state, gateway, config.user_name, config.channel).await?;

    Ok(())
}
