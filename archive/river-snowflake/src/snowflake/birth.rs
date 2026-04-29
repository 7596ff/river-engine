//! Agent birth timestamp (36-bit packed).

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::SnowflakeError;

/// 36-bit packed birth timestamp (yyyymmddhhmmss).
///
/// Encodes year (12 bits), month (4), day (5), hour (5), minute (6), second (6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentBirth(pub(crate) u64);

impl AgentBirth {
    /// Create from individual components.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, SnowflakeError> {
        if year < 2000 || year > 4095 {
            return Err(SnowflakeError::InvalidBirth("year out of range".into()));
        }
        if month < 1 || month > 12 {
            return Err(SnowflakeError::InvalidBirth("month out of range".into()));
        }
        if day < 1 || day > 31 {
            return Err(SnowflakeError::InvalidBirth("day out of range".into()));
        }
        if hour > 23 {
            return Err(SnowflakeError::InvalidBirth("hour out of range".into()));
        }
        if minute > 59 {
            return Err(SnowflakeError::InvalidBirth("minute out of range".into()));
        }
        if second > 59 {
            return Err(SnowflakeError::InvalidBirth("second out of range".into()));
        }

        // Pack: [year_offset:10][month:4][day:5][hour:5][minute:6][second:6] = 36 bits
        let year_offset = (year - 2000) as u64;

        let packed = (year_offset << 26)
            | ((month as u64) << 22)
            | ((day as u64) << 17)
            | ((hour as u64) << 12)
            | ((minute as u64) << 6)
            | (second as u64);

        Ok(Self(packed))
    }

    /// Create from current system time.
    pub fn now() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards");

        let secs = now.as_secs();
        // Calculate components from unix timestamp
        // This is a simplified calculation - proper implementation would use chrono
        let days_since_epoch = secs / 86400;
        let time_of_day = secs % 86400;

        let hour = (time_of_day / 3600) as u8;
        let minute = ((time_of_day % 3600) / 60) as u8;
        let second = (time_of_day % 60) as u8;

        // Simplified date calculation (not accounting for leap years properly)
        let mut year = 1970u16;
        let mut remaining_days = days_since_epoch as i64;

        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if remaining_days < days_in_year {
                break;
            }
            remaining_days -= days_in_year;
            year += 1;
        }

        let mut month = 1u8;
        let days_in_months = if is_leap_year(year) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };

        for days in days_in_months.iter() {
            if remaining_days < *days as i64 {
                break;
            }
            remaining_days -= *days as i64;
            month += 1;
        }

        let day = (remaining_days + 1) as u8;

        Self::new(year, month, day, hour, minute, second)
            .expect("current time should always be valid")
    }

    /// Get the raw packed value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Create from raw packed value.
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }

    /// Create from raw packed value with validation.
    pub fn try_from_u64(value: u64) -> Result<Self, crate::SnowflakeError> {
        let birth = Self(value);
        let year = birth.year();
        let month = birth.month();
        let day = birth.day();
        let hour = birth.hour();
        let minute = birth.minute();
        let second = birth.second();

        if year < 2000 || year > 3023 {
            return Err(crate::SnowflakeError::InvalidBirth(format!("year {} out of range (2000-3023)", year)));
        }
        if month < 1 || month > 12 {
            return Err(crate::SnowflakeError::InvalidBirth(format!("month {} out of range (1-12)", month)));
        }
        if day < 1 || day > 31 {
            return Err(crate::SnowflakeError::InvalidBirth(format!("day {} out of range (1-31)", day)));
        }
        if hour > 23 {
            return Err(crate::SnowflakeError::InvalidBirth(format!("hour {} out of range (0-23)", hour)));
        }
        if minute > 59 {
            return Err(crate::SnowflakeError::InvalidBirth(format!("minute {} out of range (0-59)", minute)));
        }
        if second > 59 {
            return Err(crate::SnowflakeError::InvalidBirth(format!("second {} out of range (0-59)", second)));
        }
        Ok(birth)
    }

    /// Extract year.
    pub fn year(&self) -> u16 {
        ((self.0 >> 26) as u16) + 2000
    }

    /// Extract month.
    pub fn month(&self) -> u8 {
        ((self.0 >> 22) & 0xF) as u8
    }

    /// Extract day.
    pub fn day(&self) -> u8 {
        ((self.0 >> 17) & 0x1F) as u8
    }

    /// Extract hour.
    pub fn hour(&self) -> u8 {
        ((self.0 >> 12) & 0x1F) as u8
    }

    /// Extract minute.
    pub fn minute(&self) -> u8 {
        ((self.0 >> 6) & 0x3F) as u8
    }

    /// Extract second.
    pub fn second(&self) -> u8 {
        (self.0 & 0x3F) as u8
    }

    /// Convert to ISO8601 string (date-time portion only).
    pub fn to_iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year(),
            self.month(),
            self.day(),
            self.hour(),
            self.minute(),
            self.second()
        )
    }

    /// Convert to Unix timestamp in seconds.
    pub fn to_unix_secs(&self) -> i64 {
        let year = self.year() as i64;
        let month = self.month() as i64;
        let day = self.day() as i64;
        let hour = self.hour() as i64;
        let minute = self.minute() as i64;
        let second = self.second() as i64;

        // Days from year 1970 to year
        let mut days: i64 = 0;
        for y in 1970..year {
            days += if is_leap_year(y as u16) { 366 } else { 365 };
        }

        // Days from months
        let days_in_months = if is_leap_year(year as u16) {
            [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };
        for m in 1..month {
            days += days_in_months[m as usize] as i64;
        }

        // Days in current month
        days += day - 1;

        days * 86400 + hour * 3600 + minute * 60 + second
    }
}

pub(crate) fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 30, 45).unwrap();
        assert_eq!(birth.year(), 2026);
        assert_eq!(birth.month(), 4);
        assert_eq!(birth.day(), 1);
        assert_eq!(birth.hour(), 12);
        assert_eq!(birth.minute(), 30);
        assert_eq!(birth.second(), 45);
    }

    #[test]
    fn test_iso8601() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 30, 45).unwrap();
        assert_eq!(birth.to_iso8601(), "2026-04-01T12:30:45");
    }

    #[test]
    fn test_try_from_u64_valid() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 30, 45).unwrap();
        let packed = birth.as_u64();
        let restored = AgentBirth::try_from_u64(packed).unwrap();
        assert_eq!(birth, restored);
    }

    #[test]
    fn test_try_from_u64_invalid_month() {
        let invalid = 0u64;
        let result = AgentBirth::try_from_u64(invalid);
        assert!(result.is_err());
    }
}
