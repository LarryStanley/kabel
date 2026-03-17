use std::fmt;
use std::io;

use chrono::Utc;
use uuid::Uuid;

use crate::output::Message;
use crate::storage::KabelStorage;
use crate::validate::{self, ValidationError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum InboxError {
    Validation(ValidationError),
    Io(io::Error),
    Parse(String),
}

impl fmt::Display for InboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InboxError::Validation(e) => write!(f, "validation error: {e}"),
            InboxError::Io(e) => write!(f, "io error: {e}"),
            InboxError::Parse(detail) => write!(f, "parse error: {detail}"),
        }
    }
}

impl std::error::Error for InboxError {}

impl From<io::Error> for InboxError {
    fn from(e: io::Error) -> Self {
        InboxError::Io(e)
    }
}

impl From<ValidationError> for InboxError {
    fn from(e: ValidationError) -> Self {
        InboxError::Validation(e)
    }
}

// ---------------------------------------------------------------------------
// Inbox — keyed by agent name (not session_id)
// ---------------------------------------------------------------------------

pub struct Inbox {
    storage: KabelStorage,
}

impl Inbox {
    pub fn new(storage: KabelStorage) -> Self {
        Self { storage }
    }

    /// Send a message to an agent by name.
    /// Inbox file: `inbox/{to_name}.jsonl`
    pub fn send(
        &self,
        from_name: &str,
        to_name: &str,
        content: &str,
    ) -> Result<Message, InboxError> {
        validate::validate_name(from_name)?;
        validate::validate_name(to_name)?;
        validate::validate_message(content)?;

        let message = Message {
            id: Uuid::new_v4().to_string(),
            from: from_name.to_string(),
            to: to_name.to_string(),
            to_name: to_name.to_string(),
            content: content.to_string(),
            created_at: Utc::now(),
            read: false,
        };

        let value = serde_json::to_value(&message).map_err(|e| InboxError::Parse(e.to_string()))?;

        // Write to inbox/{to_name}.jsonl
        self.storage.append_inbox(to_name, &value)?;

        Ok(message)
    }

    /// Broadcast a message to all registered agents (except sender).
    pub fn broadcast(&self, from_name: &str, content: &str) -> Result<Vec<Message>, InboxError> {
        validate::validate_name(from_name)?;
        validate::validate_message(content)?;

        let agents = self.storage.list_registry()?;
        let mut sent = Vec::new();

        for agent in &agents {
            let name = agent
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| InboxError::Parse("missing name in registry entry".into()))?;

            if name == from_name {
                continue;
            }

            let msg = self.send(from_name, name, content)?;
            sent.push(msg);
        }

