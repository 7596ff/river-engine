//! Process supervisor — spawn, monitor, restart child processes
//!
//! Each child has a label (e.g., "iris/gateway"), a binary, and args.
//! Stdout/stderr are forwarded to the orchestrator log with the label prefix.
//! On exit, the child is restarted with exponential backoff.
//! On shutdown, SIGTERM is sent first with a 10s grace period before SIGKILL.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

/// A child process definition
#[derive(Debug, Clone)]
pub struct ChildSpec {
    /// Label for logging (e.g., "iris/gateway")
    pub label: String,
    /// Binary path
    pub bin: PathBuf,
    /// CLI arguments
    pub args: Vec<String>,
}

/// Backoff state for restarts
struct Backoff {
    delay: Duration,
    healthy_since: Option<Instant>,
}

impl Backoff {
    fn new() -> Self {
        Self {
            delay: Duration::from_secs(1),
            healthy_since: None,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let d = self.delay;
        self.delay = (self.delay * 2).min(Duration::from_secs(60));
        d
    }

    fn mark_running(&mut self) {
        self.healthy_since = Some(Instant::now());
    }

    fn maybe_reset(&mut self) {
        if let Some(since) = self.healthy_since {
            if since.elapsed() > Duration::from_secs(300) {
                self.delay = Duration::from_secs(1);
            }
        }
    }
}

/// Spawn a child process with piped stdout/stderr
fn spawn_child(spec: &ChildSpec) -> std::io::Result<Child> {
    Command::new(&spec.bin)
        .args(&spec.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

/// Forward a child's stdout/stderr to tracing with a label prefix
fn forward_output(label: String, child: &mut Child) {
    if let Some(stdout) = child.stdout.take() {
        let label = label.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!("[{}] {}", label, line);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let label = label.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!("[{}] {}", label, line);
            }
        });
    }
}

/// Send SIGTERM, wait up to 10 seconds, then SIGKILL if still running.
async fn graceful_shutdown(child: &mut Child, label: &str) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill().await;
        return;
    }

    // Wait up to 10 seconds for graceful exit
    match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Ok(Ok(status)) => {
            tracing::info!(label = %label, status = %status, "Child exited gracefully");
        }
        Ok(Err(e)) => {
            tracing::warn!(label = %label, error = %e, "Error waiting for child");
        }
        Err(_) => {
            tracing::warn!(label = %label, "Child did not exit in 10s, sending SIGKILL");
            let _ = child.kill().await;
        }
    }
}

/// Run a supervised child process. Restarts on exit with backoff.
/// Stops when shutdown_rx receives a signal.
pub async fn supervise(spec: ChildSpec, mut shutdown_rx: broadcast::Receiver<()>) {
    let mut backoff = Backoff::new();
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        backoff.maybe_reset();

        tracing::info!(
            label = %spec.label,
            bin = %spec.bin.display(),
            attempt = attempt,
            "Spawning child process"
        );

        let mut child = match spawn_child(&spec) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    label = %spec.label,
                    error = %e,
                    "Failed to spawn child process"
                );
                let delay = backoff.next_delay();
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        forward_output(spec.label.clone(), &mut child);
        backoff.mark_running();

        // Wait for child exit or shutdown signal
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) => tracing::warn!(label = %spec.label, status = %s, "Child exited"),
                    Err(e) => tracing::error!(label = %spec.label, error = %e, "Child wait failed"),
                }

                let delay = backoff.next_delay();
                tracing::info!(
                    label = %spec.label,
                    delay_secs = delay.as_secs(),
                    attempt = attempt,
                    "Restarting after backoff"
                );

                // Check for shutdown during backoff
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.recv() => {
                        tracing::info!(label = %spec.label, "Shutdown during backoff, not restarting");
                        return;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!(label = %spec.label, "Shutdown signal received, sending SIGTERM");
                graceful_shutdown(&mut child, &spec.label).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_escalation() {
        let mut b = Backoff::new();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_secs(16));
        assert_eq!(b.next_delay(), Duration::from_secs(32));
        assert_eq!(b.next_delay(), Duration::from_secs(60)); // capped
        assert_eq!(b.next_delay(), Duration::from_secs(60)); // stays capped
    }

    #[test]
    fn test_child_spec() {
        let spec = ChildSpec {
            label: "iris/gateway".to_string(),
            bin: PathBuf::from("river-gateway"),
            args: vec!["--port".to_string(), "3000".to_string()],
        };
        assert_eq!(spec.label, "iris/gateway");
    }

    #[tokio::test]
    async fn test_supervise_shutdown() {
        let spec = ChildSpec {
            label: "test".to_string(),
            bin: PathBuf::from("sleep"),
            args: vec!["3600".to_string()],
        };

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let handle = tokio::spawn(supervise(spec, shutdown_rx));

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Send shutdown
        let _ = shutdown_tx.send(());

        // Should complete within grace period
        tokio::time::timeout(Duration::from_secs(15), handle)
            .await
            .expect("supervise should exit on shutdown")
            .expect("task should not panic");
    }
}
