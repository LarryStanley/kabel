# kabel

> Multi-agent communication CLI that lets multiple Claude Code and OpenCode sessions discover each other, exchange messages, and coordinate work — all through the filesystem.

[![CI](https://github.com/LarryStanley/kabel/actions/workflows/ci.yml/badge.svg)](https://github.com/LarryStanley/kabel/actions/workflows/ci.yml)

## Why kabel?

When you run multiple AI coding agents in parallel, they have no way to talk to each other. One agent changes a shared API while another builds against the old version. The lead agent becomes a message bottleneck, manually relaying between workers.

**kabel** gives agents a shared communication layer — channels, direct messages, @mentions — with zero infrastructure. No daemons, no servers, just `.kabel/` in your project directory.

## Features

- **Channels** — `kabel send '#general' "msg"` for group discussion (like Slack)
- **@mentions** — `kabel send '#general' "@tom please verify"` pings tom's personal inbox too
- **Direct messages** — `kabel send tom "private task for you"`
- **Offline messaging** — messages persist until read, even across sessions
- **Multi-backend** — Claude Code and OpenCode agents in the same team
- **YAML team spawn** — define your team in a YAML file, spawn all agents with one command
- **Per-project isolation** — each project gets its own `.kabel/` directory

## Quick start

### Option 1: YAML spawn (recommended)

Define your team in `kabel.yaml`:

```yaml
team: my-project
cwd: /path/to/project

agents:
  - name: lead
    backend: claude
    role: "Tech Lead — architecture, task delegation, code review"
    prompt: |
      You are the tech lead. After setup:
      1. Post the plan in #general
      2. Assign tasks to team members

  - name: worker
    backend: claude
    role: "Backend engineer"
    prompt: |
      You are a backend engineer. Wait for lead's task assignment.

  - name: reviewer
    backend: opencode
    role: "Code reviewer"
    prompt: |
      You are a code reviewer. Review PRs and post findings in #general.
```

Then spawn:

```bash
kabel spawn kabel.yaml
# Opens tmux with all agents in split panes, each running their own session
```

Each agent automatically:
1. Registers with kabel
2. Reads the SKILL.md for communication protocol
3. Checks for existing messages
4. Starts working on their prompt

Manage the team:

```bash
kabel status my-project   # show running agents
kabel kill my-project      # stop all agents
```

### Option 2: Manual setup

```bash
# Register yourself
kabel register --name alice --role "Backend engineer"

# See who's online
kabel discover

# Send to a channel (everyone sees it)
kabel send '#general' "API spec is done, ready for integration"

# Send a direct message
kabel send bob "Can you review my PR?"

# Read your messages (personal + channels)
kabel inbox --name alice

# Poll for new messages (in Claude Code)
/loop 2m kabel inbox --name alice
```

## Installation

### Quick install (macOS / Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/LarryStanley/kabel/main/install.sh | bash
```

### Homebrew (macOS)

```bash
brew tap LarryStanley/tap
brew install kabel
```

### Cargo (from source)

```bash
cargo install kabel
```

### GitHub Releases

Download pre-built binaries from [Releases](https://github.com/LarryStanley/kabel/releases).

### Prerequisites

**tmux** is required for `kabel spawn` (team spawning):

```bash
# macOS
brew install tmux

# Ubuntu/Debian
sudo apt install tmux
```

## CLI reference

| Command | Description |
|---------|-------------|
| `kabel discover` | List all agents (online and offline) |
| `kabel send <target> <message>` | Send to an agent or `#channel`. Supports `@mentions`. |
| `kabel send <target> <message> --from <name>` | Send with explicit sender identity |
| `kabel broadcast <message>` | Send to every agent's personal inbox |
| `kabel inbox --name <name>` | Read unread messages (personal + channels) |
| `kabel register --name <name> --role <role>` | Register in the registry |
| `kabel unregister` | Mark as offline |
| `kabel spawn [config.yaml]` | Spawn a team from YAML config (default: `kabel.yaml`) |
| `kabel kill <team>` | Kill a team's tmux session |
| `kabel status <team>` | Show running agents in a team |
| `kabel init` | Generate SKILL.md and hooks config |
| `kabel schema <command>` | Output JSON schema for a command |

### Output modes

- **TTY** — human-readable tables
- **Non-TTY** (agent via Bash tool) — NDJSON, one JSON object per line
- **`--json`** — force JSON output

## Architecture

### Storage layout

All state lives in `.kabel/` in your project directory — per-project isolation, no global state.

```
.kabel/
├── registry/
│   └── {name}.json          # Agent info (name, role, status, session_id)
├── inbox/
│   └── {name}.jsonl         # Personal messages (append-only, flock)
├── channels/
│   └── {channel}.jsonl      # Channel messages (shared, append-only)
├── cursors/
│   └── {agent}/{channel}.txt  # Per-agent read position for channels
└── sessions/
    └── {session_id}.name    # Maps session ID to agent name
```

### How agents communicate

```
  kabel send '#general' "API done" --from lead

  ┌──────────┐    channels/general.jsonl    ┌──────────┐
  │  lead    │ ──────────────────────────► │  shared  │
  │ (claude) │                             │  file    │
  └──────────┘                             └────┬─────┘
                                                │
                         ┌──────────────────────┼──────────────────────┐
                         │                      │                      │
                    check_channels         check_channels         check_channels
                         │                      │                      │
                    ┌────▼─────┐          ┌────▼─────┐          ┌────▼─────┐
                    │  worker  │          │ reviewer │          │   lead   │
                    │ (claude) │          │(opencode)│          │ (skip    │
                    │ sees msg │          │ sees msg │          │  own)    │
                    └──────────┘          └──────────┘          └──────────┘
```

### YAML team config

The `kabel spawn` command reads a YAML file to create a full team:

```yaml
team: my-project          # tmux session name
cwd: /path/to/project     # working directory
model: claude-sonnet-4    # optional default model

agents:
  - name: lead
    backend: claude        # "claude" (default) or "opencode"
    role: "Tech Lead"
    prompt: "Your instructions here"
    model: claude-opus-4   # optional per-agent override
```

**Backend support:**

| Backend | Launch | Prompt delivery | Polling |
|---------|--------|-----------------|---------|
| `claude` | `claude --dangerously-skip-permissions` | tmux paste-buffer | `/loop 2m kabel inbox` |
| `opencode` | `opencode` | tmux send-keys | manual `kabel inbox` |

Both backends share the same `.kabel/` directory, so Claude Code and OpenCode agents communicate seamlessly.

### Agent lifecycle

1. **Register** — `kabel register --name alice` adds agent to registry with `status: online`
2. **Communicate** — send/receive via channels, DMs, @mentions, broadcast
3. **Go offline** — `kabel unregister` sets `status: offline` (does NOT delete, messages preserved)
4. **Come back** — `kabel register --name alice` updates session, goes back online, reads queued messages

## Contributing

```bash
git clone https://github.com/LarryStanley/kabel.git
cd kabel
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## License

MIT — see [LICENSE](LICENSE).
