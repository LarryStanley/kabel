use std::fmt;
use std::fs;
use std::io;
use std::process::Command as ProcessCommand;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Team configuration loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub team: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub model: String,
    pub agents: Vec<AgentConfig>,
}

/// Individual agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub model: String,
}

fn default_backend() -> String {
    "claude".to_string()
}

#[derive(Debug)]
pub enum SpawnError {
    Io(io::Error),
    Yaml(serde_yaml::Error),
    Tmux(String),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpawnError::Io(e) => write!(f, "I/O error: {e}"),
            SpawnError::Yaml(e) => write!(f, "YAML parse error: {e}"),
            SpawnError::Tmux(e) => write!(f, "tmux error: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {}

impl From<io::Error> for SpawnError {
    fn from(e: io::Error) -> Self {
        SpawnError::Io(e)
    }
}

impl From<serde_yaml::Error> for SpawnError {
    fn from(e: serde_yaml::Error) -> Self {
        SpawnError::Yaml(e)
    }
}

/// Load team configuration from a YAML file.
pub fn load_config(path: &str) -> Result<TeamConfig, SpawnError> {
    let content = fs::read_to_string(path)?;
    let config: TeamConfig = serde_yaml::from_str(&content)?;
    Ok(config)
}

/// Spawn all agents defined in the team config using tmux.
pub fn spawn_team(config: &TeamConfig) -> Result<(), SpawnError> {
    let session = &config.team;
    let cwd = if config.cwd.is_empty() {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    } else {
        config.cwd.clone()
    };

    if !tmux_available() {
        return Err(SpawnError::Tmux("tmux is not installed".into()));
    }

    // Prepare prompt files
    let prompt_dir = format!("{cwd}/.kabel/prompts");
    fs::create_dir_all(&prompt_dir)?;

    // Write all prompt files first (opencode needs them at launch time)
    for agent in &config.agents {
        let init_prompt = build_init_prompt(agent, &cwd);
        let prompt_file = format!("{prompt_dir}/{}.txt", agent.name);
        fs::write(&prompt_file, &init_prompt)?;
    }

    // Kill existing session if it exists
    let _ = ProcessCommand::new("tmux")
        .args(["kill-session", "-t", session])
        .output();

    // All agents in split panes — re-tile before each split to avoid "no space" error

    // Create tmux session with first agent
    let first_agent = config
        .agents
        .first()
        .ok_or_else(|| SpawnError::Tmux("no agents defined".into()))?;

    let launch_cmd = build_launch_command(first_agent, config, &cwd);

    tmux_new_session(session, &first_agent.name, &cwd)?;
    tmux_send_keys(session, &first_agent.name, &launch_cmd)?;

    // Spawn remaining agents as split panes (all visible at once)
    for agent in config.agents.iter().skip(1) {
        let launch_cmd = build_launch_command(agent, config, &cwd);

        // Re-apply tiled layout before each split so space is evenly distributed
        // This prevents "no space for new pane" errors
        tmux_select_layout(session, "tiled")?;
        tmux_split_pane(session, &cwd)?;
        tmux_send_keys_to_current(session, &launch_cmd)?;
    }

    // Final layout
    tmux_select_layout(session, "tiled")?;

    // Ensure status bar is visible (shows window tabs for switching)
    let _ = ProcessCommand::new("tmux")
        .args(["set-option", "-t", session, "status", "on"])
        .output();

    // Wait for each agent to be ready, then send prompt
    for (i, agent) in config.agents.iter().enumerate() {
        let target = format!("{session}:0.{i}");
        let prompt_file = format!("{prompt_dir}/{}.txt", agent.name);

        eprintln!(
            "kabel: waiting for {} ({}) to be ready...",
            agent.name, agent.backend
        );
        wait_for_ready(&target, &agent.backend, 30)?;

        match agent.backend.as_str() {
            "opencode" => {
                let prompt = fs::read_to_string(&prompt_file)?;
                let oneline = prompt.replace('\n', " \\n ");
                tmux_send_keys_literal(&target, &oneline)?;
                thread::sleep(Duration::from_millis(100));
                tmux_send_enter(&target)?;
            }
            _ => {
                tmux_send_prompt_to_pane(&target, &prompt_file)?;
            }
        }

        thread::sleep(Duration::from_secs(1));
    }

    Ok(())
}

/// Kill a team's tmux session.
pub fn kill_team(team: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["kill-session", "-t", team])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to kill session '{}': {}",
            team,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

/// List tmux panes in a team session with agent info.
pub fn list_team_windows(team: &str) -> Result<Vec<String>, SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args([
            "list-panes",
            "-t",
            team,
            "-F",
            "#{pane_index}: #{pane_current_command}",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

// ── launch command builders ──────────────────────────────────────────────

fn build_launch_command(agent: &AgentConfig, config: &TeamConfig, _cwd: &str) -> String {
    let model = if !agent.model.is_empty() {
        &agent.model
    } else if !config.model.is_empty() {
        &config.model
    } else {
        ""
    };

    let session_id = uuid::Uuid::new_v4();

    match agent.backend.as_str() {
        "opencode" => {
            // OpenCode: interactive TUI, prompt sent via send-keys after ready
            let mut cmd = format!("export OPENCODE_SESSION_ID={session_id} && opencode");
            if !model.is_empty() {
                cmd.push_str(&format!(" --model {model}"));
            }
            cmd
        }
        _ => {
            // Default: claude interactive TUI
            let mut cmd = format!(
                "export CLAUDE_SESSION_ID={session_id} && claude --dangerously-skip-permissions"
            );
            if !model.is_empty() {
                cmd.push_str(&format!(" --model {model}"));
            }
            cmd
        }
    }
}

fn build_init_prompt(agent: &AgentConfig, cwd: &str) -> String {
    let mut prompt = String::new();

    // Identity
    prompt.push_str(&format!("You are {}.", agent.name));
    if !agent.role.is_empty() {
        prompt.push_str(&format!(" Your role: {}.\n\n", agent.role));
    }

    // Task
    if !agent.prompt.is_empty() {
        prompt.push_str(&format!("{}\n\n", agent.prompt));
    }

    // Kabel setup — backend-specific polling instructions
    let poll_instruction = match agent.backend.as_str() {
        "opencode" => {
            "5. Periodically run kabel inbox --name <your-name> to check messages (check once after each task)"
        }
        _ => "5. Use /loop 2m kabel inbox --name <your-name> to poll messages",
    };

    // Skill file location — check both .claude and .opencode
    let skill_path = match agent.backend.as_str() {
        "opencode" => format!("{cwd}/.opencode/skills/kabel/SKILL.md"),
        _ => format!("{cwd}/.claude/skills/kabel/SKILL.md"),
    };

    prompt.push_str(&format!(
        "## Setup steps (execute in order)\n\
         1. Run: kabel register --name {} --role '{}'\n\
         2. Read {skill_path} for team communication guide (skip if file doesn't exist)\n\
         3. Run: kabel discover to check who's online\n\
         4. Run: kabel inbox --name {} to check for unread messages\n\
         {poll_instruction}\n\
         6. Start your work",
        agent.name, agent.role, agent.name,
    ));

    prompt
}

// ── ready detection ──────────────────────────────────────────────────────

fn wait_for_ready(pane_target: &str, backend: &str, timeout_secs: u64) -> Result<(), SpawnError> {
    let start = std::time::Instant::now();

    while start.elapsed() < Duration::from_secs(timeout_secs) {
        let output = ProcessCommand::new("tmux")
            .args(["capture-pane", "-t", pane_target, "-p"])
            .output()?;

        let content = String::from_utf8_lossy(&output.stdout);

        let ready = match backend {
            "opencode" => {
                // OpenCode shows "Build" in agent selector when ready for input
                content.contains("Build")
            }
            _ => {
                // Claude Code shows "❯" when ready
                content.contains('❯')
            }
        };

        if ready {
            return Ok(());
        }

        thread::sleep(Duration::from_secs(1));
    }

    eprintln!(
        "kabel: warning: pane {pane_target} ({backend}) didn't show ready prompt within {timeout_secs}s"
    );
    Ok(())
}

// ── tmux helpers ─────────────────────────────────────────────────────────

fn tmux_available() -> bool {
    ProcessCommand::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tmux_new_session(session: &str, window: &str, cwd: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["new-session", "-d", "-s", session, "-n", window, "-c", cwd])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to create tmux session: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

#[allow(dead_code)]
fn tmux_new_window(session: &str, window: &str, cwd: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["new-window", "-t", session, "-n", window, "-c", cwd])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to create window for {window}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_split_pane(session: &str, cwd: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["split-window", "-t", session, "-c", cwd])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to split pane: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_send_keys(session: &str, window: &str, cmd: &str) -> Result<(), SpawnError> {
    let target = format!("{session}:{window}");
    let output = ProcessCommand::new("tmux")
        .args(["send-keys", "-t", &target, cmd, "Enter"])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to send keys to {target}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_send_keys_to_current(session: &str, cmd: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["send-keys", "-t", session, cmd, "Enter"])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to send keys: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_select_layout(session: &str, layout: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["select-layout", "-t", session, layout])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to set layout: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_send_keys_literal(pane_target: &str, text: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["send-keys", "-t", pane_target, "-l", text])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to send literal keys to {pane_target}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_send_enter(pane_target: &str) -> Result<(), SpawnError> {
    let output = ProcessCommand::new("tmux")
        .args(["send-keys", "-t", pane_target, "Enter"])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to send Enter to {pane_target}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn tmux_send_prompt_to_pane(pane_target: &str, prompt_file: &str) -> Result<(), SpawnError> {
    // Load file into tmux buffer
    let output = ProcessCommand::new("tmux")
        .args(["load-buffer", prompt_file])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to load buffer: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Paste into target pane
    let output = ProcessCommand::new("tmux")
        .args(["paste-buffer", "-t", pane_target])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to paste to {pane_target}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Small delay to let paste complete before pressing Enter
    thread::sleep(Duration::from_millis(300));

    // Press Enter
    let output = ProcessCommand::new("tmux")
        .args(["send-keys", "-t", pane_target, "Enter"])
        .output()?;

    if !output.status.success() {
        return Err(SpawnError::Tmux(format!(
            "failed to send Enter to {pane_target}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}
