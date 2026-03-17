//! Output formatting module for kabel CLI.
//!
//! Follows Interface Segregation and Open/Closed principles:
//! - `OutputFormatter` trait defines the contract
//! - `HumanFormatter` produces human-readable output for TTY
//! - `JsonFormatter` produces NDJSON for piping / `--json` flag
//!
//! Domain types (`AgentInfo`, `Message`) live here temporarily and will be
//! refactored into a shared `models` module once registry/inbox are created.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Output mode detection
// ---------------------------------------------------------------------------

/// Whether the CLI should emit human-readable text or machine-readable JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Json,
}

/// Decide the output mode.
///
/// * `--json` flag (`force_json = true`) always selects JSON.
/// * Otherwise, JSON is used when stdout is **not** a terminal (piped).
pub fn detect_output_mode(force_json: bool) -> OutputMode {
    if force_json || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        OutputMode::Json
    } else {
        OutputMode::Human
    }
}

// ---------------------------------------------------------------------------
// Domain types (temporary home — will move to models.rs)
// ---------------------------------------------------------------------------

/// Metadata about a registered agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub session_id: String,
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default = "default_status")]
    pub status: String,
    pub tty: String,
    pub cwd: String,
    pub registered_at: DateTime<Utc>,
    #[serde(default = "chrono::Utc::now")]
    pub last_seen_at: DateTime<Utc>,
}

fn default_status() -> String {
    "online".to_string()
}

/// A single inter-agent message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub to_name: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub read: bool,
}

// ---------------------------------------------------------------------------
// Formatter trait  (Interface Segregation)
// ---------------------------------------------------------------------------

/// Unified interface for formatting CLI output.
///
/// Each method returns a `String` ready to be printed to stdout.
pub trait OutputFormatter {
    /// Format a list of registered agents.
    fn format_agents(&self, agents: &[AgentInfo]) -> String;

    /// Format a list of messages for the current agent.
    fn format_messages(&self, messages: &[Message]) -> String;

    /// Format an error with optional detail.
    fn format_error(&self, error: &str, detail: &str) -> String;

    /// Format an informational message.
    fn format_info(&self, message: &str) -> String;
}

// ---------------------------------------------------------------------------
// Factory (Dependency Inversion)
// ---------------------------------------------------------------------------

/// Create the appropriate formatter for the given output mode.
pub fn create_formatter(mode: OutputMode) -> Box<dyn OutputFormatter> {
    match mode {
        OutputMode::Human => Box::new(HumanFormatter),
        OutputMode::Json => Box::new(JsonFormatter),
    }
}

// ---------------------------------------------------------------------------
// HumanFormatter
// ---------------------------------------------------------------------------

/// Produces human-readable, table-like output for interactive terminals.
pub struct HumanFormatter;

impl OutputFormatter for HumanFormatter {
    fn format_agents(&self, agents: &[AgentInfo]) -> String {
        if agents.is_empty() {
            return String::from("No agents registered.");
        }

        // Compute column widths.
        let name_w = agents
            .iter()
            .map(|a| a.name.len())
            .max()
            .unwrap_or(4)
            .max(4);
        let status_w = 7; // "offline" is longest
        let role_w = agents
            .iter()
            .map(|a| a.role.len())
            .max()
            .unwrap_or(4)
            .max(4);
        let cwd_w = agents.iter().map(|a| a.cwd.len()).max().unwrap_or(3).max(3);

        let mut out = String::new();

        // Header
        out.push_str(&format!(
            "{:<name_w$}  {:<status_w$}  {:<role_w$}  {:<cwd_w$}",
            "NAME", "STATUS", "ROLE", "CWD",
        ));
        out.push('\n');

        // Separator
        out.push_str(&format!(
            "{:-<name_w$}  {:-<status_w$}  {:-<role_w$}  {:-<cwd_w$}",
            "", "", "", "",
        ));
        out.push('\n');

        // Rows
        for agent in agents {
            out.push_str(&format!(
                "{:<name_w$}  {:<status_w$}  {:<role_w$}  {:<cwd_w$}",
                agent.name, agent.status, agent.role, agent.cwd,
            ));
            out.push('\n');
        }

        // Trim trailing newline for a cleaner return value.
        out.truncate(out.trim_end_matches('\n').len());
        out
    }

