//! AgentBirth - 36-bit packed timestamp representing when an agent was born.
//!
//! Bit layout: [year_offset:10][month:4][day:5][hour:5][minute:6][second:6]
//! - year_offset: Years since 2000 (0-1023, supports 2000-2999)
//! - month: 1-12
//! - day: 1-31
//! - hour: 0-23
//! - minute: 0-59
//! - second: 0-59

use serde::{Deserialize, Serialize};
use std::fmt;

/// The base year for year_offset calculation.
const BASE_YEAR: u16 = 2000;

/// Maximum year offset (10 bits = 1023).
const MAX_YEAR_OFFSET: u16 = 1023;

/// Error type for AgentBirth validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentBirthError {
    /// Year is out of valid range (2000-2999).
    InvalidYear(u16),
    /// Month is out of valid range (1-12).
    InvalidMonth(u8),
    /// Day is out of valid range (1-31).
    InvalidDay(u8),
    /// Hour is out of valid range (0-23).
    InvalidHour(u8),
    /// Minute is out of valid range (0-59).
    InvalidMinute(u8),
    /// Second is out of valid range (0-59).
    InvalidSecond(u8),
}

impl fmt::Display for AgentBirthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentBirthError::InvalidYear(y) => write!(f, "invalid year: {} (must be 2000-2999)", y),
            AgentBirthError::InvalidMonth(m) => write!(f, "invalid month: {} (must be 1-12)", m),
            AgentBirthError::InvalidDay(d) => write!(f, "invalid day: {} (must be 1-31)", d),
            AgentBirthError::InvalidHour(h) => write!(f, "invalid hour: {} (must be 0-23)", h),
            AgentBirthError::InvalidMinute(m) => write!(f, "invalid minute: {} (must be 0-59)", m),
            AgentBirthError::InvalidSecond(s) => write!(f, "invalid second: {} (must be 0-59)", s),
        }
    }
}

impl std::error::Error for AgentBirthError {}

/// A 36-bit packed timestamp representing when an agent was born.
///
/// The timestamp is packed into a u64 (using only 36 bits) with the following layout:
/// - Bits 26-35 (10 bits): year offset from 2000
/// - Bits 22-25 (4 bits): month (1-12)
/// - Bits 17-21 (5 bits): day (1-31)
/// - Bits 12-16 (5 bits): hour (0-23)
/// - Bits 6-11 (6 bits): minute (0-59)
/// - Bits 0-5 (6 bits): second (0-59)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentBirth(u64);

impl AgentBirth {
    /// Create a new AgentBirth from individual components.
    ///
    /// # Arguments
    /// * `year` - Year (2000-2999)
    /// * `month` - Month (1-12)
    /// * `day` - Day (1-31)
    /// * `hour` - Hour (0-23)
    /// * `minute` - Minute (0-59)
    /// * `second` - Second (0-59)
    ///
    /// # Errors
    /// Returns an error if any component is out of valid range.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, AgentBirthError> {
        // Validate year
        if year < BASE_YEAR || year > BASE_YEAR + MAX_YEAR_OFFSET {
            return Err(AgentBirthError::InvalidYear(year));
        }

        // Validate month (1-12)
        if !(1..=12).contains(&month) {
            return Err(AgentBirthError::InvalidMonth(month));
        }

        // Validate day (1-31)
        if !(1..=31).contains(&day) {
            return Err(AgentBirthError::InvalidDay(day));
        }

        // Validate hour (0-23)
        if hour > 23 {
            return Err(AgentBirthError::InvalidHour(hour));
        }

        // Validate minute (0-59)
        if minute > 59 {
            return Err(AgentBirthError::InvalidMinute(minute));
        }

        // Validate second (0-59)
        if second > 59 {
            return Err(AgentBirthError::InvalidSecond(second));
        }

        let year_offset = (year - BASE_YEAR) as u64;

        // Pack the components
        let packed = (year_offset << 26)
            | ((month as u64) << 22)
            | ((day as u64) << 17)
            | ((hour as u64) << 12)
            | ((minute as u64) << 6)
            | (second as u64);

        Ok(Self(packed))
    }

    /// Create an AgentBirth from a raw 36-bit value.
    ///
    /// This does not validate the individual components.
    pub fn from_raw(raw: u64) -> Self {
        // Mask to 36 bits
        Self(raw & 0xF_FFFF_FFFF)
    }

    /// Get the raw 36-bit value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Get the year (2000-2999).
    pub fn year(&self) -> u16 {
        ((self.0 >> 26) & 0x3FF) as u16 + BASE_YEAR
    }

    /// Get the month (1-12).
    pub fn month(&self) -> u8 {
        ((self.0 >> 22) & 0xF) as u8
    }

    /// Get the day (1-31).
    pub fn day(&self) -> u8 {
        ((self.0 >> 17) & 0x1F) as u8
    }

    /// Get the hour (0-23).
    pub fn hour(&self) -> u8 {
        ((self.0 >> 12) & 0x1F) as u8
    }

    /// Get the minute (0-59).
    pub fn minute(&self) -> u8 {
        ((self.0 >> 6) & 0x3F) as u8
    }

    /// Get the second (0-59).
    pub fn second(&self) -> u8 {
        (self.0 & 0x3F) as u8
    }
}

