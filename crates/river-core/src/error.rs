//! Error types for River Engine
//!
//! This module defines the error types used throughout River Engine,
//! providing structured error handling with descriptive messages.

use thiserror::Error;

/// The main error type for River Engine operations.
///
/// This enum covers all categories of errors that can occur during
/// River Engine operation, from configuration issues to runtime failures.
///
/// # Examples
///
/// ```
/// use river_core::{RiverError, RiverResult};
///
/// fn load_config() -> RiverResult<()> {
///     Err(RiverError::Config("missing required field".into()))
/// }
/// ```
#[derive(Debug, Error)]
pub enum RiverError {
    /// Configuration-related errors (invalid config, missing fields, etc.)
    #[error("Configuration error: {0}")]
    Config(String),

    /// Database operation errors (connection, query, migration failures)
    #[error("Database error: {0}")]
    Database(String),

    /// Tool execution errors (tool not found, execution failed, timeout)
    #[error("Tool execution error: {0}")]
    Tool(String),

    /// Model interaction errors (API errors, rate limits, invalid responses)
    #[error("Model error: {0}")]
    Model(String),

    /// Authentication errors (invalid token, expired credentials)
    #[error("Authentication error: {0}")]
    Auth(String),

    /// Session management errors (session not found, invalid state)
    #[error("Session error: {0}")]
    Session(String),

    /// Workspace errors (invalid path, permission denied, not found)
    #[error("Workspace error: {0}")]
    Workspace(String),

    /// Communication adapter errors (connection failed, protocol errors)
    #[error("Communication adapter error: {0}")]
    Adapter(String),

    /// Orchestrator errors (scheduling, resource allocation failures)
    #[error("Orchestrator error: {0}")]
    Orchestrator(String),

    /// Embedding server errors (connection, API, response parsing)
    #[error("Embedding error: {0}")]
    Embedding(String),

    /// Redis errors (connection, command execution, timeout)
    #[error("Redis error: {0}")]
    Redis(String),

    /// JSON serialization/deserialization errors
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Standard IO errors (file not found, permission denied, etc.)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// A specialized Result type for River Engine operations.
///
/// This type alias reduces boilerplate when working with functions
/// that can return RiverError.
pub type RiverResult<T> = Result<T, RiverError>;

impl RiverError {
    /// Creates a new Config error with the given message.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Creates a new Database error with the given message.
    pub fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    /// Creates a new Tool error with the given message.
    pub fn tool(msg: impl Into<String>) -> Self {
        Self::Tool(msg.into())
    }

    /// Creates a new Model error with the given message.
    pub fn model(msg: impl Into<String>) -> Self {
        Self::Model(msg.into())
    }

    /// Creates a new Auth error with the given message.
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth(msg.into())
    }

    /// Creates a new Session error with the given message.
    pub fn session(msg: impl Into<String>) -> Self {
        Self::Session(msg.into())
    }

    /// Creates a new Workspace error with the given message.
    pub fn workspace(msg: impl Into<String>) -> Self {
        Self::Workspace(msg.into())
    }

    /// Creates a new Adapter error with the given message.
    pub fn adapter(msg: impl Into<String>) -> Self {
        Self::Adapter(msg.into())
    }

    /// Creates a new Orchestrator error with the given message.
    pub fn orchestrator(msg: impl Into<String>) -> Self {
        Self::Orchestrator(msg.into())
    }

    /// Creates a new Embedding error with the given message.
    pub fn embedding(msg: impl Into<String>) -> Self {
        Self::Embedding(msg.into())
    }

    /// Creates a new Redis error with the given message.
    pub fn redis(msg: impl Into<String>) -> Self {
        Self::Redis(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_error_display_config() {
        let err = RiverError::Config("missing field".into());
        assert_eq!(err.to_string(), "Configuration error: missing field");
    }

    #[test]
    fn test_error_display_database() {
        let err = RiverError::Database("connection failed".into());
        assert_eq!(err.to_string(), "Database error: connection failed");
    }

    #[test]
    fn test_error_display_tool() {
        let err = RiverError::Tool("execution timeout".into());
        assert_eq!(err.to_string(), "Tool execution error: execution timeout");
    }

    #[test]
    fn test_error_display_model() {
        let err = RiverError::Model("rate limit exceeded".into());
        assert_eq!(err.to_string(), "Model error: rate limit exceeded");
    }

    #[test]
    fn test_error_display_auth() {
        let err = RiverError::Auth("invalid token".into());
        assert_eq!(err.to_string(), "Authentication error: invalid token");
    }

    #[test]
    fn test_error_display_session() {
        let err = RiverError::Session("session expired".into());
        assert_eq!(err.to_string(), "Session error: session expired");
    }

    #[test]
    fn test_error_display_workspace() {
        let err = RiverError::Workspace("path not found".into());
        assert_eq!(err.to_string(), "Workspace error: path not found");
    }

    #[test]
    fn test_error_display_adapter() {
        let err = RiverError::Adapter("connection refused".into());
        assert_eq!(
            err.to_string(),
            "Communication adapter error: connection refused"
        );
    }

    #[test]
    fn test_error_display_orchestrator() {
        let err = RiverError::Orchestrator("no available workers".into());
        assert_eq!(
            err.to_string(),
            "Orchestrator error: no available workers"
        );
    }

    #[test]
    fn test_error_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: RiverError = io_err.into();
        assert!(matches!(err, RiverError::Io(_)));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_error_from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let err: RiverError = json_err.into();
        assert!(matches!(err, RiverError::Serialization(_)));
    }

    #[test]
    fn test_error_helper_constructors() {
        let err = RiverError::config("test");
        assert!(matches!(err, RiverError::Config(ref s) if s == "test"));

        let err = RiverError::database("test");
        assert!(matches!(err, RiverError::Database(ref s) if s == "test"));

        let err = RiverError::tool("test");
        assert!(matches!(err, RiverError::Tool(ref s) if s == "test"));

        let err = RiverError::model("test");
        assert!(matches!(err, RiverError::Model(ref s) if s == "test"));

        let err = RiverError::auth("test");
        assert!(matches!(err, RiverError::Auth(ref s) if s == "test"));

        let err = RiverError::session("test");
        assert!(matches!(err, RiverError::Session(ref s) if s == "test"));

        let err = RiverError::workspace("test");
        assert!(matches!(err, RiverError::Workspace(ref s) if s == "test"));

        let err = RiverError::adapter("test");
        assert!(matches!(err, RiverError::Adapter(ref s) if s == "test"));

        let err = RiverError::orchestrator("test");
        assert!(matches!(err, RiverError::Orchestrator(ref s) if s == "test"));

        let err = RiverError::embedding("test");
        assert!(matches!(err, RiverError::Embedding(ref s) if s == "test"));

        let err = RiverError::redis("test");
        assert!(matches!(err, RiverError::Redis(ref s) if s == "test"));
    }

    #[test]
    fn test_river_result_type() {
        fn success() -> RiverResult<i32> {
            Ok(42)
        }

        fn failure() -> RiverResult<i32> {
            Err(RiverError::Config("error".into()))
        }

        assert_eq!(success().unwrap(), 42);
        assert!(failure().is_err());
    }

    #[test]
    fn test_error_is_send() {
        // Note: RiverError may not be Sync due to serde_json::Error
        // but it should be Send
        fn assert_send<T: Send>() {}
        assert_send::<RiverError>();
    }
}
