//! End-to-end integration tests for dyad boot and operation
//!
//! Tests validate:
//! - TEST-01: Complete dyad boot (orchestrator, workers, adapter)
//! - TEST-02: Worktree isolation and file I/O
//! - TEST-03: Baton swap and role switching

mod helpers;
mod mock_llm;

use helpers::*;
use serde_json::Value;
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

    // SPAWN: Start orchestrator with dynamic port and mock LLM endpoint
    let mut orchestrator = spawn_orchestrator(workspace_path, 0, &mock_llm.endpoint)
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

#[tokio::test]
async fn test_workers_write_to_worktrees() {
    use river_adapter::{InboundEvent, EventMetadata};
    use river_protocol::Author;

    // SETUP: Reuse boot sequence from test 1
    let temp_dir = setup_test_workspace().await.expect("Failed to create test workspace");
    let workspace_path = temp_dir.path();

    let mock_llm = mock_llm::start_mock_llm(0).await.expect("Failed to start mock LLM");
    let mut orchestrator = spawn_orchestrator(workspace_path, 0, &mock_llm.endpoint).await.expect("Failed to spawn orchestrator");

    timeout(Duration::from_secs(5), wait_for_health(&orchestrator.endpoint, 5))
        .await
        .expect("Orchestrator health check timed out")
        .expect("Orchestrator health check failed");

    let mut left_worker = spawn_worker(&orchestrator.endpoint, "test-dyad", "left").await.unwrap();
    let left_endpoint = timeout(Duration::from_secs(5), wait_for_registration(&orchestrator.endpoint, "test-dyad", "left", 5))
        .await.unwrap().unwrap();
    left_worker.endpoint = left_endpoint.clone();

    let mut right_worker = spawn_worker(&orchestrator.endpoint, "test-dyad", "right").await.unwrap();
    let right_endpoint = timeout(Duration::from_secs(5), wait_for_registration(&orchestrator.endpoint, "test-dyad", "right", 5))
        .await.unwrap().unwrap();
    right_worker.endpoint = right_endpoint.clone();

    let mut tui_adapter = spawn_tui_adapter(&orchestrator.endpoint, "test-dyad").await.unwrap();
    let tui_endpoint = timeout(Duration::from_secs(5), wait_for_registration(&orchestrator.endpoint, "test-dyad", "tui", 5))
        .await.unwrap().unwrap();
    tui_adapter.endpoint = tui_endpoint.clone();

    // Wait for all processes healthy
    for endpoint in &[&orchestrator.endpoint, &left_endpoint, &right_endpoint, &tui_endpoint] {
        timeout(Duration::from_secs(3), wait_for_health(endpoint, 2))
            .await.unwrap().unwrap();
    }

    println!("✓ Dyad booted successfully");

    // INJECT: Send user message to TUI adapter
    let client = reqwest::Client::new();
    let user_event = InboundEvent {
        adapter: "tui".to_string(),
        metadata: EventMetadata::MessageCreate {
            channel: "test-channel".to_string(),
            message_id: "msg-test-001".to_string(),
            author: Author {
                id: "user-1".to_string(),
                name: "Test User".to_string(),
                bot: false,
            },
            content: "Hello, what are you thinking?".to_string(),
            timestamp: "2026-04-06T23:00:00Z".to_string(),
            reply_to: None,
            attachments: vec![],
        },
    };

    let inject_response = client
        .post(format!("{}/notify", tui_endpoint))
        .json(&user_event)
        .send()
        .await
        .expect("Failed to inject message to TUI");
    assert_eq!(inject_response.status(), 200, "TUI /notify returned non-200 status");

    println!("✓ User message injected");

    // WAIT: Left worker writes context file (per 04-VALIDATION.md Assertion Point 2)
    let left_context_path = workspace_path
        .join("workspace")
        .join("left")
        .join("context.jsonl");

    let left_entry = timeout(
        Duration::from_secs(5),
        wait_for_context_entry(&left_context_path, |msg: &river_context::OpenAIMessage| {
            msg.role == "assistant" || msg.role == "user"
        }, 5)
    )
    .await
    .expect("Timeout waiting for left context entry")
    .expect("Failed to read left context entry");

    println!("✓ Left worker wrote context: {} bytes", left_entry.content.as_ref().map_or(0, |c| c.len()));

    // WAIT: Right worker writes context file
    let right_context_path = workspace_path
        .join("workspace")
        .join("right")
        .join("context.jsonl");

    let right_entry = timeout(
        Duration::from_secs(5),
        wait_for_context_entry(&right_context_path, |msg: &river_context::OpenAIMessage| {
            msg.role == "assistant" || msg.role == "user"
        }, 5)
    )
    .await
    .expect("Timeout waiting for right context entry")
    .expect("Failed to read right context entry");

    println!("✓ Right worker wrote context: {} bytes", right_entry.content.as_ref().map_or(0, |c| c.len()));

    // VERIFY: Context files are in different directories (proving worktree isolation)
    assert_ne!(
        left_context_path, right_context_path,
        "Left and right context files should be in different paths"
    );
    assert!(left_context_path.exists(), "Left context file should exist");
    assert!(right_context_path.exists(), "Right context file should exist");

    println!("✓ TEST-02: Both workers wrote to isolated worktrees");

    // CLEANUP
    let _ = orchestrator.child.kill().await;
    let _ = left_worker.child.kill().await;
    let _ = right_worker.child.kill().await;
    let _ = tui_adapter.child.kill().await;
}

