//! Adapter registration and info

use river_adapter::{AdapterInfo, Feature, RegisterRequest, RegisterResponse};
use std::collections::HashSet;

/// Discord adapter capabilities
pub fn discord_adapter_info(port: u16, bot_id: Option<String>) -> AdapterInfo {
    AdapterInfo {
        name: "discord".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        url: format!("http://localhost:{}", port),
        features: HashSet::from([
            Feature::ReadHistory,
            Feature::Reactions,
            Feature::Threads,
            Feature::Attachments,
            Feature::Embeds,
            Feature::EditMessage,
            Feature::DeleteMessage,
            Feature::TypingIndicator,
        ]),
        metadata: serde_json::json!({
            "bot_id": bot_id,
        }),
    }
}

/// Register this adapter with the gateway
pub async fn register_with_gateway(
    client: &reqwest::Client,
    gateway_url: &str,
    info: AdapterInfo,
) -> Result<(), String> {
    let url = format!("{}/adapters/register", gateway_url);

    let response: RegisterResponse = client
        .post(&url)
        .json(&RegisterRequest { adapter: info })
        .send()
        .await
        .map_err(|e| format!("registration request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("failed to parse registration response: {}", e))?;

    if response.accepted {
        Ok(())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "registration rejected".into()))
    }
}
