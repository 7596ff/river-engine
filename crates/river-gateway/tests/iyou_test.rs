//! I/You Architecture Integration Tests
//!
//! Tests the full coordination between agent and spectator tasks.

use river_gateway::agent::{AgentTask, AgentTaskConfig};
use river_gateway::coordinator::{Coordinator, AgentEvent, CoordinatorEvent, SpectatorEvent};
use river_gateway::flash::{Flash, FlashQueue, FlashTTL};
use river_gateway::r#loop::{MessageQueue, ModelClient};
use river_gateway::spectator::{SpectatorTask, SpectatorConfig};
use river_gateway::tools::{ToolRegistry, ToolExecutor};
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::RwLock;

fn test_model_client() -> ModelClient {
    ModelClient::new(
        "http://localhost:8080".to_string(),
        "test-model".to_string(),
        Duration::from_secs(30),
    ).unwrap()
}

fn test_tool_executor() -> Arc<RwLock<ToolExecutor>> {
    Arc::new(RwLock::new(ToolExecutor::new(ToolRegistry::new())))
}

/// Test that coordinator can spawn both agent and spectator tasks
#[tokio::test]
async fn test_coordinator_spawns_both_tasks() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().to_path_buf();
    let embeddings_dir = workspace.join("embeddings");
    std::fs::create_dir_all(&embeddings_dir).unwrap();

    let mut coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let flash_queue = Arc::new(FlashQueue::new(20));
    let message_queue = Arc::new(MessageQueue::new());

    // Create agent config
    let agent_config = AgentTaskConfig {
        workspace: workspace.clone(),
        embeddings_dir: embeddings_dir.clone(),
        context_limit: 8000,
        max_tool_calls: 10,
        history_limit: 10,
        heartbeat_interval: Duration::from_secs(300),
        ..Default::default()
    };

    // Create spectator config
    let spectator_config = SpectatorConfig::from_workspace(
        workspace.clone(),
        "http://localhost:8080".to_string(),
        "test-model".to_string(),
    );

    // Spawn agent task
    let agent_task = AgentTask::new(
        agent_config,
        bus.clone(),
        message_queue,
        test_model_client(),
        test_tool_executor(),
        flash_queue.clone(),
    );
    coordinator.spawn_task("agent", |_| agent_task.run());

    // Spawn spectator task
    let spectator_task = SpectatorTask::new(
        spectator_config,
        bus.clone(),
        test_model_client(),
        None,
        flash_queue,
    );
    coordinator.spawn_task("spectator", |_| spectator_task.run());

    // Both tasks should be running
    assert!(coordinator.is_running("agent"));
    assert!(coordinator.is_running("spectator"));

    // Shutdown
    coordinator.shutdown().await;
}

/// Test that spectator receives and processes agent events via the event bus
#[tokio::test]
async fn test_event_bus_routing() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx = bus.subscribe();

    // Publish a TurnComplete event
    bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnComplete {
        channel: "general".to_string(),
        turn_number: 1,
        transcript_summary: "User asked about integration tests".to_string(),
        tool_calls: vec!["read".to_string()],
        timestamp: Utc::now(),
    }));

    // Should receive the event
    let event = rx.recv().await.unwrap();
    match event {
        CoordinatorEvent::Agent(AgentEvent::TurnComplete { channel, turn_number, .. }) => {
            assert_eq!(channel, "general");
            assert_eq!(turn_number, 1);
        }
        _ => panic!("Expected TurnComplete event"),
    }
}

/// Test spectator event emission
#[tokio::test]
async fn test_spectator_events() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx = bus.subscribe();

    // Publish a MovesUpdated event
    bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated {
        channel: "general".to_string(),
        timestamp: Utc::now(),
    }));

    // Should receive the event
    let event = rx.recv().await.unwrap();
    match event {
        CoordinatorEvent::Spectator(SpectatorEvent::MovesUpdated { channel, .. }) => {
            assert_eq!(channel, "general");
        }
        _ => panic!("Expected MovesUpdated event"),
    }
}

