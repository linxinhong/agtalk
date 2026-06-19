# Repository Guidelines

## Product Goal

agtalk is a **local conversation bus for agent-agent and agent-human communication**.
It is not a general chat app. The daemon acts as the source of truth for participants,
conversations, messages, delivery state, and human approval workflows.

Primary use cases:

- CLI agent asks a human for confirmation before risky actions.
- Human sends instructions to a local agent from GUI or CLI.
- Multiple local agents exchange structured task messages.
- GUI provides inbox, conversation history, and approval panels.

## Design Principles

- Daemon is the source of truth. CLI and GUI are thin clients.
- Every message must be persisted before delivery.
- Transports are pluggable (terminal, popup, gui, process).
- Agent-human approval is a first-class workflow.

## Project Structure & Module Organization

Single Rust crate (`src-tauri/`) + Vue 3 frontend (`src/`). The binary `agtalk` dispatches on argv[1]: `gui` launches the Tauri GUI; CLI subcommands (`human`, `agent`, `join`, `peers`, `chats`, ...); `daemon start` spawns `agtalk __daemon`.

```
src-tauri/src/
  main.rs          Entry point — argv dispatch
  lib.rs           Library root
  ipc.rs           Daemon IPC protocol (ClientMsg / ServerMsg, newline-JSON over Unix socket)
  storage.rs       SQLite schema, migration, query methods (participants, conversations, messages)
  transport.rs     Transport trait + TerminalTransport / PopupTransport stubs
  server.rs        Unix socket IPC server (daemon mode)
  commands.rs      Tauri commands — bridge between Vue frontend and daemon IPC
  tests.rs         Unit tests for storage layer
  cli/
    mod.rs
    dispatch.rs    CLI subcommand routing
    client.rs      Daemon IPC client (connect, send, inbox, etc.)
src/
  views/           ConversationView.vue, SettingsView.vue
  lib/             ipc.ts (Tauri invoke wrappers), types.ts (TS interfaces matching Rust models)
  styles/main.css  CSS with light/dark theme vars
```

## Build, Test, and Development Commands

```bash
cargo build                 # Debug binary → target/debug/agtalk
cargo build --release       # Release binary
cargo check                 # Fast compile-check (no binary)
cargo test --bin agtalk     # Run storage-layer unit tests
pnpm install                # Install frontend dependencies
pnpm dev                    # Vite dev server (localhost:1421)
pnpm build                  # Production frontend build (vue-tsc + vite)
cargo clippy                # Lint Rust code
make release                # Build frontend + release binary
make deploy                 # Build release binary and copy to ~/.local/bin/agtalk
pnpm deploy                 # Same as `make deploy`
```

Daemon must be running for CLI/GUI commands that talk to it:

```bash
./target/debug/agtalk daemon start   # Spawns daemon as background process
./target/debug/agtalk daemon status  # Check if running
./target/debug/agtalk gui            # Launch Tauri GUI (dev: pnpm tauri dev -- gui)
```

CLI command reference (`human` / `agent` / `join` / `peers` / `chats` / `inbox` / ...): see [docs/commands.md](docs/commands.md).

## Coding Style & Naming Conventions

- **Rust**: standard `rustfmt` defaults. `cargo clippy` must pass. Module names `snake_case`, types `PascalCase`.
- **IPC protocol**: `serde` with `#[serde(tag = "type", rename_all = "snake_case")]` for message enums.
- **TypeScript**: `vue-tsc --noEmit` must pass (strict mode). Prefer explicit types from `src/lib/types.ts`.
- **CSS**: CSS custom properties for theming (`--bg`, `--text`, etc.). Support `prefers-color-scheme: dark`.
- **File naming**: Rust modules `snake_case.rs`, Vue components `PascalCase.vue`.

## Testing Guidelines

- **Framework**: `cargo test` with `rusqlite` in-memory databases (`Storage::open_memory()`).
- **Test location**: `src-tauri/src/tests.rs` in a `#[cfg(test)] mod tests` block.
- **Naming**: `test_<feature_name>` (e.g., `test_send_message_and_list`).
- **Isolation**: each test creates its own `Storage` instance; no shared state.
- **Run**: `cargo test --bin agtalk` (tests are in bin crate, not lib).

## Commit & Pull Request Guidelines

- Commits are in Chinese. Keep messages concise and descriptive.
- One logical change per commit.
- PRs should include: what changed, why, and how to test.
- Ensure `cargo check`, `cargo test --bin agtalk`, and `pnpm build` pass before submitting.

## Architecture Overview

Three-process model (same pattern as AskHuman):

1. **Daemon** (`agtalk __daemon`): Unix socket IPC server. Manages SQLite (`~/.config/agtalk/agtalk.db`), participant registry, message routing, transport dispatch.
2. **CLI** (`agtalk <subcommand>`): Thin IPC client connecting to daemon socket.
3. **GUI** (`agtalk gui`): Tauri window with Vue 3 frontend. Connects to daemon via Tauri commands.

Message flow: `Client → daemon → SQLite + transport delivery → recipient`. Status: `pending → delivered → read → done`.

For detailed product design and roadmap, see [docs/DESIGN.md](docs/DESIGN.md). For CLI command reference, see [docs/commands.md](docs/commands.md).
