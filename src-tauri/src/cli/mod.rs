//! CLI 子命令与入口。

pub mod client;
pub mod context;
pub mod daemon;

use crate::cli::context::Context;
use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "agtalk")]
#[command(about = "本地 Agent 对话总线")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// daemon 管理
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCommands,
    },
    /// 加入当前工作目录的 agent 网络
    Join {
        name: Option<String>,
        #[arg(short, long)]
        intro: Option<String>,
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// 离开网络
    Leave { name: Option<String> },
    /// 当前身份
    Whoami,
    /// 按 name 查找候选 mailbox
    Lookup { name: Option<String> },
    /// 发送消息到指定 UUID
    Send {
        to: String,
        body: String,
        #[arg(short, long)]
        content_type: Option<String>,
        #[arg(short, long)]
        reply_to: Option<String>,
        #[arg(long)]
        more: bool,
    },
    /// 给 human 发消息（可带审批选项）
    Human {
        body: String,
        #[arg(long, value_delimiter = ',')]
        choices: Vec<String>,
    },
    /// 回复消息
    Reply {
        message_id: String,
        body: String,
        #[arg(short, long)]
        choice: Option<String>,
    },
    /// 收件箱
    Inbox {
        #[arg(short, long)]
        all: bool,
    },
    /// 查看单条消息详情，`detail -` 取最新一条
    Detail { message_id: String },
    /// 阻塞等待消息（SSE 封装，带超时必返回）
    Wait {
        message_id: Option<String>,
        #[arg(short, long)]
        timeout: Option<u64>,
        #[arg(short, long)]
        since: Option<i64>,
    },
}

#[derive(Subcommand)]
enum DaemonCommands {
    Start,
    Stop,
    Restart,
    Status,
}

pub fn run_cli() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Daemon { cmd } => match cmd {
            DaemonCommands::Start => {
                if daemon::is_child_process() {
                    daemon::run_server()
                } else {
                    daemon::start()
                }
            }
            DaemonCommands::Stop => daemon::stop(),
            DaemonCommands::Restart => daemon::restart(),
            DaemonCommands::Status => {
                println!("{}", daemon::status());
                Ok(())
            }
        },
        Commands::Join {
            name,
            intro,
            workspace,
        } => {
            let ctx = Context::pre_join()?;
            client::join(ctx, name, intro, workspace)
        }
        Commands::Leave { name } => {
            let ctx = Context::current()?;
            client::leave(ctx, name)
        }
        Commands::Whoami => {
            let ctx = Context::current()?;
            client::whoami(ctx)
        }
        Commands::Lookup { name } => {
            let ctx = Context::current()?;
            client::lookup(ctx, name)
        }
        Commands::Send {
            to,
            body,
            content_type,
            reply_to,
            more,
        } => {
            let ctx = Context::current()?;
            client::send(ctx, to, body, content_type, reply_to, more)
        }
        Commands::Human { body, choices } => {
            let ctx = Context::current()?;
            client::human(ctx, body, choices)
        }
        Commands::Reply {
            message_id,
            body,
            choice,
        } => {
            let ctx = Context::current()?;
            client::reply(ctx, message_id, body, choice)
        }
        Commands::Inbox { all } => {
            let ctx = Context::current()?;
            client::inbox(ctx, all)
        }
        Commands::Detail { message_id } => {
            let ctx = Context::current()?;
            client::detail(ctx, message_id)
        }
        Commands::Wait {
            message_id,
            timeout,
            since,
        } => {
            let ctx = Context::current()?;
            client::wait(ctx, message_id, timeout, since)
        }
    }
}
