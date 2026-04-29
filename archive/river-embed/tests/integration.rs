//! Integration tests for river-embed.

// Note: These tests require a running embedding service.
// For CI, consider mocking the embed client.

#[cfg(test)]
mod tests {
    // This test verifies that the /next endpoint doesn't return
    // the same result as /search (cursor offset bug fix)
    #[tokio::test]
    #[ignore] // Requires running service
    async fn test_cursor_does_not_duplicate_first_result() {
        // This would require setting up the full service
        // For now, the unit tests in search.rs cover this
    }

    // Performance test - requires many chunks
    #[tokio::test]
    #[ignore] // Expensive test
    async fn test_search_performance_with_many_chunks() {
        // Would verify O(log n) performance with sqlite-vec
    }
}
