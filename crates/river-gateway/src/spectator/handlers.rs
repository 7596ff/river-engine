//! Spectator event handlers

use regex::Regex;

/// Parsed moment response from LLM
#[derive(Debug, Clone, PartialEq)]
pub struct MomentResponse {
    pub start_turn: u64,
    pub end_turn: u64,
    pub narrative: String,
}

/// Parse a moment LLM response.
///
/// Expected format:
/// ```text
/// turns: 5-20
/// ---
/// The narrative paragraph here...
/// ```
///
/// Returns Err if the format is not followed.
pub fn parse_moment_response(response: &str) -> Result<MomentResponse, String> {
    // Split on first "---"
    let parts: Vec<&str> = response.splitn(2, "---").collect();
    if parts.len() != 2 {
        return Err("No '---' separator found in response".to_string());
    }

    let header = parts[0].trim();
    let narrative = parts[1].trim().to_string();

    if narrative.is_empty() {
        return Err("Empty narrative after '---' separator".to_string());
    }

    // Parse turns: N-M
    let re = Regex::new(r"turns:\s*(\d+)\s*-\s*(\d+)").unwrap();
    let caps = re
        .captures(header)
        .ok_or_else(|| format!("No 'turns: N-M' found in header: '{}'", header))?;

    let start_turn: u64 = caps[1]
        .parse()
        .map_err(|e| format!("Invalid start turn: {}", e))?;
    let end_turn: u64 = caps[2]
        .parse()
        .map_err(|e| format!("Invalid end turn: {}", e))?;

    Ok(MomentResponse {
        start_turn,
        end_turn,
        narrative,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_moment_parses_turn_range() {
        let response = "turns: 5-20\n---\nThe agent worked through configuration issues.";
        let result = parse_moment_response(response).unwrap();
        assert_eq!(result.start_turn, 5);
        assert_eq!(result.end_turn, 20);
        assert_eq!(result.narrative, "The agent worked through configuration issues.");
    }

    #[test]
    fn test_moment_parses_with_whitespace() {
        let response = "turns:  12 - 34 \n---\n\nA multi-paragraph\nnarrative here.";
        let result = parse_moment_response(response).unwrap();
        assert_eq!(result.start_turn, 12);
        assert_eq!(result.end_turn, 34);
        assert!(result.narrative.contains("multi-paragraph"));
    }

    #[test]
    fn test_moment_rejects_missing_separator() {
        let response = "turns: 5-20\nThe agent worked through issues.";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No '---' separator"));
    }

    #[test]
    fn test_moment_rejects_missing_turn_range() {
        let response = "some header\n---\nThe narrative.";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No 'turns: N-M'"));
    }

    #[test]
    fn test_moment_rejects_empty_narrative() {
        let response = "turns: 1-10\n---\n";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty narrative"));
    }

    #[test]
    fn test_moment_rejects_empty_narrative_whitespace() {
        let response = "turns: 1-10\n---\n   \n  ";
        let result = parse_moment_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty narrative"));
    }
}
