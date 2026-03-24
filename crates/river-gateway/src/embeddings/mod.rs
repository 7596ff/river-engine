//! Zettelkasten embeddings layer

pub mod chunk;
pub mod note;

pub use chunk::{Chunk, ChunkType, Chunker};
pub use note::{Note, NoteFrontmatter, NoteType};
