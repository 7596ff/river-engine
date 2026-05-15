//! File watcher and sync service for embeddings

use crate::coordinator::events::{AgentEvent, CoordinatorEvent};
use crate::embeddings::{Chunker, Note, VectorStore};
use crate::memory::EmbeddingClient;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;

/// Trait for embedding text — allows mocking in tests
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

#[async_trait::async_trait]
impl Embedder for EmbeddingClient {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        EmbeddingClient::embed(self, text)
            .await
            .map_err(|e| e.to_string())
    }
}

pub struct SyncService<E: Embedder> {
    embeddings_dir: PathBuf,
    store: VectorStore,
    chunker: Chunker,
    embedder: E,
}

impl<E: Embedder> SyncService<E> {
    pub fn new(embeddings_dir: PathBuf, store: VectorStore, embedder: E) -> Self {
        Self {
            embeddings_dir,
            store,
            chunker: Chunker::default(),
            embedder,
        }
    }

    /// Run the sync service: full sync at startup, then listen for NoteWritten events
    pub async fn run(self, mut event_rx: broadcast::Receiver<CoordinatorEvent>) {
        match self.full_sync().await {
            Ok(stats) => {
                tracing::info!(
                    updated = stats.updated,
                    skipped = stats.skipped,
                    pruned = stats.pruned,
                    errors = stats.errors,
                    "Initial embedding sync complete"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Initial embedding sync failed");
            }
        }

        loop {
            match event_rx.recv().await {
                Ok(CoordinatorEvent::Agent(AgentEvent::NoteWritten { path, .. })) => {
                    let file_path = PathBuf::from(&path);
                    if file_path.exists() {
                        match self.sync_file(&file_path).await {
                            Ok(true) => {
                                tracing::info!(path = %path, "Synced file on write event")
                            }
                            Ok(false) => {
                                tracing::debug!(path = %path, "File unchanged")
                            }
                            Err(e) => tracing::error!(
                                path = %path,
                                error = %e,
                                "Failed to sync file on write event"
                            ),
                        }
                    }
                }
                Ok(CoordinatorEvent::Shutdown) => {
                    tracing::info!("Sync service shutting down");
                    break;
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "Sync service lagged, running full sync");
                    let _ = self.full_sync().await;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("Event bus closed, sync service stopping");
                    break;
                }
            }
        }
    }

    /// Full sync: scan all files, prune orphans
    pub async fn full_sync(&self) -> Result<SyncStats, String> {
        let mut stats = SyncStats::default();

        // Prune orphaned chunks
        if let Ok(sources) = self.store.list_sources() {
            for source in sources {
                let full_path = self.embeddings_dir.join(&source);
                if !full_path.exists() {
                    self.store.delete_source(&source)?;
                    stats.pruned += 1;
                    tracing::info!(source = %source, "Pruned orphaned chunks");
                }
            }
        }

        // Sync existing files
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

        if let Ok(count) = self.store.chunk_count() {
            if count > 1000 {
                tracing::warn!(
                    chunks = count,
                    "Corpus size exceeds recommended limit for brute-force search"
                );
            } else {
                tracing::info!(chunks = count, "Sync complete");
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

        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

        if let Ok(Some(existing_hash)) = self.store.get_hash(&rel_path) {
            if existing_hash == hash {
                return Ok(false);
            }
        }

        self.store.delete_source(&rel_path)?;

        let chunks = if let Ok(note) = Note::parse(&rel_path, &content) {
            self.chunker.chunk(&note)
        } else {
            self.chunker.chunk_raw(&rel_path, &content)
        };

        for chunk in &chunks {
            let embedding = self.embedder.embed(&chunk.content).await?;
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
    pub pruned: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct MockEmbedder;

    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
            Ok(vec![0.1, 0.2, 0.3, 0.4])
        }
    }

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
        let sync = SyncService::new(temp.path().to_path_buf(), store, MockEmbedder);

        let changed = sync.sync_file(&note_path).await.unwrap();
        assert!(changed);

        let changed = sync.sync_file(&note_path).await.unwrap();
        assert!(!changed);
    }

    #[tokio::test]
    async fn test_full_sync() {
        let temp = TempDir::new().unwrap();

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
        let sync = SyncService::new(temp.path().to_path_buf(), store, MockEmbedder);

        let stats = sync.full_sync().await.unwrap();
        assert_eq!(stats.updated, 3);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn test_orphan_pruning() {
        let temp = TempDir::new().unwrap();

        let note_path = temp.path().join("ephemeral.md");
        std::fs::write(&note_path, "# Ephemeral\nWill be deleted.").unwrap();

        let store = VectorStore::open_in_memory().unwrap();
        let sync = SyncService::new(temp.path().to_path_buf(), store.clone(), MockEmbedder);

        let stats = sync.full_sync().await.unwrap();
        assert_eq!(stats.updated, 1);

        std::fs::remove_file(&note_path).unwrap();

        let stats = sync.full_sync().await.unwrap();
        assert_eq!(stats.pruned, 1);
        assert_eq!(stats.updated, 0);
    }
}