        Ok(sent)
    }

    /// Read inbox: get all messages for an agent by name.
    pub fn read_all(&self, name: &str) -> Result<Vec<Message>, InboxError> {
        let values = self.storage.read_inbox(name)?;
        values
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| InboxError::Parse(e.to_string())))
            .collect()
    }

    /// Check inbox: get unread messages and mark them as read.
    /// Reads from `inbox/{name}.jsonl`.
    pub fn check_inbox(&self, name: &str) -> Result<Vec<Message>, InboxError> {
        let values = self.storage.mark_inbox_read(name)?;
        values
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| InboxError::Parse(e.to_string())))
            .collect()
    }

    // ── Channel operations ──────────────────────────────────────────────

    /// Send a message to a channel. All agents can read it.
    /// If content contains @name mentions, also delivers to those agents' personal inboxes.
    pub fn send_channel(
        &self,
        from_name: &str,
        channel: &str,
        content: &str,
    ) -> Result<Message, InboxError> {
        validate::validate_name(from_name)?;
        validate::validate_name(channel)?;
        validate::validate_message(content)?;

        let message = Message {
            id: Uuid::new_v4().to_string(),
            from: from_name.to_string(),
            to: format!("#{channel}"),
            to_name: format!("#{channel}"),
            content: content.to_string(),
            created_at: Utc::now(),
            read: false,
        };

        let value = serde_json::to_value(&message).map_err(|e| InboxError::Parse(e.to_string()))?;
        self.storage.append_channel(channel, &value)?;

        // Parse @mentions and deliver to mentioned agents' personal inboxes
        for mention in parse_mentions(content) {
            if mention != from_name {
                // Create a DM notification for the mentioned agent
                let dm = Message {
                    id: Uuid::new_v4().to_string(),
                    from: from_name.to_string(),
                    to: mention.clone(),
                    to_name: mention.clone(),
                    content: format!("[#{channel}] {content}"),
                    created_at: message.created_at,
                    read: false,
                };
                let dm_value =
                    serde_json::to_value(&dm).map_err(|e| InboxError::Parse(e.to_string()))?;
                // Best-effort: don't fail the channel send if DM delivery fails
                let _ = self.storage.append_inbox(&mention, &dm_value);
            }
        }

        Ok(message)
    }

    /// Check all channels for new messages since the agent's last read cursor.
    /// Returns unread channel messages and updates cursors.
    pub fn check_channels(&self, agent_name: &str) -> Result<Vec<Message>, InboxError> {
        let channels = self.storage.list_channels()?;
        let mut unread = Vec::new();

        for channel in &channels {
            let cursor = self
                .storage
                .read_cursor(agent_name, channel)?
                .unwrap_or_default();

            let all_values = self.storage.read_channel(channel)?;
            let mut latest_ts = cursor.clone();

            for value in all_values {
                let created_at = value
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                // Skip messages we've already read
                if !cursor.is_empty() && created_at <= cursor {
                    continue;
                }

                // Skip our own messages
                let from = value
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if from == agent_name {
                    if created_at > latest_ts {
                        latest_ts = created_at;
                    }
                    continue;
                }

                if created_at > latest_ts {
                    latest_ts = created_at.clone();
                }

                match serde_json::from_value(value) {
                    Ok(msg) => unread.push(msg),
                    Err(_) => continue,
                }
            }

            // Update cursor to latest timestamp
            if latest_ts != cursor {
                self.storage.write_cursor(agent_name, channel, &latest_ts)?;
            }
        }

        Ok(unread)
    }

    /// Read all messages from a channel.
    pub fn read_channel(&self, channel: &str) -> Result<Vec<Message>, InboxError> {
        let values = self.storage.read_channel(channel)?;
        values
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| InboxError::Parse(e.to_string())))
            .collect()
    }
}

