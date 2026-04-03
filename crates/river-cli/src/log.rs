//! Traffic logging in JSONL format.

use chrono::Utc;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Traffic logger.
pub struct TrafficLog {
    file: File,
}

impl TrafficLog {
    /// Create a new traffic log.
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file })
    }

    /// Log an event.
    pub fn log<T: Serialize>(&mut self, event_type: &str, data: &T) {
        let event = LogEntry {
            ts: Utc::now().to_rfc3339(),
            event_type: event_type.to_string(),
            data: serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        };
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = writeln!(self.file, "{}", json);
        }
    }

    /// Log a simple message.
    pub fn log_message(&mut self, event_type: &str, message: &str) {
        let event = LogEntry {
            ts: Utc::now().to_rfc3339(),
            event_type: event_type.to_string(),
            data: serde_json::json!({ "message": message }),
        };
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = writeln!(self.file, "{}", json);
        }
    }
}

#[derive(Serialize)]
struct LogEntry {
    ts: String,
    #[serde(rename = "type")]
    event_type: String,
    data: serde_json::Value,
}
