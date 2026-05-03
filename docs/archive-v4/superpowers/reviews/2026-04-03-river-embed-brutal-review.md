# river-embed Brutal Review

> Reviewer: Claude (no subagents)
> Date: 2026-04-03
> Spec: docs/superpowers/specs/2026-04-02-embedding-design.md

## Spec Completion Assessment

### Module Structure - PASS

| Spec Requirement | Implemented | Notes |
|------------------|-------------|-------|
| main.rs | YES | |
| config.rs | YES | |
| http.rs | YES | |
| index.rs | PARTIAL | Only error types, logic in http.rs |
| search.rs | YES | |
| chunk.rs | YES | |
| embed.rs | YES | |
| store.rs | YES | |

### HTTP Endpoints - PASS

| Endpoint | Implemented | Notes |
|----------|-------------|-------|
| POST /index | YES | |
| DELETE /source/{path} | YES | |
| POST /search | YES | |
| POST /next | YES | |
| GET /health | YES | |

### Features - PARTIAL

| Feature | Implemented | Notes |
|---------|-------------|-------|
| Orchestrator registration | YES | |
| sqlite-vec storage | **NO** | Uses plain rusqlite with blob vectors |
| Cursor-based search | YES | |
| Hash-based updates | YES | |
| Header-aware chunking | PARTIAL | Splits on headers, not paragraphs/sentences |

## CRITICAL ISSUES

### 1. NO sqlite-vec integration

**Spec explicitly requires:**
```sql
CREATE VIRTUAL TABLE chunks_vec USING vec0(
    id TEXT PRIMARY KEY,
    embedding FLOAT[{dimensions}]
);
```

**Implementation:**
```rust
// Vector storage using rusqlite.
//
// This is a simplified implementation that stores vectors as blobs
// and computes cosine similarity in Rust.
```

The implementation stores vectors as BLOBs and does full-table scans:

```rust
let mut stmt = self.conn.prepare(
    "SELECT id, source_path, line_start, line_end, text, embedding FROM chunks",
)?;
```

Then computes similarity in Rust for **every single chunk**.

**Impact:**
- O(n) search instead of O(log n) with proper indexing
- Will be extremely slow with thousands of chunks
- Memory explosion loading all embeddings

**Verdict:** CRITICAL PERFORMANCE BUG. Spec violation.

### 2. Mutex around Store blocks async tasks

```rust
pub struct AppState {
    pub store: Mutex<Store>,  // std::sync::Mutex, not tokio::sync::Mutex
    ...
}
```

Used in async handlers:
```rust
async fn handle_index(...) {
    ...
    let needs_update = {
        let store = state.store.lock().unwrap();  // Blocks tokio runtime!
        store.needs_update(source, &hash)?
    };
    ...
}
```

Using `std::sync::Mutex` in async code blocks the entire tokio worker thread, severely degrading throughput under concurrent load.

**Verdict:** IMPORTANT PERFORMANCE BUG. Use `tokio::sync::Mutex` or `spawn_blocking`.

### 3. Cursor offset logic is wrong

In `handle_search`:
```rust
let first_result = hits.into_iter().next().map(...);
let cursor = state.cursor_manager.create(query_embedding, total);
```

In `handle_next`:
```rust
let Some((query_embedding, offset, remaining)) = state.cursor_manager.advance(&req.cursor);
// ...
let hits = store.search(&query_embedding, 1, offset);
```

But `advance()` returns the **current** offset then increments:
```rust
let offset = cursor.offset;
cursor.offset += 1;
```

**Problem:** First call to `/next` gets offset=0, which is the same result returned by `/search`.

The cursor starts with `offset: 0`, but we already consumed result 0 in `/search`. First `/next` should get offset=1.

**Verdict:** BUG. First `/next` returns duplicate of `/search` result.

## IMPORTANT ISSUES

### 4. No cursor cleanup/expiration

Cursors have `expires_at` checked on access, but there's no background task cleaning up expired cursors. Memory grows unbounded if clients don't fetch all results.

