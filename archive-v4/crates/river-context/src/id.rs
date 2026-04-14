//! Snowflake ID utilities for timestamp extraction.

/// Extract timestamp (microseconds since epoch) from a snowflake ID.
///
/// Snowflake IDs are 128-bit integers where the high 64 bits contain
/// the timestamp in microseconds since Unix epoch.
///
/// # Arguments
/// * `id` - String representation of a snowflake ID
///
/// # Returns
/// * `Some(timestamp)` - Timestamp in microseconds if parsing succeeds
/// * `None` - If the ID cannot be parsed as a u128
///
/// # Example
/// ```
/// use river_context::extract_timestamp;
///
/// let id = "340282366920938463463374607431768211456"; // Example snowflake
/// if let Some(ts) = extract_timestamp(id) {
///     println!("Timestamp: {} microseconds", ts);
/// }
/// ```
pub fn extract_timestamp(id: &str) -> Option<u64> {
    let snowflake = id.parse::<u128>().ok()?;
    let high = (snowflake >> 64) as u64; // Timestamp in microseconds
    Some(high)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_timestamp_valid() {
        // Snowflake with known timestamp in high bits
        // High 64 bits = 1000000 (1 second in microseconds)
        // Low 64 bits = 0
        let snowflake: u128 = (1_000_000u128) << 64;
        let id = snowflake.to_string();

        let ts = extract_timestamp(&id).unwrap();
        assert_eq!(ts, 1_000_000);
    }

    #[test]
    fn test_extract_timestamp_with_low_bits() {
        // High 64 bits = 5000000, Low 64 bits = 12345
        let high: u128 = 5_000_000;
        let low: u128 = 12345;
        let snowflake: u128 = (high << 64) | low;
        let id = snowflake.to_string();

        let ts = extract_timestamp(&id).unwrap();
        assert_eq!(ts, 5_000_000);
    }

    #[test]
    fn test_extract_timestamp_invalid() {
        assert!(extract_timestamp("not_a_number").is_none());
        assert!(extract_timestamp("").is_none());
        assert!(extract_timestamp("-123").is_none());
    }

    #[test]
    fn test_extract_timestamp_zero() {
        let ts = extract_timestamp("0").unwrap();
        assert_eq!(ts, 0);
    }

    #[test]
    fn test_extract_timestamp_ordering() {
        // Earlier timestamp
        let early: u128 = (1_000_000u128) << 64;
        // Later timestamp
        let late: u128 = (2_000_000u128) << 64;

        let ts_early = extract_timestamp(&early.to_string()).unwrap();
        let ts_late = extract_timestamp(&late.to_string()).unwrap();

        assert!(ts_early < ts_late);
    }
}
