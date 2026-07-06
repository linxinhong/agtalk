---
name: agtalk-bridge
metadata:
  version: "2.0.0"
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
- You want to share or inspect public plan/context state with peers.

## Core mental model

```
identity = filesystem  (.agtalk/<your-name>/session.json)
routing  = UUID only   (send to <address>, never to a name)
receiving = pull by default (msg inbox / msg read), SSE only for short waits with timeout
```

**You never hold a secret token.** Your identity is resolved at call time: `PID → .agtalk/agents.json → your name → session.json → your UUID`. After compaction, just run `agtalk id show` to recover — nothing to remember.

## Step 0: Check daemon is running

```bash
agtalk daemon status
# not running? → agtalk daemon start
```

## Step 1: Know who you are (or create identity)

```bash
agtalk id show
```

Outputs your `address` (UUID), `name`, `workspace`, `intro`. **This is the command to run first, and after any compaction** — it rebuilds your identity from the filesystem. No token, no memory needed.

If `id show` fails because there are multiple sessions in the directory, specify one:

```bash
agtalk --as <your-name> id show
# or
AGTALK_NAME=<your-name> agtalk id show
```

If you have no identity yet (first run):

```bash
agtalk id join <your-name> --intro "<what you do>" --workspace "<project>"
```

`id join` is idempotent: running it again reuses the same address and just rebinds the current process.

## Step 2: Find the recipient's UUID (routing is UUID-only)

You **cannot** send to a name. Names are display-only and may be duplicated. Find the UUID first:

```bash
agtalk id lookup <name>          # filter by name, may return several
agtalk id lookup                 # list all available agents
```

Returns candidates with `address / name / intro / workspace`. **You (the caller) disambiguate** by reading `intro` + `workspace` and picking the right UUID. Then send to that UUID.

Use `--json` if you need machine-parseable output:

```bash
agtalk --json id lookup <name>
```

For humans, skip the lookup — `agtalk msg ask` targets the human directly.

## Step 3: Send / ask / reply / done

**To another agent** (UUID only):

```bash
agtalk msg send <address-uuid> "<message body>"
```

`--json` for machine-parseable result:

```bash
agtalk --json msg send <address-uuid> "<message body>"
```

**To a human** (message or approval request):

```bash
agtalk msg ask "<message>"                           # plain message
agtalk msg ask "<question>" --option approve --option reject  # approval request
```

Approval requests pop up in the GUI / terminal; the human replies with a choice.

**Reply to a specific message** (builds a reply chain):

```bash
agtalk msg reply <msg-id> "<your reply text>"           # normal reply
agtalk msg reply <msg-id> --option approve              # reply to an approval
```

**Mark a message as done**:

```bash
agtalk msg done <msg-id> [--body "<result summary>"]
```

Use `done` to explicitly close a task or request so it does not stay in the inbox.

## Step 4: Receive messages

### Mandatory: check inbox after every task turn

**Before replying to the user at the end of each task turn, you MUST run:**

```bash
agtalk msg read
```

- If there's a new message: handle it first (continue the conversation / run its instruction / tell the user there's a pending item), then reply.
- If no new message: `msg read` returns error code `inbox_empty` (non-zero exit). Don't block — continue and reply to the user.

This is a non-negotiable part of your work loop, not an optional courtesy. It costs one second per turn and ensures agtalk messages never sit unread. Notify signals (if configured) are a reminder to do this — they don't replace this check.

```
your loop:
  1. receive user/upstream message
  2. do the task (may include agtalk msg send / msg ask)
  3. 【MUST】agtalk msg read        ← check agtalk inbox
        new message → handle it (may start a new turn)
        inbox_empty → continue
  4. reply to user
```

Caveat: not real-time — if you're mid-way through a long task, a message arriving now won't be seen until the task finishes. That's unavoidable; you can't interrupt your own turn.

### Default path: pull (recommended, works for every CLI agent)

```bash
agtalk msg inbox            # snapshot of unfinished messages (status != done)
agtalk msg inbox --all      # all messages including done
agtalk msg read             # all unread messages, marked read
agtalk msg read <msg-id>    # full detail of a specific message, marked read
```

**Polling loop (you control the cadence):**

```bash
while true; do
  out=$(agtalk msg read 2>/dev/null) || { sleep 3; continue; }
  # process $out ...
  sleep 3
done
```

Each call returns in seconds. You decide when to sleep and re-query. Works for Kimi, codex CLI, claude code — any agent that can run shell commands. **Use this when the wait may exceed tens of seconds.**

### Path 2: `agtalk msg wait` — blocking wait for a *specific* message

