//! CLI 子命令与入口。

pub(crate) mod client;
pub mod context;
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
    /// 运行 YAML 编排
    Run { file: Option<PathBuf> },
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
        #[arg(short, long)]
        workspace: Option<String>,
        /// 打扰通道：auto | none | zellij | tmux | gui | webhook:<url>
        #[arg(short, long, default_value = "auto", value_name = "CHANNEL")]
        notify: String,
    },
    /// 当前身份
    Show,
    /// 按 name 查找候选 mailbox
    Lookup { name: Option<String> },
    /// 离开网络
    Leave {
        #[arg(long)]
        purge: bool,
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
        #[arg(long)]
        more: bool,
    },
    /// 回复消息
    Reply {
        message_id: String,
        body: String,
        #[arg(short, long)]
        file: Vec<PathBuf>,
        #[arg(long)]
        notify: Option<bool>,
    },
    /// 标记消息完成
    Done {
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
    },
    /// 收件箱
    Inbox {
        #[arg(short, long)]
        all: bool,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// 读取消息；无参数时读取所有未读并标记为 read
    Read { message_id: Option<String> },
    /// 阻塞等待消息（SSE 封装，带超时必返回）
    Wait {
        message_id: Option<String>,
        #[arg(short, long)]
        timeout: Option<u64>,
        #[arg(short, long)]
        since: Option<i64>,
    },
    /// 下载附件（尚未实现）
    #[command(hide = true)]
    Attachment { attachment_id: String },
}

#[derive(Subcommand)]
pub(crate) enum MemCmd {
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
    /// daemon 生命周期（尚未实现）
    #[command(hide = true)]
    Daemon {
        #[arg(short, long)]
        action: Option<String>,
    },
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
                    command: "agtalk id join <name> --intro \"<role>\" --workspace \"<project>\""
                        .to_string(),
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
            title: "Read loop".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk msg read".to_string(),
                    note: Some("If inbox_empty: continue normal work.".to_string()),
                },
            ],
        },
        AgentHelpSection {
            index: 5,
            title: "Wait / ask human".to_string(),
            examples: vec![
                AgentHelpExample {
                    command: "agtalk msg wait [msg-id] --timeout 30".to_string(),
                    note: None,
                },
                AgentHelpExample {
                    command: "agtalk msg ask \"<question>\" --option approve --option reject --wait --timeout 60".to_string(),
                    note: None,
                },
            ],
        },
        AgentHelpSection {
            index: 6,
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
            index: 7,
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
                    workspace,
                    notify,
                } => {
                    let ctx = Context::pre_join().map_err(CliError::from)?;
                    client::id::join(ctx, name, intro, workspace, notify, json)
                }
                IdCmd::Show => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::id::show(ctx, json)
                }
                IdCmd::Lookup { name } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::id::lookup(ctx, name, json)
                }
                IdCmd::Leave { purge } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::id::leave(ctx, purge, json)
                }
            },
            Commands::Msg { cmd } => match cmd {
                MsgCmd::Send {
                    to,
                    body,
                    subject,
                    file,
                    notify,
                    more,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    let files: Vec<String> = file
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    client::msg::send(ctx, to, body, subject, files, notify, more, json)
                }
                MsgCmd::Reply {
                    message_id,
                    body,
                    file,
                    notify,
                } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    let files: Vec<String> = file
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    client::msg::reply(ctx, message_id, body, files, notify, json)
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
                MsgCmd::Attachment { attachment_id } => {
                    let ctx = Context::current(as_name).map_err(CliError::from)?;
                    client::msg::attachment(ctx, attachment_id, json)
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
            "4. Read loop",
            "5. Wait / ask human",
            "6. Memory / plan",
            "7. Diagnose",
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
        assert_eq!(sections.len(), 7);
        assert_eq!(sections[0]["title"], "Identity");
        assert_eq!(sections[5]["title"], "Memory / plan");
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
    }
}
