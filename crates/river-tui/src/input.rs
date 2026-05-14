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
    use tokio::time::{sleep, Duration};

    // Read existing content first, then tail
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to read {}: {}", path.display(), e);
            return;
        }
    };

    let mut byte_offset = content.len() as u64;

    // Process existing lines
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HomeChannelEntry>(line) {
            Ok(entry) => {
                if tx.send(entry).is_err() {
                    return;
                }
            }
            Err(e) => {
                tracing::warn!("skipping malformed JSONL line: {}", e);
            }
        }
    }

    // Tail: poll for new data by checking file size
    let mut partial = String::new();
    loop {
        sleep(Duration::from_millis(100)).await;

        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_len = metadata.len();
        if file_len <= byte_offset {
            continue;
        }

        // Read new bytes
        let file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(_) => continue,
        };

        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncSeekExt;
        let mut file = file;
        if file.seek(std::io::SeekFrom::Start(byte_offset)).await.is_err() {
            continue;
        }

        let mut buf = vec![0u8; (file_len - byte_offset) as usize];
        match file.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(_) => continue,
        }

        byte_offset = file_len;

        let chunk = String::from_utf8_lossy(&buf);
        partial.push_str(&chunk);

        // Process complete lines
        while let Some(newline_pos) = partial.find('\n') {
            let line = partial[..newline_pos].trim().to_string();
            partial = partial[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<HomeChannelEntry>(&line) {
                Ok(entry) => {
                    if tx.send(entry).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    tracing::warn!("skipping malformed JSONL line: {}", e);
                }
            }
        }
    }
}