/// Parse @mentions from message content. Returns unique agent names.
/// Matches @name where name is [a-zA-Z0-9_-]+.
fn parse_mentions(content: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for word in content.split_whitespace() {
        if let Some(name) = word.strip_prefix('@') {
            // Clean trailing punctuation
            let clean: String = name
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !clean.is_empty() && seen.insert(clean.clone()) {
                mentions.push(clean);
            }
        }
    }
    mentions
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_inbox() -> (Inbox, KabelStorage, TempDir) {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let storage = KabelStorage::with_base_dir(tmp.path().to_path_buf());
        storage.ensure_dirs().unwrap();
        let inbox = Inbox::new(KabelStorage::with_base_dir(tmp.path().to_path_buf()));
        (inbox, storage, tmp)
    }

    #[test]
    fn send_creates_message_in_target_inbox_by_name() {
        let (inbox, storage, _tmp) = test_inbox();

        let msg = inbox.send("alice", "bob", "hello").unwrap();

        assert_eq!(msg.from, "alice");
        assert_eq!(msg.to, "bob");
        assert_eq!(msg.to_name, "bob");
        assert_eq!(msg.content, "hello");
        assert!(!msg.read);

        // Stored under bob's name
        let stored = storage.read_inbox("bob").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["from"], "alice");
    }

    #[test]
    fn send_invalid_from_name_returns_validation_error() {
        let (inbox, _storage, _tmp) = test_inbox();
        let result = inbox.send("bad/name", "target", "hello");
        assert!(matches!(
            result.unwrap_err(),
            InboxError::Validation(ValidationError::InvalidName(_))
        ));
    }

    #[test]
    fn send_invalid_content_returns_validation_error() {
        let (inbox, _storage, _tmp) = test_inbox();
        let result = inbox.send("alice", "bob", "bad\x00content");
        assert!(matches!(
            result.unwrap_err(),
            InboxError::Validation(ValidationError::InvalidMessage(_))
        ));
    }

    #[test]
    fn read_all_returns_all_messages() {
        let (inbox, _storage, _tmp) = test_inbox();
        inbox.send("alice", "target", "msg1").unwrap();
        inbox.send("bob", "target", "msg2").unwrap();

        let messages = inbox.read_all("target").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].from, "alice");
        assert_eq!(messages[1].from, "bob");
    }

    #[test]
    fn read_all_empty_inbox_returns_empty() {
        let (inbox, _storage, _tmp) = test_inbox();
        assert!(inbox.read_all("nobody").unwrap().is_empty());
    }

    #[test]
    fn check_inbox_returns_unread_and_marks_read() {
        let (inbox, _storage, _tmp) = test_inbox();
        inbox.send("alice", "me", "hello").unwrap();
        inbox.send("bob", "me", "world").unwrap();

        let unread = inbox.check_inbox("me").unwrap();
        assert_eq!(unread.len(), 2);
        assert!(!unread[0].read);
    }

    #[test]
    fn check_inbox_second_call_returns_empty() {
        let (inbox, _storage, _tmp) = test_inbox();
        inbox.send("alice", "me", "hi").unwrap();

        assert_eq!(inbox.check_inbox("me").unwrap().len(), 1);
        assert!(inbox.check_inbox("me").unwrap().is_empty());
    }

    #[test]
    fn broadcast_sends_to_all_except_sender() {
        let (inbox, storage, _tmp) = test_inbox();

        // Pre-populate registry (keyed by name now)
        storage
            .write_registry("agentA", &json!({"session_id":"s-a","name":"agentA","tty":"","cwd":"/","registered_at":"2026-03-16T00:00:00Z","status":"online","last_seen_at":"2026-03-16T00:00:00Z"}))
            .unwrap();
        storage
            .write_registry("agentB", &json!({"session_id":"s-b","name":"agentB","tty":"","cwd":"/","registered_at":"2026-03-16T00:00:00Z","status":"online","last_seen_at":"2026-03-16T00:00:00Z"}))
            .unwrap();
        storage
            .write_registry("agentC", &json!({"session_id":"s-c","name":"agentC","tty":"","cwd":"/","registered_at":"2026-03-16T00:00:00Z","status":"online","last_seen_at":"2026-03-16T00:00:00Z"}))
            .unwrap();

        let sent = inbox.broadcast("agentA", "broadcast msg").unwrap();
        assert_eq!(sent.len(), 2);
        for msg in &sent {
            assert_eq!(msg.from, "agentA");
            assert_ne!(msg.to, "agentA");
        }

        // Messages stored under agent names
        assert_eq!(storage.read_inbox("agentB").unwrap().len(), 1);
        assert_eq!(storage.read_inbox("agentC").unwrap().len(), 1);
        assert!(storage.read_inbox("agentA").unwrap().is_empty());
    }

    // ── parse_mentions tests ─────────────────────────────────────

    #[test]
    fn parse_mentions_extracts_names() {
        assert_eq!(parse_mentions("hello @tom please check"), vec!["tom"]);
        assert_eq!(
            parse_mentions("@alice and @bob look at this"),
            vec!["alice", "bob"]
        );
        assert_eq!(parse_mentions("no mentions here"), Vec::<String>::new());
        assert_eq!(parse_mentions("@tom, please"), vec!["tom"]); // strips comma
        assert_eq!(parse_mentions("@tom @tom"), vec!["tom"]); // dedup
    }

    // ── Channel tests ────────────────────────────────────────────

    #[test]
    fn send_channel_with_mention_delivers_dm() {
        let (inbox, storage, _tmp) = test_inbox();
        storage.ensure_dirs().unwrap();

        inbox
            .send_channel("lead", "general", "POL-144 修完 @tom 請驗證")
            .unwrap();

        // Channel has the message
        let ch_msgs = storage.read_channel("general").unwrap();
        assert_eq!(ch_msgs.len(), 1);

        // Tom also got a DM in personal inbox
        let tom_inbox = storage.read_inbox("tom").unwrap();
        assert_eq!(tom_inbox.len(), 1);
        assert_eq!(tom_inbox[0]["from"], "lead");
        assert!(tom_inbox[0]["content"]
            .as_str()
            .unwrap()
            .contains("[#general]"));
    }

    #[test]
    fn send_channel_mention_does_not_dm_sender() {
        let (inbox, storage, _tmp) = test_inbox();
        storage.ensure_dirs().unwrap();

        inbox
            .send_channel("lead", "general", "I @lead will handle this")
            .unwrap();

        // Lead should NOT get a DM to themselves
        let lead_inbox = storage.read_inbox("lead").unwrap();
        assert!(lead_inbox.is_empty());
    }

    #[test]
    fn send_channel_writes_to_channel_file() {
        let (inbox, storage, _tmp) = test_inbox();

        let msg = inbox
            .send_channel("alice", "general", "hello team")
            .unwrap();
        assert_eq!(msg.from, "alice");
        assert_eq!(msg.to, "#general");
        assert_eq!(msg.content, "hello team");

        let stored = storage.read_channel("general").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["from"], "alice");
    }

    #[test]
    fn check_channels_returns_unread_messages() {
        let (inbox, storage, _tmp) = test_inbox();
        storage.ensure_dirs().unwrap();

        inbox.send_channel("alice", "general", "msg1").unwrap();
        inbox.send_channel("bob", "general", "msg2").unwrap();

        // Charlie checks — should see both (not his own)
        let unread = inbox.check_channels("charlie").unwrap();
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0].from, "alice");
        assert_eq!(unread[1].from, "bob");

        // Charlie checks again — should be empty (cursor advanced)
        let unread2 = inbox.check_channels("charlie").unwrap();
        assert!(unread2.is_empty());
    }

    #[test]
    fn check_channels_skips_own_messages() {
        let (inbox, storage, _tmp) = test_inbox();
        storage.ensure_dirs().unwrap();

        inbox
            .send_channel("alice", "general", "from alice")
            .unwrap();
        inbox.send_channel("bob", "general", "from bob").unwrap();

        // Alice checks — should only see bob's message
        let unread = inbox.check_channels("alice").unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].from, "bob");
    }

    #[test]
    fn check_channels_new_messages_after_cursor() {
        let (inbox, storage, _tmp) = test_inbox();
        storage.ensure_dirs().unwrap();

        inbox.send_channel("alice", "general", "old msg").unwrap();

        // Bob reads
        let unread = inbox.check_channels("bob").unwrap();
        assert_eq!(unread.len(), 1);

        // Alice sends another
        inbox.send_channel("alice", "general", "new msg").unwrap();

        // Bob reads again — should only see the new one
        let unread2 = inbox.check_channels("bob").unwrap();
        assert_eq!(unread2.len(), 1);
        assert_eq!(unread2[0].content, "new msg");
    }

    #[test]
    fn offline_agent_can_receive_messages() {
        let (inbox, storage, _tmp) = test_inbox();

        // Register agent then mark offline
        storage
            .write_registry("worker1", &json!({"session_id":"s1","name":"worker1","tty":"","cwd":"/","registered_at":"2026-03-16T00:00:00Z","status":"offline","last_seen_at":"2026-03-16T00:00:00Z"}))
            .unwrap();

        // Send message to offline agent — should succeed
        let msg = inbox.send("lead", "worker1", "task for you").unwrap();
        assert_eq!(msg.to, "worker1");

        // When agent comes back, they can read it
        let messages = inbox.check_inbox("worker1").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "task for you");
    }
}
