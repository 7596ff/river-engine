//! End-to-end integration tests for dyad boot and operation
//!
//! Tests validate:
//! - TEST-01: Complete dyad boot (orchestrator, workers, adapter)
//! - TEST-02: Worktree isolation and file I/O
//! - TEST-03: Baton swap and role switching

mod helpers;
mod mock_llm;

use helpers::*;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_dyad_boots_complete() {
    // SETUP: Create isolated test workspace
    let temp_dir = setup_test_workspace().await.expect("Failed to create test workspace");
    let workspace_path = temp_dir.path();

    // SPAWN: Start mock LLM server
    let mock_llm = mock_llm::start_mock_llm(0).await.expect("Failed to start mock LLM");
    println!("Mock LLM running at {}", mock_llm.endpoint);

    // SPAWN: Start orchestrator with dynamic port
    let mut orchestrator = spawn_orchestrator(workspace_path, 0)
        .await
        .expect("Failed to spawn orchestrator");

    // WAIT: Orchestrator health check
    let orch_health = timeout(
        Duration::from_secs(5),
        wait_for_health(&orchestrator.endpoint, 5)
    ).await;
    assert!(orch_health.is_ok(), "Orchestrator health check failed or timed out");

    // SPAWN: Start left worker
    let mut left_worker = spawn_worker(&orchestrator.endpoint, "test-dyad", "left")
        .await
        .expect("Failed to spawn left worker");

    // WAIT: Left worker registration
    let left_endpoint = timeout(
        Duration::from_secs(5),
        wait_for_registration(&orchestrator.endpoint, "test-dyad", "left", 5)
    ).await
    .expect("Left worker registration timed out")
    .expect("Left worker registration failed");
    left_worker.endpoint = left_endpoint.clone();
    println!("Left worker registered at {}", left_endpoint);

    // SPAWN: Start right worker
    let mut right_worker = spawn_worker(&orchestrator.endpoint, "test-dyad", "right")
        .await
        .expect("Failed to spawn right worker");

    // WAIT: Right worker registration
    let right_endpoint = timeout(
        Duration::from_secs(5),
        wait_for_registration(&orchestrator.endpoint, "test-dyad", "right", 5)
    ).await
    .expect("Right worker registration timed out")
    .expect("Right worker registration failed");
    right_worker.endpoint = right_endpoint.clone();
    println!("Right worker registered at {}", right_endpoint);

    // SPAWN: Start TUI adapter
    let mut tui_adapter = spawn_tui_adapter(&orchestrator.endpoint, "test-dyad")
        .await
        .expect("Failed to spawn TUI adapter");

    // WAIT: TUI adapter registration (adapter type, not side-specific)
    let tui_endpoint = timeout(
        Duration::from_secs(5),
        wait_for_registration(&orchestrator.endpoint, "test-dyad", "tui", 5)
    ).await
    .expect("TUI adapter registration timed out")
    .expect("TUI adapter registration failed");
    tui_adapter.endpoint = tui_endpoint.clone();
    println!("TUI adapter registered at {}", tui_endpoint);

    // VERIFY: All processes healthy (per 04-VALIDATION.md Assertion Point 1)
    let health_checks = vec![
        wait_for_health(&orchestrator.endpoint, 2),
        wait_for_health(&left_endpoint, 2),
        wait_for_health(&right_endpoint, 2),
        wait_for_health(&tui_endpoint, 2),
    ];

    for (idx, health_check) in health_checks.into_iter().enumerate() {
        let result = timeout(Duration::from_secs(3), health_check).await;
        assert!(
            result.is_ok() && result.unwrap().is_ok(),
            "Health check {} failed (0=orch, 1=left, 2=right, 3=tui)",
            idx
        );
    }

    println!("✓ TEST-01: All processes booted and healthy");

    // CLEANUP: Kill all processes
    let _ = orchestrator.child.kill().await;
    let _ = left_worker.child.kill().await;
    let _ = right_worker.child.kill().await;
    let _ = tui_adapter.child.kill().await;
}
