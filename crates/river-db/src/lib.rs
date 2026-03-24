//! River Database — SQLite storage layer

pub mod schema;
pub mod messages;
pub mod memories;
pub mod contexts;

pub use schema::{Database, init_db};
pub use messages::{Message, MessageRole};
pub use memories::{Memory, f32_vec_to_bytes, bytes_to_f32_vec};
pub use contexts::Context;
