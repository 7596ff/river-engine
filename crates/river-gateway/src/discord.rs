//! The Discord adapter (wall ch. 06): an in-process supervised task
//! owning a gateway websocket. Inbound message-create events from the
//! listened channels (and DMs, which always pass) normalize into the
//! channel layer; outbound speak requests deliver over HTTP and are
//! logged post-acceptance with the platform msg_id. The token arrives
//! via the environment (`token_env`), never config or logs.
//!
//! The listen-set is config-named channels resolved against the
//! guild at startup; `/listen` and `/unlisten` slash commands are a
//! later card. Channel names key by id: `discord_<channel_id>`.

use std::collections::HashSet;

use anyhow::Context as _;
use tokio::sync::{mpsc, oneshot, watch};
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, GuildMarker};

use crate::channels::{Channels, channel_name};

pub const ADAPTER: &str = "discord";
pub const CHANNEL_PREFIX: &str = "discord_";

/// A speak delivery request from the tool layer; the reply carries
/// the platform message id or the error, as tool-result text.
pub struct SpeakRequest {
    pub channel: String, // engine channel name: discord_<id>
    pub content: String,
    pub reply: oneshot::Sender<anyhow::Result<String>>,
}

#[derive(Clone)]
pub struct DiscordSettings {
    /// None = DM-only: no guild channels listened.
    pub guild_id: Option<u64>,
    pub listen_names: Vec<String>,
    pub token: String,
}

