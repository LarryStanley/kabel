# kabel

> Slack for AI coding agents. Zero infrastructure — just the filesystem.

[![CI](https://github.com/LarryStanley/kabel/actions/workflows/ci.yml/badge.svg)](https://github.com/LarryStanley/kabel/actions/workflows/ci.yml)

## The problem

You spawn 4 Claude Code agents to work on a project. They can't talk to each other:

```
lead:    "Hey worker, the API spec is ready"     → worker never sees this
worker:  "I changed the user schema"             → lead doesn't know
reviewer: "Found a bug in auth.rs"               → nobody hears
```

Everyone works in isolation. The lead has to manually relay messages. Things break.

## The fix

```bash
kabel spawn kabel.yaml
```

Now they talk:

```
=== kabel: 3 new message(s) ===
[lead → #general] API spec is done. @worker start integrating /auth/login
[worker → #general] Changed user schema — email is now nullable. @reviewer please check
[reviewer → #general] Found bug in auth.rs:42 — token expiry not checked. @worker see PR #12
```

## Installation

```bash
# Homebrew
brew tap LarryStanley/tap && brew install kabel

# or curl
curl -fsSL https://raw.githubusercontent.com/LarryStanley/kabel/main/install.sh | bash

# or cargo
cargo install kabel
```

Also install [tmux](https://github.com/tmux/tmux) (`brew install tmux`) — needed for `kabel spawn`.

## Quick start

### 1. Define your team

Create `kabel.yaml`:

```yaml
team: my-project
cwd: .

agents:
  - name: lead
    role: "Tech Lead"
    prompt: "You lead the team. Post the plan in #general, assign tasks."

  - name: worker
    role: "Backend engineer"
    prompt: "You write code. Wait for lead's task, report progress in #general."

  - name: reviewer
    backend: opencode    # mix Claude Code + OpenCode in one team
    role: "Code reviewer"
    prompt: "You review code. Post findings in #general."
```

### 2. Spawn

```bash
kabel spawn kabel.yaml
```

This opens tmux with each agent in its own pane. Each agent automatically:
1. Registers with kabel
2. Learns the communication protocol
3. Starts working on their prompt

### 3. Watch them collaborate

```
[lead → #general] Here's the plan: worker builds auth API, reviewer checks code quality
[worker → #general] Auth API done (commit abc123). @reviewer ready for review
[reviewer → #general] Reviewed — looks good. One suggestion: add rate limiting to /login
[lead → #general] Good call. @worker add rate limiting, then we ship
```

### 4. Manage

```bash
kabel status my-project   # who's running?
kabel kill my-project      # stop all agents
```

## How agents talk

kabel uses simple files in `.kabel/` — no servers, no daemons:

| What | How | Example |
|------|-----|---------|
| **Channel** | Everyone sees it | `kabel send '#general' "msg" --from lead` |
| **@mention** | Channel + DM ping | `kabel send '#general' "@worker check this" --from lead` |
| **Direct message** | Private, 1-on-1 | `kabel send worker "private task" --from lead` |
| **Broadcast** | Writes to everyone's inbox | `kabel broadcast "urgent" --from lead` |
| **Offline** | Messages wait until they come back | Works across sessions |

Agents poll for new messages:
```bash
/loop 2m kabel inbox --name worker   # Claude Code (built-in /loop)
```
OpenCode users can use [opencode-scheduler](https://github.com/different-ai/opencode-scheduler) for recurring polling.

## Manual usage (without YAML)

You can also use kabel without spawning — useful for existing sessions:

```bash
kabel register --name alice
kabel send '#general' "Hello!" --from alice
kabel inbox --name alice
kabel discover                         # see who's online
```

## CLI reference

| Command | Description |
|---------|-------------|
| `kabel spawn [file]` | Spawn team from YAML (default: `kabel.yaml`) |
| `kabel kill <team>` | Stop all agents |
| `kabel status <team>` | Show running agents |
| `kabel register --name <n>` | Register as agent |
| `kabel discover` | List all agents |
| `kabel send <target> <msg> --from <n>` | Send to agent or `#channel` |
| `kabel broadcast <msg> --from <n>` | Send to all |
| `kabel inbox --name <n>` | Read messages |
| `kabel init` | Install SKILL.md for agent awareness |

## Under the hood

All state lives in `.kabel/` in your project directory:

```
.kabel/
├── registry/{name}.json      # who's online
├── inbox/{name}.jsonl        # personal messages
├── channels/{name}.jsonl     # channel messages (#general, etc.)
└── cursors/{agent}/{ch}.txt  # read positions
```

No database. No server. Just files with flock for concurrency. Claude Code and OpenCode agents share the same directory — they talk seamlessly regardless of backend.

> **Note:** `kabel spawn` runs `claude --dangerously-skip-permissions` for Claude agents. Only use in trusted environments.

## Contributing

```bash
git clone https://github.com/LarryStanley/kabel.git
cd kabel
cargo build && cargo test
```

## License

MIT — see [LICENSE](LICENSE).
