//! Inbox utilities for storing and loading tool results.

use river_context::InboxItem;
use std::path::Path;
use tokio::fs;

/// Write a tool result to the inbox.
///
/// Filename format: {adapter}_{channel_id}_{timestamp}_{tool}.json
pub async fn write_inbox_item(
    workspace: &Path,
    adapter: &str,
    channel_id: &str,
    tool: &str,
    summary: &str,
) -> std::io::Result<InboxItem> {
    let timestamp = chrono::Utc::now();
    let timestamp_str = timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let timestamp_iso = timestamp.to_rfc3339();

    let filename = format!("{}_{}_{}_{}",
        adapter,
        channel_id,
        timestamp_str,
        tool
    );

    let item = InboxItem {
        id: filename.clone(),
        timestamp: timestamp_iso,
        tool: tool.to_string(),
        channel_adapter: adapter.to_string(),
        channel_id: channel_id.to_string(),
        summary: summary.to_string(),
    };

    let inbox_dir = workspace.join("inbox");
    fs::create_dir_all(&inbox_dir).await?;

    let path = inbox_dir.join(format!("{}.json", filename));
    let json = serde_json::to_string_pretty(&item)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(&path, json).await?;

    Ok(item)
}

/// Load all inbox items for a channel.
pub async fn load_inbox_items(
    workspace: &Path,
    adapter: &str,
    channel_id: &str,
) -> Vec<InboxItem> {
    let inbox_dir = workspace.join("inbox");
    let prefix = format!("{}_{}_", adapter, channel_id);

    let mut items = Vec::new();

    let mut entries = match fs::read_dir(&inbox_dir).await {
        Ok(e) => e,
        Err(_) => return items,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let filename = entry.file_name();
        let filename_str = filename.to_string_lossy();

        if filename_str.starts_with(&prefix) && filename_str.ends_with(".json") {
            if let Ok(content) = fs::read_to_string(entry.path()).await {
                if let Ok(item) = serde_json::from_str::<InboxItem>(&content) {
                    items.push(item);
                }
            }
        }
    }

    // Sort by timestamp
    items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_write_and_load_inbox_item() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();

        let item = write_inbox_item(
            workspace,
            "discord",
            "chan123",
            "read_channel",
            "msg1150-msg1200",
        ).await.unwrap();

        assert_eq!(item.tool, "read_channel");
        assert_eq!(item.summary, "msg1150-msg1200");

        let loaded = load_inbox_items(workspace, "discord", "chan123").await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].summary, "msg1150-msg1200");
    }

    #[tokio::test]
    async fn test_load_inbox_items_filters_by_channel() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();

        write_inbox_item(workspace, "discord", "chan123", "read_channel", "a").await.unwrap();
        write_inbox_item(workspace, "discord", "chan456", "read_channel", "b").await.unwrap();

        let loaded = load_inbox_items(workspace, "discord", "chan123").await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].summary, "a");
    }
}