/// Supervise the adapter: panics and errors restart it with
/// exponential backoff (1s doubling to 60s, reset after 5 healthy
/// minutes); the agent is unaffected throughout (wall ch. 06).
pub async fn run_supervised(
    settings: DiscordSettings,
    channels: Channels,
    mut speak_rx: mpsc::Receiver<SpeakRequest>,
    mut shutdown: watch::Receiver<bool>,
    working: watch::Receiver<Option<String>>,
) {
    let mut backoff = std::time::Duration::from_secs(1);
    loop {
        let started = std::time::Instant::now();
        let result =
            run_once(&settings, &channels, &mut speak_rx, &mut shutdown, &working).await;
        if *shutdown.borrow() {
            return;
        }
        match result {
            Ok(()) => return, // clean exit without shutdown: channel closed
            Err(e) => tracing::warn!(error = %e, "discord adapter failed; restarting"),
        }
        if started.elapsed().as_secs() >= 300 {
            backoff = std::time::Duration::from_secs(1);
        }
        tokio::select! {
            _ = async { let _ = shutdown.wait_for(|&s| s).await; } => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(std::time::Duration::from_secs(60));
    }
}

/// Aborts a task when dropped — the typing ticker dies with its
/// connection attempt.
struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Re-send the typing indicator every 8s while the agent is working
/// a discord channel (the platform expires it at ~10s).
async fn typing_loop(
    http: std::sync::Arc<twilight_http::Client>,
    mut working: watch::Receiver<Option<String>>,
) {
    loop {
        let target = working
            .borrow_and_update()
            .clone()
            .and_then(|c| c.strip_prefix(CHANNEL_PREFIX).and_then(|s| s.parse::<u64>().ok()));
        match target {
            Some(id) => {
                match http.create_typing_trigger(Id::<ChannelMarker>::new(id)).await {
                    Ok(_) => tracing::debug!(channel = id, "typing trigger sent"),
                    Err(e) => tracing::warn!(channel = id, error = %e, "typing trigger failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {}
                    changed = working.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
            None => {
                if working.changed().await.is_err() {
                    return;
                }
            }
        }
    }
}

async fn run_once(
    settings: &DiscordSettings,
    channels: &Channels,
    speak_rx: &mut mpsc::Receiver<SpeakRequest>,
    shutdown: &mut watch::Receiver<bool>,
    working: &watch::Receiver<Option<String>>,
) -> anyhow::Result<()> {
    let http = std::sync::Arc::new(twilight_http::Client::new(settings.token.clone()));
    let _typing = AbortOnDrop(tokio::spawn(typing_loop(http.clone(), working.clone())));

    let me = http.current_user().await?.model().await?;
    let listen: HashSet<u64> = match settings.guild_id {
        Some(guild_id) => {
            let guild: Id<GuildMarker> = Id::new(guild_id);
            let guild_channels = http.guild_channels(guild).await?.model().await?;
            guild_channels
                .iter()
                .filter(|c| {
                    c.name
                        .as_deref()
                        .is_some_and(|n| settings.listen_names.iter().any(|l| l == n))
                })
                .map(|c| c.id.get())
                .collect()
        }
        None => HashSet::new(), // DM-only
    };
    tracing::info!(
        listening = listen.len(),
        dm_only = settings.guild_id.is_none(),
        "discord adapter connected as {}",
        me.name
    );

    let intents =
        Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES | Intents::MESSAGE_CONTENT;
    let mut shard = Shard::new(ShardId::ONE, settings.token.clone(), intents);

    loop {
        tokio::select! {
            biased;
            _ = async { let _ = shutdown.wait_for(|&s| s).await; } => return Ok(()),
            request = speak_rx.recv() => match request {
                Some(request) => {
                    let outcome = deliver(&http, channels, &request.channel, &request.content).await;
                    let _ = request.reply.send(outcome);
                }
                None => return Ok(()),
            },
            event = shard.next_event(EventTypeFlags::all()) => match event {
                Some(Ok(Event::MessageCreate(msg))) => {
                    let is_dm = msg.guild_id.is_none();
                    if admit(
                        is_dm,
                        listen.contains(&msg.channel_id.get()),
                        msg.author.id == me.id,
                        msg.author.bot,
                    ) {
                        let engine_channel =
                            channel_name(ADAPTER, &msg.channel_id.get().to_string());
                        if let Err(e) = channels
                            .inbound(
                                &engine_channel,
                                &msg.author.name,
                                Some(&msg.author.id.get().to_string()),
                                &msg.content,
                                ADAPTER,
                                Some(&msg.id.get().to_string()),
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "discord inbound failed");
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    anyhow::bail!("gateway error: {e}");
                }
                None => anyhow::bail!("gateway stream ended"),
            },
        }
    }
}

/// Deliver, then log post-acceptance — the agent entry doubles as the
/// cursor (wall ch. 05).
async fn deliver(
    http: &twilight_http::Client,
    channels: &Channels,
    engine_channel: &str,
    content: &str,
) -> anyhow::Result<String> {
    let id_text = engine_channel
        .strip_prefix(CHANNEL_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("not a discord channel: {engine_channel}"))?;
    let channel_id: Id<ChannelMarker> = Id::new(
        id_text
            .parse::<u64>()
            .with_context(|| format!("bad discord channel id {id_text:?}"))?,
    );
    let sent = http
        .create_message(channel_id)
        .content(content)
        .await?
        .model()
        .await?;
    let msg_id = sent.id.get().to_string();
    channels.agent_spoke(engine_channel, content, ADAPTER, Some(&msg_id))?;
    Ok(msg_id)
}

/// The admission rule: DMs always pass; guild messages need the
/// listen-set; the bot's own messages never pass; other bots do —
/// me, or not-me, is the only distinction the engine makes.
pub fn admit(is_dm: bool, in_listen_set: bool, author_is_self: bool, author_is_bot: bool) -> bool {
    if author_is_self {
        return false;
    }
    // Another bot speaking is still "not-me" (wall ch. 05), but its
    // own relays of our messages would loop; bots are admitted in
    // DMs and listened channels like anyone else.
    let _ = author_is_bot;
    is_dm || in_listen_set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_rules() {
        assert!(admit(true, false, false, false), "DMs always pass");
        assert!(admit(false, true, false, false), "listened channel passes");
        assert!(!admit(false, false, false, false), "unlistened guild channel filtered");
        assert!(!admit(true, true, true, false), "own messages never pass");
        assert!(admit(false, true, false, true), "another bot is just not-me");
    }
}
