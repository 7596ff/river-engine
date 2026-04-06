//! Integration test helper utilities for spawning processes and polling state.

use river_context::OpenAIMessage;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use std::process::Stdio;
use tempfile::TempDir;

/// Test orchestrator handle.
pub struct TestOrchestrator {
    pub child: Child,
    pub endpoint: String,
    pub workspace_dir: PathBuf,
}

/// Spawn orchestrator with minimal config in temp workspace.
pub async fn spawn_orchestrator(workspace_dir: &Path, port: u16) -> Result<TestOrchestrator, Box<dyn std::error::Error>> {
    // Create minimal river.json config in workspace_dir pointing to workspace path
    let config_path = workspace_dir.join("river.json");
    let config_json = serde_json::json!({
        "workspace": workspace_dir.join("workspace").to_string_lossy(),
        "port": port,
        "dyads": []
    });
    std::fs::write(&config_path, serde_json::to_string_pretty(&config_json)?)?;

    // Initialize git repo in workspace (per pitfall 4 from RESEARCH.md)
    let workspace_path = workspace_dir.join("workspace");
    std::fs::create_dir_all(&workspace_path)?;
    let _ = Command::new("git")
        .arg("init")
        .current_dir(&workspace_path)
        .output()
        .await?;

    // Configure git user for commits
    Command::new("git")
        .args(&["config", "user.name", "Test User"])
        .current_dir(&workspace_path)
        .output()
        .await?;

    Command::new("git")
        .args(&["config", "user.email", "test@example.com"])
        .current_dir(&workspace_path)
        .output()
        .await?;

    // Spawn orchestrator with port 0 (OS-assigned, per D-20)
    let mut cmd = Command::new("river-orchestrator");
    cmd.arg("--config")
        .arg(&config_path)
        .arg("--port")
        .arg("0")  // Use port 0, discover actual port from stdout
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn()?;

    // Return with placeholder endpoint (tests must call wait_for_health to get real port)
    Ok(TestOrchestrator {
        child,
        endpoint: format!("http://127.0.0.1:{}", port),
        workspace_dir: workspace_dir.to_path_buf(),
    })
}

/// Test worker handle.
pub struct TestWorker {
    pub child: Child,
    pub endpoint: String,
    pub dyad: String,
    pub side: String,
}

/// Spawn worker process.
pub async fn spawn_worker(
    orchestrator_endpoint: &str,
    dyad: &str,
    side: &str,
) -> Result<TestWorker, Box<dyn std::error::Error>> {
    let mut cmd = Command::new("river-worker");
    cmd.arg("--orchestrator")
        .arg(orchestrator_endpoint)
        .arg("--dyad")
        .arg(dyad)
        .arg("--side")
        .arg(side)
        .arg("--port")
        .arg("0")  // OS-assigned port per D-20
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn()?;

    Ok(TestWorker {
        child,
        endpoint: String::new(),  // Populated by wait_for_registration
        dyad: dyad.to_string(),
        side: side.to_string(),
    })
}

/// Test adapter handle.
pub struct TestAdapter {
    pub child: Child,
    pub endpoint: String,
}

/// Spawn TUI adapter process.
pub async fn spawn_tui_adapter(
    orchestrator_endpoint: &str,
    dyad: &str,
) -> Result<TestAdapter, Box<dyn std::error::Error>> {
    let mut cmd = Command::new("river-tui");
    cmd.arg("--orchestrator")
        .arg(orchestrator_endpoint)
        .arg("--dyad")
        .arg(dyad)
        .arg("--port")
        .arg("0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn()?;

    Ok(TestAdapter {
        child,
        endpoint: String::new(),  // Populated by wait_for_registration
    })
}

/// Poll orchestrator registry until worker/adapter registers.
pub async fn wait_for_registration(
    orchestrator_endpoint: &str,
    dyad: &str,
    side: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let start = Instant::now();

    loop {
        if let Ok(resp) = client.get(format!("{}/registry", orchestrator_endpoint)).send().await {
            if let Ok(text) = resp.text().await {
                // Parse registry JSON to find worker endpoint
                // Expected format: array of ProcessEntry with dyad, side, endpoint fields
                if text.contains(dyad) && text.contains(side) {
                    // Extract endpoint from JSON (simple string parsing for test code)
                    // Look for "endpoint":"http://..." pattern
                    if let Some(start_idx) = text.find("\"endpoint\":\"") {
                        let substr = &text[start_idx + 12..];
                        if let Some(end_idx) = substr.find("\"") {
                            return Ok(substr[..end_idx].to_string());
                        }
                    }
                }
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            return Err(format!("Worker {}/{} not registered after {} seconds", dyad, side, timeout_secs));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Poll health endpoint until 200 response.
pub async fn wait_for_health(
    endpoint: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let start = Instant::now();

    loop {
        if let Ok(resp) = client.get(format!("{}/health", endpoint)).send().await {
            if resp.status() == 200 {
                return Ok(());
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            return Err(format!("Health check failed for {} after {} seconds", endpoint, timeout_secs));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Poll context file until entry matching predicate appears.
pub async fn wait_for_context_entry<F>(
    context_path: &Path,
    predicate: F,
    timeout_secs: u64,
) -> Result<OpenAIMessage, String>
where
    F: Fn(&OpenAIMessage) -> bool,
{
    let start = Instant::now();

    loop {
        if let Ok(content) = std::fs::read_to_string(context_path) {
            for line in content.lines().rev() {
                if let Ok(entry) = serde_json::from_str::<OpenAIMessage>(line) {
                    if predicate(&entry) {
                        return Ok(entry);
                    }
                }
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            return Err(format!("Timeout waiting for context entry after {} seconds", timeout_secs));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Read latest context entry from JSONL file.
pub async fn read_latest_context_entry(context_path: &Path) -> Result<OpenAIMessage, String> {
    let content = std::fs::read_to_string(context_path)
        .map_err(|e| format!("Failed to read context: {}", e))?;

    content
        .lines()
        .last()
        .ok_or_else(|| "Context file is empty".to_string())
        .and_then(|line| {
            serde_json::from_str::<OpenAIMessage>(line)
                .map_err(|e| format!("Failed to parse context entry: {}", e))
        })
}

/// Create temporary workspace for test isolation.
pub async fn setup_test_workspace() -> Result<TempDir, Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let workspace_path = temp_dir.path().join("workspace");

    // Create workspace structure
    std::fs::create_dir_all(workspace_path.join("shared"))?;
    std::fs::create_dir_all(workspace_path.join("conversations"))?;

    // Initialize git repo
    Command::new("git")
        .arg("init")
        .current_dir(&workspace_path)
        .output()
        .await?;

    // Configure git user for commits
    Command::new("git")
        .args(&["config", "user.name", "Test User"])
        .current_dir(&workspace_path)
        .output()
        .await?;

    Command::new("git")
        .args(&["config", "user.email", "test@example.com"])
        .current_dir(&workspace_path)
        .output()
        .await?;

    Ok(temp_dir)
}
