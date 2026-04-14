//! Conversation file management for the worker.

use river_protocol::conversation::Conversation;
use river_protocol::Channel;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Sanitize a string for use in file paths.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Get the conversation file path for a channel.
pub fn conversation_path_for_channel(workspace: &Path, channel: &Channel) -> PathBuf {
    // For now, DMs and channels without guild go to dm/
    // Guild channels go to adapter/guild_id-guild_name/channel_id-channel_name.txt
    let channel_name = channel.name.as_deref().unwrap_or("unknown");

    workspace
        .join("conversations")
        .join(&channel.adapter)
        .join("dm")
        .join(format!("{}-{}.txt", channel.id, sanitize(channel_name)))
}

/// Get the backchannel file path.
pub fn backchannel_path(workspace: &Path) -> PathBuf {
    workspace.join("conversations").join("backchannel.txt")
}

/// Compact all conversation files in the workspace that need it.
pub fn compact_conversations(workspace: &Path) -> std::io::Result<()> {
    let conversations_dir = workspace.join("conversations");
    if !conversations_dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(&conversations_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "txt") {
            if let Ok(mut convo) = Conversation::load(path) {
                if convo.needs_compaction() {
                    tracing::info!(path = %path.display(), "Compacting conversation file");
                    convo.compact();
                    convo.save(path)?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("hello world"), "hello_world");
        assert_eq!(sanitize("general-chat"), "general-chat");
        assert_eq!(sanitize("my_channel"), "my_channel");
        assert_eq!(sanitize("test/path"), "test_path");
    }

    #[test]
    fn test_conversation_path_for_channel() {
        let workspace = Path::new("/workspace");
        let channel = Channel {
            adapter: "discord".to_string(),
            id: "123456".to_string(),
            name: Some("general".to_string()),
        };

        let path = conversation_path_for_channel(workspace, &channel);
        assert!(path.to_str().unwrap().contains("discord"));
        assert!(path.to_str().unwrap().contains("123456-general.txt"));
    }

    #[test]
    fn test_backchannel_path() {
        let workspace = Path::new("/workspace");
        let path = backchannel_path(workspace);
        assert_eq!(
            path,
            PathBuf::from("/workspace/conversations/backchannel.txt")
        );
    }
}
