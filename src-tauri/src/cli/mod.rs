//! CLI 子命令与入口。

pub(crate) mod client;
pub mod commands;
pub mod context;
pub mod context_error;
pub mod daemon;
pub mod graph;
pub mod output;
pub mod output_doctor;
pub mod output_misc;
pub mod runner;

use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, run_with_output, CliError};
use crate::proto::{AgentHelpExample, AgentHelpMore, AgentHelpSection, ServerMsg};
use clap::Parser;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "agtalk")]
#[command(about = "本地 Agent 对话总线")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(
    help_template = "{name} {version}\n{about}\n\nUsage: {usage}\n\nCommands:\n{subcommands}\n\nOptions:\n{options}\n\n{after_help}"
)]
#[command(after_help = "Run `agtalk` without arguments for the agent quick guide.")]
struct Cli {
    /// 指定当前命令使用的本地身份（name）
    #[arg(long = "as", global = true)]
    as_name: Option<String>,

    /// 以稳定 JSON 输出结果
    #[arg(long, global = true)]
    json: bool,

    /// 输出完整 agent 使用指南（Markdown）
    #[arg(long, global = true)]
    agent_guide: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

pub(crate) use crate::cli::commands::*;

pub fn run_cli() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    if cli.agent_guide {
        let msg = ServerMsg::AgentGuide {
            markdown: crate::mem::guide::agent_guide_markdown(),
        };
        print_server_msg(json, &msg);
        return ExitCode::SUCCESS;
    }
    run_with_output(json, || run(cli, json))
}

fn agent_help_message() -> ServerMsg {
    let sections = vec![
        AgentHelpSection {
            index: 1,
            title: "Identity".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk id show".to_string(),
                    note: None,
                },
                AgentHelpExample {
                    command: "agtalk id join <name> --intro \"<role>\"".to_string(),
                    note: None,
                },
            ],
        },
        AgentHelpSection {
            index: 2,
            title: "Find target".to_string(),
            examples: vec![AgentHelpExample {
                command: "agtalk id lookup [name]".to_string(),
                note: None,
            }],
        },
        AgentHelpSection {
            index: 3,
            title: "Send / reply / done".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk msg send <address-uuid> \"<body>\"".to_string(),
                    note: None,
                },
                AgentHelpExample {
                    command: "agtalk msg reply <msg-id> \"<body>\"".to_string(),
                    note: None,
                },
                AgentHelpExample {
                    command: "agtalk msg done [msg-id]".to_string(),
                    note: None,
                },
            ],
        },
        AgentHelpSection {
            index: 4,
            title: "Run (preferred send entry)".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk run".to_string(),
                    note: Some("Run .agtalk/<agent>/runs/default.yaml".to_string()),
                },
                AgentHelpExample {
                    command: "agtalk run review".to_string(),
                    note: Some("Run .agtalk/<agent>/runs/review.yaml".to_string()),
                },
            ],
        },
        AgentHelpSection {
            index: 5,
            title: "Read loop".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk msg read".to_string(),
                    note: Some("If inbox_empty: continue normal work.".to_string()),
                },
            ],
        },
        AgentHelpSection {
            index: 6,
            title: "Wait / ask human".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk msg wait [sent-msg-id] --timeout 30".to_string(),
                    note: Some("<sent-msg-id> is the id returned by msg send / msg ask".to_string()),
                },
                AgentHelpExample {
                    command: "agtalk msg ask \"<question>\" --option approve --option reject --timeout 60".to_string(),
                    note: Some("waits up to 300s by default; --no-wait to skip".to_string()),
                },
            ],
        },
        AgentHelpSection {
            index: 7,
            title: "Memory / plan".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk mem plan update --status working --summary \"<current work>\"".to_string(),
                    note: Some("status: idle | working | waiting | blocked".to_string()),
                },
                AgentHelpExample {
                    command: "agtalk mem plan status --target <UUID-or-name>".to_string(),
                    note: Some("read a peer's public summary".to_string()),
                },
                AgentHelpExample {
                    command: "agtalk mem plan show --target <UUID-or-name>".to_string(),
                    note: None,
                },
                AgentHelpExample {
                    command: "agtalk mem pack [topic]".to_string(),
                    note: None,
                },
            ],
        },
        AgentHelpSection {
            index: 8,
            title: "Diagnose".to_string(),
            examples: vec![AgentHelpExample {
                command: "agtalk tool doctor".to_string(),
                note: None,
            }],
        },
    ];

    let more = vec![
        AgentHelpMore {
            command: "agtalk --agent-guide".to_string(),
            description: "full agent guide".to_string(),
        },
        AgentHelpMore {
            command: "agtalk --help".to_string(),
            description: "full command tree".to_string(),
        },
        AgentHelpMore {
            command: "agtalk <cmd> --help".to_string(),
            description: "command flags".to_string(),
        },
    ];

    let text = format_agent_help_text(&more, &sections);

    ServerMsg::AgentHelp {
        text,
        full_docs: "agtalk --agent-guide".to_string(),
        more,
        sections,
    }
}