impl fmt::Display for AgentBirth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year(),
            self.month(),
            self.day(),
            self.hour(),
            self.minute(),
            self.second()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_birth_creation() {
        let birth = AgentBirth::new(2024, 3, 15, 14, 30, 45).unwrap();
        assert_eq!(birth.year(), 2024);
        assert_eq!(birth.month(), 3);
        assert_eq!(birth.day(), 15);
        assert_eq!(birth.hour(), 14);
        assert_eq!(birth.minute(), 30);
        assert_eq!(birth.second(), 45);
    }

    #[test]
    fn test_agent_birth_roundtrip() {
        let birth = AgentBirth::new(2026, 12, 31, 23, 59, 59).unwrap();
        let raw = birth.as_u64();
        let reconstructed = AgentBirth::from_raw(raw);

        assert_eq!(birth, reconstructed);
        assert_eq!(reconstructed.year(), 2026);
        assert_eq!(reconstructed.month(), 12);
        assert_eq!(reconstructed.day(), 31);
        assert_eq!(reconstructed.hour(), 23);
        assert_eq!(reconstructed.minute(), 59);
        assert_eq!(reconstructed.second(), 59);
    }

    #[test]
    fn test_agent_birth_min_values() {
        let birth = AgentBirth::new(2000, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(birth.year(), 2000);
        assert_eq!(birth.month(), 1);
        assert_eq!(birth.day(), 1);
        assert_eq!(birth.hour(), 0);
        assert_eq!(birth.minute(), 0);
        assert_eq!(birth.second(), 0);
    }

    #[test]
    fn test_agent_birth_max_values() {
        let birth = AgentBirth::new(2999, 12, 31, 23, 59, 59).unwrap();
        assert_eq!(birth.year(), 2999);
        assert_eq!(birth.month(), 12);
        assert_eq!(birth.day(), 31);
        assert_eq!(birth.hour(), 23);
        assert_eq!(birth.minute(), 59);
        assert_eq!(birth.second(), 59);
    }

    #[test]
    fn test_agent_birth_invalid_year_low() {
        let result = AgentBirth::new(1999, 1, 1, 0, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidYear(1999))));
    }

    #[test]
    fn test_agent_birth_invalid_year_high() {
        let result = AgentBirth::new(3024, 1, 1, 0, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidYear(3024))));
    }

    #[test]
    fn test_agent_birth_invalid_month_zero() {
        let result = AgentBirth::new(2024, 0, 1, 0, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidMonth(0))));
    }

    #[test]
    fn test_agent_birth_invalid_month_high() {
        let result = AgentBirth::new(2024, 13, 1, 0, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidMonth(13))));
    }

    #[test]
    fn test_agent_birth_invalid_day_zero() {
        let result = AgentBirth::new(2024, 1, 0, 0, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidDay(0))));
    }

    #[test]
    fn test_agent_birth_invalid_day_high() {
        let result = AgentBirth::new(2024, 1, 32, 0, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidDay(32))));
    }

    #[test]
    fn test_agent_birth_invalid_hour() {
        let result = AgentBirth::new(2024, 1, 1, 24, 0, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidHour(24))));
    }

    #[test]
    fn test_agent_birth_invalid_minute() {
        let result = AgentBirth::new(2024, 1, 1, 0, 60, 0);
        assert!(matches!(result, Err(AgentBirthError::InvalidMinute(60))));
    }

    #[test]
    fn test_agent_birth_invalid_second() {
        let result = AgentBirth::new(2024, 1, 1, 0, 0, 60);
        assert!(matches!(result, Err(AgentBirthError::InvalidSecond(60))));
    }

    #[test]
    fn test_agent_birth_display() {
        let birth = AgentBirth::new(2024, 3, 15, 14, 30, 45).unwrap();
        assert_eq!(format!("{}", birth), "2024-03-15T14:30:45");
    }

    #[test]
    fn test_agent_birth_36_bits_only() {
        // Ensure only 36 bits are used
        let birth = AgentBirth::new(2999, 12, 31, 23, 59, 59).unwrap();
        let raw = birth.as_u64();
        // 36 bits = 0xF_FFFF_FFFF
        assert!(raw <= 0xF_FFFF_FFFF);
    }

    #[test]
    fn test_agent_birth_serde_roundtrip() {
        let birth = AgentBirth::new(2024, 6, 15, 10, 25, 30).unwrap();
        let json = serde_json::to_string(&birth).unwrap();
        let deserialized: AgentBirth = serde_json::from_str(&json).unwrap();
        assert_eq!(birth, deserialized);
    }
}
