//! Context assembly with hot/warm/cold layers

use crate::embeddings::VectorStore;
use crate::flash::FlashQueue;
use crate::model::ChatMessage;
use std::path::PathBuf;

/// Token budget allocation for context layers
#[derive(Debug, Clone)]
pub struct ContextBudget {
    pub total: usize,
    pub system: usize,
    pub warm_moves: usize,
    pub warm_flashes: usize,
    pub warm_retrieved: usize,
    pub hot: usize,
    pub hot_min_messages: usize,
    pub output_reserved: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            total: 128_000,
            system: 4_000,
            warm_moves: 4_000,
            warm_flashes: 2_000,
            warm_retrieved: 8_000,
            hot: 8_192,
            hot_min_messages: 3,
            output_reserved: 8_000,
        }
    }
}

/// Assembled context ready for model call
#[derive(Debug)]
pub struct AssembledContext {
    pub messages: Vec<ChatMessage>,
    pub token_estimate: usize,
    pub layer_stats: LayerStats,
}

/// Statistics about what went into each layer
#[derive(Debug, Default)]
pub struct LayerStats {
    pub system_tokens: usize,
    pub moves_tokens: usize,
    pub flashes_count: usize,
    pub flashes_tokens: usize,
    pub retrieved_count: usize,
    pub retrieved_tokens: usize,
    pub hot_messages: usize,
    pub hot_tokens: usize,
}

/// Rough token count estimation (~4 chars per token for English)
fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() { return 0; }
    (text.len() + 3) / 4
}

/// Estimate tokens for a ChatMessage
fn message_tokens(msg: &ChatMessage) -> usize {
    let content_tokens = msg.content.as_deref().map_or(0, estimate_tokens);
    let tool_tokens = msg.tool_calls.as_ref().map_or(0, |calls| {
        calls.iter().map(|tc| estimate_tokens(&tc.function.name) + estimate_tokens(&tc.function.arguments)).sum()
    });
    content_tokens + tool_tokens + 4
}

/// Truncate text to approximately fit within a token budget
fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars {
        return text.to_string();
    }
    let truncated = &text[..max_chars];
    if let Some(last_newline) = truncated.rfind('\n') {
        truncated[..last_newline].to_string()
    } else {
        truncated.to_string()
    }
}

/// Assembles context from hot/warm/cold layers
pub struct ContextAssembler {
    budget: ContextBudget,
    embeddings_dir: PathBuf,
}

impl ContextAssembler {
    pub fn new(budget: ContextBudget, embeddings_dir: PathBuf) -> Self {
        Self { budget, embeddings_dir }
    }

    /// Assemble context for a turn
    pub async fn assemble(
        &self,
        channel: &str,
        system_prompt: &str,
        recent_messages: &[ChatMessage],
        flash_queue: &FlashQueue,
        vector_store: Option<&VectorStore>,
        query_embedding: Option<&[f32]>,
    ) -> AssembledContext {
        let mut messages = Vec::new();
        let mut stats = LayerStats::default();
        let mut total_tokens = 0;

        // 1. System layer
        let system_msg = ChatMessage::system(system_prompt);
        let sys_tokens = message_tokens(&system_msg);
        stats.system_tokens = sys_tokens;
        total_tokens += sys_tokens;
        messages.push(system_msg);

        // 2. Warm: Moves (channel-specific)
        let moves_content = self.load_moves(channel).await;
        if !moves_content.is_empty() {
            let moves_tokens = estimate_tokens(&moves_content);
            let truncated = if moves_tokens > self.budget.warm_moves {
                truncate_to_tokens(&moves_content, self.budget.warm_moves)
            } else {
                moves_content
            };
            let moves_tokens = estimate_tokens(&truncated);
            stats.moves_tokens = moves_tokens;
            total_tokens += moves_tokens;
            messages.push(ChatMessage::system(format!(
                "[Conversation arc for this channel]\n{}",
                truncated
            )));
        }

        // 3. Warm: Flashes (global)
        let flashes = flash_queue.active().await;
        if !flashes.is_empty() {
            let mut flash_content = String::new();
            let mut flash_tokens = 0;
            for flash in &flashes {
                let entry = format!("[{}]\n{}\n\n", flash.source, flash.content);
                let entry_tokens = estimate_tokens(&entry);
                if flash_tokens + entry_tokens > self.budget.warm_flashes {
                    break;
                }
                flash_content.push_str(&entry);
                flash_tokens += entry_tokens;
                stats.flashes_count += 1;
            }
            if !flash_content.is_empty() {
                stats.flashes_tokens = flash_tokens;
                total_tokens += flash_tokens;
                messages.push(ChatMessage::system(format!(
                    "[Surfaced memories]\n{}",
                    flash_content.trim()
                )));
            }
        }

        // 4. Warm: Retrieved (semantic search)
        if let (Some(store), Some(embedding)) = (vector_store, query_embedding) {
            if let Ok(results) = store.search(embedding, 10) {
                let mut retrieved_content = String::new();
                let mut retrieved_tokens = 0;
                for result in &results {
                    if result.similarity < 0.5 { break; }
                    let entry = format!("[{}] (similarity: {:.2})\n{}\n\n",
                        result.source_path, result.similarity, result.content);
                    let entry_tokens = estimate_tokens(&entry);
                    if retrieved_tokens + entry_tokens > self.budget.warm_retrieved {
                        break;
                    }
                    retrieved_content.push_str(&entry);
                    retrieved_tokens += entry_tokens;
                    stats.retrieved_count += 1;
                }
                if !retrieved_content.is_empty() {
                    stats.retrieved_tokens = retrieved_tokens;
                    total_tokens += retrieved_tokens;
                    messages.push(ChatMessage::system(format!(
                        "[Related memories]\n{}",
                        retrieved_content.trim()
                    )));
                }
            }
        }

        // 5. Hot: Recent messages (token-budgeted, minimum floor)
        let mut hot_messages = Vec::new();
        let mut hot_tokens = 0;
        for (i, msg) in recent_messages.iter().rev().enumerate() {
            let msg_tokens = message_tokens(msg);
            if i >= self.budget.hot_min_messages && hot_tokens + msg_tokens > self.budget.hot {
                break;
            }
            hot_messages.push(msg.clone());
            hot_tokens += msg_tokens;
        }
        hot_messages.reverse();
        stats.hot_messages = hot_messages.len();
        stats.hot_tokens = hot_tokens;
        total_tokens += hot_tokens;
        messages.extend(hot_messages);

        AssembledContext {
            messages,
            token_estimate: total_tokens,
            layer_stats: stats,
        }
    }

