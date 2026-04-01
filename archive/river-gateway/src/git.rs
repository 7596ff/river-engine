//! Git integration for workspace auto-commit
//!
//! Commits workspace changes after each agent cycle with ISO 8601 timestamps.
//! Detects conflicts and reports them rather than stopping the loop.

use std::path::Path;
use std::process::Command;

/// Result of a git auto-commit attempt
#[derive(Debug)]
pub enum GitCommitResult {
    /// No changes to commit
    NoChanges,
    /// Successfully committed changes
    Committed {
        /// Files that were committed
        files: Vec<String>,
        /// Commit hash (short form)
        commit_hash: String,
    },
    /// Conflicts detected - agent should be notified
    Conflicts {
        /// Files with conflicts
        conflicting_files: Vec<String>,
    },
    /// Git operation failed
    Error(String),
}

/// Git author for the agent (acting self)
pub const AGENT_AUTHOR: &str = "agent <agent@river-engine>";

/// Git author for the spectator (observing self)
pub const SPECTATOR_AUTHOR: &str = "spectator <spectator@river-engine>";

/// Git operations for workspace management
pub struct GitOps {
    workspace: std::path::PathBuf,
}

impl GitOps {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }

    /// Check if the workspace has uncommitted changes
    pub fn has_changes(&self) -> Result<bool, String> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.workspace)
            .output()
            .map_err(|e| format!("Failed to run git status: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git status failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(!stdout.trim().is_empty())
    }

    /// Get list of changed files
    pub fn changed_files(&self) -> Result<Vec<String>, String> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.workspace)
            .output()
            .map_err(|e| format!("Failed to run git status: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git status failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                // Status format: "XY filename" where XY is 2-char status
                if line.len() > 3 {
                    line[3..].to_string()
                } else {
                    line.to_string()
                }
            })
            .collect();

        Ok(files)
    }

    /// Check for merge conflicts
    pub fn has_conflicts(&self) -> Result<Vec<String>, String> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(&self.workspace)
            .output()
            .map_err(|e| format!("Failed to run git diff: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git diff failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let conflicts: Vec<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|s| s.to_string())
            .collect();

        Ok(conflicts)
    }

    /// Commit all changes with an auto-generated message
    pub fn commit_if_changed(&self) -> GitCommitResult {
        // Check for conflicts first
        match self.has_conflicts() {
            Ok(conflicts) if !conflicts.is_empty() => {
                return GitCommitResult::Conflicts {
                    conflicting_files: conflicts,
                };
            }
            Err(e) => {
                tracing::warn!("Failed to check for conflicts: {}", e);
                // Continue anyway - commit might still work
            }
            _ => {}
        }

        // Check if there are changes
        let files = match self.changed_files() {
            Ok(f) if f.is_empty() => return GitCommitResult::NoChanges,
            Ok(f) => f,
            Err(e) => return GitCommitResult::Error(e),
        };

        // Stage all changes
        let add_output = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.workspace)
            .output();

        if let Err(e) = add_output {
            return GitCommitResult::Error(format!("Failed to run git add: {}", e));
        }

        let add_output = add_output.unwrap();
        if !add_output.status.success() {
            let stderr = String::from_utf8_lossy(&add_output.stderr);
            return GitCommitResult::Error(format!("git add failed: {}", stderr));
        }

        // Generate commit message with ISO 8601 timestamp
        let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let message = format!("auto: agent cycle at {}", timestamp);

        // Commit
        let commit_output = Command::new("git")
            .args(["commit", "-m", &message])
            .current_dir(&self.workspace)
            .output();

        if let Err(e) = commit_output {
            return GitCommitResult::Error(format!("Failed to run git commit: {}", e));
        }

        let commit_output = commit_output.unwrap();
        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            // Check if it's "nothing to commit" which is not really an error
            if stderr.contains("nothing to commit") {
                return GitCommitResult::NoChanges;
            }
            return GitCommitResult::Error(format!("git commit failed: {}", stderr));
        }

        // Get the commit hash
        let hash_output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&self.workspace)
            .output();

        let commit_hash = match hash_output {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            _ => "unknown".to_string(),
        };

        GitCommitResult::Committed { files, commit_hash }
    }

    /// Commit all changes with a specific message and author
    ///
    /// Author format: "name <email>" e.g. "agent <agent@river-engine>"
    /// Use AGENT_AUTHOR or SPECTATOR_AUTHOR constants.
    pub fn commit_as(&self, message: &str, author: &str) -> GitCommitResult {
        // Check for conflicts first
        match self.has_conflicts() {
            Ok(conflicts) if !conflicts.is_empty() => {
                return GitCommitResult::Conflicts {
                    conflicting_files: conflicts,
                };
            }
            Err(e) => {
                tracing::warn!("Failed to check for conflicts: {}", e);
            }
            _ => {}
        }

        // Check if there are changes
        let files = match self.changed_files() {
            Ok(f) if f.is_empty() => return GitCommitResult::NoChanges,
            Ok(f) => f,
            Err(e) => return GitCommitResult::Error(e),
        };

        // Stage all changes
        let add_output = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.workspace)
            .output();

        if let Err(e) = add_output {
            return GitCommitResult::Error(format!("Failed to run git add: {}", e));
        }

        let add_output = add_output.unwrap();
        if !add_output.status.success() {
            let stderr = String::from_utf8_lossy(&add_output.stderr);
            return GitCommitResult::Error(format!("git add failed: {}", stderr));
        }

        // Commit with specified author
        let commit_output = Command::new("git")
            .args(["commit", "--author", author, "-m", message])
            .current_dir(&self.workspace)
            .output();

        if let Err(e) = commit_output {
            return GitCommitResult::Error(format!("Failed to run git commit: {}", e));
        }

        let commit_output = commit_output.unwrap();
        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            if stderr.contains("nothing to commit") {
                return GitCommitResult::NoChanges;
            }
            return GitCommitResult::Error(format!("git commit failed: {}", stderr));
        }

        // Get the commit hash
        let hash_output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&self.workspace)
            .output();

        let commit_hash = match hash_output {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            _ => "unknown".to_string(),
        };

        GitCommitResult::Committed { files, commit_hash }
    }

    /// Check if the workspace is a git repository
    pub fn is_git_repo(&self) -> bool {
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&self.workspace)
            .output();

        matches!(output, Ok(o) if o.status.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_git_repo() -> TempDir {
        let dir = TempDir::new().unwrap();

        // Initialize git repo
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Configure git user for commits
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        dir
    }

    #[test]
    fn test_is_git_repo() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());
        assert!(git.is_git_repo());
    }

    #[test]
    fn test_not_git_repo() {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path());
        assert!(!git.is_git_repo());
    }

    #[test]
    fn test_no_changes() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create and commit initial file
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        assert!(!git.has_changes().unwrap());
    }

    #[test]
    fn test_has_changes() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Make a change
        fs::write(dir.path().join("test.txt"), "world").unwrap();

        assert!(git.has_changes().unwrap());
    }

    #[test]
    fn test_commit_if_changed() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Make a change
        fs::write(dir.path().join("test.txt"), "world").unwrap();

        let result = git.commit_if_changed();
        assert!(matches!(result, GitCommitResult::Committed { .. }));

        // Verify no more changes
        assert!(!git.has_changes().unwrap());
    }

    #[test]
    fn test_commit_no_changes() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // No changes - should return NoChanges
        let result = git.commit_if_changed();
        assert!(matches!(result, GitCommitResult::NoChanges));
    }

    #[test]
    fn test_changed_files() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Make changes
        fs::write(dir.path().join("test.txt"), "world").unwrap();
        fs::write(dir.path().join("new.txt"), "new file").unwrap();

        let files = git.changed_files().unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_commit_as_with_author() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Make a change
        fs::write(dir.path().join("note.md"), "# Observation").unwrap();

        // Commit as spectator
        let result = git.commit_as("observe: room note", SPECTATOR_AUTHOR);
        assert!(matches!(result, GitCommitResult::Committed { .. }));

        // Verify author in git log
        let log_output = Command::new("git")
            .args(["log", "-1", "--format=%an <%ae>"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let author = String::from_utf8_lossy(&log_output.stdout);
        assert_eq!(author.trim(), "spectator <spectator@river-engine>");
    }

    #[test]
    fn test_commit_as_agent() {
        let dir = setup_git_repo();
        let git = GitOps::new(dir.path());

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Make a change
        fs::write(dir.path().join("code.rs"), "fn main() {}").unwrap();

        // Commit as agent
        let result = git.commit_as("feat: add main function", AGENT_AUTHOR);
        assert!(matches!(result, GitCommitResult::Committed { .. }));

        // Verify author in git log
        let log_output = Command::new("git")
            .args(["log", "-1", "--format=%an <%ae>"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let author = String::from_utf8_lossy(&log_output.stdout);
        assert_eq!(author.trim(), "agent <agent@river-engine>");
    }
}
