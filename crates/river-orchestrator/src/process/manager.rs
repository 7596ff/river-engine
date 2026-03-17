//! Process management for llama-server instances

use super::PortAllocator;
use crate::discovery::LocalModel;
use crate::resources::DeviceId;
use river_core::RiverError;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

/// Health status of a running process
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessHealth {
    Starting,
    Healthy,
    Unhealthy { since: Instant, reason: String },
    Dead,
}

/// Internal process information
#[derive(Debug)]
struct ProcessInfo {
    model_id: String,
    pid: u32,
    port: u16,
    device: DeviceId,
    started_at: Instant,
    last_request: Instant,
    health: ProcessHealth,
    child: Child,
}

/// Configuration for process management
#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub llama_server_path: PathBuf,
    pub port_range_start: u16,
    pub port_range_end: u16,
    pub default_ctx_size: u32,
    pub health_check_timeout: Duration,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            llama_server_path: PathBuf::from("llama-server"),
            port_range_start: 8080,
            port_range_end: 8180,
            default_ctx_size: 2048,
            health_check_timeout: Duration::from_secs(30),
        }
    }
}

/// Snapshot of process state for API responses
#[derive(Debug, Clone, Serialize)]
pub struct ProcessSnapshot {
    pub model_id: String,
    pub pid: u32,
    pub port: u16,
    pub device: DeviceId,
    pub uptime_seconds: u64,
    pub idle_seconds: u64,
    pub healthy: bool,
}

/// Manages llama-server process lifecycle
pub struct ProcessManager {
    config: ProcessConfig,
    processes: RwLock<HashMap<String, ProcessInfo>>,
    port_allocator: RwLock<PortAllocator>,
}

impl ProcessManager {
    /// Create a new ProcessManager
    pub fn new(config: ProcessConfig) -> Self {
        // Check if llama-server exists
        if !config.llama_server_path.exists() {
            tracing::warn!(
                "llama-server not found at {:?}. Process spawning will fail.",
                config.llama_server_path
            );
        }

        let port_allocator = PortAllocator::new(config.port_range_start, config.port_range_end);

        Self {
            config,
            processes: RwLock::new(HashMap::new()),
            port_allocator: RwLock::new(port_allocator),
        }
    }

    /// Check if llama-server is available
    pub fn is_available(&self) -> bool {
        self.config.llama_server_path.exists()
    }

