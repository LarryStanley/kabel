use std::process;

use clap::Parser;

use kabel::cli::{Cli, Command};
use kabel::inbox::Inbox;
use kabel::output::{create_formatter, detect_output_mode, OutputFormatter};
use kabel::registry::Registry;
use kabel::storage::KabelStorage;

fn main() {
    let cli = Cli::parse();
    let mode = detect_output_mode(cli.json);
    let fmt = create_formatter(mode);

    if let Err(e) = run(cli.command, &*fmt) {
        eprintln!("{e}");
        process::exit(1);
    }
}

fn run(command: Command, fmt: &dyn OutputFormatter) -> Result<(), Box<dyn std::error::Error>> {
    let storage = KabelStorage::new();

    match command {
        Command::Discover => {
            let registry = Registry::new(storage.clone());
            let agents = registry.discover()?;
            println!("{}", fmt.format_agents(&agents));
        }

        Command::Send {
            name,
            message,
            from,
            dry_run,
        } => {
            let is_channel = name.starts_with('#');

            if dry_run {
                println!(
                    "{}",
                    fmt.format_info(&format!("Would send to {}: {}", name, message))
                );
                return Ok(());
            }

            let from_name = from.unwrap_or(get_self_name(&storage)?);
            let inbox = Inbox::new(storage);

            if is_channel {
                let channel = name.trim_start_matches('#');
                let msg = inbox.send_channel(&from_name, channel, &message)?;
                println!(
                    "{}",
                    fmt.format_info(&format!("Message sent to #{} (id: {})", channel, msg.id))
                );
            } else {
                let msg = inbox.send(&from_name, &name, &message)?;
                println!(
                    "{}",
                    fmt.format_info(&format!("Message sent to {} (id: {})", name, msg.id))
                );
            }
        }

        Command::Broadcast {
            message,
            from,
            dry_run,
        } => {
            if dry_run {
                let registry = Registry::new(storage.clone());
                let agents = registry.discover()?;
                println!(
                    "{}",
                    fmt.format_info(&format!(
                        "Would broadcast to {} agent(s): {}",
                        agents.len(),
                        message
                    ))
                );
                return Ok(());
            }

            let from_name = from.unwrap_or(get_self_name(&storage)?);
            let inbox = Inbox::new(storage);
            let sent = inbox.broadcast(&from_name, &message)?;
            println!(
                "{}",
                fmt.format_info(&format!("Broadcast sent to {} agent(s)", sent.len()))
            );
        }

        Command::Inbox { name } => {
            let self_name = name.unwrap_or(get_self_name(&storage)?);
            let inbox = Inbox::new(storage);

            // Read personal inbox + channels
            let mut messages = inbox.check_inbox(&self_name)?;
            if let Ok(channel_msgs) = inbox.check_channels(&self_name) {
                messages.extend(channel_msgs);
            }
            messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));

            if messages.is_empty() {
                println!("{}", fmt.format_info("No unread messages."));
            } else {
                println!("{}", fmt.format_messages(&messages));
            }
        }

        Command::Schema { command } => {
            let schema = get_schema(&command)?;
            println!("{schema}");
        }

        Command::Register { name, role } => {
            let agent_name = name.unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "agent".to_string())
            });

            let registry = Registry::new(storage.clone());
            let info = registry.register(&agent_name, role.as_deref())?;

            if let Ok(session_id) = get_session_id() {
                let _ = storage.write_session_name(&session_id, &agent_name);
            }

            println!(
                "{}",
                fmt.format_info(&format!(
                    "Registered as '{}' (session: {})",
                    info.name, info.session_id
                ))
            );
        }

        Command::Unregister => match get_session_id() {
            Ok(session_id) => {
                let registry = Registry::new(storage.clone());
                match registry.unregister(&session_id) {
                    Ok(()) => println!("{}", fmt.format_info("Marked offline")),
                    Err(e) => eprintln!("kabel: unregister warning: {e}"),
                }
                let _ = storage.remove_session_name(&session_id);
            }
            Err(_) => eprintln!("kabel: unregister warning: no session ID"),
        },

        Command::Init => {
            init_project(fmt)?;
        }

        Command::Spawn { config } => {
            let team_config = kabel::spawn::load_config(&config)?;
            println!(
                "{}",
                fmt.format_info(&format!(
                    "Spawning team '{}' with {} agent(s)...",
                    team_config.team,
                    team_config.agents.len()
                ))
            );
            kabel::spawn::spawn_team(&team_config)?;
            println!(
                "{}",
                fmt.format_info(&format!(
                    "Team '{}' spawned in tmux. Attach with: tmux attach -t {}",
                    team_config.team, team_config.team
                ))
            );
        }

        Command::Kill { team } => {
            kabel::spawn::kill_team(&team)?;
            println!("{}", fmt.format_info(&format!("Team '{team}' killed")));
        }

        Command::Status { team } => {
            let windows = kabel::spawn::list_team_windows(&team)?;
            if windows.is_empty() {
                println!(
                    "{}",
                    fmt.format_info(&format!("Team '{team}' is not running"))
                );
            } else {
                println!(
                    "{}",
                    fmt.format_info(&format!(
                        "Team '{team}' running with {} agent(s): {}",
                        windows.len(),
                        windows.join(", ")
                    ))
                );
            }
        }

        Command::Serve { port } => {
            println!(
                "{}",
                fmt.format_info(&format!("kabel serve is not yet implemented (port {port})"))
            );
        }
    }

    Ok(())
}

