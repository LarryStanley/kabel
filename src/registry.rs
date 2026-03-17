use std::fmt;

use chrono::Utc;
use uuid::Uuid;

use crate::output::AgentInfo;
use crate::storage::KabelStorage;
use crate::validate::{self, ValidationError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum RegistryError {
    Validation(ValidationError),
    Io(std::io::Error),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Validation(e) => write!(f, "validation error: {e}"),
            RegistryError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<std::io::Error> for RegistryError {
    fn from(e: std::io::Error) -> Self {
        RegistryError::Io(e)
    }
}

impl From<ValidationError> for RegistryError {
    fn from(e: ValidationError) -> Self {
        RegistryError::Validation(e)
    }
}

// ---------------------------------------------------------------------------
// Registry — keyed by agent name (not session_id)
// ---------------------------------------------------------------------------

pub struct Registry {
    storage: KabelStorage,
}

impl Registry {
    pub fn new(storage: KabelStorage) -> Self {
        Self { storage }
    }

    /// Register (or re-register) an agent by name.
    /// If the name already exists, updates session_id/status/tty/cwd/last_seen_at.
    /// Registry files are keyed by name: `registry/{name}.json`.
    pub fn register(&self, name: &str, role: Option<&str>) -> Result<AgentInfo, RegistryError> {
        validate::validate_name(name)?;

        let session_id = resolve_session_id().unwrap_or_else(|| Uuid::new_v4().to_string());
        let tty = detect_tty();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let now = Utc::now();

        // If agent already exists, preserve registered_at and merge role
        let (registered_at, effective_role) = match self.storage.read_registry(name) {
            Ok(existing) => {
                let existing_info: Result<AgentInfo, _> = serde_json::from_value(existing);
                match existing_info {
                    Ok(prev) => (
                        prev.registered_at,
                        role.map(|r| r.to_string()).unwrap_or(prev.role),
                    ),
                    Err(_) => (now, role.unwrap_or_default().to_string()),
                }
            }
            Err(_) => (now, role.unwrap_or_default().to_string()),
        };

        let info = AgentInfo {
            session_id,
            name: name.to_string(),
            role: effective_role,
            status: "online".to_string(),
            tty,
            cwd,
            registered_at,
            last_seen_at: now,
        };

        self.register_with_info(&info)?;
        Ok(info)
    }

    /// Register an agent using a pre-built AgentInfo (useful for testing).
    /// Writes to `registry/{name}.json`.
    pub fn register_with_info(&self, info: &AgentInfo) -> Result<(), RegistryError> {
        let data = serde_json::to_value(info)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.storage.write_registry(&info.name, &data)?;
        Ok(())
    }

    /// Mark agent as offline by session_id. Does NOT delete the registry file.
    /// Scans registry to find the agent with matching session_id.
    pub fn unregister(&self, session_id: &str) -> Result<(), RegistryError> {
        let agents = self.discover()?;
        for mut agent in agents {
            if agent.session_id == session_id {
                agent.status = "offline".to_string();
                agent.last_seen_at = Utc::now();
                self.register_with_info(&agent)?;
                return Ok(());
            }
        }
        // Agent not found — not an error (may have been cleaned up)
        Ok(())
    }

    /// List all registered agents (both online and offline).
    pub fn discover(&self) -> Result<Vec<AgentInfo>, RegistryError> {
        let entries = self.storage.list_registry()?;
        let mut agents = Vec::new();
        for value in entries {
            match serde_json::from_value(value) {
                Ok(agent) => agents.push(agent),
                Err(e) => {
                    eprintln!("kabel: warning: skipping corrupted registry entry: {e}");
                }
            }
        }
        Ok(agents)
    }

    /// Find an agent by name.
    pub fn find_by_name(&self, name: &str) -> Result<Option<AgentInfo>, RegistryError> {
        match self.storage.read_registry(name) {
            Ok(value) => {
                let agent: AgentInfo = serde_json::from_value(value)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Some(agent))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Find agent name by session_id (for hooks that only know session_id).
    pub fn find_name_by_session_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, RegistryError> {
        let agents = self.discover()?;
        Ok(agents
            .into_iter()
            .find(|a| a.session_id == session_id)
            .map(|a| a.name))
    }
}

/// Resolve session ID from environment variables.
/// Checks CLAUDE_SESSION_ID first, then OPENCODE_SESSION_ID.
pub fn resolve_session_id() -> Option<String> {
    std::env::var("CLAUDE_SESSION_ID")
        .or_else(|_| std::env::var("OPENCODE_SESSION_ID"))
        .ok()
}

/// Detect the current TTY, returning "unknown" if unavailable.
fn detect_tty() -> String {
    #[cfg(unix)]
    {
        if let Some(name) = get_tty_name() {
            return name;
        }
    }
    "unknown".to_string()
}

#[cfg(unix)]
fn get_tty_name() -> Option<String> {
    std::process::Command::new("tty")
        .stdin(std::process::Stdio::inherit())
        .output()
        .ok()
        .and_then(|out| {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.starts_with('/') {
                Some(s)
            } else {
                None
            }
        })
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn test_registry() -> (Registry, TempDir) {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let storage = KabelStorage::with_base_dir(tmp.path().to_path_buf());
        let registry = Registry::new(storage);
        (registry, tmp)
    }

    fn sample_agent(name: &str, session_id: &str) -> AgentInfo {
        AgentInfo {
            session_id: session_id.to_string(),
            name: name.to_string(),
            role: String::new(),
            status: "online".to_string(),
            tty: "/dev/ttys001".to_string(),
            cwd: "/tmp/test".to_string(),
            registered_at: Utc::now(),
            last_seen_at: Utc::now(),
        }
    }

    #[test]
    fn register_agent_appears_in_discover() {
        let (registry, _tmp) = test_registry();
        let info = sample_agent("worker1", "sess-100");
        registry.register_with_info(&info).unwrap();

        let agents = registry.discover().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "worker1");
        assert_eq!(agents[0].status, "online");
    }

    #[test]
    fn register_invalid_name_returns_validation_error() {
        let (registry, _tmp) = test_registry();
        let result = registry.register("bad/name", None);
        assert!(result.is_err());
        match result.unwrap_err() {
            RegistryError::Validation(ValidationError::InvalidName(_)) => {}
            other => panic!("expected ValidationError::InvalidName, got: {other:?}"),
        }
    }

    #[test]
    fn unregister_marks_offline_not_delete() {
        let (registry, _tmp) = test_registry();
        let info = sample_agent("lead", "sess-200");
        registry.register_with_info(&info).unwrap();

        // Unregister by session_id
        registry.unregister("sess-200").unwrap();

        // Agent still exists but is offline
        let agents = registry.discover().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "lead");
        assert_eq!(agents[0].status, "offline");
    }

    #[test]
    fn re_register_updates_session_and_goes_online() {
        let (registry, _tmp) = test_registry();

        // Register with old session
        let mut info = sample_agent("worker1", "old-session");
        info.role = "backend".to_string();
        registry.register_with_info(&info).unwrap();

        // Mark offline
        registry.unregister("old-session").unwrap();
        assert_eq!(registry.discover().unwrap()[0].status, "offline");

        // Re-register with new session (simulating new Claude Code session)
        let mut new_info = sample_agent("worker1", "new-session");
        new_info.role = "backend".to_string();
        new_info.status = "online".to_string();
        registry.register_with_info(&new_info).unwrap();

        let agents = registry.discover().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].session_id, "new-session");
        assert_eq!(agents[0].status, "online");
        assert_eq!(agents[0].role, "backend");
    }

    #[test]
    fn discover_empty_returns_empty() {
        let (registry, _tmp) = test_registry();
        assert!(registry.discover().unwrap().is_empty());
    }

    #[test]
    fn find_by_name_returns_correct_agent() {
        let (registry, _tmp) = test_registry();
        registry
            .register_with_info(&sample_agent("alpha", "s1"))
            .unwrap();
        registry
            .register_with_info(&sample_agent("beta", "s2"))
            .unwrap();

        let found = registry.find_by_name("beta").unwrap().unwrap();
        assert_eq!(found.name, "beta");
        assert_eq!(found.session_id, "s2");
    }

    #[test]
    fn find_by_name_nonexistent_returns_none() {
        let (registry, _tmp) = test_registry();
        registry
            .register_with_info(&sample_agent("alpha", "s1"))
            .unwrap();
        assert!(registry.find_by_name("ghost").unwrap().is_none());
    }

    #[test]
    fn find_name_by_session_id_works() {
        let (registry, _tmp) = test_registry();
        registry
            .register_with_info(&sample_agent("worker1", "sess-abc"))
            .unwrap();
        registry
            .register_with_info(&sample_agent("worker2", "sess-def"))
            .unwrap();

        assert_eq!(
            registry.find_name_by_session_id("sess-abc").unwrap(),
            Some("worker1".to_string())
        );
        assert_eq!(
            registry.find_name_by_session_id("sess-def").unwrap(),
            Some("worker2".to_string())
        );
        assert_eq!(registry.find_name_by_session_id("unknown").unwrap(), None);
    }

    #[test]
    fn multiple_agents_all_appear() {
        let (registry, _tmp) = test_registry();
        registry
            .register_with_info(&sample_agent("a", "s1"))
            .unwrap();
        registry
            .register_with_info(&sample_agent("b", "s2"))
            .unwrap();
        registry
            .register_with_info(&sample_agent("c", "s3"))
            .unwrap();

        let mut agents = registry.discover().unwrap();
        agents.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].name, "a");
        assert_eq!(agents[1].name, "b");
        assert_eq!(agents[2].name, "c");
    }
}
