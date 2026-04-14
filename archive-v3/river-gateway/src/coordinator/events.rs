//! Events for coordinator communication between agent and spectator

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Events emitted by the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Agent is about to start a turn
    TurnStarted {
        channel: String,
        turn_number: u64,
        timestamp: DateTime<Utc>,
    },
    /// Agent completed a turn (includes transcript summary)
    TurnComplete {
        channel: String,
        turn_number: u64,
        transcript_summary: String,
        tool_calls: Vec<String>,
        timestamp: DateTime<Utc>,
    },
    /// Agent wrote a note to embeddings/
    NoteWritten {
        path: String,
        timestamp: DateTime<Utc>,
    },
    /// Agent switched channels
    ChannelSwitched {
        from: String,
        to: String,
        timestamp: DateTime<Utc>,
    },
    /// Context is getting full
    ContextPressure {
        usage_percent: f64,
        timestamp: DateTime<Utc>,
    },
}

/// Events emitted by the spectator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpectatorEvent {
    /// Memory surfaced into flash queue
    Flash {
        content: String,
        source: String,
        ttl_turns: u8,
        timestamp: DateTime<Utc>,
    },
    /// Pattern or observation noticed
    Observation {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Urgent signal (context pressure, drift, etc.)
    Warning {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Moves file updated for a channel
    MovesUpdated {
        channel: String,
        timestamp: DateTime<Utc>,
    },
    /// Arc compressed into a moment
    MomentCreated {
        summary: String,
        timestamp: DateTime<Utc>,
    },
}

/// All events on the bus
#[derive(Debug, Clone)]
pub enum CoordinatorEvent {
    Agent(AgentEvent),
    Spectator(SpectatorEvent),
    /// System-level events
    Shutdown,
}