    fn format_messages(&self, messages: &[Message]) -> String {
        if messages.is_empty() {
            return String::from("No messages.");
        }

        let mut out = String::new();
        out.push_str(&format!("=== kabel: {} new message(s) ===", messages.len()));

        for msg in messages {
            out.push('\n');
            let to_display = if msg.to_name.is_empty() {
                &msg.to
            } else {
                &msg.to_name
            };
            out.push_str(&format!(
                "[{} \u{2192} {}] {}",
                msg.from, to_display, msg.content
            ));
        }
        out
    }

    fn format_error(&self, error: &str, detail: &str) -> String {
        if detail.is_empty() {
            format!("Error: {error}")
        } else {
            format!("Error: {error}\n  {detail}")
        }
    }

    fn format_info(&self, message: &str) -> String {
        message.to_string()
    }
}

// ---------------------------------------------------------------------------
// JsonFormatter
// ---------------------------------------------------------------------------

/// Produces machine-readable JSON / NDJSON output.
pub struct JsonFormatter;

impl OutputFormatter for JsonFormatter {
    fn format_agents(&self, agents: &[AgentInfo]) -> String {
        // Single JSON object wrapping the array.
        let wrapper = serde_json::json!({ "agents": agents });
        serde_json::to_string(&wrapper).expect("failed to serialize agents")
    }

