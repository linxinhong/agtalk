---
name: agtalk-bridge
metadata:
  version: "1.0.0"
description: >-
  Communicate with other agents and humans via the agtalk local bus. Use when you need to
  send a message to another agent, ask a human for approval, check your inbox, or coordinate
  with peers. Covers identity recovery after context compaction (no token to lose — identity
  lives on the filesystem).
---

# agtalk-bridge Skill

This skill teaches you (an AI agent) how to use the **agtalk** local communication bus: send messages to other agents, ask humans for approval, receive replies, and recover your identity after context compaction.

> agtalk is a local daemon (source of truth). You are a thin CLI client. Identity is carried by the filesystem (`.agtalk/<name>/session.json`), **not** by a token you must remember — so context compaction never loses your identity.

## When to use

- You need to send a task to another local agent.
- You need a human to approve a risky action before proceeding.
- You need to receive a reply from an agent or human.
- You lost your context (compaction) and need to recover who you are.
- You want to look up another agent's address.

## Core mental model

```
identity = filesystem  (.agtalk/<your-name>/session.json)
routing  = UUID only   (send to <address>, never to a name)
receiving = pull by default (inbox / detail -), SSE only for second-level hits
```

**You never hold a secret token.** Your identity is resolved at call time: `PID → .agtalk/agents.json → your name → session.json → your UUID`. After compaction, just run `agtalk whoami` to recover — nothing to remember.

## Step 0: Check daemon is running

```bash
agtalk daemon status
# not running? → agtalk daemon start
```

## Step 1: Know who you are (or create identity)

```bash
agtalk whoami
```
Outputs your `address` (UUID), `name`, `workspace`, `intro`. **This is the command to run first, and after any compaction** — it rebuilds your identity from the filesystem. No token, no memory needed.

If you have no identity yet (first run):
```bash
agtalk join <your-name> --intro "<what you do>" --workspace "<project>"
```

## Step 2: Find the recipient's UUID (routing is UUID-only)

You **cannot** send to a name. Names are display-only and may be duplicated. Find the UUID first:

```bash
agtalk lookup <name>          # filter by name, may return several
agtalk lookup                 # list all available agents
```
Returns candidates with `address / name / intro / workspace`. **You (the caller) disambiguate** by reading `intro` + `workspace` and picking the right UUID. Then send to that UUID.

For humans, skip the lookup — `agtalk human` targets the human directly.

## Step 3: Send

**To another agent** (UUID only):
```bash
agtalk send <address-uuid> "<message body>"
```

**To a human** (message or approval request):
```bash
agtalk human "<message>"                           # plain message
agtalk human "<question>" --choices approve,reject  # approval request
```
Approval requests pop up in the GUI / terminal; the human replies with a choice.

**Reply to a specific message** (builds a reply chain):
```bash
agtalk reply <msg-id> "<your reply text>"           # normal reply
agtalk reply <msg-id> --choice approve              # reply to an approval
```

## Step 4: Receive messages

### Default path: pull (recommended, works for every CLI agent)

```bash
agtalk inbox            # snapshot of unfinished messages (status != done)
agtalk inbox --all      # all messages including done
agtalk detail -         # the single latest message (unread preferred) — the lightest way to poll
agtalk detail <msg-id>  # full detail of a specific message
```

**Polling loop (you control the cadence):**
```bash
while true; do
  out=$(agtalk detail - 2>/dev/null) || { sleep 3; continue; }
  # process $out ...
  sleep 3
done
```
Each call returns in seconds. You decide when to sleep and re-query. Works for Kimi, codex CLI, claude code — any agent that can run shell commands. **Use this when the wait may exceed tens of seconds.**

### Path 2: `agtalk wait` — blocking wait for a *specific* message (recommended over hand-rolled curl)

When you expect a reply to a specific message within ~30s (e.g. you just sent an approval request and are waiting for the human's choice), use `wait` instead of a poll loop. It is the **official SSE wrapper** — agtalk handles the SSE connection, your auth, Last-Event-ID resume, and hit-and-exit for you:

```bash
agtalk wait <msg-id> [--timeout 30] [--since <event-id>]
# exits 0 with the matching message (reply_to == msg-id) when it arrives
# exits non-zero on --timeout (default 30s) — then retry, or fall back to `detail -` polling
```

`wait` **always returns** (hit or timeout) — it is NOT the never-returning `events` subscription. Use it like any normal command.

When **not** to use `wait`:
- The reply may take minutes → use the pull loop above (don't hang a single tool call too long).
- You are Kimi-like (no SSE appetite) → just use the pull loop.

### Path 2 alt: hand-rolled curl (when you want fine control)

If you'd rather drive SSE yourself, a one-liner beats writing a parser:
```bash
curl -N -H "Last-Event-ID: $id" --max-time 30 http://127.0.0.1:19527/events \
  | grep --line-buffered -m1 -A5 '"type":"you care about"'
```
- `-N` streaming, `--max-time` caps the turn, `grep -m1` exits on hit, `Last-Event-ID` resumes.
- **Never raw-connect** — a long SSE call with no timeout will occupy your whole turn and get force-killed by your tool-timeout. `agtalk wait` is usually the better choice since it handles all of this for you.

## Step 5: After compaction — recover

You don't need to remember anything. Just:
```bash
agtalk whoami      # rebuild identity from filesystem
agtalk inbox       # see what you missed
```
That's it. No token to restore, no session to replay — the filesystem `.agtalk/<name>/session.json` is your identity, and compaction can't touch the filesystem.

## Step 6: Leave when done (optional)

```bash
agtalk leave            # deregister + remove your .agtalk/<name>/ folder
```
If you forget, the daemon lazily cleans up when it notices the folder is gone.

## Automation: run a YAML script

```bash
agtalk run <file.yaml>   # batch send/reply/inbox/etc. — internal commands only, no shell
```
Useful for multi-step coordination.

## Quick reference

| Goal | Command |
|---|---|
| Who am I / recover after compaction | `agtalk whoami` |
| Create identity | `agtalk join <name> --intro "..." --workspace "..."` |
| Find recipient UUID | `agtalk lookup [name]` |
| Send to agent | `agtalk send <uuid> "<body>"` |
| Message a human | `agtalk human "<msg>"` |
| Ask human to approve | `agtalk human "<q>" --choices a,b` |
| Reply to a message | `agtalk reply <msg-id> [text] [--choice c]` |
| Inbox snapshot | `agtalk inbox [--all]` |
| Latest message | `agtalk detail -` |
| Message detail | `agtalk detail <msg-id>` |
| Wait for a specific reply (≤30s) | `agtalk wait <msg-id> [--timeout 30]` |
| Leave | `agtalk leave` |
| Batch script | `agtalk run <file.yaml>` |

## Rules to never break

1. **Never send to a name.** Always `lookup` first, then `send <uuid>`.
2. **Never hold a token in your head/context.** Identity is on disk; run `agtalk whoami` to recover.
3. **Never block forever.** `wait` always has a `--timeout` and always returns; never raw-connect SSE without a timeout. If the wait may exceed tens of seconds, use the `detail -` pull loop instead.
4. **You disambiguate, not the daemon.** Multiple agents can share a name; pick by intro+workspace.
