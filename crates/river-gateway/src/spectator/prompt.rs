//! Prompt loading and template substitution

use std::path::Path;

/// Load a prompt file. Returns None if the file does not exist.
pub fn load_prompt(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Substitute `{key}` placeholders in a template with values.
pub fn substitute(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{}}}", key), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_prompt_exists() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test.md");
        std::fs::write(&path, "You are the spectator.").unwrap();

        let result = load_prompt(&path);
        assert_eq!(result, Some("You are the spectator.".to_string()));
    }

    #[test]
    fn test_load_prompt_missing() {
        let result = load_prompt(Path::new("/nonexistent/prompt.md"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_substitute_single_var() {
        let template = "Turn {turn_number} completed.";
        let result = substitute(template, &[("turn_number", "5")]);
        assert_eq!(result, "Turn 5 completed.");
    }

    #[test]
    fn test_substitute_multiple_vars() {
        let template = "Channel: {channel}, Moves: {moves}";
        let result = substitute(template, &[("channel", "general"), ("moves", "1,2,3")]);
        assert_eq!(result, "Channel: general, Moves: 1,2,3");
    }

    #[test]
    fn test_substitute_no_vars() {
        let template = "No variables here.";
        let result = substitute(template, &[]);
        assert_eq!(result, "No variables here.");
    }

    #[test]
    fn test_substitute_missing_var_left_as_is() {
        let template = "Hello {name}, your id is {id}.";
        let result = substitute(template, &[("name", "Iris")]);
        assert_eq!(result, "Hello Iris, your id is {id}.");
    }
}