    fn format_messages(&self, messages: &[Message]) -> String {
        // NDJSON — one JSON object per line.
        messages
            .iter()
            .map(|m| serde_json::to_string(m).expect("failed to serialize message"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_error(&self, error: &str, detail: &str) -> String {
        let obj = serde_json::json!({ "error": error, "detail": detail });
        serde_json::to_string(&obj).expect("failed to serialize error")
    }

    fn format_info(&self, message: &str) -> String {
        let obj = serde_json::json!({ "info": message });
        serde_json::to_string(&obj).expect("failed to serialize info")
    }
}

// ===========================================================================
// Tests (written first — TDD)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // -- helpers ------------------------------------------------------------

    fn sample_agents() -> Vec<AgentInfo> {
        vec![
            AgentInfo {
                session_id: "abc-123".into(),
                name: "lead".into(),
                role: "tech lead".into(),
                status: "online".into(),
                tty: "/dev/ttys001".into(),
                cwd: "/project".into(),
                registered_at: Utc::now(),
                last_seen_at: Utc::now(),
            },
            AgentInfo {
                session_id: "def-456".into(),
                name: "worker1".into(),
                role: "".into(),
                status: "offline".into(),
                tty: "/dev/ttys002".into(),
                cwd: "/project/api".into(),
                registered_at: Utc::now(),
                last_seen_at: Utc::now(),
            },
        ]
    }

    fn sample_messages() -> Vec<Message> {
        vec![
            Message {
                id: "m1".into(),
                from: "lead".into(),
                to: "worker1".into(),
                to_name: String::new(),
                content: "API spec 完成了，請開始串接 /auth/login".into(),
                created_at: Utc::now(),
                read: false,
            },
            Message {
                id: "m2".into(),
                from: "worker2".into(),
                to: "worker1".into(),
                to_name: String::new(),
                content: "我改了 user schema，email 欄位變成 nullable".into(),
                created_at: Utc::now(),
                read: false,
            },
        ]
    }

    // -- 1. detect_output_mode(true) returns Json ---------------------------

    #[test]
    fn detect_output_mode_force_json_returns_json() {
        assert_eq!(detect_output_mode(true), OutputMode::Json);
    }

    // -- 2. HumanFormatter formats error correctly --------------------------

    #[test]
    fn human_format_error_without_detail() {
        let fmt = HumanFormatter;
        let result = fmt.format_error("not found", "");
        assert_eq!(result, "Error: not found");
    }

    #[test]
    fn human_format_error_with_detail() {
        let fmt = HumanFormatter;
        let result = fmt.format_error("not found", "agent 'x' does not exist");
        assert_eq!(result, "Error: not found\n  agent 'x' does not exist");
    }

    // -- 3. JsonFormatter formats error as JSON -----------------------------

    #[test]
    fn json_format_error_is_valid_json() {
        let fmt = JsonFormatter;
        let result = fmt.format_error("not found", "missing agent");
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("should be valid JSON");
        assert_eq!(parsed["error"], "not found");
        assert_eq!(parsed["detail"], "missing agent");
    }

    // -- 4. HumanFormatter formats info as plain text -----------------------

    #[test]
    fn human_format_info_plain_text() {
        let fmt = HumanFormatter;
        let result = fmt.format_info("Agent registered.");
        assert_eq!(result, "Agent registered.");
    }

    // -- 5. JsonFormatter formats info as JSON ------------------------------

    #[test]
    fn json_format_info_is_valid_json() {
        let fmt = JsonFormatter;
        let result = fmt.format_info("Agent registered.");
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("should be valid JSON");
        assert_eq!(parsed["info"], "Agent registered.");
    }

    // -- 6. JsonFormatter format_agents produces valid JSON with "agents" key

    #[test]
    fn json_format_agents_has_agents_key() {
        let fmt = JsonFormatter;
        let agents = sample_agents();
        let result = fmt.format_agents(&agents);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("should be valid JSON");
        assert!(parsed["agents"].is_array());
        assert_eq!(parsed["agents"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["agents"][0]["name"], "lead");
        assert_eq!(parsed["agents"][1]["name"], "worker1");
    }

    // -- 7. HumanFormatter format_messages shows arrow notation -------------

    #[test]
    fn human_format_messages_arrow_notation() {
        let fmt = HumanFormatter;
        let msgs = sample_messages();
        let result = fmt.format_messages(&msgs);

        assert!(result.contains("=== kabel:"));
        assert!(result.contains("2 new message(s)"));
        assert!(result.contains("[lead → worker1] API spec 完成了，請開始串接 /auth/login"));
        assert!(result.contains("[worker2 → worker1] 我改了 user schema，email 欄位變成 nullable"));
    }

    // -- 8. JsonFormatter format_messages produces NDJSON -------------------

    #[test]
    fn json_format_messages_ndjson() {
        let fmt = JsonFormatter;
        let msgs = sample_messages();
        let result = fmt.format_messages(&msgs);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2, "NDJSON should have one line per message");

        for line in &lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each line should be valid JSON");
            assert!(parsed["from"].is_string());
            assert!(parsed["to"].is_string());
            assert!(parsed["content"].is_string());
        }

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["from"], "lead");
        assert_eq!(first["to"], "worker1");
    }

    // -- Extra: HumanFormatter format_agents table output -------------------

    #[test]
    fn human_format_agents_table() {
        let fmt = HumanFormatter;
        let agents = sample_agents();
        let result = fmt.format_agents(&agents);

        assert!(result.contains("NAME"));
        assert!(result.contains("STATUS"));
        assert!(result.contains("ROLE"));
        assert!(result.contains("CWD"));
        assert!(result.contains("lead"));
        assert!(result.contains("online"));
        assert!(result.contains("offline"));
        assert!(result.contains("tech lead"));
        assert!(result.contains("/project/api"));
    }

    #[test]
    fn human_format_agents_empty() {
        let fmt = HumanFormatter;
        let result = fmt.format_agents(&[]);
        assert_eq!(result, "No agents registered.");
    }

    #[test]
    fn human_format_messages_empty() {
        let fmt = HumanFormatter;
        let result = fmt.format_messages(&[]);
        assert_eq!(result, "No messages.");
    }

    // -- Extra: create_formatter returns correct type -----------------------

    #[test]
    fn create_formatter_human() {
        let fmt = create_formatter(OutputMode::Human);
        // HumanFormatter returns plain text for info
        assert_eq!(fmt.format_info("hello"), "hello");
    }

    #[test]
    fn create_formatter_json() {
        let fmt = create_formatter(OutputMode::Json);
        let result = fmt.format_info("hello");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["info"], "hello");
    }
}
