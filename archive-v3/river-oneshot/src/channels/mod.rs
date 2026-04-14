//! Input/output channel adapters.
//!
//! Provides different ways to receive input and send output:
//! - stdin/stdout (default)
//! - File-based input
//! - Webhook (single request)
//!
//! Implemented in Phase 5.

use anyhow::Result;
use std::path::Path;

/// Input source for cycles.
#[derive(Debug, Clone)]
pub enum InputSource {
    /// Standard input (interactive).
    Stdin,
    /// Read from a file.
    File(std::path::PathBuf),
    /// Accept a webhook on a port.
    Webhook(u16),
}

impl Default for InputSource {
    fn default() -> Self {
        Self::Stdin
    }
}

/// Output sink for results.
#[derive(Debug, Clone)]
pub enum OutputSink {
    /// Standard output.
    Stdout,
    /// Write to a file.
    File(std::path::PathBuf),
    /// POST to a URL.
    Http(String),
}

impl Default for OutputSink {
    fn default() -> Self {
        Self::Stdout
    }
}

/// Read a line from stdin.
pub fn read_line() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line)
}

/// Read initial input from a source.
pub fn read_initial(source: &InputSource) -> Result<Option<String>> {
    match source {
        InputSource::Stdin => Ok(None), // Will be read in main loop
        InputSource::File(path) => {
            let content = std::fs::read_to_string(path)?;
            Ok(Some(content.trim().to_string()))
        }
        InputSource::Webhook(_port) => {
            // TODO: Implement in Phase 5
            Ok(None)
        }
    }
}
