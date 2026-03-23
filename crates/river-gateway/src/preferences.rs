//! Agent preferences (self-managed via PREFERENCES.toml)

use chrono::Utc;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Agent preferences loaded from PREFERENCES.toml
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Preferences {
    #[serde(default)]
    pub time: TimePreferences,

    #[serde(default)]
    pub identity: IdentityPreferences,

    #[serde(default)]
    pub schedule: SchedulePreferences,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TimePreferences {
    pub timezone: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct IdentityPreferences {
    pub preferred_name: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SchedulePreferences {
    pub working_hours: Option<String>,
}

impl Preferences {
    /// Load preferences from workspace/PREFERENCES.toml
    pub fn load(workspace: &Path) -> Self {
        let path = workspace.join("PREFERENCES.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Get timezone (defaults to UTC)
    pub fn timezone(&self) -> &str {
        self.time.timezone.as_deref().unwrap_or("UTC")
    }

    /// Get preferred name if set
    pub fn preferred_name(&self) -> Option<&str> {
        self.identity.preferred_name.as_deref()
    }
}

/// Format current time in the given timezone
pub fn format_current_time(tz_name: &str) -> String {
    let now = Utc::now();

    match tz_name.parse::<Tz>() {
        Ok(tz) => {
            let local = now.with_timezone(&tz);
            format!(
                "{} ({})",
                local.format("%A, %B %e, %Y at %l:%M %p").to_string().trim(),
                tz_name
            )
        }
        Err(_) => {
            format!(
                "{} UTC",
                now.format("%A, %B %e, %Y at %l:%M %p").to_string().trim()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_preferences_load_missing_file() {
        let prefs = Preferences::load(Path::new("/nonexistent"));
        assert_eq!(prefs.timezone(), "UTC");
    }

    #[test]
    fn test_preferences_load_valid() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("PREFERENCES.toml"),
            r#"
[time]
timezone = "America/Los_Angeles"

[identity]
preferred_name = "River"
"#,
        )
        .unwrap();

        let prefs = Preferences::load(dir.path());
        assert_eq!(prefs.timezone(), "America/Los_Angeles");
        assert_eq!(prefs.preferred_name(), Some("River"));
    }

    #[test]
    fn test_preferences_partial() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("PREFERENCES.toml"),
            "[time]\ntimezone = \"Europe/London\"\n",
        )
        .unwrap();

        let prefs = Preferences::load(dir.path());
        assert_eq!(prefs.timezone(), "Europe/London");
        assert_eq!(prefs.preferred_name(), None);
    }

    #[test]
    fn test_format_time_with_timezone() {
        let result = format_current_time("America/Los_Angeles");
        assert!(result.contains("America/Los_Angeles"));
    }

    #[test]
    fn test_format_time_invalid_timezone() {
        let result = format_current_time("Invalid/Zone");
        assert!(result.contains("UTC"));
    }

    #[test]
    fn test_format_time_utc() {
        let result = format_current_time("UTC");
        assert!(result.contains("UTC"));
    }
}
