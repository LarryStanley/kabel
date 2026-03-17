use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use fs2::FileExt;
use serde_json::Value;

fn validate_session_id(session_id: &str) -> io::Result<()> {
    crate::validate::validate_name(session_id)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))
}

#[derive(Clone)]
pub struct KabelStorage {
    base_dir: PathBuf,
}

impl Default for KabelStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl KabelStorage {
    /// Create a KabelStorage.
    /// Uses `.kabel/` in the current directory (per-project isolation).
    /// Falls back to `~/.kabel/` if cwd is not writable.
    pub fn new() -> Self {
        // Check KABEL_DIR env var first
        if let Ok(dir) = std::env::var("KABEL_DIR") {
            return Self {
                base_dir: PathBuf::from(dir),
            };
        }

        // Default: .kabel/ in current working directory (per-project)
        if let Ok(cwd) = std::env::current_dir() {
            let local = cwd.join(".kabel");
            return Self { base_dir: local };
        }

        // Fallback: ~/.kabel/
        let base_dir = dirs_or_home().join(".kabel");
        Self { base_dir }
    }

    /// Create a KabelStorage with a custom base directory (useful for testing).
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Ensure all subdirectories exist.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(self.registry_dir())?;
        fs::create_dir_all(self.inbox_dir())?;
        fs::create_dir_all(self.channels_dir())?;
        fs::create_dir_all(self.cursors_dir())?;
        Ok(())
    }

    // ── Registry operations ────────────────────────────────────────────

    pub fn registry_dir(&self) -> PathBuf {
        self.base_dir.join("registry")
    }

    /// Atomically write a session JSON file into registry/.
    /// Writes to a tempfile first, then renames for crash-safety.
    pub fn write_registry(&self, session_id: &str, data: &Value) -> io::Result<()> {
        validate_session_id(session_id)?;
        self.ensure_dirs()?;
        let target = self.registry_dir().join(format!("{session_id}.json"));
        let tmp = self.registry_dir().join(format!(".{session_id}.json.tmp"));

        let serialized = serde_json::to_string_pretty(data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        fs::write(&tmp, serialized.as_bytes())?;
        fs::rename(&tmp, &target)?;
        Ok(())
    }

    /// Read a session JSON file from registry/.
    pub fn read_registry(&self, session_id: &str) -> io::Result<Value> {
        validate_session_id(session_id)?;
        let path = self.registry_dir().join(format!("{session_id}.json"));
        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Remove a session JSON file from registry/.
    pub fn remove_registry(&self, session_id: &str) -> io::Result<()> {
        validate_session_id(session_id)?;
        let path = self.registry_dir().join(format!("{session_id}.json"));
        fs::remove_file(&path)
    }

    /// List all registered sessions by reading every .json file in registry/.
    pub fn list_registry(&self) -> io::Result<Vec<Value>> {
        let dir = self.registry_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                match fs::read_to_string(&path).and_then(|content| {
                    serde_json::from_str(&content)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                }) {
                    Ok(value) => results.push(value),
                    Err(e) => {
                        eprintln!(
                            "kabel: warning: skipping corrupted registry file {:?}: {e}",
                            path.file_name().unwrap_or_default()
                        );
                    }
                }
            }
        }
        Ok(results)
    }

    // ── Inbox operations ───────────────────────────────────────────────

    pub fn inbox_dir(&self) -> PathBuf {
        self.base_dir.join("inbox")
    }

    /// Append a JSON message as a single JSONL line to the inbox file.
    /// Uses flock (exclusive lock) for safe concurrent writes.
    pub fn append_inbox(&self, session_id: &str, message: &Value) -> io::Result<()> {
        validate_session_id(session_id)?;
        self.ensure_dirs()?;
        let path = self.inbox_dir().join(format!("{session_id}.jsonl"));

        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        file.lock_exclusive()?;

        let mut writer = io::BufWriter::new(&file);
        let line = serde_json::to_string(message)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        writeln!(writer, "{line}")?;
        writer.flush()?;

        file.unlock()?;
        Ok(())
    }

    /// Read all messages from an inbox file. Returns an empty vec if the file
    /// does not exist.
    pub fn read_inbox(&self, session_id: &str) -> io::Result<Vec<Value>> {
        validate_session_id(session_id)?;
        let path = self.inbox_dir().join(format!("{session_id}.jsonl"));
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&path)?;
        let reader = io::BufReader::new(file);
        let mut messages = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            messages.push(value);
        }
        Ok(messages)
    }

    /// List all agent names that have inbox files.
    pub fn list_inbox_names(&self) -> io::Result<Vec<String>> {
        let dir = self.inbox_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        Ok(names)
    }

    /// Read all unread messages, mark every message as read, and rewrite the
    /// file. Returns only the previously-unread messages.
    /// Uses flock for safe concurrent access.
    pub fn mark_inbox_read(&self, session_id: &str) -> io::Result<Vec<Value>> {
        validate_session_id(session_id)?;
        self.ensure_dirs()?;
        let path = self.inbox_dir().join(format!("{session_id}.jsonl"));
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::OpenOptions::new().read(true).open(&path)?;

        file.lock_exclusive()?;

        // Read current contents
        let reader = io::BufReader::new(&file);
        let mut all_messages = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            all_messages.push(value);
        }

        // Collect unread messages
        let unread: Vec<Value> = all_messages
            .iter()
            .filter(|m| m.get("read") != Some(&Value::Bool(true)))
            .cloned()
            .collect();

        // Mark all as read
        let updated: Vec<Value> = all_messages
            .into_iter()
            .map(|mut m| {
                if let Some(obj) = m.as_object_mut() {
                    obj.insert("read".to_string(), Value::Bool(true));
                }
                m
            })
            .collect();

        // Atomic rewrite via tempfile + rename
        let tmp_path = self.inbox_dir().join(format!(".{session_id}.jsonl.tmp"));
        {
            let mut writer = io::BufWriter::new(fs::File::create(&tmp_path)?);
            for msg in &updated {
                let line = serde_json::to_string(msg)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                writeln!(writer, "{line}")?;
            }
            writer.flush()?;
        }
        fs::rename(&tmp_path, &path)?;

        file.unlock()?;
        Ok(unread)
    }

    // ── Identity (per-session name) ─────────────────────────────────────

    fn sessions_dir(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }

    /// Save agent name for a session: `~/.kabel/sessions/{session_id}.name`
    pub fn write_session_name(&self, session_id: &str, name: &str) -> io::Result<()> {
        validate_session_id(session_id)?;
        let dir = self.sessions_dir();
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{session_id}.name")), name)?;
        Ok(())
    }

    /// Read agent name for a session.
    pub fn read_session_name(&self, session_id: &str) -> io::Result<Option<String>> {
        validate_session_id(session_id)?;
        let path = self.sessions_dir().join(format!("{session_id}.name"));
        if !path.exists() {
            return Ok(None);
        }
        let name = fs::read_to_string(&path)?.trim().to_string();
        if name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    /// Remove session name file (on unregister).
    pub fn remove_session_name(&self, session_id: &str) -> io::Result<()> {
        validate_session_id(session_id)?;
        let path = self.sessions_dir().join(format!("{session_id}.name"));
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    // ── Channel operations ──────────────────────────────────────────────

    pub fn channels_dir(&self) -> PathBuf {
        self.base_dir.join("channels")
    }

    pub fn cursors_dir(&self) -> PathBuf {
        self.base_dir.join("cursors")
    }

    /// Append a message to a channel JSONL file.
    pub fn append_channel(&self, channel: &str, message: &Value) -> io::Result<()> {
        validate_session_id(channel)?; // reuse name validation
        self.ensure_dirs()?;
        let path = self.channels_dir().join(format!("{channel}.jsonl"));

        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        file.lock_exclusive()?;
        let mut writer = io::BufWriter::new(&file);
        let line = serde_json::to_string(message)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        writeln!(writer, "{line}")?;
        writer.flush()?;
        file.unlock()?;
        Ok(())
    }

    /// Read all messages from a channel.
    pub fn read_channel(&self, channel: &str) -> io::Result<Vec<Value>> {
        validate_session_id(channel)?;
        let path = self.channels_dir().join(format!("{channel}.jsonl"));
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = io::BufReader::new(file);
        let mut messages = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(value) => messages.push(value),
                Err(_) => continue, // skip corrupted lines
            }
        }
        Ok(messages)
    }

    /// List all channel names.
    pub fn list_channels(&self) -> io::Result<Vec<String>> {
        let dir = self.channels_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        Ok(names)
    }

    /// Read the cursor (last read timestamp) for an agent on a channel.
    pub fn read_cursor(&self, agent_name: &str, channel: &str) -> io::Result<Option<String>> {
        validate_session_id(agent_name)?;
        validate_session_id(channel)?;
        let path = self
            .cursors_dir()
            .join(agent_name)
            .join(format!("{channel}.txt"));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed))
        }
    }

    /// Write the cursor (last read timestamp) for an agent on a channel.
    pub fn write_cursor(&self, agent_name: &str, channel: &str, timestamp: &str) -> io::Result<()> {
        validate_session_id(agent_name)?;
        validate_session_id(channel)?;
        let dir = self.cursors_dir().join(agent_name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{channel}.txt")), timestamp)?;
        Ok(())
    }
}

