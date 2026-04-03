//! Parsing and formatting snowflake IDs.

use crate::{Snowflake, SnowflakeError};

/// Parse a hex string "high-low" to Snowflake.
pub fn parse(s: &str) -> Result<Snowflake, SnowflakeError> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(SnowflakeError::InvalidFormat(
            "expected format: high-low".into(),
        ));
    }

    let high = u64::from_str_radix(parts[0], 16)
        .map_err(|_| SnowflakeError::InvalidFormat("invalid high component".into()))?;
    let low = u64::from_str_radix(parts[1], 16)
        .map_err(|_| SnowflakeError::InvalidFormat("invalid low component".into()))?;

    Ok(Snowflake { high, low })
}

/// Format Snowflake as hex string "high-low".
pub fn format(id: &Snowflake) -> String {
    id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_format_roundtrip() {
        let original = "0000000000123456-1a2b3c4d5e6f7890";
        let parsed = parse(original).unwrap();
        let formatted = format(&parsed);
        assert_eq!(formatted, original);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse("invalid").is_err());
        assert!(parse("abc-def-ghi").is_err());
        assert!(parse("zzzz-0000").is_err());
    }
}
