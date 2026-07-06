//! CLI 子命令与入口。

pub(crate) mod client;
pub mod context;
pub mod daemon;
pub mod output;

use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, run_with_output, CliError};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "agtalk")]
#[command(about = "本地 Agent 对话总线")]
struct Cli {
    /// 指定当前命令使用的本地身份（name）
    #[arg(long = "as", global = true)]
    as_name: Option<String>,

    /// 以稳定 JSON 输出结果
    #[arg(long, global = true)]
    json: bool,

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
    /// 记忆/协作状态（预留）
    Mem {
        #[command(subcommand)]
        cmd: MemCmd,
    },
    /// 工具/诊断
    Tool {
        #[command(subcommand)]
        cmd: ToolCmd,
    },
    /// 配置（预留）
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// 运行 YAML 编排（预留）
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
        #[arg(short, long, default_value = "auto")]
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
        message: String,
        #[arg(long, value_delimiter = ',')]
        question: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        option: Vec<String>,
        #[arg(long)]
        recommended: Option<String>,
        #[arg(long)]
        single: bool,
        #[arg(long)]
        select_only: bool,
        #[arg(long)]
        wait: bool,
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
    /// 读取消息，`read -` 取最新一条
    Read { message_id: String },
    /// 阻塞等待消息（SSE 封装，带超时必返回）
    Wait {
        message_id: Option<String>,
        #[arg(short, long)]
        timeout: Option<u64>,
        #[arg(short, long)]
        since: Option<i64>,
    },
    /// 下载附件（预留）
    Attachment { attachment_id: String },
}

#[derive(Subcommand)]
pub(crate) enum MemCmd {
    /// 查看 plan（预留）
    Plan {
        #[command(subcommand)]
        cmd: MemPlanCmd,
    },
    /// 添加记忆（预留）
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
    /// 搜索记忆（预留）
    Search {
        query: String,
        #[arg(short, long)]
        topic: Option<String>,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// 查看单条记忆（预留）
    Show { id: String },
    /// 列出记忆（预留）
    List {
        #[arg(short, long)]
        topic: Option<String>,
    },
    /// 打包记忆为 prompt（预留）
    Pack {
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
    /// daemon 生命周期（预留 action）
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
    /// 显示全部配置（预留）
    Show,
    /// 读取配置项（预留）
    Get { key: String },
    /// 设置配置项（预留）
    Set { key: String, value: String },
    /// 显示配置文件路径（预留）
    Path,
}

pub fn run_cli() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    run_with_output(json, || run(cli, json))
}

fn run(cli: Cli, json: bool) -> Result<(), CliError> {
    let as_name = cli.as_name.as_deref();
    match cli.command {
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
    }
}
