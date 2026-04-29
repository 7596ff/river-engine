//! Path building for conversation files

use std::path::{Path, PathBuf};
use super::CONVERSATIONS_DIR;

/// Sanitize a user-provided name for safe filesystem use
/// - Replaces path separators with _
/// - Replaces null bytes with _
/// - Limits to 50 characters
pub fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c => c,
        })
        .take(50)
        .collect();

    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

/// Build path for Discord conversation file
pub fn build_discord_path(
    workspace: &Path,
    guild_id: Option<&str>,
    guild_name: Option<&str>,
    channel_id: &str,
    channel_name: &str,
) -> PathBuf {
    let mut path = workspace.join(CONVERSATIONS_DIR).join("discord");

    match (guild_id, guild_name) {
        (Some(gid), Some(gname)) => {
            path = path.join(format!("{}-{}", gid, sanitize_name(gname)));
        }
        (Some(gid), None) => {
            path = path.join(format!("{}-unknown", gid));
        }
        (None, _) => {
            path = path.join("dm");
        }
    }

    path.join(format!("{}-{}.txt", channel_id, sanitize_name(channel_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("general"), "general");
        assert_eq!(sanitize_name("my/channel"), "my_channel");
        assert_eq!(sanitize_name("my\\channel"), "my_channel");
        assert_eq!(sanitize_name(""), "unknown");
    }

    #[test]
    fn test_build_discord_path_with_guild() {
        let workspace = Path::new("/workspace");
        let path = build_discord_path(
            workspace,
            Some("123456"),
            Some("myserver"),
            "789012",
            "general",
        );
        assert_eq!(
            path,
            PathBuf::from("/workspace/conversations/discord/123456-myserver/789012-general.txt")
        );
    }

    #[test]
    fn test_build_discord_path_dm() {
        let workspace = Path::new("/workspace");
        let path = build_discord_path(workspace, None, None, "111222", "alice");
        assert_eq!(
            path,
            PathBuf::from("/workspace/conversations/discord/dm/111222-alice.txt")
        );
    }
}
