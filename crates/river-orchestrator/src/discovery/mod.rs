//! Model discovery and metadata extraction

pub mod gguf;
pub mod scanner;

pub use gguf::{parse_gguf, GgufMetadata, QuantizationType};
pub use scanner::{LocalModel, ModelScanner};
