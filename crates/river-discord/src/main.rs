use clap::Parser;
use river_discord::{
    channels::ChannelState,
    client::DiscordClient,
    commands::{handle_interaction, register_commands},
    config::{Args, DiscordConfig},
    discord_adapter_info,
    gateway::GatewayClient,
    handler::EventHandler,
    outbound::{create_router, AppState},
    register_with_gateway,
};
use std::net::SocketAddr;
use std::sync::Arc;
use twilight_gateway::Event;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let config = DiscordConfig::from_args(args)?;

    let auth_token = river_core::require_auth_token().map_err(|e| anyhow::anyhow!("{}", e))?;
    let authed_client = river_core::build_authed_client(&auth_token);

    tracing::info!("Starting River Discord Adapter");

    // Load channel state
    let channels =
        ChannelState::load(config.initial_channels.clone(), config.state_file.clone()).await;
    tracing::info!("Loaded channel state");

    // Create gateway client (with auth)
    let gateway_client = Arc::new(GatewayClient::new(
        authed_client.clone(),
        config.gateway_url.clone(),
    ));

    // Create Discord client
    let mut discord = DiscordClient::new(&config.token, config.guild_id).await?;
    tracing::info!("Connected to Discord");

    // Get application ID for slash commands
    let app_info = discord
        .http()
        .current_user_application()
        .await?
        .model()
        .await?;
    let application_id = app_info.id;

    // Register slash commands
    register_commands(discord.http(), application_id, discord.guild_id()).await?;

    // Create event handler
    let event_handler = EventHandler::new(channels.clone(), gateway_client.clone());

    // Create HTTP server state (with auth)
    let http_state = AppState::new(channels.clone(), config.listen_port, auth_token);

    // Set up Discord sender for outbound messages
    http_state.set_discord(discord.sender()).await;

    // Spawn HTTP server
    let http_state_clone = http_state.clone();
    let listen_port = config.listen_port;
    tokio::spawn(async move {
        let app = create_router(http_state_clone);
        let addr = SocketAddr::from(([127, 0, 0, 1], listen_port));
        tracing::info!("HTTP server listening on {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // Self-register with gateway (non-blocking, non-fatal, with auth)
    let adapter_info = discord_adapter_info(config.listen_port, Some(application_id.to_string()));
    let gateway_url = config.gateway_url.clone();
    let reg_client = authed_client.clone();
    tokio::spawn(async move {
        for attempt in 1..=3 {
            match register_with_gateway(&reg_client, &gateway_url, adapter_info.clone()).await {
                Ok(()) => {
                    tracing::info!("Registered with gateway on attempt {}", attempt);
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to register with gateway (attempt {}): {}",
                        attempt,
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5 * attempt as u64)).await;
                }
            }
        }
        tracing::warn!("Failed to register with gateway after 3 attempts (continuing anyway)");
    });

    // Spawn gateway health check loop
    let gateway_for_health = gateway_client.clone();
    let state_for_health = http_state.clone();
    tokio::spawn(async move {
        loop {
            let reachable = gateway_for_health.health_check().await;
            state_for_health.set_gateway_reachable(reachable);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    // Main event loop
    tracing::info!("Entering event loop");
    loop {
        // Update connection status
        http_state.set_discord_connected(discord.is_connected());

        // Wait for next event
        let Some(event) = discord.next_event().await else {
            tracing::warn!("Discord connection closed, attempting reconnect...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        };

        match event {
            Event::Ready(_) => {
                tracing::info!("Discord ready");
                http_state.set_discord_connected(true);
            }
            Event::MessageCreate(msg) => {
                event_handler.handle_message(msg).await;
            }
            Event::ReactionAdd(reaction) => {
                event_handler.handle_reaction(reaction).await;
            }
            Event::InteractionCreate(interaction) => {
                let http = discord.http().clone();
                let channels = channels.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_interaction(&http, interaction.0, channels).await {
                        tracing::error!("Failed to handle interaction: {}", e);
                    }
                });
            }
            _ => {}
        }
    }
}
