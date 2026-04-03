//! Markdown-aware chunking.

/// Configuration for chunking.
pub struct ChunkConfig {
    /// Maximum tokens per chunk (~400 tokens).
    pub max_tokens: usize,
    /// Lines of overlap between chunks.
    pub overlap_lines: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_tokens: 400,
            overlap_lines: 2,
        }
    }
}

/// A chunk of text with source information.
#[derive(Debug, Clone)]
pub struct TextChunk {
    pub text: String,
    pub line_start: usize,
    pub line_end: usize,
}

/// Chunk markdown content into smaller pieces.
pub fn chunk_markdown(content: &str, config: &ChunkConfig) -> Vec<TextChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return vec![];
    }

    let mut chunks = Vec::new();
    let mut current_chunk_lines: Vec<&str> = Vec::new();
    let mut current_start = 1; // 1-indexed
    let mut current_tokens = 0;

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let line_tokens = estimate_tokens(line);

        // Check if this is a header
        let is_header = line.starts_with('#');

        // If we hit a top-level header and have content, start a new chunk
        if is_header && !current_chunk_lines.is_empty() {
            chunks.push(TextChunk {
                text: current_chunk_lines.join("\n"),
                line_start: current_start,
                line_end: line_num - 1,
            });

            // Add overlap from previous chunk
            let overlap_start = current_chunk_lines.len().saturating_sub(config.overlap_lines);
            current_chunk_lines = current_chunk_lines[overlap_start..].to_vec();
            current_start = line_num.saturating_sub(config.overlap_lines);
            current_tokens = current_chunk_lines.iter().map(|l| estimate_tokens(l)).sum();
        }

        // Check if adding this line would exceed max tokens
        if current_tokens + line_tokens > config.max_tokens && !current_chunk_lines.is_empty() {
            chunks.push(TextChunk {
                text: current_chunk_lines.join("\n"),
                line_start: current_start,
                line_end: line_num - 1,
            });

            // Add overlap
            let overlap_start = current_chunk_lines.len().saturating_sub(config.overlap_lines);
            current_chunk_lines = current_chunk_lines[overlap_start..].to_vec();
            current_start = line_num.saturating_sub(config.overlap_lines);
            current_tokens = current_chunk_lines.iter().map(|l| estimate_tokens(l)).sum();
        }

        current_chunk_lines.push(line);
        current_tokens += line_tokens;
    }

    // Add final chunk
    if !current_chunk_lines.is_empty() {
        chunks.push(TextChunk {
            text: current_chunk_lines.join("\n"),
            line_start: current_start,
            line_end: lines.len(),
        });
    }

    chunks
}

/// Estimate tokens in a string (rough approximation: ~4 chars per token).
fn estimate_tokens(s: &str) -> usize {
    (s.len() + 3) / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_simple() {
        let content = "# Header\n\nSome text here.\n\nMore text.";
        let config = ChunkConfig::default();
        let chunks = chunk_markdown(content, &config);

        assert!(!chunks.is_empty());
        assert!(chunks[0].text.contains("Header"));
    }

    #[test]
    fn test_chunk_preserves_lines() {
        let content = "Line 1\nLine 2\nLine 3";
        let config = ChunkConfig::default();
        let chunks = chunk_markdown(content, &config);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[0].line_end, 3);
    }
}