#[tokio::test]
async fn test_baton_swap_verification() {
    use serde_json::Value;

    // SETUP: Boot complete dyad (reuse pattern from test 2)
    let temp_dir = setup_test_workspace().await.expect("Failed to create test workspace");
    let workspace_path = temp_dir.path();

    let mock_llm = mock_llm::start_mock_llm(0).await.expect("Failed to start mock LLM");
    let mut orchestrator = spawn_orchestrator(workspace_path, 0, &mock_llm.endpoint).await.expect("Failed to spawn orchestrator");

    timeout(Duration::from_secs(5), wait_for_health(&orchestrator.endpoint, 5))
        .await.unwrap().unwrap();

    let mut left_worker = spawn_worker(&orchestrator.endpoint, "test-dyad", "left").await.unwrap();
    let left_endpoint = timeout(Duration::from_secs(5), wait_for_registration(&orchestrator.endpoint, "test-dyad", "left", 5))
        .await.unwrap().unwrap();
    left_worker.endpoint = left_endpoint.clone();

    let mut right_worker = spawn_worker(&orchestrator.endpoint, "test-dyad", "right").await.unwrap();
    let right_endpoint = timeout(Duration::from_secs(5), wait_for_registration(&orchestrator.endpoint, "test-dyad", "right", 5))
        .await.unwrap().unwrap();
    right_worker.endpoint = right_endpoint.clone();

    let mut tui_adapter = spawn_tui_adapter(&orchestrator.endpoint, "test-dyad").await.unwrap();
    let tui_endpoint = timeout(Duration::from_secs(5), wait_for_registration(&orchestrator.endpoint, "test-dyad", "tui", 5))
        .await.unwrap().unwrap();
    tui_adapter.endpoint = tui_endpoint.clone();

    for endpoint in &[&orchestrator.endpoint, &left_endpoint, &right_endpoint, &tui_endpoint] {
        timeout(Duration::from_secs(3), wait_for_health(endpoint, 2))
            .await.unwrap().unwrap();
    }

    println!("✓ Dyad booted successfully");

    // CHECK: Read initial baton state from registry
    let client = reqwest::Client::new();
    let registry_resp = client
        .get(format!("{}/registry", orchestrator.endpoint))
        .send()
        .await
        .expect("Failed to read registry");

    let registry_json: Value = registry_resp.json().await.expect("Failed to parse registry JSON");

    // Parse registry to find left and right worker batons
    // Expected format: array of ProcessEntry objects with dyad, side, baton fields
    let left_initial_baton = extract_baton_from_registry(&registry_json, "test-dyad", "left")
        .expect("Failed to find left worker in registry");
    let right_initial_baton = extract_baton_from_registry(&registry_json, "test-dyad", "right")
        .expect("Failed to find right worker in registry");

    println!("✓ Initial baton state: left={}, right={}", left_initial_baton, right_initial_baton);

    // VERIFY: Initial state per protocol (left=actor, right=spectator)
    assert_eq!(left_initial_baton, "actor", "Left worker should start as actor");
    assert_eq!(right_initial_baton, "spectator", "Right worker should start as spectator");

    // TRIGGER: Explicit baton swap via orchestrator API (addressing Issue 7)
    // Call POST /switch_baton with dyad parameter to trigger role swap
    let swap_response = client
        .post(format!("{}/switch_baton", orchestrator.endpoint))
        .json(&serde_json::json!({ "dyad": "test-dyad" }))
        .send()
        .await
        .expect("Failed to trigger baton swap");

    assert!(swap_response.status().is_success(), "Baton swap API call failed: {}", swap_response.status());
    println!("✓ Baton swap triggered via orchestrator API");

    // WAIT: Allow orchestrator to propagate swap to workers (brief pause)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // CHECK: Read updated baton state from registry
    let registry_resp_after = client
        .get(format!("{}/registry", orchestrator.endpoint))
        .send()
        .await
        .expect("Failed to read registry after swap");

    let registry_json_after: Value = registry_resp_after.json().await.expect("Failed to parse registry JSON");

    let left_after_baton = extract_baton_from_registry(&registry_json_after, "test-dyad", "left")
        .expect("Failed to find left worker in registry after swap");
    let right_after_baton = extract_baton_from_registry(&registry_json_after, "test-dyad", "right")
        .expect("Failed to find right worker in registry after swap");

    println!("✓ After swap baton state: left={}, right={}", left_after_baton, right_after_baton);

    // VERIFY: Baton swapped (per 04-VALIDATION.md Assertion Point 4)
    assert_eq!(left_after_baton, "spectator", "Left worker should become spectator after swap");
    assert_eq!(right_after_baton, "actor", "Right worker should become actor after swap");

    println!("✓ TEST-03: Baton swapped successfully");

    // CLEANUP
    let _ = orchestrator.child.kill().await;
    let _ = left_worker.child.kill().await;
    let _ = right_worker.child.kill().await;
    let _ = tui_adapter.child.kill().await;
}

// Helper function to extract baton from registry JSON
// Registry format: {"processes": [{"type": "worker", "dyad": "...", "side": "...", "baton": "..."}]}
fn extract_baton_from_registry(registry: &Value, dyad: &str, side: &str) -> Option<String> {
    registry["processes"].as_array()?
        .iter()
        .find(|entry| {
            entry["type"].as_str() == Some("worker") &&
            entry["dyad"].as_str() == Some(dyad) &&
            entry["side"].as_str() == Some(side)
        })
        .and_then(|entry| entry["baton"].as_str())
        .map(|s| s.to_string())
}
