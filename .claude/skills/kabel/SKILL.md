---
name: kabel
description: >
  Use when you need to communicate with other agents
  or discover who is online. kabel enables multi-agent coordination
  through a filesystem-based message passing system.
---

## Important: kabel is a system CLI tool

`kabel` is a binary installed in PATH. **Call it directly via Bash tool**, not a script.

## Setup

At the start of your session, register yourself:
  Bash: kabel register --name <your-name>

Then poll for messages periodically (every 2-5 minutes):
  /loop 2m kabel inbox --name <your-name>

## Commands (run via Bash tool)

Register (once at session start):
  Bash: kabel register --name <your-name>

See all agents (online and offline):
  Bash: kabel discover

Read messages (personal + channels):
  Bash: kabel inbox --name <your-name>

Send to a channel (everyone sees it):
  Bash: kabel send '#general' "message" --from <your-name>

Send to a channel with @mention (recipient also gets a DM notification):
  Bash: kabel send '#general' "fixed the bug @tom please verify" --from <your-name>

Send a direct message:
  Bash: kabel send <agent-name> "message" --from <your-name>

Broadcast to all agents (writes to each personal inbox):
  Bash: kabel broadcast "message" --from <your-name>

Mark offline (at session end):
  Bash: kabel unregister

## Channel vs DM vs Broadcast

| Method | Usage | When to use |
|--------|-------|-------------|
| **Channel** | `kabel send '#general' "msg"` | Team discussion, status updates, anything everyone should see. Use @name to ping someone. |
| **DM** | `kabel send tom "msg"` | Assign specific tasks, private communication |
| **Broadcast** | `kabel broadcast "msg"` | Urgent notifications (written to every agent's personal inbox) |

## Polling for messages

kabel does not push messages to you. You must **actively check**. Use loop:

  /loop 2m kabel inbox --name <your-name>

Or manually check at natural break points:
  Bash: kabel inbox --name <your-name>

## Rules (must follow)

1. **Register at session start** — `kabel register --name <your-name>`
2. **Always use --name or --from** — `kabel inbox --name alice`, `kabel send '#general' "msg" --from alice`
3. **Use channels for team discussion** — `kabel send '#general' "msg"` so everyone sees the conversation
4. **Use DM for specific tasks** — only use `kabel send <name> "msg"` for tasks assigned to one person
5. **You can message offline agents** — they'll see it when they come back
6. **Respond to messages** — when you receive a message, take action and reply
7. **Be specific** — include enough context so the recipient can act immediately
8. **Poll regularly** — use `/loop 2m kabel inbox --name <your-name>`

## Message format tips

Good channel message — specific, actionable:
  kabel send '#general' "Fixed auth bug (commit abc123), API now returns 401 for expired tokens" --from alice

Good DM — specific task assignment:
  kabel send tom "Please monitor the next 3 deployment windows and report results" --from lead

Bad message — too vague:
  kabel send '#general' "done" --from alice
