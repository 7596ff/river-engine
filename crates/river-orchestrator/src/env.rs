//! Environment file loading and variable expansion
//!
//! Loads key=value files into the process environment (existing env wins).
//! Expands $VAR references in strings before JSON parsing.

use std::path::Path;

/// Load an env file into the process environment.
/// Existing environment variables take precedence (are NOT overwritten).
pub fn load_env_file(path: &Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read env file {:?}: {}", path, e))?;

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse KEY=VALUE
        let Some((key, value)) = line.split_once('=') else {
            tracing::warn!(line = line_num + 1, "Skipping malformed env line (no '='): {}", line);
            continue;
        };

        let key = key.trim();
        let value = value.trim();

        if key.is_empty() {
            continue;
        }

        // Existing environment wins
        if std::env::var(key).is_ok() {
            tracing::debug!(key = key, "Env var already set, skipping env file value");
            continue;
        }

        unsafe { std::env::set_var(key, value) };
    }

    Ok(())
}

/// Expand $VAR references in a string using the current process environment.
/// Returns an error if any referenced variable is not defined.
pub fn expand_vars(input: &str) -> anyhow::Result<String> {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Collect variable name (letters, digits, underscore — must start with letter or _)
            let mut var_name = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || next == '_' {
                    var_name.push(next);
                    chars.next();
                } else {
                    break;
                }
            }

            if var_name.is_empty() {
                // Bare $ with no variable name — keep it literal
                result.push('$');
                continue;
            }

            match std::env::var(&var_name) {
                Ok(value) => result.push_str(&value),
                Err(_) => {
                    anyhow::bail!(
                        "Undefined environment variable: ${} (referenced in config)",
                        var_name
                    );
                }
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_expand_vars_simple() {
        unsafe { std::env::set_var("TEST_EXPAND_A", "hello") };
        let result = expand_vars("value is $TEST_EXPAND_A").unwrap();
        assert_eq!(result, "value is hello");
        unsafe { std::env::remove_var("TEST_EXPAND_A") };
    }

    #[test]
    fn test_expand_vars_multiple() {
        unsafe { std::env::set_var("TEST_EXPAND_X", "foo") };
        unsafe { std::env::set_var("TEST_EXPAND_Y", "bar") };
        let result = expand_vars("$TEST_EXPAND_X and $TEST_EXPAND_Y").unwrap();
        assert_eq!(result, "foo and bar");
        unsafe { std::env::remove_var("TEST_EXPAND_X") };
        unsafe { std::env::remove_var("TEST_EXPAND_Y") };
    }

    #[test]
    fn test_expand_vars_in_json() {
        unsafe { std::env::set_var("TEST_GUILD", "123456") };
        let input = r#"{"guild_id": "$TEST_GUILD"}"#;
        let result = expand_vars(input).unwrap();
        assert_eq!(result, r#"{"guild_id": "123456"}"#);
        unsafe { std::env::remove_var("TEST_GUILD") };
    }

    #[test]
    fn test_expand_vars_undefined_is_error() {
        let result = expand_vars("$DEFINITELY_NOT_SET_EVER_12345");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("DEFINITELY_NOT_SET_EVER_12345"));
    }

    #[test]
    fn test_expand_vars_no_vars() {
        let result = expand_vars("no variables here").unwrap();
        assert_eq!(result, "no variables here");
    }

    #[test]
    fn test_expand_vars_bare_dollar_non_alnum() {
        // $ followed by space — kept literal
        let result = expand_vars("price is $ 5").unwrap();
        assert_eq!(result, "price is $ 5");
    }

    #[test]
    fn test_load_env_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.env");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "# comment").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "TEST_ENV_LOAD_A=hello").unwrap();
        writeln!(file, "TEST_ENV_LOAD_B=world").unwrap();

        load_env_file(&path).unwrap();

        assert_eq!(std::env::var("TEST_ENV_LOAD_A").unwrap(), "hello");
        assert_eq!(std::env::var("TEST_ENV_LOAD_B").unwrap(), "world");

        unsafe { std::env::remove_var("TEST_ENV_LOAD_A") };
        unsafe { std::env::remove_var("TEST_ENV_LOAD_B") };
    }

    #[test]
    fn test_load_env_file_existing_wins() {
        unsafe { std::env::set_var("TEST_ENV_EXIST", "original") };

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.env");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "TEST_ENV_EXIST=overwritten").unwrap();

        load_env_file(&path).unwrap();

        assert_eq!(std::env::var("TEST_ENV_EXIST").unwrap(), "original");
        unsafe { std::env::remove_var("TEST_ENV_EXIST") };
    }
}
