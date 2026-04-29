//! Session management module
//!
//! Manages the primary session and provides infrastructure for future sub-sessions.
//! Currently implements basic session tracking for the primary agent session.

use river_core::{SnowflakeGenerator, SnowflakeType};
use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Session identifier - "main" for primary, generated for sub-sessions
pub const PRIMARY_SESSION_ID: &str = "main";

/// Session state tracking
#[derive(Debug, Clone)]
pub struct Session {
    /// Session identifier
    pub id: String,
    /// When the session was created (Unix timestamp)
    pub created_at: i64,
    /// When the session was last active (Unix timestamp)
    pub last_active_at: i64,
    /// Number of cycles completed in this session
    pub cycle_count: u64,
    /// Number of messages processed in this session
    pub message_count: u64,
    /// Whether this is the primary session
    pub is_primary: bool,
}

impl Session {
    /// Create a new primary session
    pub fn primary() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Self {
            id: PRIMARY_SESSION_ID.to_string(),
            created_at: now,
            last_active_at: now,
            cycle_count: 0,
            message_count: 0,
            is_primary: true,
        }
    }

    /// Create a new sub-session with a generated ID
    pub fn sub_session(snowflake_gen: &SnowflakeGenerator) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let id = snowflake_gen.next_id(SnowflakeType::Session);

        Self {
            id: id.to_string(),
            created_at: now,
            last_active_at: now,
            cycle_count: 0,
            message_count: 0,
            is_primary: false,
        }
    }

    /// Record that a cycle was completed
    pub fn record_cycle(&mut self) {
        self.cycle_count += 1;
        self.update_last_active();
    }

    /// Record that messages were processed
    pub fn record_messages(&mut self, count: u64) {
        self.message_count += count;
        self.update_last_active();
    }

    /// Update last active timestamp
    pub fn update_last_active(&mut self) {
        self.last_active_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
    }

    /// Get session uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        (now - self.created_at).max(0) as u64
    }

    /// Get seconds since last activity
    pub fn idle_seconds(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        (now - self.last_active_at).max(0) as u64
    }
}

/// Session manager for tracking active sessions
pub struct SessionManager {
    /// The primary session
    primary: Session,
    /// Active sub-sessions (future use)
    #[allow(dead_code)]
    sub_sessions: Vec<Session>,
    /// Snowflake generator for creating session IDs (used for future sub-sessions)
    #[allow(dead_code)]
    snowflake_gen: Arc<SnowflakeGenerator>,
}

impl fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionManager")
            .field("primary", &self.primary)
            .field("sub_sessions", &self.sub_sessions)
            .field("snowflake_gen", &"<SnowflakeGenerator>")
            .finish()
    }
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(snowflake_gen: Arc<SnowflakeGenerator>) -> Self {
        Self {
            primary: Session::primary(),
            sub_sessions: Vec::new(),
            snowflake_gen,
        }
    }

    /// Get the primary session
    pub fn primary(&self) -> &Session {
        &self.primary
    }

    /// Get mutable reference to primary session
    pub fn primary_mut(&mut self) -> &mut Session {
        &mut self.primary
    }

    /// Get the current session ID (primary for now)
    pub fn current_session_id(&self) -> &str {
        &self.primary.id
    }

    /// Record a completed cycle on the primary session
    pub fn record_cycle(&mut self) {
        self.primary.record_cycle();
    }

    /// Record processed messages on the primary session
    pub fn record_messages(&mut self, count: u64) {
        self.primary.record_messages(count);
    }

    /// Get session statistics
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            session_id: self.primary.id.clone(),
            uptime_seconds: self.primary.uptime_seconds(),
            cycle_count: self.primary.cycle_count,
            message_count: self.primary.message_count,
            idle_seconds: self.primary.idle_seconds(),
        }
    }

    // Future: Methods for sub-session management
    // pub fn create_sub_session(&mut self) -> &Session
    // pub fn destroy_sub_session(&mut self, id: &str) -> bool
    // pub fn switch_session(&mut self, id: &str) -> bool
}

/// Session statistics for reporting
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub session_id: String,
    pub uptime_seconds: u64,
    pub cycle_count: u64,
    pub message_count: u64,
    pub idle_seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use river_core::AgentBirth;

    fn test_snowflake_gen() -> Arc<SnowflakeGenerator> {
        let birth = AgentBirth::new(2026, 3, 17, 12, 0, 0).unwrap();
        Arc::new(SnowflakeGenerator::new(birth))
    }

    #[test]
    fn test_primary_session() {
        let session = Session::primary();
        assert_eq!(session.id, PRIMARY_SESSION_ID);
        assert!(session.is_primary);
        assert_eq!(session.cycle_count, 0);
        assert_eq!(session.message_count, 0);
    }

    #[test]
    fn test_sub_session() {
        let gen = test_snowflake_gen();
        let session = Session::sub_session(&gen);
        assert!(!session.is_primary);
        assert_ne!(session.id, PRIMARY_SESSION_ID);
    }

    #[test]
    fn test_record_cycle() {
        let mut session = Session::primary();
        assert_eq!(session.cycle_count, 0);
        session.record_cycle();
        assert_eq!(session.cycle_count, 1);
        session.record_cycle();
        assert_eq!(session.cycle_count, 2);
    }

    #[test]
    fn test_record_messages() {
        let mut session = Session::primary();
        assert_eq!(session.message_count, 0);
        session.record_messages(5);
        assert_eq!(session.message_count, 5);
        session.record_messages(3);
        assert_eq!(session.message_count, 8);
    }

    #[test]
    fn test_session_manager() {
        let gen = test_snowflake_gen();
        let mut manager = SessionManager::new(gen);

        assert_eq!(manager.current_session_id(), PRIMARY_SESSION_ID);

        manager.record_cycle();
        manager.record_messages(10);

        let stats = manager.stats();
        assert_eq!(stats.cycle_count, 1);
        assert_eq!(stats.message_count, 10);
    }

    #[test]
    fn test_session_stats() {
        let gen = test_snowflake_gen();
        let manager = SessionManager::new(gen);
        let stats = manager.stats();

        assert_eq!(stats.session_id, PRIMARY_SESSION_ID);
        assert!(stats.uptime_seconds < 2); // Should be very small in test
    }
}
