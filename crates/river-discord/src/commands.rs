//! Slash command registration and handling

use crate::channels::ChannelState;
use std::sync::Arc;
use twilight_http::Client as HttpClient;
use twilight_model::application::command::CommandType;
use twilight_model::application::interaction::application_command::CommandOptionValue;
use twilight_model::application::interaction::{Interaction, InteractionData};
use twilight_model::channel::message::MessageFlags;
use twilight_model::guild::Permissions;
use twilight_model::http::interaction::{InteractionResponse, InteractionResponseData, InteractionResponseType};
use twilight_model::id::marker::{ApplicationMarker, GuildMarker};
use twilight_model::id::Id;
use twilight_util::builder::command::{ChannelBuilder, CommandBuilder};

/// Register slash commands for a guild
pub async fn register_commands(
    http: &HttpClient,
    application_id: Id<ApplicationMarker>,
    guild_id: Id<GuildMarker>,
) -> anyhow::Result<()> {
    let commands = vec![
        CommandBuilder::new("listen", "Add a channel to the listen set", CommandType::ChatInput)
            .option(ChannelBuilder::new("channel", "The channel to listen to").required(true))
            .default_member_permissions(Permissions::MANAGE_CHANNELS)
            .build(),
        CommandBuilder::new(
            "unlisten",
            "Remove a channel from the listen set",
            CommandType::ChatInput,
        )
        .option(ChannelBuilder::new("channel", "The channel to stop listening to").required(true))
        .default_member_permissions(Permissions::MANAGE_CHANNELS)
        .build(),
        CommandBuilder::new(
            "channels",
            "List all channels being listened to",
            CommandType::ChatInput,
        )
        .default_member_permissions(Permissions::MANAGE_CHANNELS)
        .build(),
    ];

    http.interaction(application_id)
        .set_guild_commands(guild_id, &commands)
        .await?;

    tracing::info!("Registered slash commands for guild {}", guild_id);
    Ok(())
}

/// Handle an interaction
pub async fn handle_interaction(
    http: &HttpClient,
    interaction: Interaction,
    channels: Arc<ChannelState>,
) -> anyhow::Result<()> {
    // Extract command data from the interaction
    let Some(InteractionData::ApplicationCommand(command)) = interaction.data else {
        return Ok(());
    };

    let response_content = match command.name.as_str() {
        "listen" => {
            let channel_id = command
                .options
                .iter()
                .find(|o| o.name == "channel")
                .and_then(|o| match &o.value {
                    CommandOptionValue::Channel(id) => Some(id.get()),
                    _ => None,
                });

            if let Some(id) = channel_id {
                channels.add(id).await;
                format!("Now listening to <#{}>", id)
            } else {
                "Invalid channel".to_string()
            }
        }
        "unlisten" => {
            let channel_id = command
                .options
                .iter()
                .find(|o| o.name == "channel")
                .and_then(|o| match &o.value {
                    CommandOptionValue::Channel(id) => Some(id.get()),
                    _ => None,
                });

            if let Some(id) = channel_id {
                if channels.remove(id).await {
                    format!("Stopped listening to <#{}>", id)
                } else {
                    format!("<#{}> was not in the listen set", id)
                }
            } else {
                "Invalid channel".to_string()
            }
        }
        "channels" => {
            let list = channels.list().await;
            if list.is_empty() {
                "Not listening to any channels".to_string()
            } else {
                let channel_mentions: Vec<String> = list.iter().map(|id| format!("<#{}>", id)).collect();
                format!("Listening to: {}", channel_mentions.join(", "))
            }
        }
        _ => "Unknown command".to_string(),
    };

    // Send ephemeral response
    let response = InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            content: Some(response_content),
            flags: Some(MessageFlags::EPHEMERAL),
            ..Default::default()
        }),
    };

    http.interaction(interaction.application_id)
        .create_response(interaction.id, &interaction.token, &response)
        .await?;

    Ok(())
}
