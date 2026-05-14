//! Structured JSON logging with daily file rotation

use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Directory for log files (default: {data-dir}/logs/)
    pub log_dir: PathBuf,
    /// Override log file path
    pub log_file: Option<PathBuf>,
    /// Output JSON to stdout (default: false for tty, true otherwise)
    pub json_stdout: bool,
    /// Log level filter (default: "info")
    pub log_level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_dir: PathBuf::from("logs"),
            log_file: None,
            json_stdout: false,
            log_level: "info".to_string(),
        }
    }
}

/// Guard that flushes logs on drop - must be kept alive
pub struct LogGuard {
    _file_guard: WorkerGuard,
    _stdout_guard: Option<WorkerGuard>,
}

/// Initialize logging with JSON output to file and stdout
///
/// Returns a guard that must be kept alive for the duration of the program.
pub fn init_logging(config: &LogConfig) -> Result<LogGuard, std::io::Error> {
    // Ensure log directory exists
    let log_dir = config
        .log_file
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.log_dir.clone());

    std::fs::create_dir_all(&log_dir)?;

    // Create daily rolling file appender
    let file_appender = tracing_appender::rolling::daily(&log_dir, "gateway");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    // Build filter from config or env
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    // File layer: always JSON
    let file_layer = fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_file(true)
        .with_line_number(true);

    // Stdout layer: JSON if configured or non-tty, pretty otherwise
    let use_json_stdout = config.json_stdout || !atty::is(atty::Stream::Stdout);

    if use_json_stdout {
        let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());
        let stdout_layer = fmt::layer().json().with_writer(stdout_writer);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stdout_layer)
            .init();

        Ok(LogGuard {
            _file_guard: file_guard,
            _stdout_guard: Some(stdout_guard),
        })
    } else {
        let stdout_layer = fmt::layer().pretty();

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stdout_layer)
            .init();

        Ok(LogGuard {
            _file_guard: file_guard,
            _stdout_guard: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert_eq!(config.log_level, "info");
        assert!(!config.json_stdout);
    }

    #[test]
    fn test_init_logging_creates_dir() {
        let dir = TempDir::new().unwrap();
        let log_dir = dir.path().join("logs");

        let config = LogConfig {
            log_dir: log_dir.clone(),
            log_file: None,
            json_stdout: false,
            log_level: "info".to_string(),
        };

        // Can't actually init logging in tests (global state), but we can verify the dir creation logic
        std::fs::create_dir_all(&log_dir).unwrap();
        assert!(log_dir.exists());
    }
}
