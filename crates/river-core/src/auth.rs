//! Shared authentication — token loading and validation.
//!
//! Every river-engine service reads RIVER_AUTH_TOKEN from the environment
//! and validates it on non-health HTTP endpoints.

use crate::RiverError;

/// Read RIVER_AUTH_TOKEN from the environment.
/// Returns Err if missing or empty.
pub fn require_auth_token() -> Result<String, RiverError> {
    match std::env::var("RIVER_AUTH_TOKEN") {
        Ok(token) if !token.is_empty() => Ok(token),
        Ok(_) => Err(RiverError::config(
            "RIVER_AUTH_TOKEN is set but empty — set a token in .env or the environment",
        )),
        Err(_) => Err(RiverError::config(
            "RIVER_AUTH_TOKEN not set — create a .env file or set the environment variable",
        )),
    }
}

/// Validate a bearer token from an Authorization header value.
/// `auth_header` is the raw value of the Authorization header.
/// Returns true if it matches "Bearer <expected>".
pub fn validate_bearer(auth_header: &str, expected: &str) -> bool {
    match auth_header.strip_prefix("Bearer ") {
        Some(token) => !token.is_empty() && token == expected,
        None => false,
    }
}

/// Build a reqwest::Client with a default Authorization header.
/// Use this for all outbound HTTP calls that need auth.
pub fn build_authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    let value = format!("Bearer {}", token);
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&value)
            .expect("auth token contains invalid header characters"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_bearer_valid() {
        assert!(validate_bearer("Bearer my-secret-token", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_wrong_token() {
        assert!(!validate_bearer("Bearer wrong-token", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_missing_prefix() {
        assert!(!validate_bearer("my-secret-token", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_empty_header() {
        assert!(!validate_bearer("", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_bearer_only() {
        assert!(!validate_bearer("Bearer ", "my-secret-token"));
    }

    #[test]
    fn test_validate_bearer_case_sensitive_prefix() {
        assert!(!validate_bearer("bearer my-secret-token", "my-secret-token"));
    }

    #[test]
    fn test_require_auth_token_from_env() {
        std::env::set_var("RIVER_AUTH_TOKEN", "test-token-123");
        let result = require_auth_token();
        assert_eq!(result.unwrap(), "test-token-123");
        std::env::remove_var("RIVER_AUTH_TOKEN");
    }

    #[test]
    fn test_require_auth_token_missing() {
        std::env::remove_var("RIVER_AUTH_TOKEN");
        let result = require_auth_token();
        assert!(result.is_err());
    }

    #[test]
    fn test_require_auth_token_empty() {
        std::env::set_var("RIVER_AUTH_TOKEN", "");
        let result = require_auth_token();
        assert!(result.is_err());
        std::env::remove_var("RIVER_AUTH_TOKEN");
    }
}