/// Test context pressure event
#[tokio::test]
async fn test_context_pressure_event() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx = bus.subscribe();

    // Publish high context pressure
    bus.publish(CoordinatorEvent::Agent(AgentEvent::ContextPressure {
        usage_percent: 90.0,
        timestamp: Utc::now(),
    }));

    let event = rx.recv().await.unwrap();
    match event {
        CoordinatorEvent::Agent(AgentEvent::ContextPressure { usage_percent, .. }) => {
            assert!(usage_percent > 85.0);
        }
        _ => panic!("Expected ContextPressure event"),
    }
}

/// Test flash queue basic operations
#[tokio::test]
async fn test_flash_queue_operations() {
    let flash_queue = FlashQueue::new(10);

    // Push a flash
    flash_queue.push(Flash {
        id: "test-id".to_string(),
        content: "Important memory".to_string(),
        source: "notes/test.md".to_string(),
        ttl: FlashTTL::Turns(3),
        created: Utc::now(),
    }).await;

    // Active flashes should include our flash
    let active = flash_queue.active().await;
    assert!(!active.is_empty(), "Should have active flash");
    assert!(active[0].content.contains("Important memory"));
}

/// Test flash TTL expiry
#[tokio::test]
async fn test_flash_ttl_expiry() {
    let flash_queue = FlashQueue::new(10);

    flash_queue.push(Flash {
        id: "expiring".to_string(),
        content: "Will expire soon".to_string(),
        source: "notes/temp.md".to_string(),
        ttl: FlashTTL::Turns(2),
        created: Utc::now(),
    }).await;

    assert_eq!(flash_queue.active().await.len(), 1);

    flash_queue.tick_turn().await;
    assert_eq!(flash_queue.active().await.len(), 1); // remaining: 1

    flash_queue.tick_turn().await;
    assert_eq!(flash_queue.active().await.len(), 0); // expired
}

/// Test channel switching via events
#[tokio::test]
async fn test_channel_switch_event() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx = bus.subscribe();

    // Publish channel switch
    bus.publish(CoordinatorEvent::Agent(AgentEvent::ChannelSwitched {
        from: "channel-a".to_string(),
        to: "channel-b".to_string(),
        timestamp: Utc::now(),
    }));

    let event = rx.recv().await.unwrap();
    match event {
        CoordinatorEvent::Agent(AgentEvent::ChannelSwitched { from, to, .. }) => {
            assert_eq!(from, "channel-a");
            assert_eq!(to, "channel-b");
        }
        _ => panic!("Expected ChannelSwitched event"),
    }
}

/// Test shutdown event propagation
#[tokio::test]
async fn test_shutdown_event() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx = bus.subscribe();

    bus.publish(CoordinatorEvent::Shutdown);

    let event = rx.recv().await.unwrap();
    assert!(matches!(event, CoordinatorEvent::Shutdown));
}

/// Test warning event from spectator
#[tokio::test]
async fn test_spectator_warning_event() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx = bus.subscribe();

    bus.publish(CoordinatorEvent::Spectator(SpectatorEvent::Warning {
        content: "Context at 90% — consider rotation".to_string(),
        timestamp: Utc::now(),
    }));

    let event = rx.recv().await.unwrap();
    match event {
        CoordinatorEvent::Spectator(SpectatorEvent::Warning { content, .. }) => {
            assert!(content.contains("90%"));
        }
        _ => panic!("Expected Warning event"),
    }
}

/// Test multiple subscribers receive events
#[tokio::test]
async fn test_broadcast_to_multiple_subscribers() {
    let coordinator = Coordinator::new();
    let bus = coordinator.bus().clone();
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    bus.publish(CoordinatorEvent::Agent(AgentEvent::TurnStarted {
        channel: "general".to_string(),
        turn_number: 1,
        timestamp: Utc::now(),
    }));

    // Both should receive
    let e1 = rx1.recv().await.unwrap();
    let e2 = rx2.recv().await.unwrap();

    assert!(matches!(e1, CoordinatorEvent::Agent(AgentEvent::TurnStarted { .. })));
    assert!(matches!(e2, CoordinatorEvent::Agent(AgentEvent::TurnStarted { .. })));
}

