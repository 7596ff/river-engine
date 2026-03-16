//! Database layer

mod schema;
mod messages;

pub use schema::{Database, init_db};
pub use messages::{Message, MessageRole};
