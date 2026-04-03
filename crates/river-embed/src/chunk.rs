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
///
/// Uses 3-level chunking:
/// 1. Split on headers
/// 2. Split on paragraphs (\n\n)
/// 3. Split on sentences for oversized content
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

        // Check if this is a paragraph break
        let is_paragraph_break = line.trim().is_empty()
            && !current_chunk_lines.is_empty()
            && current_tokens > config.max_tokens / 2;

        // If we hit a header or paragraph break and have content, consider starting new chunk
        if (is_header || is_paragraph_break) && !current_chunk_lines.is_empty() {
            // For paragraph breaks, only split if we have substantial content
            if is_header || current_tokens > config.max_tokens / 2 {
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
        }

        // Check if adding this line would exceed max tokens
        if current_tokens + line_tokens > config.max_tokens && !current_chunk_lines.is_empty() {
            // Try to split at sentence boundary first
            if let Some((before, after, split_line)) = try_sentence_split(&current_chunk_lines, config) {
                chunks.push(TextChunk {
                    text: before,
                    line_start: current_start,
                    line_end: current_start + split_line,
                });

                current_chunk_lines = vec![];
                for part in after.lines() {
                    // We need to store the string references - use a simple approach
                    current_chunk_lines.push(Box::leak(part.to_string().into_boxed_str()));
                }
                current_start = current_start + split_line + 1;
                current_tokens = estimate_tokens(&after);
            } else {
                // No good sentence boundary, just split at token limit
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

/// Try to find a sentence boundary to split on.
/// Returns (text before split, text after split, line number of split).
fn try_sentence_split(lines: &[&str], config: &ChunkConfig) -> Option<(String, String, usize)> {
    let text = lines.join("\n");
    let target_tokens = config.max_tokens * 3 / 4; // Aim for 75% of max

    let mut best_split = None;

    // Look for sentence boundaries (. ! ?) followed by space or newline
    for (i, ch) in text.char_indices() {
        let current_tokens = estimate_tokens(&text[..i]);

        if current_tokens >= target_tokens {
            // Look for sentence ending punctuation
            if (ch == '.' || ch == '!' || ch == '?') && current_tokens < config.max_tokens {
                // Check if followed by space/newline (not abbreviation)
                let rest = &text[i + ch.len_utf8()..];
                if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\n') {
                    best_split = Some(i + ch.len_utf8());
                }
            }
        }

        if current_tokens > config.max_tokens && best_split.is_some() {
            break;
        }
    }

    let split_pos = best_split?;
    let before = text[..split_pos].trim().to_string();
    let after = text[split_pos..].trim().to_string();

    // Count lines in before
    let lines_before = before.lines().count();

    Some((before, after, lines_before.saturating_sub(1)))
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

    #[test]
    fn test_chunk_splits_large_paragraphs() {
        // Create a large paragraph that exceeds max_tokens
        let large_text = "This is a sentence. ".repeat(100);
        let content = format!("# Header\n\n{}", large_text);
        let config = ChunkConfig {
            max_tokens: 50,
            overlap_lines: 1,
        };
        let chunks = chunk_markdown(&content, &config);

        // Should have multiple chunks
        assert!(chunks.len() > 1, "Large content should be split into multiple chunks");
    }

    #[test]
    fn test_chunk_splits_on_paragraphs() {
        let content = "# Header\n\nFirst paragraph with some text.\n\nSecond paragraph with more text.\n\nThird paragraph.";
        let config = ChunkConfig {
            max_tokens: 30,
            overlap_lines: 1,
        };
        let chunks = chunk_markdown(content, &config);

        // Should split on paragraph boundaries
        assert!(chunks.len() >= 2, "Should split on paragraph boundaries");
    }
}