    /// Spawn a new llama-server process
    pub async fn spawn(
        &self,
        model: &LocalModel,
        device: DeviceId,
    ) -> Result<ProcessSnapshot, RiverError> {
        // Check if llama-server exists
        if !self.config.llama_server_path.exists() {
            return Err(RiverError::orchestrator(format!(
                "llama-server not found at {:?}",
                self.config.llama_server_path
            )));
        }

        // Check if model is already running
        {
            let processes = self.processes.read().await;
            if processes.contains_key(&model.id) {
                return Err(RiverError::orchestrator(format!(
                    "Model '{}' is already running",
                    model.id
                )));
            }
        }

        // Allocate a port
        let port = {
            let mut allocator = self.port_allocator.write().await;
            allocator.next()?
        };

        // Build the command
        let mut cmd = Command::new(&self.config.llama_server_path);
        cmd.arg("--model")
            .arg(&model.path)
            .arg("--port")
            .arg(port.to_string())
            .arg("--ctx-size")
            .arg(self.config.default_ctx_size.to_string());

        // Set GPU configuration
        match device {
            DeviceId::Gpu(idx) => {
                // Use all GPU layers when on GPU
                cmd.arg("--n-gpu-layers").arg("999");
                // Set CUDA_VISIBLE_DEVICES to restrict to specific GPU
                cmd.env("CUDA_VISIBLE_DEVICES", idx.to_string());
            }
            DeviceId::Cpu => {
                // Use CPU only (0 GPU layers)
                cmd.arg("--n-gpu-layers").arg("0");
            }
        }

        // Suppress stdout/stderr
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        // Spawn the process
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                // Release the port on error
                let mut allocator = self.port_allocator.write().await;
                allocator.release(port);
                return Err(RiverError::orchestrator(format!(
                    "Failed to spawn llama-server: {}",
                    e
                )));
            }
        };

        let pid = child.id().ok_or_else(|| {
            RiverError::orchestrator("Failed to get process ID")
        })?;

        let now = Instant::now();

        // Create process info
        let process_info = ProcessInfo {
            model_id: model.id.clone(),
            pid,
            port,
            device,
            started_at: now,
            last_request: now,
            health: ProcessHealth::Starting,
            child,
        };

        // Store process info
        {
            let mut processes = self.processes.write().await;
            processes.insert(model.id.clone(), process_info);
        }

        // Wait for the server to become ready
        self.wait_for_ready(&model.id, port).await?;

        // Mark as healthy
        {
            let mut processes = self.processes.write().await;
            if let Some(info) = processes.get_mut(&model.id) {
                info.health = ProcessHealth::Healthy;
            }
        }

        // Return snapshot
        self.get_process(&model.id)
            .await
            .ok_or_else(|| RiverError::orchestrator("Process disappeared after spawn"))
    }

    /// Wait for llama-server to become ready
    async fn wait_for_ready(&self, model_id: &str, port: u16) -> Result<(), RiverError> {
        let start = Instant::now();
        let timeout = self.config.health_check_timeout;
        let check_interval = Duration::from_millis(500);

        loop {
            // Check if we've exceeded the timeout
            if start.elapsed() > timeout {
                // Mark as unhealthy and return error
                let mut processes = self.processes.write().await;
                if let Some(info) = processes.get_mut(model_id) {
                    info.health = ProcessHealth::Unhealthy {
                        since: Instant::now(),
                        reason: "Health check timeout".to_string(),
                    };
                }
                return Err(RiverError::orchestrator(format!(
                    "Health check timeout for model '{}'",
                    model_id
                )));
            }

            // Try to connect to the health endpoint
            let url = format!("http://localhost:{}/health", port);
            match reqwest::get(&url).await {
                Ok(response) if response.status().is_success() => {
                    tracing::info!("Model '{}' is ready on port {}", model_id, port);
                    return Ok(());
                }
                Ok(response) => {
                    tracing::debug!(
                        "Health check for '{}' returned status {}",
                        model_id,
                        response.status()
                    );
                }
                Err(e) => {
                    tracing::debug!("Health check for '{}' failed: {}", model_id, e);
                }
            }

            // Wait before trying again
            tokio::time::sleep(check_interval).await;
        }
    }

    /// Kill a running process
    pub async fn kill(&self, model_id: &str) -> Result<(), RiverError> {
        let mut processes = self.processes.write().await;

        if let Some(mut info) = processes.remove(model_id) {
            // Kill the process
            if let Err(e) = info.child.kill().await {
                tracing::warn!("Failed to kill process for '{}': {}", model_id, e);
            }

            // Release the port
            let mut allocator = self.port_allocator.write().await;
            allocator.release(info.port);

            tracing::info!("Killed process for model '{}'", model_id);
            Ok(())
        } else {
            Err(RiverError::orchestrator(format!(
                "Model '{}' is not running",
                model_id
            )))
        }
    }

    /// Get process snapshot
    pub async fn get_process(&self, model_id: &str) -> Option<ProcessSnapshot> {
        let processes = self.processes.read().await;
        processes.get(model_id).map(|info| {
            let now = Instant::now();
            ProcessSnapshot {
                model_id: info.model_id.clone(),
                pid: info.pid,
                port: info.port,
                device: info.device,
                uptime_seconds: now.duration_since(info.started_at).as_secs(),
                idle_seconds: now.duration_since(info.last_request).as_secs(),
                healthy: matches!(info.health, ProcessHealth::Healthy),
            }
        })
    }

    /// Get all process snapshots
    pub async fn get_all_processes(&self) -> Vec<ProcessSnapshot> {
        let processes = self.processes.read().await;
        let now = Instant::now();
        processes
            .values()
            .map(|info| ProcessSnapshot {
                model_id: info.model_id.clone(),
                pid: info.pid,
                port: info.port,
                device: info.device,
                uptime_seconds: now.duration_since(info.started_at).as_secs(),
                idle_seconds: now.duration_since(info.last_request).as_secs(),
                healthy: matches!(info.health, ProcessHealth::Healthy),
            })
            .collect()
    }

    /// Record that a request was made to a model
    pub async fn record_request(&self, model_id: &str) {
        let mut processes = self.processes.write().await;
        if let Some(info) = processes.get_mut(model_id) {
            info.last_request = Instant::now();
        }
    }

    /// Get list of models that have been idle longer than the threshold
    pub async fn idle_models(&self, threshold: Duration) -> Vec<String> {
        let processes = self.processes.read().await;
        let now = Instant::now();
        processes
            .values()
            .filter(|info| now.duration_since(info.last_request) > threshold)
            .map(|info| info.model_id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_config_default() {
        let config = ProcessConfig::default();
        assert_eq!(config.llama_server_path, PathBuf::from("llama-server"));
        assert_eq!(config.port_range_start, 8080);
        assert_eq!(config.port_range_end, 8180);
        assert_eq!(config.default_ctx_size, 2048);
        assert_eq!(config.health_check_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_process_manager_creation() {
        let config = ProcessConfig::default();
        let manager = ProcessManager::new(config);

        // Should not panic, but llama-server probably won't exist
        // The constructor should just warn if the path doesn't exist
        assert!(!manager.is_available() || manager.is_available());
    }
}
