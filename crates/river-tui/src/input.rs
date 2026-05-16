//! JSONL input reader — reads from stdin or tails a file
//!
//! File tailing uses inotify/kqueue (via `notify` crate) for zero-CPU-when-idle
//! watching, with a fallback poll to catch any missed events.

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
    use notify::{Event, EventKind, RecursiveMode, Watcher};
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    use tokio::time::{sleep, Duration};

    // Read existing content first
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

    // Set up filesystem watcher
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel();

    let mut watcher = match notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) => {
                    let _ = notify_tx.send(());
                }
                _ => {}
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("failed to create file watcher: {}", e);
            tracing::info!("falling back to polling");
            read_file_poll(path, tx, byte_offset).await;
            return;
        }
    };

    // Watch the parent directory (more reliable for files that get replaced)
    let watch_path = path.parent().unwrap_or(&path);
    if let Err(e) = watcher.watch(watch_path.as_ref(), RecursiveMode::NonRecursive) {
        tracing::error!("failed to watch {}: {}", watch_path.display(), e);
        tracing::info!("falling back to polling");
        read_file_poll(path, tx, byte_offset).await;
        return;
    }

    // Tail loop: wait for fs notifications or periodic fallback check
    let mut partial = String::new();
    loop {
        // Wait for either a filesystem event or a 5-second fallback poll
        tokio::select! {
            _ = notify_rx.recv() => {
                // Drain any queued notifications to coalesce rapid writes
                while notify_rx.try_recv().is_ok() {}
            }
            _ = sleep(Duration::from_secs(5)) => {
                // Periodic fallback in case we miss an event
            }
        }

        // Small delay to coalesce rapid writes (e.g. multiple JSONL lines at once)
        sleep(Duration::from_millis(20)).await;

        // Check for new data
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_len = metadata.len();
        if file_len <= byte_offset {
            continue;
        }

        // Read new bytes
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(_) => continue,
        };

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

/// Fallback polling implementation if filesystem watcher fails to initialize
async fn read_file_poll(
    path: PathBuf,
    tx: mpsc::UnboundedSender<HomeChannelEntry>,
    mut byte_offset: u64,
) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    use tokio::time::{sleep, Duration};

    let mut partial = String::new();
    loop {
        sleep(Duration::from_secs(1)).await;

        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_len = metadata.len();
        if file_len <= byte_offset {
            continue;
        }

        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(_) => continue,
        };

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
