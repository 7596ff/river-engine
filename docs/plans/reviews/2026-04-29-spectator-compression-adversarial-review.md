# Adversarial Review: Spectator Compression Plan

## 1. Code blocks that won't compile
- **Issue:** The plan assumes `Database` is `Sync` and safe to share via `Arc<Mutex<Database>>`. `rusqlite::Connection` is `!Sync`.
  - **Plan Quote:** `db: Arc<Mutex<Database>>`
  - **Codebase Quote:** `pub struct Database { conn: Connection }`
  - **Explanation:** Passing a `std::sync::Mutex` around `await` points in `SpectatorTask` will panic or cause deadlocks. Additionally, `Database` needs `Send` implementation to be shared safely.

- **Issue:** `crates/river-gateway/Cargo.toml` is missing new dependencies.
  - **Plan Quote:** (No mention of `Cargo.toml`)
  - **Codebase Quote:** (Current `Cargo.toml` lacks `regex`, `serde_json`, `tempfile`)
  - **Explanation:** The new `parse_moment_response` and `prompt::substitute` will fail to link.

## 2. Deletion impact gaps
| Deleted Symbol | Reference File | Plan accounts for it? |
| :--- | :--- | :--- |
| `Compressor` | `crates/river-gateway/tests/iyou_test.rs` | Deletes file, but doesn't mention other potential transitive dependencies. |
| `Curator` | `crates/river-gateway/tests/iyou_test.rs` | Same as above. |
| `RoomWriter` | `crates/river-gateway/tests/iyou_test.rs` | Same as above. |

## 3. server.rs wiring gaps
- **Gap:** The plan does not provide the exact lines to remove for `VectorStore` and `FlashQueue` passing.
- **Diff:**
```rust
// Current
let spectator_task = SpectatorTask::new(
    spectator_config,
    coordinator.bus().clone(),
    spectator_model,
    Some(Arc::new(vector_store)), // REMOVE
    flash_queue.clone(),           // REMOVE
);
// Proposed by plan
let spectator_task = SpectatorTask::new(
    spectator_config,
    coordinator.bus().clone(),
    spectator_model,
    db_arc.clone(),
);
```

## 4. AgentTask gaps
- **Persistence:** The plan assumes `persist_turn_messages` can be called synchronously in `turn_cycle`. This will block the event loop if the DB is under load.

## 5. Type mismatches
- **MessageRole:** `crates/river-db/src/messages.rs` uses `MessageRole` which is an enum. The `AgentTask` code provided uses `chat_msg.role.as_str()` and manually maps it. The code in the plan assumes this mapping exists/is correct, but it must be implemented in the gateway to link `AgentTask` to `river-db`.

## 6. Missing test updates
- Integration tests in `tests/` directory (outside of `iyou_test.rs`) that rely on the old spectator configuration will fail to compile. The plan assumes only `iyou_test.rs` is affected.

## 7. Grades
- **Compilability: C**
- **Completeness: B**
- **Accuracy: B**
- **Independence: C**
