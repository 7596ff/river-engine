//! File watcher and sync service for embeddings

use crate::embeddings::{Chunker, Note, VectorStore};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Mock embedding function - will be replaced with real embedding client
async fn embed_text(_content: &str) -> Result<Vec<f32>, String> {
    // Return a simple mock embedding for now
    // In production, this would call an embedding API
    Ok(vec![0.1, 0.2, 0.3, 0.4])
}

pub struct SyncService {
    embeddings_dir: PathBuf,
    store: VectorStore,
    chunker: Chunker,
}

impl SyncService {
    pub fn new(embeddings_dir: PathBuf, store: VectorStore) -> Self {
        Self {
            embeddings_dir,
            store,
            chunker: Chunker::default(),
        }
    }

    /// Full sync: scan all files in embeddings dir, sync each
    pub async fn full_sync(&self) -> Result<SyncStats, String> {
        let mut stats = SyncStats::default();

        let files = self.list_markdown_files()?;
        for path in files {
            match self.sync_file(&path).await {
                Ok(changed) => {
                    if changed {
                        stats.updated += 1;
                    } else {
                        stats.skipped += 1;
                    }
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to sync file");
                    stats.errors += 1;
                }
            }
        }

        Ok(stats)
    }

    /// Sync a single file: hash, diff, chunk, embed, store
    pub async fn sync_file(&self, path: &Path) -> Result<bool, String> {
        let rel_path = path
            .strip_prefix(&self.embeddings_dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", rel_path, e))?;

        // Hash content
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

        // Check if unchanged
        if let Ok(Some(existing_hash)) = self.store.get_hash(&rel_path) {
            if existing_hash == hash {
                return Ok(false); // No change
            }
        }

        // Delete old chunks for this file
        self.store.delete_source(&rel_path)?;

        // Parse and chunk
        let chunks = if let Ok(note) = Note::parse(&rel_path, &content) {
            self.chunker.chunk(&note)
        } else {
            self.chunker.chunk_raw(&rel_path, &content)
        };

        // Embed and store each chunk
        for chunk in &chunks {
            let embedding = embed_text(&chunk.content).await?;

            self.store.upsert_chunk(
                &chunk.id,
                &chunk.source_path,
                &chunk.content,
                &format!("{:?}", chunk.chunk_type),
                chunk.channel.as_deref(),
                &hash,
                &embedding,
            )?;
        }

        tracing::info!(path = %rel_path, chunks = chunks.len(), "Synced file");
        Ok(true)
    }

    fn list_markdown_files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        Self::walk_dir(&self.embeddings_dir, &mut files)?;
        Ok(files)
    }

    fn walk_dir(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                Self::walk_dir(&path, files)?;
            } else if path.extension().map_or(false, |ext| ext == "md") {
                files.push(path);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SyncStats {
    pub updated: usize,
    pub skipped: usize,
    pub errors: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_sync_file() {
        let temp = TempDir::new().unwrap();
        let note_path = temp.path().join("test.md");

        let content = r#"---
id: "test-001"
created: 2026-03-23T12:00:00Z
author: agent
type: note
tags: []
---

# Test Note
This is test content."#;

        std::fs::write(&note_path, content).unwrap();

        let store = VectorStore::open_in_memory().unwrap();
        let sync = SyncService::new(temp.path().to_path_buf(), store);

        // First sync should update
        let changed = sync.sync_file(&note_path).await.unwrap();
        assert!(changed);

        // Second sync should skip (no change)
        let changed = sync.sync_file(&note_path).await.unwrap();
        assert!(!changed);
    }

    #[tokio::test]
    async fn test_full_sync() {
        let temp = TempDir::new().unwrap();

        // Create some test files
        for i in 0..3 {
            let path = temp.path().join(format!("note{}.md", i));
            let content = format!(
                r#"---
id: "note-{}"
created: 2026-03-23T12:00:00Z
author: agent
type: note
tags: []
---

Content {}"#,
                i, i
            );
            std::fs::write(&path, content).unwrap();
        }

        let store = VectorStore::open_in_memory().unwrap();
        let sync = SyncService::new(temp.path().to_path_buf(), store);

        let stats = sync.full_sync().await.unwrap();
        assert_eq!(stats.updated, 3);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.errors, 0);
    }
}
