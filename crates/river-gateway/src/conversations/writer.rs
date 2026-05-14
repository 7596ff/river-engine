//! Single-writer task for conversation files

use super::{Conversation, WriteOp};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct ConversationWriter {
    rx: mpsc::Receiver<WriteOp>,
    conversations: HashMap<PathBuf, Conversation>, // In-memory cache
}

impl ConversationWriter {
    pub fn new(rx: mpsc::Receiver<WriteOp>) -> Self {
        Self {
            rx,
            conversations: HashMap::new(),
        }
    }

    pub async fn run(&mut self) {
        while let Some(op) = self.rx.recv().await {
            let path = op.path().clone();
            let conv = self.get_or_load(&path);
            Self::apply(conv, op);
            if let Err(e) = conv.save(&path) {
                tracing::error!("Failed to save conversation {:?}: {}", path, e);
            }
        }
    }

    fn get_or_load(&mut self, path: &PathBuf) -> &mut Conversation {
        if !self.conversations.contains_key(path) {
            let conv = Conversation::load(path).unwrap_or_default();
            self.conversations.insert(path.clone(), conv);
        }
        self.conversations.get_mut(path).unwrap()
    }

    fn apply(conv: &mut Conversation, op: WriteOp) {
        match op {
            WriteOp::Message { msg, .. } => {
                conv.apply_message(msg);
            }
            WriteOp::ReactionAdd {
                message_id,
                emoji,
                user,
                ..
            } => {
                if let Some(msg) = conv.get_mut(&message_id) {
                    msg.add_reaction(&emoji, &user);
                }
            }
            WriteOp::ReactionRemove {
                message_id,
                emoji,
                user,
                ..
            } => {
                if let Some(msg) = conv.get_mut(&message_id) {
                    msg.remove_reaction(&emoji, &user);
                }
            }
            WriteOp::ReactionCount {
                message_id,
                emoji,
                count,
                ..
            } => {
                if let Some(msg) = conv.get_mut(&message_id) {
                    msg.update_reaction_count(&emoji, count);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::{Author, Message, MessageDirection};

    fn test_msg() -> Message {
        Message {
            direction: MessageDirection::Unread,
            timestamp: "2026-03-23 14:30:00".to_string(),
            id: "msg1".to_string(),
            author: Author {
                name: "alice".to_string(),
                id: "111".to_string(),
            },
            content: "test message".to_string(),
            reactions: vec![],
        }
    }

    #[test]
    fn test_conversation_apply_message_new() {
        let mut conv = Conversation::default();
        let msg = Message {
            id: "msg1".into(),
            ..test_msg()
        };
        conv.apply_message(msg);
        assert_eq!(conv.messages.len(), 1);
    }

    #[test]
    fn test_conversation_apply_message_merge_duplicate() {
        let mut conv = Conversation::default();
        let msg1 = Message {
            id: "msg1".into(),
            content: "hello".into(),
            ..test_msg()
        };
        conv.apply_message(msg1);

        let msg2 = Message {
            id: "msg1".into(),
            content: "hello edited".into(),
            ..test_msg()
        };
        conv.apply_message(msg2);

        assert_eq!(conv.messages.len(), 1); // Still 1
        assert_eq!(conv.messages[0].content, "hello edited");
    }

    #[test]
    fn test_message_add_reaction() {
        let mut msg = test_msg();
        msg.add_reaction("👍", "bob");
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].users, vec!["bob"]);

        msg.add_reaction("👍", "charlie");
        assert_eq!(msg.reactions[0].users, vec!["bob", "charlie"]);
    }

    #[test]
    fn test_message_remove_reaction() {
        let mut msg = test_msg();
        msg.add_reaction("👍", "bob");
        msg.remove_reaction("👍", "bob");
        assert!(msg.reactions.is_empty());
    }

    #[test]
    fn test_message_update_reaction_count() {
        let mut msg = test_msg();
        msg.update_reaction_count("👍", 5);
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].emoji, "👍");
        assert_eq!(msg.reactions[0].unknown_count, 5);
    }