    /// Load moves file for a channel
    async fn load_moves(&self, channel: &str) -> String {
        let sanitized = channel.replace(['/', '\\', ' '], "-");
        let moves_path = self.embeddings_dir.join("moves").join(format!("{}.md", sanitized));
        tokio::fs::read_to_string(&moves_path).await.unwrap_or_default()
    }

    /// Check what changes when switching channels
    pub fn channel_switch_description(old: &str, new: &str) -> String {
        format!(
            "Channel switch: {} → {}. Moves and hot context will change. Flashes persist.",
            old, new
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flash::{Flash, FlashTTL};
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello"), 2);
        assert!(estimate_tokens(&"a".repeat(400)) <= 110);
    }

    #[test]
    fn test_truncate_to_tokens() {
        let text = "line1\nline2\nline3\nline4";
        let truncated = truncate_to_tokens(text, 3);
        assert!(truncated.len() <= 12);
    }

    #[tokio::test]
    async fn test_assemble_system_only() {
        let temp = TempDir::new().unwrap();
        let assembler = ContextAssembler::new(ContextBudget::default(), temp.path().to_path_buf());
        let flash_queue = FlashQueue::new(10);

        let result = assembler.assemble(
            "test-channel",
            "You are a helpful assistant.",
            &[],
            &flash_queue,
            None,
            None,
        ).await;

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].role, "system");
        assert!(result.layer_stats.system_tokens > 0);
    }

    #[tokio::test]
    async fn test_assemble_with_flashes() {
        let temp = TempDir::new().unwrap();
        let assembler = ContextAssembler::new(ContextBudget::default(), temp.path().to_path_buf());
        let flash_queue = FlashQueue::new(10);

        flash_queue.push(Flash {
            id: "f1".into(),
            content: "Remember to use kebab-case for CSS classes.".into(),
            source: "notes/css-conventions.md".into(),
            ttl: FlashTTL::Turns(5),
            created: Utc::now(),
        }).await;

        let result = assembler.assemble(
            "test-channel",
            "You are a helpful assistant.",
            &[],
            &flash_queue,
            None,
            None,
        ).await;

        assert_eq!(result.messages.len(), 2);
        assert!(result.messages[1].content.as_ref().unwrap().contains("Surfaced memories"));
        assert_eq!(result.layer_stats.flashes_count, 1);
    }

    #[tokio::test]
    async fn test_assemble_with_hot_messages() {
        let temp = TempDir::new().unwrap();
        let assembler = ContextAssembler::new(ContextBudget::default(), temp.path().to_path_buf());
        let flash_queue = FlashQueue::new(10);

        let recent = vec![
            ChatMessage::user("Hello!"),
            ChatMessage::assistant(Some("Hi there!".into()), None),
            ChatMessage::user("How are you?"),
        ];

        let result = assembler.assemble(
            "test-channel",
            "You are a helpful assistant.",
            &recent,
            &flash_queue,
            None,
            None,
        ).await;

        assert_eq!(result.layer_stats.hot_messages, 3);
        assert_eq!(result.messages.len(), 4); // 1 system + 3 hot
    }

    #[tokio::test]
    async fn test_assemble_with_moves() {
        let temp = TempDir::new().unwrap();
        let moves_dir = temp.path().join("moves");
        std::fs::create_dir_all(&moves_dir).unwrap();
        std::fs::write(moves_dir.join("test-channel.md"), "## Current arc\nDiscussing project setup.").unwrap();

        let assembler = ContextAssembler::new(ContextBudget::default(), temp.path().to_path_buf());
        let flash_queue = FlashQueue::new(10);

        let result = assembler.assemble(
            "test-channel",
            "You are a helpful assistant.",
            &[],
            &flash_queue,
            None,
            None,
        ).await;

        assert_eq!(result.messages.len(), 2);
        assert!(result.messages[1].content.as_ref().unwrap().contains("Conversation arc"));
        assert!(result.layer_stats.moves_tokens > 0);
    }

    #[test]
    fn test_channel_switch_description() {
        let desc = ContextAssembler::channel_switch_description("general", "dev");
        assert!(desc.contains("general"));
        assert!(desc.contains("dev"));
        assert!(desc.contains("Flashes persist"));
    }
}
