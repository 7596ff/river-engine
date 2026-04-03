//! Timestamp extraction from snowflakes.

use crate::Snowflake;

/// Extract ISO8601 timestamp from Snowflake.
///
/// Combines birth + relative timestamp to produce absolute time.
pub fn timestamp_iso8601(id: &Snowflake) -> String {
    let birth = id.birth();
    let birth_unix_secs = birth.to_unix_secs();
    let relative_micros = id.timestamp_micros();

    let total_micros = (birth_unix_secs as u64) * 1_000_000 + relative_micros;
    let secs = total_micros / 1_000_000;
    let micros = total_micros % 1_000_000;

    // Convert to date-time components
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let hour = (time_of_day / 3600) as u8;
    let minute = ((time_of_day % 3600) / 60) as u8;
    let second = (time_of_day % 60) as u8;

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

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
        year, month, day, hour, minute, second, micros
    )
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentBirth, SnowflakeType};

    #[test]
    fn test_timestamp_extraction() {
        let birth = AgentBirth::new(2026, 4, 1, 12, 0, 0).unwrap();
        let id = crate::Snowflake::new(0, birth, SnowflakeType::Message, 0);

        let ts = timestamp_iso8601(&id);
        assert!(ts.starts_with("2026-04-01T12:00:00"));
    }
}