### 5. Chunking algorithm incomplete

**Spec says:**
> 1. Split on top-level headers (`#`, `##`)
> 2. If section > max_tokens, split on paragraphs (`\n\n`)
> 3. If paragraph > max_tokens, split on sentences

**Implementation only does:**
- Split on any header (`line.starts_with('#')`)
- Split on token limit (no paragraph/sentence awareness)

The algorithm doesn't match the spec. Large paragraphs without headers won't be split intelligently.

### 6. Token estimation is crude

```rust
fn estimate_tokens(s: &str) -> usize {
    (s.len() + 3) / 4  // ~4 chars per token
}
```

For non-ASCII content (e.g., Chinese, Japanese), this will be wildly wrong. Real tokenizers vary significantly. A proper approximation should account for whitespace and punctuation.

### 7. Registration request format differs from spec

**Spec:**
```json
{
  "endpoint": "...",
  "embed": {
    "name": "embed"
  }
}
```

**Implementation:** Matches, but spec says response should have `accepted: bool` and `model: EmbedModelConfig`. Implementation has `model: Option<EmbedModelConfig>` which is safer but diverges.

### 8. No retry on embedding failure

If the embedding model is temporarily unavailable, the index operation fails completely. No backoff/retry. Consider a retry policy.

### 9. Database not persisted in tests

No integration tests. The store tests are also missing.

## MINOR ISSUES

### 10. Manual error types instead of thiserror

`StoreError`, `EmbedError`, `IndexError` all implement Display and Error manually. `thiserror` is in dependencies but not used for these.

### 11. Unused dependency

`thiserror` is declared but not actually used anywhere in the crate.

### 12. No tracing/logging

Only `eprintln!` in main.rs. No proper logging framework integration.

### 13. Cursor ID generation is weak

```rust
fn generate_cursor_id() -> String {
    let mut rng = rand::rng();
    let hex: String = (0..8)
        .map(|_| format!("{:x}", rng.random::<u8>()))
        .collect();
    format!("emb_{}", hex)
}
```

This generates 8 hex digits = 32 bits. Collision probability is non-negligible. Consider UUID or longer random strings.

### 14. Search hardcodes limit=100

```rust
store.search(&query_embedding, 100, 0)
```

Magic number. Should be configurable.

## Code Quality Assessment

### Strengths

1. **Clean API design** - HTTP endpoints match spec well
2. **Proper error types** - Each module has its own error enum
3. **Cursor management** - TTL-based expiration is a good pattern
4. **Content hashing** - Avoids redundant re-indexing
5. **Auto-detect embedding format** - Supports Ollama and OpenAI-compatible APIs

### Weaknesses

1. **No sqlite-vec** - Fundamental spec violation
2. **Blocking mutex** - Async anti-pattern
3. **Cursor offset bug** - Returns duplicates
4. **No tests** - Only basic chunk tests
5. **Crude tokenization** - Will cause chunking issues

## Summary

| Category | Score | Notes |
|----------|-------|-------|
| Spec Completion | 50% | Missing sqlite-vec is a dealbreaker |
| Code Quality | 55% | Async blocking, offset bug |
| Performance | 20% | O(n) search is unacceptable |
| Documentation | 60% | Module docs exist, inline comments sparse |
| Testing | 20% | Only chunk.rs has tests |

### Blocking Issues

1. **No sqlite-vec** - Full table scan will not scale
2. **Cursor offset bug** - First `/next` returns duplicate
3. **std::sync::Mutex in async** - Blocks tokio threads

### Recommended Actions

1. Integrate sqlite-vec for actual vector indexing
2. Fix cursor offset to start at 1 after `/search`
3. Replace `std::sync::Mutex` with `tokio::sync::Mutex` or use `spawn_blocking`
4. Add cursor cleanup background task
5. Improve chunking to match spec (paragraphs, sentences)
6. Add integration tests for search flow
7. Use proper logging instead of eprintln