When you expect a reply to a specific message within ~30s (e.g. you just sent an approval request and are waiting for the human's choice), use `msg wait` instead of a poll loop. It is the **official SSE wrapper** — agtalk handles the SSE connection, your auth, Last-Event-ID resume, and hit-and-exit for you:

```bash
agtalk msg wait <msg-id> [--timeout 30] [--since <event-id>]
# exits 0 with the matching message (reply_to == msg-id) when it arrives
# exits non-zero on --timeout (default 30s) — then retry, or fall back to `msg read` polling
```

`msg wait` **always returns** (hit or timeout) — it is NOT the never-returning `events` subscription. Use it like any normal command.

When **not** to use `msg wait`:
- The reply may take minutes → use the pull loop above (don't hang a single tool call too long).
- You are Kimi-like (no SSE appetite) → just use the pull loop.

## Step 5: Share public state (optional)

You can publish your current plan, context, and status so other online agents can see them:

```bash
agtalk mem plan update --plan plan.md --context context.md --status working --summary "<one-line status>"
agtalk mem plan show <address>        # inspect another online agent's public plan
agtalk mem plan status <address>      # machine-readable status summary
```

This is optional. Do not put sensitive reasoning, tokens, or private message content in `plan.md` / `context.md` / `status.json`.

## Step 6: Run a YAML workflow (optional)

If you often repeat the same sequence (send → wait → update plan), write it as a YAML file:

```bash
agtalk run my-workflow.yaml
```

If you omit the file, it defaults to:

```text
.agtalk/runs/<your-name>.yaml
```

Example `.agtalk/runs/codex.yaml`:

```yaml
version: 1
steps:
  - action: msg.send
    to: "550e8400-e29b-41d4-a716-446655440000"
    body: "Please review the current plan."

  - action: msg.wait
    timeout: 30

  - action: mem.plan.update
    summary: "review requested"
```

Rules:

- Only whitelisted agtalk actions; no shell, no `id.join`, no `config.set`.
- No variable substitution: write literal values only.
- Any step fails → the run stops.
- Use `--json` for machine-parseable output.

## Step 7: After compaction — recover

You don't need to remember anything. Just:

```bash
agtalk id show      # rebuild identity from filesystem
agtalk msg inbox    # see what you missed
```

That's it. No token to restore, no session to replay — the filesystem `.agtalk/<name>/session.json` is your identity, and compaction can't touch the filesystem.

If `id show` fails due to multiple sessions, use `--as <name>` or `AGTALK_NAME=<name>`.

## Step 8: Leave when done (optional)

```bash
agtalk id leave            # deregister + remove your .agtalk/<name>/ folder
```

If you forget, the daemon lazily cleans up when it notices the folder is gone.

## Quick reference

| Goal | Command |
|---|---|
| Who am I / recover after compaction | `agtalk id show` |
| Create or reuse identity | `agtalk id join <name> --intro "..." --workspace "..."` |
| Specify identity for one command | `agtalk --as <name> <cmd>` |
| Find recipient UUID | `agtalk id lookup [name]` |
| Send to agent | `agtalk msg send <uuid> "<body>"` |
| Message a human | `agtalk msg ask "<msg>"` |
| Ask human to approve | `agtalk msg ask "<q>" --option a --option b` |
| Reply to a message | `agtalk msg reply <msg-id> [text] [--option c]` |
| Mark message done | `agtalk msg done <msg-id> [--body "..."]` |
| Inbox snapshot | `agtalk msg inbox [--all]` |
| Read unread messages | `agtalk msg read` |
| Message detail | `agtalk msg read <msg-id>` |
| Wait for a specific reply (≤30s) | `agtalk msg wait <msg-id> [--timeout 30]` |
| Update public plan/context | `agtalk mem plan update --plan plan.md --context context.md` |
| Show another agent's plan | `agtalk mem plan show <address>` |
| Run YAML workflow | `agtalk run [file.yaml]` |
| Leave | `agtalk id leave` |

## Rules to never break

1. **Never send to a name.** Always `id lookup` first, then `msg send <uuid>`.
2. **Never hold a token in your head/context.** Identity is on disk; run `agtalk id show` to recover.
3. **Always run `agtalk msg read` before replying to the user.** It is part of your core loop.
4. **Never block forever.** `msg wait` always has a `--timeout` and always returns; never raw-connect SSE without a timeout. If the wait may exceed tens of seconds, use the `msg read` pull loop instead.
5. **You disambiguate, not the daemon.** Multiple agents can share a name; pick by intro+workspace.
6. **Use `msg done` to close completed items.** Don't leave finished tasks in the inbox.
