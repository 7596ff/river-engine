//! Zettelkasten embeddings layer

pub mod chunk;
pub mod note;
pub mod store;

pub use chunk::{Chunk, ChunkType, Chunker};
pub use note::{Note, NoteFrontmatter, NoteType};
pub use store::{SearchResult, VectorStore};
