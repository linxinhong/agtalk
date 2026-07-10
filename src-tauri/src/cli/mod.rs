//! CLI 子命令与入口。

pub(crate) mod client;
pub mod context;
pub mod context_error;
pub mod daemon;
pub mod output;
pub mod runner;

use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, run_with_output, CliError};
use crate::proto::{AgentHelpExample, AgentHelpMore, AgentHelpSection, ServerMsg};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
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

#[derive(Subcommand)]
enum Commands {
    /// daemon 管理
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCommands,
    },
    /// 身份管理
    Id {
        #[command(subcommand)]
        cmd: IdCmd,
    },
    /// 消息管理
    Msg {
        #[command(subcommand)]
        cmd: MsgCmd,
    },
    /// 记忆/协作状态
    Mem {
        #[command(subcommand)]
        cmd: MemCmd,
    },
    /// 工具/诊断
    Tool {
        #[command(subcommand)]
        cmd: ToolCmd,
    },
    /// 配置
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// 运行 agent 私有 runs 目录中的 YAML 发送/协作 spec
    Run {
        /// run spec 名称（无扩展名）或显式 YAML 文件路径；省略时读取 default.yaml
        #[arg(value_name = "NAME_OR_PATH")]
        file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum DaemonCommands {
    Start,
    Stop,
    Restart,
    Status,
}

#[derive(Subcommand)]
pub(crate) enum IdCmd {
    /// 加入当前工作目录的 agent 网络（幂等：已存在则复用）
    Join {
        name: Option<String>,
        #[arg(short, long)]
        intro: Option<String>,
        /// 打扰通道：auto | none | plugin:<name>（省略时默认 auto，会重新 discover 覆盖旧 session 的 none）
        #[arg(short, long, value_name = "CHANNEL")]
        notify: Option<String>,
    },
    /// 当前身份
    Show,
    /// 按 name 查找候选 mailbox
    Lookup { name: Option<String> },
    /// 离开网络
    Leave {
        /// 按 UUID 精确离开，不再解析当前身份
        #[arg(long, value_name = "ADDRESS")]
        address: Option<String>,
        /// 同时删除本地 .agtalk/<name>/ 目录
        #[arg(long)]
        purge: bool,
    },
    /// 清理无效/未激活身份（默认 dry-run）
    Cleanup {
        #[arg(long)]
        execute: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum MsgCmd {
    /// 发送消息到指定 UUID
    Send {
        to: String,
        body: String,
        #[arg(short, long)]
        subject: Option<String>,
        #[arg(short, long)]
        file: Vec<PathBuf>,
        #[arg(long)]
        notify: Option<bool>,
        /// 不在 notify 注入后自动发送 Enter（默认自动执行）
        #[arg(long)]
        no_enter: bool,
        #[arg(long)]
        more: bool,
    },
    /// 回复消息（message_id 支持短 ID 或完整 UUID）
    Reply {
        #[arg(value_name = "MSG-ID")]
        message_id: String,
        body: String,
        #[arg(short, long)]
        file: Vec<PathBuf>,
        #[arg(long)]
        notify: Option<bool>,
        /// 不在 notify 注入后自动发送 Enter（默认自动执行）
        #[arg(long)]
        no_enter: bool,
    },
    /// 标记消息完成（message_id 支持短 ID 或完整 UUID；省略则取最新一条）
    Done {
        #[arg(value_name = "MSG-ID")]
        message_id: Option<String>,
        #[arg(short, long)]
        body: Option<String>,
        #[arg(short, long)]
        file: Vec<PathBuf>,
    },
    /// 向 human 发询问/审批
    Ask {
        /// 询问/审批的标题或主问题
        message: String,
        /// 追加的细化问题，可多次指定（逗号分隔）
        #[arg(long, value_delimiter = ',')]
        question: Vec<String>,
        /// 选项，可多次指定（逗号分隔），如 approve,reject
        #[arg(long, value_delimiter = ',')]
        option: Vec<String>,
        /// 推荐选项，需在 --option 中
        #[arg(long)]
        recommended: Option<String>,
        /// 仅允许单选
        #[arg(long)]
        single: bool,
        /// 仅返回选择结果，不附加说明
        #[arg(long)]
        select_only: bool,
        /// 发送后阻塞等待回复
        #[arg(long)]
        wait: bool,
        /// 等待超时秒数
        #[arg(short, long)]
        timeout: Option<u64>,
        /// 是否触发 notify 打扰层（默认 true）
        #[arg(long)]
        notify: Option<bool>,
    },
    /// 收件箱
    Inbox {
        #[arg(short, long)]
        all: bool,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// 读取消息；无参数时读取所有未读并标记为 read；message_id 支持短 ID 或完整 UUID
    Read {
        #[arg(value_name = "MSG-ID")]
        message_id: Option<String>,
    },
    /// 阻塞等待消息（SSE 封装，带超时必返回）；message_id 支持短 ID 或完整 UUID
    Wait {
        /// 自己刚发送出去的消息 ID（msg send / msg ask 返回的 id），支持短 ID；省略则等待下一条发给当前 agent 的消息
        #[arg(value_name = "SENT-MSG-ID")]
        message_id: Option<String>,
        #[arg(short, long)]
        timeout: Option<u64>,
        #[arg(short, long)]
        since: Option<i64>,
    },
}

#[derive(Subcommand)]
pub(crate) enum MemCmd {
    /// 协作关系索引（本地 .agtalk/<agent>/relations.json）
    Relation {
        #[command(subcommand)]
        cmd: MemRelationCmd,
    },
    /// 查看 plan
    Plan {
        #[command(subcommand)]
        cmd: MemPlanCmd,
    },
    /// 添加记忆
    Add {
        text: String,
        #[arg(short, long)]
        topic: Option<String>,
        #[arg(short, long)]
        ty: Option<String>,
        #[arg(short, long)]
        title: Option<String>,
        #[arg(short, long, value_delimiter = ',')]
        tag: Vec<String>,
    },
    /// 搜索记忆
    Search {
        query: String,
        #[arg(short, long)]
        topic: Option<String>,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// 查看单条记忆
    Show { id: String },
    /// 列出记忆
    List {
        #[arg(short, long)]
        topic: Option<String>,
    },
    /// 打包 memory/内置 guide 为 prompt
    Pack {
        /// memory topic，例如 agtalk/agent-guide
        topic_pos: Option<String>,
        /// memory topic，例如 agtalk/agent-guide（与位置参数等效，优先）
        #[arg(short, long)]
        topic: Option<String>,
        #[arg(short, long)]
        limit: Option<usize>,
    },
}

#[derive(Subcommand)]
pub(crate) enum MemRelationCmd {
    /// 列出所有协作 peer
    List,
    /// 查看某个 peer 的关系详情
    Show { name_or_address: String },
    /// 更新 peer 的手动字段（role / tags / note）
    Update {
        name_or_address: String,
        #[arg(short, long)]
        role: Option<String>,
        #[arg(short, long, value_delimiter = ',')]
        tag: Vec<String>,
        #[arg(short, long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum MemPlanCmd {
    Show {
        #[arg(short, long)]
        target: Option<String>,
    },
    Update {
        #[arg(short, long)]
        plan: Option<String>,
        #[arg(short, long)]
        context: Option<String>,
        #[arg(short, long)]
        status: Option<String>,
        #[arg(short, long)]
        summary: Option<String>,
    },
    Status {
        #[arg(short, long)]
        target: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ToolCmd {
    /// 诊断当前环境
    Doctor {
        /// 输出完整检查矩阵
        #[arg(long)]
        debug: bool,
    },
    /// 显示版本
    Version,
    /// 显示二进制路径
    Path,
}

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
    /// 显示全部配置
    Show,
    /// 读取配置项
    Get { key: String },
    /// 设置配置项
    Set { key: String, value: String },
    /// 显示配置文件路径
    Path,
}

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
                    command: "agtalk msg ask \"<question>\" --option approve --option reject --wait --timeout 60".to_string(),
                    note: None,
                },
            ],
        },
        AgentHelpSection {
            index: 7,
            title: "Memory / plan".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk mem plan show".to_string(),
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
fn resolve_notify(
    notify: &str,
    agent_name: Option<&str>,
) -> Result<(String, Option<serde_json::Value>), CliError> {
    let notify = notify.trim();
    if notify.eq_ignore_ascii_case("none") {
        return Ok(("none".to_string(), None));
    }
    if notify.eq_ignore_ascii_case("auto") {
        for candidate in ["zellij", "tmux"] {
            let channel_name = format!("plugin:{}", candidate);
            if let Ok(channel) = crate::notify::plugin::PluginChannel::new(candidate) {
                match channel.discover_with_name(agent_name) {
                    Ok(endpoint) if endpoint.ready => {
                        return Ok((channel_name, Some(endpoint.endpoint)));
                    }
                    _ => continue,
                }
            }
        }
        return Ok(("none".to_string(), None));
    }
    if let Some(plugin_name) = notify.strip_prefix("plugin:") {
        let channel = crate::notify::plugin::PluginChannel::new(plugin_name)
            .map_err(|e| CliError::new("notify_plugin_invalid", e.to_string()))?;
        match channel.discover_with_name(agent_name) {
            Ok(endpoint) if endpoint.ready => Ok((notify.to_string(), Some(endpoint.endpoint))),
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
        Ok((notify.to_string(), None))
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
                    let (notify, notify_endpoint) = resolve_notify(notify_input, name.as_deref())?;
                    client::id::join(ctx, name, intro, notify, notify_endpoint, json)
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
                    wait,
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
                        wait,
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
                let ctx = Context::current(as_name).ok();
                client::config::dispatch(ctx, cmd, json)
            }
            Commands::Run { file } => {
                let ctx = Context::current(as_name).ok();
                client::run::run(ctx, file, json)
            }
        },
        None => {
            print_agent_help(json);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_bare_agtalk_has_no_command() {
        let cli = Cli::try_parse_from(["agtalk"]).unwrap();
        assert!(!cli.json);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_json_without_subcommand() {
        let cli = Cli::try_parse_from(["agtalk", "--json"]).unwrap();
        assert!(cli.json);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_id_join_without_notify_defaults_to_none_option() {
        let cli = Cli::try_parse_from(["agtalk", "id", "join", "nora"]).unwrap();
        let cmd = match cli.command {
            Some(Commands::Id { cmd }) => cmd,
            _ => panic!("expected Id join"),
        };
        match cmd {
            IdCmd::Join { notify, .. } => assert!(notify.is_none()),
            _ => panic!("expected Join"),
        }
    }

    #[test]
    fn parse_id_join_notify_none_is_some() {
        let cli =
            Cli::try_parse_from(["agtalk", "id", "join", "nora", "--notify", "none"]).unwrap();
        let cmd = match cli.command {
            Some(Commands::Id { cmd }) => cmd,
            _ => panic!("expected Id join"),
        };
        match cmd {
            IdCmd::Join { notify, .. } => assert_eq!(notify, Some("none".to_string())),
            _ => panic!("expected Join"),
        }
    }

    #[test]
    fn parse_mem_pack_positional_topic() {
        let cli =
            Cli::try_parse_from(["agtalk", "mem", "pack", "agent-learning-handbook"]).unwrap();
        let cmd = match cli.command {
            Some(Commands::Mem { cmd }) => cmd,
            _ => panic!("expected Mem pack"),
        };
        match cmd {
            MemCmd::Pack {
                topic_pos, topic, ..
            } => {
                assert_eq!(topic_pos, Some("agent-learning-handbook".to_string()));
                assert!(topic.is_none());
            }
            _ => panic!("expected Pack"),
        }
    }

    #[test]
    fn parse_mem_pack_named_topic_takes_precedence() {
        let cli = Cli::try_parse_from([
            "agtalk",
            "mem",
            "pack",
            "positional-topic",
            "--topic",
            "named-topic",
        ])
        .unwrap();
        let cmd = match cli.command {
            Some(Commands::Mem { cmd }) => cmd,
            _ => panic!("expected Mem pack"),
        };
        match cmd {
            MemCmd::Pack {
                topic_pos, topic, ..
            } => {
                assert_eq!(topic_pos, Some("positional-topic".to_string()));
                assert_eq!(topic, Some("named-topic".to_string()));
            }
            _ => panic!("expected Pack"),
        }
    }

    #[test]
    fn agent_help_text_is_quick_guide() {
        let msg = agent_help_message();
        let text = match msg {
            ServerMsg::AgentHelp { text, .. } => text,
            _ => panic!("expected AgentHelp"),
        };
        assert!(text.starts_with("agtalk agent quick guide"));
        assert!(text.contains("More:"));
        assert!(text.contains("agtalk --agent-guide"));
        assert!(text.contains("agtalk --help"));
        assert!(text.contains("agtalk <cmd> --help"));
        assert!(!text.contains("agtalk mem pack agtalk/agent-guide"));
        assert!(text.contains("inbox_empty means no message, not failure"));
        assert!(text.contains("Prefer agtalk run for reusable or record-worthy sends"));
        assert!(text.contains("If target notify_ready=true, do not wait after send"));
        assert!(text.contains("Wait only when target has no reliable notify"));
        assert!(text.contains("4. Run (preferred send entry)"));
        assert!(text.contains("agtalk run"));
        assert!(!text.contains("agent-learning-handbook"));
        // quick guide 不展开完整 Commands: 树
        assert!(!text.contains("Commands:"));
    }

    #[test]
    fn agent_help_has_all_sections() {
        let msg = agent_help_message();
        let text = match msg {
            ServerMsg::AgentHelp { text, .. } => text,
            _ => panic!("expected AgentHelp"),
        };
        let sections = [
            "1. Identity",
            "2. Find target",
            "3. Send / reply / done",
            "4. Run (preferred send entry)",
            "5. Read loop",
            "6. Wait / ask human",
            "7. Memory / plan",
            "8. Diagnose",
        ];
        for s in sections {
            assert!(text.contains(s), "missing section: {}", s);
        }
        // full guide entry appears exactly in More
        assert!(text.contains("agtalk --agent-guide"));
    }

    #[test]
    fn agent_help_json_has_structured_sections() {
        let msg = agent_help_message();
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "agent_help");
        assert_eq!(parsed["full_docs"], "agtalk --agent-guide");

        let more = parsed["more"].as_array().unwrap();
        assert_eq!(more.len(), 3);
        assert!(more.iter().any(|m| m["command"] == "agtalk --agent-guide"));
        assert!(more.iter().any(|m| m["command"] == "agtalk --help"));
        assert!(more.iter().any(|m| m["command"] == "agtalk <cmd> --help"));

        let sections = parsed["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 8);
        assert_eq!(sections[0]["title"], "Identity");
        assert_eq!(sections[3]["title"], "Run (preferred send entry)");
        assert_eq!(sections[6]["title"], "Memory / plan");
    }

    #[test]
    fn parse_mem_relation_list() {
        let cli = Cli::try_parse_from(["agtalk", "mem", "relation", "list"]).unwrap();
        let cmd = match cli.command {
            Some(Commands::Mem { cmd }) => cmd,
            _ => panic!("expected Mem relation list"),
        };
        assert!(matches!(
            cmd,
            MemCmd::Relation {
                cmd: MemRelationCmd::List
            }
        ));
    }

    #[test]
    fn parse_mem_relation_show() {
        let cli = Cli::try_parse_from(["agtalk", "mem", "relation", "show", "nora"]).unwrap();
        let cmd = match cli.command {
            Some(Commands::Mem { cmd }) => cmd,
            _ => panic!("expected Mem relation show"),
        };
        match cmd {
            MemCmd::Relation {
                cmd: MemRelationCmd::Show { name_or_address },
            } => assert_eq!(name_or_address, "nora"),
            _ => panic!("expected relation show"),
        }
    }

    #[test]
    fn parse_mem_relation_update() {
        let cli = Cli::try_parse_from([
            "agtalk",
            "mem",
            "relation",
            "update",
            "nora",
            "--role",
            "reviewer",
            "--tag",
            "rust,frontend",
            "--note",
            "good partner",
        ])
        .unwrap();
        let cmd = match cli.command {
            Some(Commands::Mem { cmd }) => cmd,
            _ => panic!("expected Mem relation update"),
        };
        match cmd {
            MemCmd::Relation {
                cmd:
                    MemRelationCmd::Update {
                        name_or_address,
                        role,
                        tag,
                        note,
                    },
            } => {
                assert_eq!(name_or_address, "nora");
                assert_eq!(role, Some("reviewer".to_string()));
                assert_eq!(tag, vec!["rust".to_string(), "frontend".to_string()]);
                assert_eq!(note, Some("good partner".to_string()));
            }
            _ => panic!("expected relation update"),
        }
    }

    #[test]
    fn parse_agent_guide_flag_without_subcommand() {
        let cli = Cli::try_parse_from(["agtalk", "--agent-guide"]).unwrap();
        assert!(cli.agent_guide);
        assert!(cli.command.is_none());
    }

    #[test]
    fn agent_guide_markdown_is_non_empty() {
        let markdown = crate::mem::guide::agent_guide_markdown();
        assert!(!markdown.is_empty());
        assert!(markdown.contains("agtalk --agent-guide"));
        assert!(markdown.contains("inbox_empty"));
        assert!(markdown.contains("agtalk run"));
        assert!(markdown.contains("notify_ready"));
        assert!(markdown.contains("history.jsonl"));
    }
}
