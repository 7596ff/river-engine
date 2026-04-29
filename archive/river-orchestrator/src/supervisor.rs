//! Process spawning and health checks.

use crate::config::{AdapterConfig, DyadConfig};
use river_adapter::Side;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// Process handle with metadata.
#[derive(Debug)]
pub struct ProcessHandle {
    pub child: Child,
    pub endpoint: Option<String>,
    pub consecutive_failures: u32,
}

/// Process key for identification.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ProcessKey {
    Worker { dyad: String, side: Side },
    Adapter { dyad: String, adapter_type: String },
    Embed { name: String },
}

/// Process supervisor state.
#[derive(Debug, Default)]
pub struct Supervisor {
    processes: HashMap<ProcessKey, ProcessHandle>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn the embed service.
    pub async fn spawn_embed(
        &mut self,
        orchestrator_url: &str,
        name: &str,
    ) -> Result<(), SupervisorError> {
        let child = Command::new("river-embed")
            .arg("--orchestrator")
            .arg(orchestrator_url)
            .arg("--name")
            .arg(name)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(SupervisorError::SpawnFailed)?;

        let key = ProcessKey::Embed { name: name.to_string() };
        self.processes.insert(
            key,
            ProcessHandle {
                child,
                endpoint: None,
                consecutive_failures: 0,
            },
        );
        Ok(())
    }

    /// Spawn a worker.
    pub async fn spawn_worker(
        &mut self,
        orchestrator_url: &str,
        dyad: &str,
        side: Side,
    ) -> Result<(), SupervisorError> {
        let side_str = match side {
            Side::Left => "left",
            Side::Right => "right",
        };

        let child = Command::new("river-worker")
            .arg("--orchestrator")
            .arg(orchestrator_url)
            .arg("--dyad")
            .arg(dyad)
            .arg("--side")
            .arg(side_str)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(SupervisorError::SpawnFailed)?;

        let key = ProcessKey::Worker {
            dyad: dyad.to_string(),
            side,
        };
        self.processes.insert(
            key,
            ProcessHandle {
                child,
                endpoint: None,
                consecutive_failures: 0,
            },
        );
        Ok(())
    }

    /// Spawn an adapter.
    pub async fn spawn_adapter(
        &mut self,
        orchestrator_url: &str,
        dyad: &str,
        adapter_config: &AdapterConfig,
    ) -> Result<(), SupervisorError> {
        let side_str = match adapter_config.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        // Use explicit path from config
        let child = Command::new(&adapter_config.path)
            .arg("--orchestrator")
            .arg(orchestrator_url)
            .arg("--dyad")
            .arg(dyad)
            .arg("--side")
            .arg(side_str)
            .arg("--type")
            .arg(adapter_config.adapter_type())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(SupervisorError::SpawnFailed)?;

        let key = ProcessKey::Adapter {
            dyad: dyad.to_string(),
            adapter_type: adapter_config.adapter_type().to_string(),
        };
        self.processes.insert(
            key,
            ProcessHandle {
                child,
                endpoint: None,
                consecutive_failures: 0,
            },
        );
        Ok(())
    }

    /// Set endpoint for a process after registration.
    pub fn set_endpoint(&mut self, key: &ProcessKey, endpoint: String) {
        if let Some(handle) = self.processes.get_mut(key) {
            handle.endpoint = Some(endpoint);
        }
    }

    /// Record a health check failure.
    pub fn record_failure(&mut self, key: &ProcessKey) -> u32 {
        if let Some(handle) = self.processes.get_mut(key) {
            handle.consecutive_failures += 1;
            handle.consecutive_failures
        } else {
            0
        }
    }

    /// Reset failure count on successful health check.
    pub fn reset_failures(&mut self, key: &ProcessKey) {
        if let Some(handle) = self.processes.get_mut(key) {
            handle.consecutive_failures = 0;
        }
    }

    /// Remove a dead process from tracking (doesn't kill - already dead).
    pub fn remove(&mut self, key: &ProcessKey) {
        self.processes.remove(key);
    }

