use std::fmt;

/// Structured validation error with variants for each input type.
#[derive(Debug, PartialEq)]
pub enum ValidationError {
    InvalidName(String),
    InvalidMessage(String),
    InvalidPath(String),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::InvalidName(detail) => write!(f, "invalid_target: {detail}"),
            ValidationError::InvalidMessage(detail) => write!(f, "invalid_message: {detail}"),
            ValidationError::InvalidPath(detail) => write!(f, "invalid_path: {detail}"),
        }
    }
}

impl ValidationError {
    /// Convert to the JSON error format expected by the CLI.
    /// Exit code should be 2 for all validation errors.
    pub fn to_json(&self) -> String {
        let (error, detail) = match self {
            ValidationError::InvalidName(d) => ("invalid_target", d.as_str()),
            ValidationError::InvalidMessage(d) => ("invalid_message", d.as_str()),
            ValidationError::InvalidPath(d) => ("invalid_path", d.as_str()),
        };
        serde_json::json!({"error": error, "detail": detail}).to_string()
    }

    /// The process exit code for validation failures.
    pub const fn exit_code(&self) -> i32 {
        2
    }
}

/// Validate an agent name or session_id.
/// Only `[a-zA-Z0-9_-]` is allowed; rejects `/` `.` `?` `#` `%` and empty input.
pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::InvalidName(
            "name must not be empty".to_string(),
        ));
    }
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' {
            return Err(ValidationError::InvalidName(format!(
                "name contains forbidden character: {ch}"
            )));
        }
    }
    Ok(())
}

/// Validate message content.
/// Rejects ASCII control characters 0x00–0x1F (except `\n`) and DEL (0x7F).
pub fn validate_message(content: &str) -> Result<(), ValidationError> {
    for ch in content.chars() {
        if ch == '\n' {
            continue;
        }
        if ch.is_ascii_control() {
            let code = ch as u32;
            return Err(ValidationError::InvalidMessage(format!(
                "message contains forbidden control character: U+{code:04X}"
            )));
        }
    }
    Ok(())
}

/// Validate a file path relative to `~/.kabel/`.
/// Must not contain `..` segments and must not start with `/`.
pub fn validate_path(path: &str) -> Result<(), ValidationError> {
    if path.is_empty() {
        return Err(ValidationError::InvalidPath(
            "path must not be empty".to_string(),
        ));
    }
    if path.starts_with('/') {
        return Err(ValidationError::InvalidPath(
            "path must not be absolute (starts with /)".to_string(),
        ));
    }
    use std::path::{Component, Path};
    if Path::new(path)
        .components()
        .any(|c| c == Component::ParentDir)
    {
        return Err(ValidationError::InvalidPath(
            "path must not contain '..' traversal".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_name ────────────────────────────────────────────

    #[test]
    fn valid_names_pass() {
        assert!(validate_name("agent1").is_ok());
        assert!(validate_name("my-session").is_ok());
        assert!(validate_name("my_session").is_ok());
        assert!(validate_name("ABC123").is_ok());
        assert!(validate_name("a").is_ok());
    }

    #[test]
    fn empty_name_fails() {
        let err = validate_name("").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidName("name must not be empty".to_string())
        );
    }

    #[test]
    fn name_with_slash_fails() {
        let err = validate_name("bad/name").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidName("name contains forbidden character: /".to_string())
        );
    }

    #[test]
    fn name_with_dot_fails() {
        let err = validate_name("bad.name").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidName("name contains forbidden character: .".to_string())
        );
    }

    #[test]
    fn name_with_question_mark_fails() {
        let err = validate_name("bad?name").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidName("name contains forbidden character: ?".to_string())
        );
    }

    #[test]
    fn name_with_hash_fails() {
        let err = validate_name("bad#name").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidName("name contains forbidden character: #".to_string())
        );
    }

    #[test]
    fn name_with_percent_fails() {
        let err = validate_name("bad%name").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidName("name contains forbidden character: %".to_string())
        );
    }

    // ── validate_message ─────────────────────────────────────────

    #[test]
    fn valid_message_passes() {
        assert!(validate_message("Hello, world!").is_ok());
        assert!(validate_message("line1\nline2").is_ok());
        assert!(validate_message("").is_ok());
        assert!(validate_message("日本語もOK").is_ok());
    }

    #[test]
    fn message_with_null_byte_fails() {
        let err = validate_message("bad\x00msg").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidMessage(
                "message contains forbidden control character: U+0000".to_string()
            )
        );
    }

    #[test]
    fn message_with_control_char_0x01_fails() {
        let err = validate_message("bad\x01msg").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidMessage(
                "message contains forbidden control character: U+0001".to_string()
            )
        );
    }

    #[test]
    fn message_with_del_0x7f_fails() {
        let err = validate_message("bad\x7Fmsg").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidMessage(
                "message contains forbidden control character: U+007F".to_string()
            )
        );
    }

    #[test]
    fn message_newline_is_allowed() {
        assert!(validate_message("hello\nworld").is_ok());
    }

    // ── validate_path ────────────────────────────────────────────

    #[test]
    fn valid_path_passes() {
        assert!(validate_path("agents/worker/log.json").is_ok());
        assert!(validate_path("session.json").is_ok());
        assert!(validate_path("data/nested/file.txt").is_ok());
    }

    #[test]
    fn path_with_dot_dot_fails() {
        let err = validate_path("../escape").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidPath("path must not contain '..' traversal".to_string())
        );
    }

    #[test]
    fn path_with_embedded_dot_dot_fails() {
        let err = validate_path("agents/../../etc/passwd").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidPath("path must not contain '..' traversal".to_string())
        );
    }

    #[test]
    fn path_starting_with_slash_fails() {
        let err = validate_path("/etc/passwd").unwrap_err();
        assert_eq!(
            err,
            ValidationError::InvalidPath("path must not be absolute (starts with /)".to_string())
        );
    }

    // ── JSON output / exit code ──────────────────────────────────

    #[test]
    fn error_json_format() {
        let err = ValidationError::InvalidName("name contains forbidden character: /".to_string());
        let json_str = err.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["error"], "invalid_target");
        assert_eq!(parsed["detail"], "name contains forbidden character: /");
    }

    #[test]
    fn error_exit_code_is_2() {
        let err = ValidationError::InvalidName("test".to_string());
        assert_eq!(err.exit_code(), 2);
        let err = ValidationError::InvalidMessage("test".to_string());
        assert_eq!(err.exit_code(), 2);
        let err = ValidationError::InvalidPath("test".to_string());
        assert_eq!(err.exit_code(), 2);
    }
}
