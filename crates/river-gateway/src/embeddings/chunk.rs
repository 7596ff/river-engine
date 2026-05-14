//! Chunking strategies for different note types

use crate::embeddings::note::{Note, NoteType};

/// Types of chunks
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkType {
    Note,
    Move,
    Moment,
    RoomNote,
    Fragment,
}

/// A chunk ready for embedding
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: String,
    pub source_path: String,
    pub content: String,
    pub chunk_type: ChunkType,
    pub channel: Option<String>,
}

/// Chunker that splits notes into embeddable pieces
pub struct Chunker {
    max_chunk_tokens: usize,
}

impl Chunker {
    pub fn new(max_chunk_tokens: usize) -> Self {
        Self { max_chunk_tokens }
    }

    /// Chunk a parsed note
    pub fn chunk(&self, note: &Note) -> Vec<Chunk> {
        let chunk_type = match note.frontmatter.note_type {
            NoteType::Note => ChunkType::Note,
            NoteType::Move => ChunkType::Move,
            NoteType::Moment => ChunkType::Moment,
            NoteType::RoomNote => ChunkType::RoomNote,
        };

        let max_chars = self.max_chunk_tokens * 4;

        if note.content.len() <= max_chars {
            return vec![Chunk {
                id: format!("{}:0", note.source_path),
                source_path: note.source_path.clone(),
                content: note.content.clone(),
                chunk_type,
                channel: note.frontmatter.channel.clone(),
            }];
        }

        self.split_by_sections(
            &note.source_path,
            &note.content,
            chunk_type,
            note.frontmatter.channel.clone(),
            max_chars,
        )
    }

    /// Chunk a raw markdown file (no frontmatter)
    pub fn chunk_raw(&self, path: &str, content: &str) -> Vec<Chunk> {
        let max_chars = self.max_chunk_tokens * 4;
        if content.len() <= max_chars {
            return vec![Chunk {
                id: format!("{}:0", path),
                source_path: path.to_string(),
                content: content.to_string(),
                chunk_type: ChunkType::Fragment,
                channel: None,
            }];
        }
        self.split_by_sections(path, content, ChunkType::Fragment, None, max_chars)
    }

    fn split_by_sections(
        &self,
        path: &str,
        content: &str,
        chunk_type: ChunkType,
        channel: Option<String>,
        max_chars: usize,
    ) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut idx = 0;

        for line in content.lines() {
            if line.starts_with('#') && current.len() > max_chars / 2 {
                if !current.trim().is_empty() {
                    chunks.push(Chunk {
                        id: format!("{}:{}", path, idx),
                        source_path: path.to_string(),
                        content: current.trim().to_string(),
                        chunk_type: chunk_type.clone(),
                        channel: channel.clone(),
                    });
                    idx += 1;
                    current = String::new();
                }
            }
            current.push_str(line);
            current.push('\n');

            if current.len() >= max_chars {
                chunks.push(Chunk {
                    id: format!("{}:{}", path, idx),
                    source_path: path.to_string(),
                    content: current.trim().to_string(),
                    chunk_type: chunk_type.clone(),
                    channel: channel.clone(),
                });
                idx += 1;
                current = String::new();
            }
        }

        if !current.trim().is_empty() {
            chunks.push(Chunk {
                id: format!("{}:{}", path, idx),
                source_path: path.to_string(),
                content: current.trim().to_string(),
                chunk_type: chunk_type.clone(),
                channel: channel.clone(),
            });
        }

        chunks
    }
}

impl Default for Chunker {
    fn default() -> Self {
        Self::new(400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_content_single_chunk() {
        let chunker = Chunker::default();
        let chunks = chunker.chunk_raw("test.md", "Small content");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].id, "test.md:0");
    }

    #[test]
    fn test_large_content_multiple_chunks() {
        let chunker = Chunker::new(10); // Very small for testing (max_chars = 40)
                                        // Create multiline content that exceeds max_chars
        let content = (0..10)
            .map(|i| format!("Line {} with some text", i))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunker.chunk_raw("test.md", &content);
        assert!(
            chunks.len() > 1,
            "Expected multiple chunks, got {}",
            chunks.len()
        );
    }
}