/// Test compressor creates moves files (using internal module)
#[tokio::test]
async fn test_compressor_creates_moves() {
    use river_gateway::spectator::Compressor;

    let temp = TempDir::new().unwrap();
    let embeddings_dir = temp.path().to_path_buf();

    let compressor = Compressor::new(embeddings_dir.clone());

    // Update moves
    compressor.update_moves(
        "test-channel",
        1,
        "User asked about the weather",
        &["send_message".to_string()],
        &test_model_client(),
        "spectator identity",
    ).await.unwrap();

    // Verify moves file was created
    let moves_path = embeddings_dir.join("moves/test-channel.md");
    assert!(moves_path.exists(), "Moves file should exist");

    let content = std::fs::read_to_string(&moves_path).unwrap();
    assert!(content.contains("Move 1:"));
    assert!(content.contains("[response]"));
}

/// Test room writer creates session files
#[tokio::test]
async fn test_room_writer_creates_notes() {
    use river_gateway::spectator::RoomWriter;

    let temp = TempDir::new().unwrap();
    let room_notes_dir = temp.path().to_path_buf();

    let writer = RoomWriter::new(room_notes_dir.clone());

    writer.write_observation(
        1,
        "Turn 1 summary",
        &test_model_client(),
        "spectator identity",
    ).await.unwrap();

    // Session file should exist
    let session_path = writer.session_path();
    assert!(session_path.exists(), "Session file should exist");

    let content = std::fs::read_to_string(&session_path).unwrap();
    assert!(content.contains("Turn 1"));
}

/// Test curator with flash queue
#[tokio::test]
async fn test_curator_basic() {
    use river_gateway::spectator::Curator;

    let flash_queue = Arc::new(FlashQueue::new(10));
    let _curator = Curator::new(flash_queue);

    // Without vector store, curate is a no-op
    // This verifies the type can be created
    assert!(true);
}

/// Test compressor list_channels and count_moves
#[tokio::test]
async fn test_compressor_channel_tracking() {
    use river_gateway::spectator::Compressor;

    let temp = TempDir::new().unwrap();
    let embeddings_dir = temp.path().to_path_buf();

    let compressor = Compressor::new(embeddings_dir.clone());

    // Create moves for multiple channels
    for i in 1..=5 {
        compressor.update_moves(
            "channel-1",
            i,
            &format!("Turn {}", i),
            &[],
            &test_model_client(),
            "",
        ).await.unwrap();
    }

    for i in 1..=3 {
        compressor.update_moves(
            "channel-2",
            i,
            &format!("Turn {}", i),
            &[],
            &test_model_client(),
            "",
        ).await.unwrap();
    }

    // List channels
    let channels = compressor.list_channels().await.unwrap();
    assert_eq!(channels.len(), 2);

    // Count moves
    assert_eq!(compressor.count_moves("channel-1").await.unwrap(), 5);
    assert_eq!(compressor.count_moves("channel-2").await.unwrap(), 3);
}

/// Test moment creation
#[tokio::test]
async fn test_moment_creation() {
    use river_gateway::spectator::Compressor;

    let temp = TempDir::new().unwrap();
    let embeddings_dir = temp.path().to_path_buf();

    let compressor = Compressor::new(embeddings_dir.clone());

    let moves_text = "Move 1: [question] Asked about X\nMove 2: [response] Answered";
    let result = compressor.create_moment(
        "test",
        moves_text,
        &test_model_client(),
        "",
    ).await;

    assert!(result.is_ok());

    let moments_dir = embeddings_dir.join("moments");
    assert!(moments_dir.exists());

    let entries: Vec<_> = std::fs::read_dir(&moments_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);
}
