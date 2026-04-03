# river-embed Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: Critical

## Summary

river-embed has fundamental performance issues - it uses O(n) full table scan instead of sqlite-vec for vector search. Also has async blocking issues with std::sync::Mutex and a cursor offset bug. Estimated effort: 3-4 days.

## Critical Issues

### Issue 1: No sqlite-vec integration (O(n) search)

- **Source:** Both reviews
- **Problem:** Spec requires sqlite-vec virtual table for efficient vector search. Implementation stores vectors as BLOBs and does full table scan with Rust-side cosine similarity calculation.
- **Fix:**
  1. Add `sqlite-vec = "0.1"` dependency
  2. Load extension at connection: `conn.load_extension("vec0", None)?`
  3. Create virtual table:
     ```sql
     CREATE VIRTUAL TABLE chunks_vec USING vec0(
         id TEXT PRIMARY KEY,
         embedding FLOAT[{dimensions}]
     );
     ```
  4. Use vec0 for similarity search instead of loading all chunks
- **Files:** `crates/river-embed/Cargo.toml`, `crates/river-embed/src/store.rs`
- **Tests:** Search performance test with 1000+ chunks

### Issue 2: std::sync::Mutex blocks async runtime

- **Source:** Both reviews
- **Problem:** `AppState` uses `std::sync::Mutex<Store>`, and handlers call `.lock().unwrap()` which blocks tokio worker threads.
- **Fix:** Either:
  - Use `tokio::sync::Mutex` for async-aware locking, OR
  - Use `spawn_blocking` for database operations
- **Files:** `crates/river-embed/src/http.rs`
- **Tests:** Concurrent request test

### Issue 3: Cursor offset bug (duplicate first result)

- **Source:** Brutal review
- **Problem:** `/search` returns first result, creates cursor with offset=0. First `/next` call gets offset=0 (same result). Should start at offset=1.
- **Fix:** Initialize cursor with `offset: 1` after returning first result from `/search`
- **Files:** `crates/river-embed/src/search.rs`
- **Tests:** Test that /next returns different result than /search

### Issue 4: Foreign keys not enabled

- **Source:** First review
- **Problem:** SQLite doesn't enforce foreign keys by default. `ON DELETE CASCADE` won't work.
- **Fix:** Add `conn.execute("PRAGMA foreign_keys = ON", [])?` after opening connection
- **Files:** `crates/river-embed/src/store.rs`
- **Tests:** Test that deleting source cascades to chunks

## Important Issues

### Issue 5: Chunking algorithm incomplete

- **Source:** Both reviews
- **Problem:** Spec requires 3-level chunking (headers → paragraphs → sentences). Implementation only splits on headers and token limit.
- **Fix:** Implement paragraph splitting (`\n\n`) and sentence splitting for oversized chunks
- **Files:** `crates/river-embed/src/chunk.rs`
- **Tests:** Test with large paragraphs that need splitting

### Issue 6: No cursor cleanup

- **Source:** Both reviews
- **Problem:** Expired cursors only removed on access. Unused cursors accumulate forever.
- **Fix:** Add background task to periodically clean expired cursors:
  ```rust
  tokio::spawn(async move {
      loop {
          tokio::time::sleep(Duration::from_secs(60)).await;
          cursor_manager.cleanup_expired().await;
      }
  });
  ```
- **Files:** `crates/river-embed/src/search.rs`, `crates/river-embed/src/main.rs`
- **Tests:** Test that expired cursors are cleaned up

### Issue 7: No request timeout on embed client

- **Source:** First review
- **Problem:** Embedding requests can hang indefinitely
- **Fix:** Configure reqwest client with timeout:
  ```rust
  reqwest::Client::builder()
      .timeout(Duration::from_secs(30))
      .build()?
  ```
- **Files:** `crates/river-embed/src/embed.rs`
- **Tests:** Mock slow endpoint test

### Issue 8: Index logic misplaced

- **Source:** First review
- **Problem:** `index.rs` only has error types. Indexing logic is in `http.rs`.
- **Fix:** Move `index_content_async` from `http.rs` to `index.rs`
- **Files:** `crates/river-embed/src/index.rs`, `crates/river-embed/src/http.rs`
- **Tests:** Existing tests should continue to pass

## Minor Issues

### Issue 9: Dead code (Cursor.id field)

- **Source:** First review
- **Problem:** `Cursor.id` field is set but never read
- **Fix:** Remove the field or use it
- **Files:** `crates/river-embed/src/search.rs`
- **Tests:** N/A

### Issue 10: Hardcoded search limit

- **Source:** Both reviews
- **Problem:** Search always fetches top-100 results
- **Fix:** Make configurable via constant or config
- **Files:** `crates/river-embed/src/http.rs`
- **Tests:** N/A

### Issue 11: No graceful shutdown

- **Source:** First review
- **Problem:** Server has no signal handling
- **Fix:** Add `with_graceful_shutdown(shutdown_signal())`
- **Files:** `crates/river-embed/src/main.rs`
- **Tests:** Manual testing

## Spec Updates Needed

None - implementation should match spec.

## Verification Checklist

- [ ] sqlite-vec integrated and working
- [ ] Vector search is O(log n) not O(n)
- [ ] tokio::sync::Mutex or spawn_blocking used
- [ ] Cursor offset starts at 1 after /search
- [ ] Foreign keys enabled and cascade works
- [ ] Chunking handles large paragraphs
- [ ] Cursor cleanup task running
- [ ] Embed client has timeout
- [ ] Store tests added
- [ ] Integration tests pass
