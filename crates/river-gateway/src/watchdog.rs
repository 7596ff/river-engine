//! Systemd watchdog integration

use std::time::Duration;
use tokio::task::JoinHandle;

/// Spawn a background task that pings the systemd watchdog
///
/// This runs independently of the agent loop. If the process hangs
/// completely (e.g., deadlock), it stops pinging and systemd restarts.
pub fn spawn_watchdog_task(interval_secs: u64) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]) {
                tracing::warn!(error = %e, "Failed to ping systemd watchdog");
            } else {
                tracing::trace!(event = "watchdog.ping", "Pinged systemd watchdog");
            }
        }
    })
}

/// Notify systemd that the service is ready
pub fn notify_ready() {
    if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        tracing::debug!(error = %e, "Failed to notify systemd ready (may not be running under systemd)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watchdog_task_spawns() {
        let handle = spawn_watchdog_task(1);
        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        // Should complete without panic
    }
}