fn format_agent_help_text(more: &[AgentHelpMore], sections: &[AgentHelpSection]) -> String {
    let mut lines = vec![
        "agtalk agent quick guide".to_string(),
        String::new(),
        "Rules:".to_string(),
        "  - Route only by UUID. Use id lookup to find address.".to_string(),
        "  - name is display only, not routing.".to_string(),
        "  - Prefer agtalk run for reusable or record-worthy sends, even single-message workflows."
            .to_string(),
        "  - If target notify_ready=true, do not wait after send; rely on notify + msg read."
            .to_string(),
        "  - Wait only when target has no reliable notify or you need a short synchronous answer."
            .to_string(),
        "  - Before replying to user, run msg read.".to_string(),
        "  - inbox_empty means no message, not failure.".to_string(),
        "  - Use --json when parsing output.".to_string(),
    ];

    for section in sections {
        lines.push(String::new());
        lines.push(format!("{}. {}", section.index, section.title));
        for ex in &section.examples {
            lines.push(format!("  {}", ex.command));
            if let Some(note) = &ex.note {
                lines.push(format!("  # {}", note));
            }
        }
    }

    lines.push(String::new());
    lines.push("More:".to_string());
    for m in more {
        lines.push(format!("  {:<38} {}", m.command, m.description));
    }

    lines.join("\n")
}

fn print_agent_help(json: bool) {
    let msg = agent_help_message();
    print_server_msg(json, &msg);
}

/// CLI 侧解析 notify 通道。对 `auto` / `plugin:<name>` 会调用插件 `discover`。
/// 失败时返回错误，不自动降级，让 agent 明确知道原因。
/// 对 `auto`，如果没有任何 plugin 就绪，返回 `("none", None)`。
/// `agent_name` 仅在 join 场景传入，用于让插件重命名当前 pane/tab 等上下文。
struct ResolvedNotify {
    channel: String,
    endpoint: Option<serde_json::Value>,
    diagnostics: Vec<crate::proto::NotifyProbe>,
}

fn resolve_notify(notify: &str, agent_name: Option<&str>) -> Result<ResolvedNotify, CliError> {
    let notify = notify.trim();
    if notify.eq_ignore_ascii_case("none") {
        return Ok(ResolvedNotify {
            channel: "none".to_string(),
            endpoint: None,
            diagnostics: Vec::new(),
        });
    }
    if notify.eq_ignore_ascii_case("auto") {
        let mut diagnostics = Vec::new();
        for candidate in ["zellij", "tmux"] {
            let channel_name = format!("plugin:{}", candidate);
            match crate::notify::plugin::PluginChannel::new(candidate) {
                Ok(channel) => match channel.discover_with_name(agent_name) {
                    Ok(endpoint) if endpoint.ready => {
                        return Ok(ResolvedNotify {
                            channel: channel_name,
                            endpoint: Some(endpoint.endpoint),
                            diagnostics: Vec::new(),
                        });
                    }
                    Ok(endpoint) => diagnostics.push(crate::proto::NotifyProbe {
                        name: channel_name,
                        status: "not_ready".to_string(),
                        message: endpoint.message,
                    }),
                    Err(error) => diagnostics.push(crate::proto::NotifyProbe {
                        name: channel_name,
                        status: "error".to_string(),
                        message: error.to_string(),
                    }),
                },
                Err(error) => diagnostics.push(crate::proto::NotifyProbe {
                    name: channel_name,
                    status: "error".to_string(),
                    message: error.to_string(),
                }),
            }
        }
        return Ok(ResolvedNotify {
            channel: "none".to_string(),
            endpoint: None,
            diagnostics,
        });
    }
    if let Some(plugin_name) = notify.strip_prefix("plugin:") {
        let channel = crate::notify::plugin::PluginChannel::new(plugin_name)
            .map_err(|e| CliError::new("notify_plugin_invalid", e.to_string()))?;
        match channel.discover_with_name(agent_name) {
            Ok(endpoint) if endpoint.ready => Ok(ResolvedNotify {
                channel: notify.to_string(),
                endpoint: Some(endpoint.endpoint),
                diagnostics: Vec::new(),
            }),
            Ok(endpoint) => Err(CliError::new(
                "notify_plugin_not_ready",
                format!(
                    "plugin:{} discover 未就绪: {}",
                    plugin_name, endpoint.message
                ),
            )),
            Err(e) => Err(CliError::new(
                "notify_plugin_discover_failed",
                format!("plugin:{} discover 失败: {}", plugin_name, e),
            )),
        }
    } else {
        // 未知通道保持原样，由 daemon 决定如何处理。
        Ok(ResolvedNotify {
            channel: notify.to_string(),
            endpoint: None,
            diagnostics: Vec::new(),
        })
    }
}

