//! The .env file (wall ch. 09): plain KEY=value lines, gitignored,
//! loaded at startup. Already-set environment variables win over the
//! file.

/// Parse .env text into key/value pairs. Blank lines and `#` comments
/// are skipped; a value is everything after the first `=`, trimmed,
/// with one matching pair of surrounding quotes removed if present.
/// Malformed lines are errors — a secrets file is no place for
/// guessing.
pub fn parse(text: &str) -> Result<Vec<(String, String)>, Vec<String>> {
    let mut pairs = Vec::new();
    let mut errors = Vec::new();

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            errors.push(format!("line {}: expected KEY=value", line_no + 1));
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            errors.push(format!("line {}: invalid key {key:?}", line_no + 1));
            continue;
        }
        let value = value.trim();
        let value = strip_quotes(value);
        pairs.push((key.to_string(), value.to_string()));
    }

    if errors.is_empty() { Ok(pairs) } else { Err(errors) }
}

fn strip_quotes(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// Apply parsed pairs to this process's environment. Existing
/// variables win over the file.
pub fn apply(pairs: Vec<(String, String)>) {
    for (key, value) in pairs {
        if std::env::var_os(&key).is_none() {
            // SAFETY: called once at startup, before any threads that
            // read the environment are spawned.
            unsafe { std::env::set_var(&key, &value) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_pairs() {
        let text = "ANTHROPIC_KEY=sk-ant-123\n\n# a comment\nDISCORD_TOKEN_ADA=tok\n";
        let pairs = parse(text).unwrap();
        assert_eq!(
            pairs,
            vec![
                ("ANTHROPIC_KEY".into(), "sk-ant-123".into()),
                ("DISCORD_TOKEN_ADA".into(), "tok".into()),
            ]
        );
    }

    #[test]
    fn strips_one_pair_of_quotes() {
        let pairs = parse("A=\"with spaces\"\nB='single'\nC=\"unbalanced'").unwrap();
        assert_eq!(pairs[0].1, "with spaces");
        assert_eq!(pairs[1].1, "single");
        assert_eq!(pairs[2].1, "\"unbalanced'");
    }

    #[test]
    fn value_may_contain_equals() {
        let pairs = parse("KEY=abc=def").unwrap();
        assert_eq!(pairs[0].1, "abc=def");
    }

    #[test]
    fn malformed_lines_are_collected() {
        let errors = parse("JUSTAWORD\nBAD KEY=x\nGOOD=1").unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("line 1"));
        assert!(errors[1].contains("line 2"));
    }
}
