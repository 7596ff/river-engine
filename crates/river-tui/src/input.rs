//! JSONL input reader — reads from stdin or tails a file

use river_core::channels::entry::HomeChannelEntry;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Read JSONL entries from stdin or a file and send parsed entries to the channel.
pub async fn run_reader(file: Option<PathBuf>, tx: mpsc::UnboundedSender<HomeChannelEntry>) {
    if let Some(path) = file {
        read_file(path, tx).await;
    } else {
        read_stdin(tx).await;
    }
}

async fn read_stdin(tx: mpsc::UnboundedSender<HomeChannelEntry>) {
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HomeChannelEntry>(&line) {
            Ok(entry) => {
                if tx.send(entry).is_err() {
                    break;
                }
            }
            Err(e) => {
                tracing::warn!("skipping malformed JSONL line: {}", e);
            }
        }
    }
}

async fn read_file(path: PathBuf, tx: mpsc::UnboundedSender<HomeChannelEntry>) {
    use tokio::fs::File;
    use tokio::time::{sleep, Duration};

    let file = match File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("failed to open {}: {}", path.display(), e);
            return;
        }
    };

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF — wait and try again (tail behavior)
                sleep(Duration::from_millis(100)).await;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<HomeChannelEntry>(trimmed) {
                    Ok(entry) => {
                        if tx.send(entry).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("skipping malformed JSONL line: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("read error: {}", e);
                break;
            }
        }
    }
}