    /// Send SIGTERM to all processes for graceful shutdown.
    /// On non-Unix platforms, falls back to immediate kill.
    pub async fn terminate_all(&mut self) {
        for (key, handle) in &self.processes {
            #[cfg(unix)]
            {
                if let Some(pid) = handle.child.id() {
                    let pid = Pid::from_raw(pid as i32);
                    if let Err(e) = kill(pid, Signal::SIGTERM) {
                        tracing::warn!("Failed to send SIGTERM to {:?}: {}", key, e);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                // On non-Unix, we can't send SIGTERM, so just mark for kill
                tracing::info!("Non-Unix platform, will force kill {:?}", key);
            }
        }
    }

    /// Wait for all processes to exit with timeout, then SIGKILL stragglers.
    /// Spec requires 5 minute grace period for SIGTERM.
    pub async fn shutdown(&mut self, timeout: Duration) {
        tracing::info!("Sending SIGTERM to all processes, waiting up to {:?}", timeout);
        self.terminate_all().await;

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if self.processes.is_empty() {
                tracing::info!("All processes exited gracefully");
                break;
            }

            if tokio::time::Instant::now() > deadline {
                tracing::warn!(
                    "Shutdown timeout after {:?}, sending SIGKILL to {} remaining processes",
                    timeout,
                    self.processes.len()
                );
                for (key, handle) in &mut self.processes {
                    if let Err(e) = handle.child.kill().await {
                        tracing::warn!("Failed to SIGKILL {:?}: {}", key, e);
                    }
                }
                self.processes.clear();
                break;
            }

            // Check for exited processes
            let mut exited = Vec::new();
            for (key, handle) in &mut self.processes {
                match handle.child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::debug!("Process {:?} exited with status {:?}", key, status);
                        exited.push(key.clone());
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("Error checking process {:?}: {}", key, e);
                        exited.push(key.clone());
                    }
                }
            }
            for key in exited {
                self.processes.remove(&key);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Get all process keys with endpoints for health checking.
    pub fn endpoints_for_health_check(&self) -> Vec<(ProcessKey, String)> {
        self.processes
            .iter()
            .filter_map(|(k, h)| h.endpoint.as_ref().map(|e| (k.clone(), e.clone())))
            .collect()
    }
}

/// Supervisor error.
#[derive(Debug)]
pub enum SupervisorError {
    SpawnFailed(std::io::Error),
    WorktreeCreationFailed(String),
}

impl std::fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupervisorError::SpawnFailed(e) => write!(f, "Failed to spawn process: {}", e),
            SupervisorError::WorktreeCreationFailed(e) => write!(f, "Failed to create git worktree: {}", e),
        }
    }
}

impl std::error::Error for SupervisorError {}

/// Thread-safe supervisor wrapper.
pub type SharedSupervisor = Arc<RwLock<Supervisor>>;

pub fn new_shared_supervisor() -> SharedSupervisor {
    Arc::new(RwLock::new(Supervisor::new()))
}

/// Run health checks on all processes.
pub async fn run_health_checks(
    client: &reqwest::Client,
    supervisor: &SharedSupervisor,
) -> Vec<ProcessKey> {
    let endpoints = {
        let sup = supervisor.read().await;
        sup.endpoints_for_health_check()
    };

    let mut dead_processes = Vec::new();

    for (key, endpoint) in endpoints {
        let url = format!("{}/health", endpoint);
        let result = client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await;

        let mut sup = supervisor.write().await;
        match result {
            Ok(resp) if resp.status().is_success() => {
                sup.reset_failures(&key);
            }
            _ => {
                let failures = sup.record_failure(&key);
                if failures >= 3 {
                    tracing::error!("Process {:?} dead after 3 health check failures", key);
                    dead_processes.push(key);
                }
            }
        }
    }

    dead_processes
}

/// Check if a directory is a valid git worktree.
/// Returns true if .git exists and is a file containing "gitdir:" (linked worktree).
fn is_valid_worktree(path: &Path) -> bool {
    let git_path = path.join(".git");
    if !git_path.exists() {
        return false;
    }

    // For linked worktrees, .git is a file containing "gitdir: ..."
    if git_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_path) {
            return content.contains("gitdir:");
        }
    }

    false
}

