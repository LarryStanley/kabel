use clap::{Parser, Subcommand};

/// kabel — Multi-agent communication CLI for Claude Code sessions
#[derive(Parser, Debug)]
#[command(name = "kabel", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Force JSON output regardless of TTY
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List all online agents
    Discover,

    /// Send a message to a specific agent or channel (#name)
    Send {
        /// Target agent name or #channel
        name: String,
        /// Message content
        message: String,
        /// Your agent name (if not auto-detected)
        #[arg(long)]
        from: Option<String>,
        /// Show what would be done without actually sending
        #[arg(long)]
        dry_run: bool,
    },

    /// Broadcast a message to all online agents
    Broadcast {
        /// Message content
        message: String,
        /// Your agent name (if not auto-detected)
        #[arg(long)]
        from: Option<String>,
        /// Show what would be done without actually sending
        #[arg(long)]
        dry_run: bool,
    },

    /// Read and display unread messages (personal + channels)
    Inbox {
        /// Your agent name (if not auto-detected)
        #[arg(long)]
        name: Option<String>,
    },

    /// Register this agent in the kabel registry
    Register {
        /// Agent name (defaults to directory name)
        #[arg(long)]
        name: Option<String>,
        /// Agent role description
        #[arg(long)]
        role: Option<String>,
    },

    /// Mark this agent as offline
    Unregister,

    /// Output JSON schema for a command
    Schema {
        /// Command name to get schema for
        command: String,
    },

    /// Initialize kabel hooks and SKILL.md in the current project
    Init,

    /// Spawn a team of agents from a YAML config file
    Spawn {
        /// Path to team YAML config file
        #[arg(default_value = "kabel.yaml")]
        config: String,
    },

    /// Kill a running team (tmux session)
    Kill {
        /// Team name (tmux session name)
        team: String,
    },

    /// Show status of a running team
    Status {
        /// Team name (tmux session name)
        team: String,
    },

    /// Start HTTP API server for the web dashboard
    Serve {
        /// Port to listen on
        #[arg(long, default_value = "4200")]
        port: u16,
    },
}