/// Helper: return the user's home directory.
fn dirs_or_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

// ════════════════════════════════════════════════════════════════════════
//  Tests — written FIRST per TDD
// ════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    /// Helper: create a KabelStorage backed by a fresh temp directory.
    fn test_storage() -> (KabelStorage, TempDir) {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let storage = KabelStorage::with_base_dir(tmp.path().to_path_buf());
        (storage, tmp)
    }

    // 1. ensure_dirs creates registry/ and inbox/ subdirectories
    #[test]
    fn ensure_dirs_creates_subdirectories() {
        let (storage, _tmp) = test_storage();
        storage.ensure_dirs().unwrap();

        assert!(storage.registry_dir().is_dir());
        assert!(storage.inbox_dir().is_dir());
    }

    // 2. write_registry + read_registry roundtrip
    #[test]
    fn write_and_read_registry_roundtrip() {
        let (storage, _tmp) = test_storage();
        let data = json!({"agent": "coder", "pid": 1234});

        storage.write_registry("session-1", &data).unwrap();
        let read_back = storage.read_registry("session-1").unwrap();

        assert_eq!(read_back, data);
    }

    // 3. remove_registry deletes the file
    #[test]
    fn remove_registry_deletes_file() {
        let (storage, _tmp) = test_storage();
        let data = json!({"agent": "coder"});

        storage.write_registry("session-2", &data).unwrap();
        assert!(storage.registry_dir().join("session-2.json").exists());

        storage.remove_registry("session-2").unwrap();
        assert!(!storage.registry_dir().join("session-2.json").exists());
    }

    // 4. list_registry returns all registered agents
    #[test]
    fn list_registry_returns_all() {
        let (storage, _tmp) = test_storage();

        storage
            .write_registry("a", &json!({"agent": "alpha"}))
            .unwrap();
        storage
            .write_registry("b", &json!({"agent": "beta"}))
            .unwrap();

        let mut list = storage.list_registry().unwrap();
        // Sort for deterministic comparison (fs::read_dir order is unspecified)
        list.sort_by(|a, b| {
            a["agent"]
                .as_str()
                .unwrap()
                .cmp(b["agent"].as_str().unwrap())
        });

        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["agent"], "alpha");
        assert_eq!(list[1]["agent"], "beta");
    }

    // 5. list_registry on empty dir returns empty vec
    #[test]
    fn list_registry_empty_dir() {
        let (storage, _tmp) = test_storage();
        storage.ensure_dirs().unwrap();

        let list = storage.list_registry().unwrap();
        assert!(list.is_empty());
    }

    // 6. append_inbox creates file and appends message
    #[test]
    fn append_inbox_creates_and_appends() {
        let (storage, _tmp) = test_storage();
        let msg = json!({"text": "hello"});

        storage.append_inbox("sess-1", &msg).unwrap();

        let path = storage.inbox_dir().join("sess-1.jsonl");
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed, msg);
    }

    // 7. append_inbox multiple messages creates valid JSONL
    #[test]
    fn append_inbox_multiple_messages_valid_jsonl() {
        let (storage, _tmp) = test_storage();

        storage.append_inbox("sess-2", &json!({"seq": 1})).unwrap();
        storage.append_inbox("sess-2", &json!({"seq": 2})).unwrap();
        storage.append_inbox("sess-2", &json!({"seq": 3})).unwrap();

        let content = fs::read_to_string(storage.inbox_dir().join("sess-2.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);

        // Each line must be valid JSON
        for (i, line) in lines.iter().enumerate() {
            let v: Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["seq"], (i + 1) as i64);
        }
    }

    // 8. read_inbox returns all messages
    #[test]
    fn read_inbox_returns_all_messages() {
        let (storage, _tmp) = test_storage();

        storage.append_inbox("sess-3", &json!({"a": 1})).unwrap();
        storage.append_inbox("sess-3", &json!({"b": 2})).unwrap();

        let messages = storage.read_inbox("sess-3").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["a"], 1);
        assert_eq!(messages[1]["b"], 2);
    }

    // 9. read_inbox on non-existent file returns empty vec
    #[test]
    fn read_inbox_nonexistent_returns_empty() {
        let (storage, _tmp) = test_storage();
        storage.ensure_dirs().unwrap();

        let messages = storage.read_inbox("does-not-exist").unwrap();
        assert!(messages.is_empty());
    }

    // 10. mark_inbox_read returns only unread messages and marks all as read
    #[test]
    fn mark_inbox_read_returns_unread_and_marks_all() {
        let (storage, _tmp) = test_storage();

        // Append two unread messages
        storage
            .append_inbox("sess-4", &json!({"text": "msg1"}))
            .unwrap();
        storage
            .append_inbox("sess-4", &json!({"text": "msg2"}))
            .unwrap();

        // First mark_inbox_read: both are unread
        let unread = storage.mark_inbox_read("sess-4").unwrap();
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0]["text"], "msg1");
        assert_eq!(unread[1]["text"], "msg2");

        // Second mark_inbox_read: all should be read now, returns empty
        let unread2 = storage.mark_inbox_read("sess-4").unwrap();
        assert!(unread2.is_empty());

        // Verify file still has all messages, all marked read
        let all = storage.read_inbox("sess-4").unwrap();
        assert_eq!(all.len(), 2);
        for msg in &all {
            assert_eq!(msg["read"], true);
        }
    }

    // Bonus: mark_inbox_read on non-existent file returns empty vec
    #[test]
    fn mark_inbox_read_nonexistent_returns_empty() {
        let (storage, _tmp) = test_storage();
        storage.ensure_dirs().unwrap();

        let unread = storage.mark_inbox_read("no-such-session").unwrap();
        assert!(unread.is_empty());
    }

    // Bonus: mark_inbox_read with mix of read and unread
    #[test]
    fn mark_inbox_read_mixed_read_unread() {
        let (storage, _tmp) = test_storage();

        // One already-read message, one unread
        storage
            .append_inbox("sess-5", &json!({"text": "old", "read": true}))
            .unwrap();
        storage
            .append_inbox("sess-5", &json!({"text": "new"}))
            .unwrap();

        let unread = storage.mark_inbox_read("sess-5").unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0]["text"], "new");
    }

    // Session ID with path traversal characters is rejected
    #[test]
    fn session_id_path_traversal_rejected() {
        let (storage, _tmp) = test_storage();
        storage.ensure_dirs().unwrap();

        // write_registry
        assert!(storage
            .write_registry("../etc/passwd", &json!({"bad": true}))
            .is_err());
        assert!(storage
            .write_registry("foo/bar", &json!({"bad": true}))
            .is_err());
        assert!(storage
            .write_registry("foo.bar", &json!({"bad": true}))
            .is_err());

        // read_registry
        assert!(storage.read_registry("../etc/passwd").is_err());

        // remove_registry
        assert!(storage.remove_registry("../etc/passwd").is_err());

        // append_inbox
        assert!(storage
            .append_inbox("../etc/passwd", &json!({"bad": true}))
            .is_err());

        // read_inbox
        assert!(storage.read_inbox("../etc/passwd").is_err());

        // mark_inbox_read
        assert!(storage.mark_inbox_read("../etc/passwd").is_err());
    }
}
