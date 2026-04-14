//! Inbox file writing operations

use river_core::RiverResult;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

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

/// Build inbox file path for a Discord message
pub fn build_discord_path(
    workspace: &Path,
    guild_id: Option<&str>,
    guild_name: Option<&str>,
    channel_id: &str,
    channel_name: &str,
) -> PathBuf {
    let mut path = workspace.join("inbox").join("discord");

    match (guild_id, guild_name) {
        (Some(gid), Some(gname)) => {
            let dir_name = format!("{}-{}", gid, sanitize_name(gname));
            path = path.join(dir_name);
        }
        (Some(gid), None) => {
            let dir_name = format!("{}-unknown", gid);
            path = path.join(dir_name);
        }
        (None, _) => {
            // DM - no guild
            path = path.join("dm");
        }
    }

    let file_name = format!("{}-{}.txt", channel_id, sanitize_name(channel_name));
    path.join(file_name)
}

/// Ensure the parent directory exists
pub fn ensure_parent_dir(path: &Path) -> RiverResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Append a line to an inbox file
pub fn append_line(path: &Path, line: &str) -> RiverResult<()> {
    ensure_parent_dir(path)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    writeln!(file, "{}", line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sanitize_name_simple() {
        assert_eq!(sanitize_name("general"), "general");
        assert_eq!(sanitize_name("my-channel"), "my-channel");
    }

    #[test]
    fn test_sanitize_name_path_separators() {
        assert_eq!(sanitize_name("my/channel"), "my_channel");
        assert_eq!(sanitize_name("my\\channel"), "my_channel");
    }

    #[test]
    fn test_sanitize_name_null_byte() {
        assert_eq!(sanitize_name("my\0channel"), "my_channel");
    }

    #[test]
    fn test_sanitize_name_length_limit() {
        let long_name = "a".repeat(100);
        let sanitized = sanitize_name(&long_name);
        assert_eq!(sanitized.len(), 50);
    }

    #[test]
    fn test_sanitize_name_empty() {
        assert_eq!(sanitize_name(""), "unknown");
    }

    #[test]
    fn test_sanitize_name_unicode() {
        assert_eq!(sanitize_name("cafe"), "cafe");
        assert_eq!(sanitize_name("channel"), "channel");
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
            PathBuf::from("/workspace/inbox/discord/123456-myserver/789012-general.txt")
        );
    }

    #[test]
    fn test_build_discord_path_dm() {
        let workspace = Path::new("/workspace");
        let path = build_discord_path(
            workspace,
            None,
            None,
            "111222",
            "alice",
        );
        assert_eq!(
            path,
            PathBuf::from("/workspace/inbox/discord/dm/111222-alice.txt")
        );
    }

    #[test]
    fn test_build_discord_path_sanitizes_names() {
        let workspace = Path::new("/workspace");
        let path = build_discord_path(
            workspace,
            Some("123"),
            Some("my/server"),
            "456",
            "gen/eral",
        );
        assert_eq!(
            path,
            PathBuf::from("/workspace/inbox/discord/123-my_server/456-gen_eral.txt")
        );
    }

    #[test]
    fn test_append_line_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("inbox/discord/123-server/456-channel.txt");

        append_line(&path, "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello").unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "[ ] 2026-03-18 22:15:32 abc123 <alice:123> hello\n");
    }

    #[test]
    fn test_append_line_appends() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        append_line(&path, "line 1").unwrap();
        append_line(&path, "line 2").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "line 1\nline 2\n");
    }
}