fn run(cli: Cli, json: bool) -> Result<(), CliError> {
    let as_name = cli.as_name.as_deref();
    match cli.command {
        Some(cmd) => match cmd {
            Commands::Daemon { cmd } => match cmd {
                DaemonCommands::Start => {
                    if daemon::is_child_process() {
                        daemon::run_server().map_err(CliError::from)
                    } else {
                        daemon::start(json).map_err(CliError::from)
                    }
                }
                DaemonCommands::Stop => daemon::stop(json).map_err(CliError::from),
                DaemonCommands::Restart => daemon::restart(json).map_err(CliError::from),
                DaemonCommands::Status => {
                    let msg = daemon::status_info()?;
                    print_server_msg(json, &msg);
                    Ok(())
                }
            },
            Commands::Id { cmd } => match cmd {
                IdCmd::Join {
                    name,
                    intro,
                    notify,
                } => {
                    let ctx = Context::pre_join().map_err(CliError::from)?;
                    let notify_input = notify.as_deref().unwrap_or("auto");
                    let resolved = resolve_notify(notify_input, name.as_deref())?;
                    client::id::join(
                        ctx,
                        name,
                        intro,
                        resolved.channel,
                        resolved.endpoint,
                        resolved.diagnostics,
                        json,
                    )
                }
                IdCmd::Show => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::id::show(ctx, json)
                }
                IdCmd::Lookup { name } => {
                    // lookup 是“无身份命令”：不需要当前 session，只访问 daemon。
                    let ctx = Context::daemon_only().map_err(CliError::from)?;
                    client::id::lookup(ctx, name, json)
                }
                IdCmd::Leave { address, purge } => {
                    if let Some(address) = address {
                        let ctx = Context::for_address(address.clone()).map_err(CliError::from)?;
                        client::id::leave_by_address(ctx, address, purge, json)
                    } else {
                        let ctx = Context::for_leave(as_name).map_err(CliError::from)?;
                        client::id::leave(ctx, purge, json)
                    }
                }
                IdCmd::Cleanup { execute } => {
                    let ctx = Context::daemon_only().map_err(CliError::from)?;
                    client::id::cleanup(ctx, execute, json)
                }
            },
            Commands::Msg { cmd } => match cmd {
                MsgCmd::Send {
                    to,
                    body,
                    subject,
                    file,
                    notify,
                    no_enter,
                    more,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    let files: Vec<String> = file
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    let send_enter = if no_enter { Some(false) } else { None };
                    client::msg::send(
                        ctx, to, body, subject, files, notify, send_enter, more, json,
                    )
                }
                MsgCmd::Reply {
                    message_id,
                    body,
                    file,
                    notify,
                    no_enter,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    let files: Vec<String> = file
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    let send_enter = if no_enter { Some(false) } else { None };
                    client::msg::reply(ctx, message_id, body, files, notify, send_enter, json)
                }
                MsgCmd::Done {
                    message_id,
                    body,
                    file,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    let files: Vec<String> = file
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    client::msg::done(ctx, message_id, body, files, json)
                }
                MsgCmd::Ask {
                    message,
                    question,
                    option,
                    recommended,
                    single,
                    select_only,
                    no_wait,
                    timeout,
                    notify,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::msg::ask(
                        ctx,
                        message,
                        question,
                        option,
                        recommended,
                        single,
                        select_only,
                        no_wait,
                        timeout,
                        notify,
                        json,
                    )
                }
                MsgCmd::Inbox { all, limit } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::msg::inbox(ctx, all, limit, json)
                }
                MsgCmd::Read { message_id } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::msg::read(ctx, message_id, json)
                }
                MsgCmd::Wait {
                    message_id,
                    timeout,
                    since,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::msg::wait(ctx, message_id, timeout, since, json)
                }
            },
            Commands::Mem { cmd } => {
                let ctx = Context::current(as_name).map_err(CliError::from)?;
                client::mem::dispatch(ctx, cmd, json)
            }
            Commands::Tool { cmd } => {
                let ctx = Context::current(as_name).ok();
                client::tool::dispatch(ctx, cmd, json, as_name)
            }
            Commands::Config { cmd } => {
                if matches!(cmd, ConfigCmd::Gui) {
                    crate::run_gui();
                    return Ok(());
                }
                let ctx = Context::current(as_name).ok();
                client::config::dispatch(ctx, cmd, json)
            }
            Commands::Run { file } => {
                let ctx = Context::current(as_name).ok();
                client::run::run(ctx, file, json)
            }
            Commands::Graph { cmd } => {
                let ctx = Context::current(as_name).map_err(CliError::from)?;
                graph::dispatch(ctx, cmd, json)
            }
        },
        None => {
            print_agent_help(json);
            Ok(())
        }
    }
}
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
