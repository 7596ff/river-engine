//! Model directory scanning

use super::gguf::{parse_gguf, GgufMetadata};
use std::collections::HashSet;
use std::path::PathBuf;

/// A local GGUF model
#[derive(Debug, Clone)]
pub struct LocalModel {
    pub id: String,
    pub path: PathBuf,
    pub metadata: GgufMetadata,
}

/// Scanner for discovering GGUF models in directories
pub struct ModelScanner {
    model_dirs: Vec<PathBuf>,
}

impl ModelScanner {
    pub fn new(model_dirs: Vec<PathBuf>) -> Self {
        Self { model_dirs }
    }

    /// Scan all directories and return discovered models
    /// - Skip non-existent directories with warning log
    /// - Only process .gguf files
    /// - Track seen IDs to skip duplicates (first wins, log warning)
    /// - Skip files that fail to parse (log warning)
    pub fn scan(&self) -> Vec<LocalModel> {
        let mut models = Vec::new();
        let mut seen_ids = HashSet::new();

        for dir in &self.model_dirs {
            // Skip non-existent directories
            if !dir.exists() {
                tracing::warn!("Model directory does not exist: {:?}", dir);
                continue;
            }

            // Read directory entries
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::warn!("Failed to read directory {:?}: {}", dir, e);
                    continue;
                }
            };

            // Process each file in the directory
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        tracing::warn!("Failed to read directory entry: {}", e);
                        continue;
                    }
                };

                let path = entry.path();

                // Only process .gguf files
                if !path.is_file() || !path.extension().map_or(false, |ext| ext == "gguf") {
                    continue;
                }

                // Generate ID from filename (without extension)
                let id = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => {
                        tracing::warn!("Failed to extract filename from path: {:?}", path);
                        continue;
                    }
                };

                // Skip duplicates (first wins)
                if seen_ids.contains(&id) {
                    tracing::warn!("Duplicate model ID '{}' found at {:?}, skipping", id, path);
                    continue;
                }

                // Parse GGUF metadata
                let metadata = match parse_gguf(&path) {
                    Ok(metadata) => metadata,
                    Err(e) => {
                        tracing::warn!("Failed to parse GGUF file {:?}: {}", path, e);
                        continue;
                    }
                };

                // Add to results
                seen_ids.insert(id.clone());
                models.push(LocalModel { id, path, metadata });
            }
        }

        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scanner_empty_dirs() {
        let scanner = ModelScanner::new(vec![]);
        let models = scanner.scan();
        assert!(models.is_empty());
    }

    #[test]
    fn test_scanner_nonexistent_dir() {
        let scanner = ModelScanner::new(vec![PathBuf::from("/nonexistent/path")]);
        let models = scanner.scan();
        assert!(models.is_empty());
    }
}