/// Ensure a git worktree exists at the specified location.
/// If the worktree already exists and is valid, reuses it.
/// If the directory exists but is not a valid worktree, cleans and recreates it.
async fn ensure_worktree_exists(
    repo_path: &Path,
    worktree_name: &str,
    branch_name: &str,
) -> Result<(), SupervisorError> {
    let repo_str = repo_path.to_string_lossy();
    let worktree_path = repo_path.join(worktree_name);
    let worktree_str = worktree_path.to_string_lossy();

    // Check if worktree already exists and is valid
    if worktree_path.exists() && is_valid_worktree(&worktree_path) {
        tracing::info!("Reusing existing worktree {} at {:?}", worktree_name, worktree_path);
        return Ok(());
    }

    // If directory exists but is not a valid worktree, clean it
    if worktree_path.exists() {
        tracing::warn!("Invalid worktree at {:?}, cleaning", worktree_path);
        tokio::fs::remove_dir_all(&worktree_path)
            .await
            .map_err(|e| SupervisorError::WorktreeCreationFailed(format!("Failed to remove invalid worktree directory: {}", e)))?;
    }

    // Create branch if it doesn't exist (ignore error if branch already exists)
    Command::new("git")
        .args(&["-C", &repo_str, "branch", branch_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok(); // Ignore error - branch may already exist

    // Create worktree
    let output = Command::new("git")
        .args(&["-C", &repo_str, "worktree", "add", &worktree_str, branch_name])
        .output()
        .await
        .map_err(|e| SupervisorError::WorktreeCreationFailed(format!("Failed to execute git worktree command: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SupervisorError::WorktreeCreationFailed(stderr.into()));
    }

    tracing::info!("Created worktree {} at {:?} on branch {}", worktree_name, worktree_path, branch_name);
    Ok(())
}

/// Spawn all processes for a dyad.
pub async fn spawn_dyad(
    supervisor: &SharedSupervisor,
    orchestrator_url: &str,
    dyad_name: &str,
    dyad_config: &DyadConfig,
) -> Result<(), SupervisorError> {
    // Create worktrees for left and right workers before spawning
    let workspace_path = &dyad_config.workspace;

    tracing::info!("Creating worktrees for dyad {} at {:?}", dyad_name, workspace_path);

    // Create left worktree on 'left' branch
    ensure_worktree_exists(workspace_path, "left", "left").await?;

    // Create right worktree on 'right' branch
    ensure_worktree_exists(workspace_path, "right", "right").await?;

    tracing::info!("Worktrees ready for dyad {}", dyad_name);

    // Lock supervisor and spawn workers/adapters
    let mut sup = supervisor.write().await;

    // Spawn left worker
    sup.spawn_worker(orchestrator_url, dyad_name, Side::Left).await?;

    // Spawn right worker
    sup.spawn_worker(orchestrator_url, dyad_name, Side::Right).await?;

    // Spawn adapters
    for adapter in &dyad_config.adapters {
        sup.spawn_adapter(orchestrator_url, dyad_name, adapter).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_key_equality() {
        let key1 = ProcessKey::Worker {
            dyad: "dyad1".into(),
            side: Side::Left,
        };
        let key2 = ProcessKey::Worker {
            dyad: "dyad1".into(),
            side: Side::Left,
        };
        let key3 = ProcessKey::Worker {
            dyad: "dyad1".into(),
            side: Side::Right,
        };

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_process_key_adapter() {
        let key1 = ProcessKey::Adapter {
            dyad: "dyad1".into(),
            adapter_type: "discord".into(),
        };
        let key2 = ProcessKey::Adapter {
            dyad: "dyad1".into(),
            adapter_type: "slack".into(),
        };

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_supervisor_new() {
        let sup = Supervisor::new();
        assert!(sup.endpoints_for_health_check().is_empty());
    }

    #[test]
    fn test_supervisor_set_endpoint_nonexistent() {
        let mut sup = Supervisor::new();
        // Setting endpoint for nonexistent process should be a no-op
        sup.set_endpoint(
            &ProcessKey::Worker {
                dyad: "dyad1".into(),
                side: Side::Left,
            },
            "http://localhost:3001".into(),
        );
        // Should not crash, just do nothing
        assert!(sup.endpoints_for_health_check().is_empty());
    }

    #[test]
    fn test_supervisor_failure_tracking() {
        let mut sup = Supervisor::new();
        let key = ProcessKey::Embed { name: "embed".into() };

        // Recording failure for nonexistent process returns 0
        assert_eq!(sup.record_failure(&key), 0);

        // Reset failures for nonexistent process is a no-op
        sup.reset_failures(&key);
    }

    #[test]
    fn test_supervisor_error_display() {
        let err = SupervisorError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "binary not found",
        ));
        let display = format!("{}", err);
        assert!(display.contains("Failed to spawn process"));
        assert!(display.contains("binary not found"));
    }

    // ========== NEW TDD TESTS FOR WORKTREE FUNCTIONALITY ==========

    #[test]
    fn test_is_valid_worktree_returns_true_for_valid_worktree() {
        use tempfile::TempDir;
        use std::fs;

        // Create a temporary directory with a .git file containing "gitdir:"
        let temp = TempDir::new().unwrap();
        let worktree_path = temp.path();
        let git_file = worktree_path.join(".git");
        fs::write(&git_file, "gitdir: /some/repo/.git/worktrees/test").unwrap();

        assert!(is_valid_worktree(worktree_path));
    }

    #[test]
    fn test_is_valid_worktree_returns_false_when_directory_not_exists() {
        use std::path::PathBuf;

        let nonexistent = PathBuf::from("/nonexistent/path");
        assert!(!is_valid_worktree(&nonexistent));
    }

    #[test]
    fn test_is_valid_worktree_returns_false_when_git_is_directory() {
        use tempfile::TempDir;
        use std::fs;

        // Create a temporary directory with .git as a directory (main repo, not linked worktree)
        let temp = TempDir::new().unwrap();
        let worktree_path = temp.path();
        fs::create_dir(worktree_path.join(".git")).unwrap();

        assert!(!is_valid_worktree(worktree_path));
    }

    #[tokio::test]
    async fn test_ensure_worktree_exists_creates_worktree_when_not_exists() {
        use tempfile::TempDir;
        use tokio::process::Command;

        // Create a temporary git repository
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path();

        // Initialize git repo with a commit
        Command::new("git")
            .args(&["init"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["commit", "--allow-empty", "-m", "Initial commit"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        // Create worktree
        let result = ensure_worktree_exists(repo_path, "test-worktree", "test-branch").await;
        assert!(result.is_ok());

        // Verify worktree exists and is valid
        let worktree_path = repo_path.join("test-worktree");
        assert!(worktree_path.exists());
        assert!(is_valid_worktree(&worktree_path));
    }

    #[tokio::test]
    async fn test_ensure_worktree_exists_reuses_valid_existing_worktree() {
        use tempfile::TempDir;
        use tokio::process::Command;

        // Create a temporary git repository with a worktree
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path();

        // Initialize git repo
        Command::new("git")
            .args(&["init"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["commit", "--allow-empty", "-m", "Initial commit"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        // Create worktree first time
        ensure_worktree_exists(repo_path, "test-worktree", "test-branch").await.unwrap();

        // Call again - should reuse
        let result = ensure_worktree_exists(repo_path, "test-worktree", "test-branch").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ensure_worktree_exists_cleans_invalid_directory() {
        use tempfile::TempDir;
        use tokio::process::Command;
        use std::fs;

        // Create a temporary git repository
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path();

        // Initialize git repo
        Command::new("git")
            .args(&["init"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(&["commit", "--allow-empty", "-m", "Initial commit"])
            .current_dir(repo_path)
            .output()
            .await
            .unwrap();

        // Create a directory that is NOT a valid worktree
        let worktree_path = repo_path.join("test-worktree");
        fs::create_dir(&worktree_path).unwrap();
        fs::write(worktree_path.join("somefile.txt"), "content").unwrap();

        // This should clean the directory and create a valid worktree
        let result = ensure_worktree_exists(repo_path, "test-worktree", "test-branch").await;
        assert!(result.is_ok());
        assert!(is_valid_worktree(&worktree_path));
    }
}
