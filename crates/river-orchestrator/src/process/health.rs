//! Background health monitoring for llama-server processes

use super::ProcessManager;
use std::sync::Arc;
use std::time::Duration;

/// Run health check loop in background
pub async fn health_check_loop(manager: Arc<ProcessManager>, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;

        // Get all model IDs to check
        let model_ids: Vec<String> = manager.get_all_processes().await
            .into_iter()
            .map(|p| p.model_id)
            .collect();

        for model_id in model_ids {
            if let Some(snapshot) = manager.get_process(&model_id).await {
                let healthy = check_endpoint_health(snapshot.port).await;
                if !healthy {
                    manager.mark_unhealthy(&model_id, "Health check failed").await;
                }
            }
        }
    }
}

async fn check_endpoint_health(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let url = format!("http://127.0.0.1:{}/health", port);
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