/// Get the current agent's name.
/// Priority: per-session file → registry lookup → cwd fallback.
fn get_self_name(storage: &KabelStorage) -> Result<String, Box<dyn std::error::Error>> {
    // 1. Per-session name file
    if let Ok(session_id) = get_session_id() {
        if let Ok(Some(name)) = storage.read_session_name(&session_id) {
            return Ok(name);
        }
        // 2. Registry lookup by session_id
        let registry = Registry::new(storage.clone());
        if let Ok(Some(name)) = registry.find_name_by_session_id(&session_id) {
            return Ok(name);
        }
    }

    // 3. Fallback: cwd name
    Ok(std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "agent".to_string()))
}

fn get_session_id() -> Result<String, Box<dyn std::error::Error>> {
    kabel::registry::resolve_session_id()
        .ok_or_else(|| "CLAUDE_SESSION_ID or OPENCODE_SESSION_ID not set".into())
}

fn get_schema(command: &str) -> Result<String, Box<dyn std::error::Error>> {
    let schema = match command {
        "discover" => serde_json::json!({
            "command": "kabel discover",
            "description": "List all online agents",
            "output": {
                "type": "object",
                "properties": {
                    "agents": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "session_id": {"type": "string"},
                                "name": {"type": "string"},
                                "tty": {"type": "string"},
                                "cwd": {"type": "string"},
                                "registered_at": {"type": "string", "format": "date-time"}
                            }
                        }
                    }
                }
            }
        }),
        "send" => serde_json::json!({
            "command": "kabel send <name> <message>",
            "description": "Send a message to a specific agent",
            "args": {
                "name": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"},
                "message": {"type": "string"}
            }
        }),
        "broadcast" => serde_json::json!({
            "command": "kabel broadcast <message>",
            "description": "Broadcast a message to all online agents",
            "args": {
                "message": {"type": "string"}
            }
        }),
        "inbox" => serde_json::json!({
            "command": "kabel inbox",
            "description": "Read and display unread messages",
            "output": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "from": {"type": "string"},
                        "to": {"type": "string"},
                        "content": {"type": "string"},
                        "created_at": {"type": "string", "format": "date-time"},
                        "read": {"type": "boolean"}
                    }
                }
            }
        }),
        _ => return Err(format!("unknown command: {command}").into()),
    };
    Ok(serde_json::to_string_pretty(&schema)?)
}

fn init_project(fmt: &dyn OutputFormatter) -> Result<(), Box<dyn std::error::Error>> {
    let skill_md = include_str!("../skills/SKILL.md");

    // Install for Claude Code
    std::fs::create_dir_all(".claude/skills/kabel")?;
    std::fs::write(".claude/skills/kabel/SKILL.md", skill_md)?;
    println!("{}", fmt.format_info("Wrote .claude/skills/kabel/SKILL.md"));

    // Install for OpenCode (if .opencode/ exists)
    if std::path::Path::new(".opencode").exists() {
        std::fs::create_dir_all(".opencode/skills/kabel")?;
        std::fs::write(".opencode/skills/kabel/SKILL.md", skill_md)?;
        println!(
            "{}",
            fmt.format_info("Wrote .opencode/skills/kabel/SKILL.md")
        );
    }

    // Add .kabel/ to .gitignore if not already there
    let gitignore_path = ".gitignore";
    let needs_gitignore = if std::path::Path::new(gitignore_path).exists() {
        let content = std::fs::read_to_string(gitignore_path)?;
        !content.contains(".kabel")
    } else {
        true
    };
    if needs_gitignore {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(gitignore_path)?;
        writeln!(f, "\n.kabel/")?;
        println!("{}", fmt.format_info("Added .kabel/ to .gitignore"));
    }

    println!(
        "{}",
        fmt.format_info("Done. Agents can now use kabel for team communication.")
    );

    Ok(())
}
