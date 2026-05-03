# river-embed Code Review

> Reviewer: Claude Code Review Agent
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-02-embedding-design.md

## Executive Summary

The `river-embed` crate implements a vector search service with sqlite-vec storage. The implementation covers the core functionality specified but has **significant gaps** in spec compliance, particularly around storage (does NOT use sqlite-vec), error handling, and test coverage. The code compiles and basic tests pass, but the implementation is incomplete for production use.

**Overall Assessment: NEEDS WORK**

---

## 1. Spec Compliance Checklist

### Crate Structure

| Spec Requirement | Status | Notes |
|------------------|--------|-------|
| `main.rs` - CLI parsing, startup | PASS | Implemented correctly |
| `config.rs` - Config from orchestrator | PASS | Implemented correctly |
| `http.rs` - Axum server, all endpoints | PASS | All 5 endpoints implemented |
| `index.rs` - Indexing logic | PARTIAL | Logic is in `http.rs`, `index.rs` only has error types |
| `search.rs` - Search logic, cursor management | PASS | Implemented correctly |
| `chunk.rs` - Markdown-aware chunking | PARTIAL | Basic implementation, algorithm deviates from spec |
| `embed.rs` - Embedding client | PASS | Supports both Ollama and OpenAI formats |
| `store.rs` - sqlite-vec storage | **FAIL** | Does NOT use sqlite-vec at all |

### Dependencies (Cargo.toml)