    #[test]
    fn test_message_update_reaction_count_with_users() {
        let mut msg = test_msg();
        msg.add_reaction("👍", "bob");
        msg.add_reaction("👍", "charlie");
        msg.update_reaction_count("👍", 5);

        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].users.len(), 2);
        // 5 total - 2 users = 3 unknown
        assert_eq!(msg.reactions[0].unknown_count, 3);
    }

    #[test]
    fn test_message_merge_content() {
        let mut msg1 = test_msg();
        msg1.content = "original content".to_string();

        let msg2 = Message {
            content: "edited content".to_string(),
            ..test_msg()
        };

        msg1.merge(&msg2);
        assert_eq!(msg1.content, "edited content");
    }

    #[test]
    fn test_message_merge_reactions() {
        let mut msg1 = test_msg();
        msg1.add_reaction("👍", "bob");

        let mut msg2 = test_msg();
        msg2.add_reaction("👍", "charlie");
        msg2.add_reaction("❤️", "alice");

        msg1.merge(&msg2);

        assert_eq!(msg1.reactions.len(), 2);
        let thumbs_up = msg1.reactions.iter().find(|r| r.emoji == "👍").unwrap();
        assert_eq!(thumbs_up.users.len(), 2);
        assert!(thumbs_up.users.contains(&"bob".to_string()));
        assert!(thumbs_up.users.contains(&"charlie".to_string()));
    }

    #[test]
    fn test_message_merge_direction_escalate() {
        let mut msg1 = test_msg();
        msg1.direction = MessageDirection::Unread;

        let mut msg2 = test_msg();
        msg2.direction = MessageDirection::Read;

        msg1.merge(&msg2);
        assert_eq!(msg1.direction, MessageDirection::Read);
    }

    #[test]
    fn test_message_merge_direction_no_escalate() {
        let mut msg1 = test_msg();
        msg1.direction = MessageDirection::Read;

        let mut msg2 = test_msg();
        msg2.direction = MessageDirection::Unread;

        msg1.merge(&msg2);
        // Should stay Read, not downgrade to Unread
        assert_eq!(msg1.direction, MessageDirection::Read);
    }

    #[test]
    fn test_conversation_get_mut() {
        let mut conv = Conversation::default();
        conv.apply_message(test_msg());

        let msg = conv.get_mut("msg1");
        assert!(msg.is_some());
        assert_eq!(msg.unwrap().id, "msg1");

        let missing = conv.get_mut("missing");
        assert!(missing.is_none());
    }

    #[test]
    fn test_writer_apply_message() {
        let (tx, rx) = mpsc::channel(100);
        let _writer = ConversationWriter::new(rx);

        let path = PathBuf::from("/tmp/test_conv.txt");
        let mut conv = Conversation::default();

        let op = WriteOp::Message {
            path: path.clone(),
            msg: test_msg(),
        };

        ConversationWriter::apply(&mut conv, op);
        assert_eq!(conv.messages.len(), 1);

        drop(tx); // Clean up
    }

    #[test]
    fn test_writer_apply_reaction_add() {
        let (tx, rx) = mpsc::channel(100);
        let _writer = ConversationWriter::new(rx);

        let path = PathBuf::from("/tmp/test_conv.txt");
        let mut conv = Conversation::default();
        conv.apply_message(test_msg());

        let op = WriteOp::ReactionAdd {
            path: path.clone(),
            message_id: "msg1".to_string(),
            emoji: "👍".to_string(),
            user: "bob".to_string(),
        };

        ConversationWriter::apply(&mut conv, op);
        assert_eq!(conv.messages[0].reactions.len(), 1);

        drop(tx); // Clean up
    }

    #[test]
    fn test_writer_apply_reaction_remove() {
        let (tx, rx) = mpsc::channel(100);
        let _writer = ConversationWriter::new(rx);

        let path = PathBuf::from("/tmp/test_conv.txt");
        let mut conv = Conversation::default();
        let mut msg = test_msg();
        msg.add_reaction("👍", "bob");
        conv.apply_message(msg);

        let op = WriteOp::ReactionRemove {
            path: path.clone(),
            message_id: "msg1".to_string(),
            emoji: "👍".to_string(),
            user: "bob".to_string(),
        };

        ConversationWriter::apply(&mut conv, op);
        assert!(conv.messages[0].reactions.is_empty());

        drop(tx); // Clean up
    }

    #[test]
    fn test_writer_apply_reaction_count() {
        let (tx, rx) = mpsc::channel(100);
        let _writer = ConversationWriter::new(rx);

        let path = PathBuf::from("/tmp/test_conv.txt");
        let mut conv = Conversation::default();
        conv.apply_message(test_msg());

        let op = WriteOp::ReactionCount {
            path: path.clone(),
            message_id: "msg1".to_string(),
            emoji: "👍".to_string(),
            count: 5,
        };

        ConversationWriter::apply(&mut conv, op);
        assert_eq!(conv.messages[0].reactions.len(), 1);
        assert_eq!(conv.messages[0].reactions[0].unknown_count, 5);

        drop(tx); // Clean up
    }
}