| Spec Requirement | Status | Notes |
|------------------|--------|-------|
| `river-snowflake` | PASS | Present |
| `tokio` | PASS | Present (workspace) |
| `axum` | PASS | Present (workspace) |
| `reqwest` | PASS | Present (workspace) |
| `rusqlite` with bundled | PASS | Present, but wrong version (0.32 vs spec's 0.31) |
| `sqlite-vec = "0.1"` | **FAIL** | NOT PRESENT - critical missing dependency |
| `serde`, `serde_json` | PASS | Present |
| `clap` with derive | PASS | Present |

**Additional dependencies not in spec:** `zerocopy`, `sha2`, `rand`, `futures`, `thiserror`

### CLI

| Spec Requirement | Status | Notes |
|------------------|--------|-------|
| `--orchestrator <URL>` | PASS | Implemented |
| `--name <NAME>` with default "embed" | PASS | Implemented |
| `--port <PORT>` with default 0 | PASS | Implemented |
| `-h, --help` | PASS | Clap provides this |

**Extra CLI option:** `--db <PATH>` (default: "embed.db") - reasonable addition, not in spec

### Configuration Types

| Spec Requirement | Status | Notes |
|------------------|--------|-------|
| `EmbedConfig` struct | **FAIL** | Not defined - args used directly instead |
| `RegistrationRequest` | PASS | Implemented |
| `RegistrationResponse` | PASS | Implemented |
| `EmbedModelConfig` | PASS | Implemented correctly |

### Startup Sequence

| Step | Status | Notes |
|------|--------|-------|
| 1. Parse CLI args | PASS | Done |
| 2. Bind HTTP server | PASS | Done before registration |
| 3. Register with orchestrator | PASS | Done |
| 4. Initialize sqlite-vec database | **FAIL** | Uses plain rusqlite, not sqlite-vec |
| 5. Ready for requests | PASS | Server starts |

### HTTP API

| Endpoint | Status | Notes |
|----------|--------|-------|
| `POST /index` | PASS | Implemented |
| `DELETE /source/{path}` | PASS | Implemented with path wildcard |
| `POST /search` | PASS | Implemented |
| `POST /next` | PASS | Implemented |
| `GET /health` | PASS | Implemented |

### Storage Schema

| Requirement | Status | Notes |
|-------------|--------|-------|
| `sources` table | PASS | Correct schema |
| `chunks` table | PARTIAL | Has embedding as BLOB instead of separate vec table |
| `chunks_vec` virtual table | **FAIL** | NOT IMPLEMENTED - uses Rust-side cosine similarity |
| Foreign key with CASCADE | PARTIAL | Declared but SQLite doesn't enforce by default |

### Types

| Type | Status | Notes |
|------|--------|-------|
| `Chunk` struct | PARTIAL | `TextChunk` has different structure |
| `SearchResult` | PASS | Matches spec |
| `SearchResponse` | PASS | Matches spec |
| `Cursor` | PASS | Matches spec (has extra `total_results` field) |

### Chunking Algorithm

| Requirement | Status | Notes |
|-------------|--------|-------|
| Split on `#`, `##` headers | PARTIAL | Splits on any header `#*` |
| If section > max_tokens, split on `\n\n` | **FAIL** | Not implemented |
| If paragraph > max_tokens, split on sentences | **FAIL** | Not implemented |
| Track line numbers | PASS | Implemented |
| Add overlap from previous chunk's end | PASS | Implemented |

### Error Handling

| Requirement | Status | Notes |
|-------------|--------|-------|
| Embedding model unreachable -> 503 | PASS | Returns SERVICE_UNAVAILABLE |
| Empty content -> 400 | PASS | Returns BAD_REQUEST |
| Invalid cursor -> 404 | PASS | Returns NOT_FOUND |
| Expired cursor -> 404 with message | PASS | Returns NOT_FOUND |
| Database locked -> retry with backoff | **FAIL** | Not implemented |
| Corruption -> 500 | PARTIAL | Returns 500 but no special handling |

---

## 2. Critical Issues (Must Fix)

### 2.1 sqlite-vec Not Implemented

**File:** `/home/cassie/river-engine/crates/river-embed/src/store.rs`

The spec explicitly requires sqlite-vec for vector storage with a virtual table:

```sql
CREATE VIRTUAL TABLE chunks_vec USING vec0(
    id TEXT PRIMARY KEY,
    embedding FLOAT[{dimensions}]
);
```

**Actual implementation:** Stores vectors as raw BLOB in the `chunks` table and performs linear scan with Rust-computed cosine similarity:

```rust
// store.rs lines 183-214
let mut stmt = self.conn.prepare(
    "SELECT id, source_path, line_start, line_end, text, embedding FROM chunks",
)?;
// ... loads ALL chunks into memory and computes similarity
```

**Impact:** This is an O(n) scan for every search query. With thousands of chunks, this will be extremely slow compared to sqlite-vec's efficient vector index.

**Recommendation:** Add `sqlite-vec = "0.1"` dependency and implement proper virtual table. Load the extension at connection time:

```rust
conn.load_extension("vec0", None)?;
```

### 2.2 Foreign Key Constraints Not Enabled

**File:** `/home/cassie/river-engine/crates/river-embed/src/store.rs`

SQLite does not enforce foreign key constraints by default. The `ON DELETE CASCADE` will not work.

```rust
// Missing from init_schema:
self.conn.execute("PRAGMA foreign_keys = ON", [])?;
```

**Impact:** Deleting a source will NOT cascade delete its chunks, leading to orphaned data.

### 2.3 Index Logic Misplaced

**File:** `/home/cassie/river-engine/crates/river-embed/src/index.rs`

The spec shows `index.rs` should contain "Indexing logic (chunk, embed, store)". Instead:
- `index.rs` only contains error type definitions (35 lines)
- All indexing logic is in `http.rs` (function `index_content_async`, lines 112-178)

**Recommendation:** Move `index_content_async` to `index.rs` and export it.

### 2.4 Chunking Algorithm Incomplete

**File:** `/home/cassie/river-engine/crates/river-embed/src/chunk.rs`

The spec requires a 3-level chunking strategy:
1. Split on headers
2. If section > max_tokens, split on paragraphs (`\n\n`)
3. If paragraph > max_tokens, split on sentences

**Actual implementation:** Only splits on headers OR when token limit exceeded line-by-line. Missing:
- Paragraph-aware splitting
- Sentence-aware splitting

```rust
// chunk.rs line 45 - only checks for headers
let is_header = line.starts_with('#');
```

---

## 3. Important Issues (Should Fix)

### 3.1 Mutex Contention on Store

**File:** `/home/cassie/river-engine/crates/river-embed/src/http.rs`

The store is wrapped in `Mutex<Store>` and locked synchronously:

```rust
pub struct AppState {
    pub store: Mutex<Store>,
    // ...
}

// Usage in handlers:
let store = state.store.lock().unwrap();
```

**Issues:**
- `unwrap()` on mutex will panic if lock is poisoned
- Blocking mutex holds in async context can cause deadlocks
- Multiple short locks per request (lines 132-135, 142-145, 160-175)

**Recommendation:** Use `tokio::sync::Mutex` or better yet, use `spawn_blocking` for database operations:

```rust
tokio::task::spawn_blocking(move || {
    let store = state.store.lock().unwrap();
    store.search(&query_embedding, 100, 0)
}).await?
```

### 3.2 No Cursor Cleanup

**File:** `/home/cassie/river-engine/crates/river-embed/src/search.rs`

Expired cursors are only removed when accessed (line 73-74):

```rust
if Instant::now() > cursor.expires_at {
    cursors.remove(id);
    return None;
}
```

**Impact:** Cursors that are never accessed again accumulate in memory forever.

**Recommendation:** Add a background task to periodically clean up expired cursors.

### 3.3 Dead Code Warning

**File:** `/home/cassie/river-engine/crates/river-embed/src/search.rs`

```
warning: field `id` is never read
  --> crates/river-embed/src/search.rs:29:9
   |
28 | pub struct Cursor {
   |            ------ field in this struct
29 |     pub id: String,
```

The `Cursor.id` field is set but never used. Either remove it or use it.

### 3.4 No Request Timeout/Rate Limiting

**File:** `/home/cassie/river-engine/crates/river-embed/src/embed.rs`

The embed client has no timeout configuration:

```rust
let response = request.send().await?;
```

**Impact:** Slow embedding service can hang the entire server.

**Recommendation:** Configure reqwest client with timeout:

```rust
reqwest::Client::builder()
    .timeout(Duration::from_secs(30))
    .build()?
```

### 3.5 No Graceful Shutdown

**File:** `/home/cassie/river-engine/crates/river-embed/src/main.rs`

The server has no signal handling:

```rust
axum::serve(listener, app).await?;
```

**Recommendation:** Add graceful shutdown with signal handler:

```rust
axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

### 3.6 Hardcoded Search Limit

**File:** `/home/cassie/river-engine/crates/river-embed/src/http.rs`

Search always fetches top-100 results:

```rust
match store.search(&query_embedding, 100, 0) {
```

**Recommendation:** Make this configurable or at least use a constant.

---

## 4. Test Coverage Gaps

### Current Tests

Only 2 tests exist, both in `chunk.rs`:
- `test_chunk_simple` - Basic sanity check
- `test_chunk_preserves_lines` - Line number tracking

### Missing Test Coverage

| Component | Missing Tests |
|-----------|---------------|
| `store.rs` | No tests for any store operations |
| `embed.rs` | No tests for embed client (needs mocking) |
| `search.rs` | No tests for cursor management |
| `http.rs` | No integration tests for endpoints |
| `chunk.rs` | No tests for overlap, header splitting, token limits |

### Recommended Tests

1. **Store tests:**
   - `test_store_insert_and_search`
   - `test_store_delete_source_cascades`
   - `test_store_needs_update`
   - `test_store_dimension_mismatch`

2. **Search tests:**
   - `test_cursor_creation`
   - `test_cursor_advance`
   - `test_cursor_expiration`
   - `test_hit_to_result_format`

3. **Chunk tests:**
   - `test_chunk_splits_on_headers`
   - `test_chunk_respects_token_limit`
   - `test_chunk_overlap`
   - `test_empty_content`

4. **Integration tests:**
   - `test_index_and_search_roundtrip`
   - `test_delete_removes_chunks`

---

## 5. Documentation Gaps

### Missing Module Documentation

| File | Has doc comment | Quality |
|------|-----------------|---------|
| `main.rs` | Yes (1 line) | Minimal |
| `config.rs` | Yes (1 line) | Minimal |
| `http.rs` | Yes (1 line) | Minimal |
| `index.rs` | Yes (1 line) | Minimal |
| `search.rs` | Yes (1 line) | Minimal |
| `chunk.rs` | Yes (1 line) | Minimal |
| `embed.rs` | Yes (1 line) | Minimal |
| `store.rs` | Yes (5 lines) | Good - includes caveat about impl |

### Missing Function Documentation

Most public functions lack doc comments:
- `EmbedClient::new` - no docs
- `EmbedClient::embed` - no docs
- `EmbedClient::embed_one` - no docs
- `Store::open` - no docs
- `Store::search` - no docs
- `CursorManager::create` - no docs
- `CursorManager::advance` - no docs
- All HTTP handlers - no docs

### Missing Type Documentation

- `SearchHit` - no docs
- `IndexRequest` / `IndexResponse` - no docs
- `DeleteResponse` - no docs
- `ErrorResponse` - no docs

---

## 6. Suggestions (Nice to Have)

### 6.1 Tracing/Logging

No logging throughout the crate except basic `eprintln!` in main. Consider adding `tracing` for structured logging.

### 6.2 Metrics

No observability. Consider adding:
- Request latency histograms
- Embedding generation time
- Search query time
- Chunk count gauges

### 6.3 Batch Embedding Optimization

The `embed()` function makes concurrent individual requests:

```rust
let futures: Vec<_> = texts.iter().map(|t| self.embed_one(t)).collect();
```

Many embedding APIs support batch requests. Consider detecting batch support and using it when available.

### 6.4 Connection Pooling

Single database connection. For higher concurrency, consider connection pool with `r2d2` or `deadpool`.

### 6.5 OpenAI Request Format

The spec mentions OpenAI uses `input` field, but implementation uses Ollama's `prompt`:

```rust
// Spec says OpenAI uses:
// { "model": "...", "input": "text to embed" }

// Implementation uses Ollama format for both:
let ollama_req = OllamaRequest {
    model: &self.config.name,
    prompt: text,
};
```

This may not work with actual OpenAI API.

---

## 7. What Was Done Well

1. **Clean module separation** - Each file has a clear, single responsibility
2. **Error type design** - Custom error types with proper `From` implementations
3. **Cursor management** - Well-designed with expiration and refresh
4. **Async architecture** - Proper use of async/await patterns
5. **CLI design** - Clean argument parsing with sensible defaults
6. **Registration flow** - Correctly implements orchestrator registration
7. **Score conversion** - Correctly converts distance to similarity score
8. **Content hashing** - Properly skips re-indexing unchanged content

---

## 8. Summary of Required Changes

### Critical (Must Fix Before Merge)
1. Implement sqlite-vec storage with virtual table
2. Enable foreign key constraints
3. Move indexing logic from `http.rs` to `index.rs`
4. Complete chunking algorithm (paragraph and sentence splitting)

### Important (Should Fix Soon)
5. Fix mutex usage in async context
6. Add cursor cleanup background task
7. Remove dead `Cursor.id` field
8. Add request timeout to embed client
9. Add graceful shutdown handling
10. Make search limit configurable

### Test Coverage
11. Add store unit tests
12. Add cursor management tests
13. Add chunking edge case tests
14. Add integration tests

### Documentation
15. Add function-level documentation
16. Add type documentation
17. Improve module-level documentation

---

## Appendix: File Reference

| File | Lines | Purpose |
|------|-------|---------|
| `/home/cassie/river-engine/crates/river-embed/Cargo.toml` | 33 | Package manifest |
| `/home/cassie/river-engine/crates/river-embed/src/main.rs` | 113 | CLI and startup |
| `/home/cassie/river-engine/crates/river-embed/src/config.rs` | 36 | Configuration types |
| `/home/cassie/river-engine/crates/river-embed/src/http.rs` | 338 | HTTP server and handlers |
| `/home/cassie/river-engine/crates/river-embed/src/index.rs` | 36 | Index error types |
| `/home/cassie/river-engine/crates/river-embed/src/search.rs` | 120 | Search and cursor logic |
| `/home/cassie/river-engine/crates/river-embed/src/chunk.rs` | 123 | Markdown chunking |
| `/home/cassie/river-engine/crates/river-embed/src/embed.rs` | 124 | Embedding client |
| `/home/cassie/river-engine/crates/river-embed/src/store.rs` | 256 | SQLite storage |
