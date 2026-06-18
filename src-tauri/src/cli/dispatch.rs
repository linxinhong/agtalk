//! CLI 命令分派：agtalk 单一二进制入口。

use super::client::Client;
use anyhow::{anyhow, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};
use comfy_table::{presets::UTF8_FULL, Table};
use serde_json::json;
use std::process::exit;

fn label(text: &str, color: anstyle::AnsiColor) -> String {
    let style = anstyle::Style::new().fg_color(Some(color.into()));
    format!("{}{}{}", style.render(), text, style.render_reset())
}

#[derive(Debug, Parser)]
#[command(
    name = "agtalk",
    version = "0.1.0",
    about = "Agent 与 Agent，Agent 与人协作的本地通信工具",
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(long = "agent-help", help = "面向 AI 的精简用法")]
    agent_help: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(
        about = "向人类发送消息或提问",
        after_long_help = "业务选项（附加在消息正文后，手动解析）:\n  -q, --question <text>     提出问题，可多次出现\n  -o, --option <text>       添加预定义回答选项\n  -o!, --option! <text>     同 -o，并标记为推荐答案\n  --single                  单选，默认多选\n  --select-only             严格选择，禁用自由文本\n  --output <text|json>      输出格式，默认 text"
    )]
    Human(HumanCommand),
    #[command(about = "向 Agent 发送任务或回复")]
    Agent(AgentCommand),
    #[command(about = "加入本地通信网络")]
    Join(JoinCommand),
    #[command(about = "离开本地通信网络")]
    Leave,
    #[command(about = "查看 Agent 自己的信息")]
    Me,
    #[command(about = "列出所有在线参与者")]
    Peers,
    #[command(about = "查看收件箱")]
    Inbox,
    #[command(about = "查看对话列表")]
    Chats,
    #[command(about = "初始化环境")]
    Init,
    #[command(about = "打开设置界面")]
    Settings,
    #[command(about = "管理后台服务")]
    Daemon(DaemonCommand),
}

#[derive(Debug, Args)]
struct HumanCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, help = "消息正文与选项")]
    args: Vec<String>,
}

#[derive(Debug, Args)]
struct AgentCommand {
    #[arg(short = 'n', long = "name", help = "指定 Agent")]
    name: Option<String>,
    #[arg(short = 's', long = "subject", help = "消息主题")]
    subject: Option<String>,
    #[arg(short = 'r', long = "reply-to", help = "回复指定消息")]
    reply_to: Option<String>,
    #[arg(short = 'd', long = "done", help = "标记消息已完成")]
    done: Option<String>,
    #[arg(short = 'i', long = "notify", help = "提醒 Agent 查收消息")]
    notify: bool,
    #[arg(help = "消息正文")]
    message: Vec<String>,
}

#[derive(Debug, Args)]
struct JoinCommand {
    name: String,
    #[arg(long = "intro", help = "Agent 自我介绍")]
    intro: Option<String>,
    #[arg(long = "transport", default_value = "terminal", help = "Agent 的通知方式")]
    transport: String,
}

#[derive(Debug, Args)]
struct DaemonCommand {
    #[arg(default_value = "status", value_parser = ["start", "stop", "restart", "status"])]
    action: String,
}

fn socket_path() -> String {
    crate::paths::socket_path()
}

fn current_participant_name() -> String {
    if let Ok(name) = std::env::var("AGTALK_AGENT_NAME") {
        if !name.trim().is_empty() {
            return name;
        }
    }
    std::fs::read_to_string(crate::paths::current_participant_path())
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "me".into())
}

fn write_current_participant(name: &str) -> Result<()> {
    let path = crate::paths::current_participant_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, name)?;
    Ok(())
}

fn remove_current_participant() -> Result<()> {
    let path = crate::paths::current_participant_path();
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn print_help() {
    println!("agtalk 0.1.0");
    println!("Agent 与 Agent，Agent 与人协作的本地通信工具");
    println!();
    println!("用法:");
    println!("  agtalk <命令> [参数]");
    println!();
    println!("常用命令:");
    println!("  agtalk human <消息>               向人类发送消息或提问");
    println!("  agtalk agent <消息>               向 Agent 发送任务或回复");
    println!("  agtalk join  <name>              加入本地通信网络");
    println!("  agtalk inbox                     查看收件箱");
    println!("  agtalk chats                     查看对话列表");
    println!("  agtalk peers                     列出所有在线参与者");
    println!();
    println!("环境:");
    println!("  agtalk init                      初始化环境");
    println!("  agtalk settings                  打开设置界面");
    println!("  agtalk daemon <action>           管理后台服务：start, stop, restart, status");
    println!();
    println!("帮助:");
    println!("  agtalk --help, -h                显示帮助信息");
    println!("  agtalk --agent-help              面向 AI 的精简用法");
}

pub fn dispatch(argv: &[String]) {
    let cli = Cli::try_parse_from(argv).unwrap_or_else(|err| err.exit());

    if cli.agent_help {
        print_agent_help();
        return;
    }

    let Some(command) = cli.command else {
        let mut cmd = Cli::command();
        let _ = cmd.print_long_help();
        println!();
        exit(1);
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("无法创建 tokio runtime");

    let result = match command {
        Commands::Human(cmd) => {
            let (questions, is_single, select_only, output_json) = parse_human_args(&cmd.args);
            if questions.is_empty() {
                eprintln!("用法: agtalk human <消息> [-q 问题 -o 选项 ...]");
                exit(1);
            }
            if select_only && questions.iter().any(|q| q.options.is_empty()) {
                eprintln!("--select-only 要求每题至少有 -o 选项");
                exit(1);
            }
            rt.block_on(handle_ask_flow("me", &questions, is_single, select_only, output_json))
        }
        Commands::Agent(cmd) => {
            let args = AgentArgs {
                message: cmd.message.join(" "),
                name: cmd.name,
                subject: cmd.subject,
                reply_to: cmd.reply_to,
                done: cmd.done,
                notify: cmd.notify,
            };
            if args.done.is_none() && args.name.is_none() {
                eprintln!("错误: agtalk agent 发送消息需要 -n <name>");
                exit(1);
            }
            if args.done.is_none() && args.message.is_empty() {
                eprintln!("错误: agtalk agent 缺少消息正文");
                exit(1);
            }
            rt.block_on(handle_agent(args))
        }
        Commands::Join(cmd) => {
            let args = JoinArgs {
                name: cmd.name,
                intro: cmd.intro,
                transport: cmd.transport,
            };
            rt.block_on(handle_join(&args))
        }
        Commands::Leave => rt.block_on(handle_leave()),
        Commands::Me => rt.block_on(handle_me()),
        Commands::Peers => rt.block_on(handle_peers()),
        Commands::Inbox => {
            let p = current_participant_name();
            rt.block_on(handle_inbox(&p))
        }
        Commands::Chats => {
            let p = current_participant_name();
            rt.block_on(handle_chats(Some(&p)))
        }
        Commands::Init => rt.block_on(handle_init()),
        Commands::Settings => {
            match std::env::current_exe() {
                Ok(exe) => std::process::Command::new(exe)
                    .arg("gui")
                    .spawn()
                    .map(|_| ())
                    .map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Daemon(cmd) => {
            let args = vec![cmd.action];
            rt.block_on(handle_daemon(&args))
        }
    };

    if let Err(e) = result {
        eprintln!("错误: {}", e);
        exit(1);
    }
}

// ── 参数解析 ──────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
struct QuestionArgs {
    message: String,
    options: Vec<(String, bool)>, // (text, recommended)
}

#[derive(Debug, PartialEq, Eq)]
struct AgentArgs {
    message: String,
    name: Option<String>,
    subject: Option<String>,
    reply_to: Option<String>,
    done: Option<String>,
    notify: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct JoinArgs {
    name: String,
    intro: Option<String>,
    transport: String,
}

fn parse_human_args(argv: &[String]) -> (Vec<QuestionArgs>, bool, bool, bool) {
    let mut message_text = String::new();
    let mut questions: Vec<QuestionArgs> = Vec::new();
    let mut is_single = false;
    let mut select_only = false;
    let mut output_json = false;
    let mut saw_question = false;

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "-q" | "--question" => {
                saw_question = true;
                if i + 1 >= argv.len() { break; }
                i += 1;
                questions.push(QuestionArgs { message: argv[i].clone(), options: vec![] });
            }
            "-o" | "--option" | "-o!" | "--option!" => {
                if i + 1 >= argv.len() { break; }
                i += 1;
                let recommended = arg.ends_with('!');
                if questions.is_empty() {
                    questions.push(QuestionArgs { message: String::new(), options: vec![] });
                }
                if let Some(q) = questions.last_mut() {
                    q.options.push((argv[i].clone(), recommended));
                }
            }
            "--single" => { is_single = true; }
            "--select-only" => { select_only = true; }
            "--output" => {
                if i + 1 >= argv.len() { break; }
                i += 1;
                output_json = argv[i] == "json";
            }
            _ => {
                if !saw_question {
                    if message_text.is_empty() { message_text = arg.clone(); }
                    else { message_text = format!("{} {}", message_text, arg); }
                }
            }
        }
        i += 1;
    }

    if !message_text.is_empty() && (questions.is_empty() || questions[0].message.is_empty()) {
        if questions.is_empty() {
            questions.push(QuestionArgs { message: message_text, options: vec![] });
        } else {
            questions[0].message = message_text;
        }
    }

    (questions, is_single, select_only, output_json)
}

#[allow(dead_code)]
fn parse_agent_args(argv: &[String]) -> Result<AgentArgs> {
    let mut message_parts: Vec<String> = Vec::new();
    let mut name = None;
    let mut subject = None;
    let mut reply_to = None;
    let mut done = None;
    let mut notify = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-n" | "--name" => { i += 1; if i >= argv.len() { return Err(anyhow!("--name 缺少参数")); } name = Some(argv[i].clone()); }
            "-s" | "--subject" => { i += 1; if i >= argv.len() { return Err(anyhow!("--subject 缺少参数")); } subject = Some(argv[i].clone()); }
            "-r" | "--reply-to" => { i += 1; if i >= argv.len() { return Err(anyhow!("--reply-to 缺少参数")); } reply_to = Some(argv[i].clone()); }
            "-d" | "--done" => { i += 1; if i >= argv.len() { return Err(anyhow!("--done 缺少参数")); } done = Some(argv[i].clone()); }
            "-i" | "--notify" => { notify = true; }
            arg => { message_parts.push(arg.to_string()); }
        }
        i += 1;
    }

    if done.is_none() && name.is_none() {
        return Err(anyhow!("agtalk agent 发送消息需要 -n <name>"));
    }
    if done.is_none() && message_parts.is_empty() {
        return Err(anyhow!("agtalk agent 缺少消息正文"));
    }

    Ok(AgentArgs {
        message: message_parts.join(" "),
        name,
        subject,
        reply_to,
        done,
        notify,
    })
}

#[allow(dead_code)]
fn parse_join_args(argv: &[String]) -> JoinArgs {
    let name = argv.first().cloned().unwrap_or_default();
    let mut intro = None;
    let mut transport = "terminal".to_string();

    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--intro" => { i += 1; if i < argv.len() { intro = Some(argv[i].clone()); } }
            "--transport" => { i += 1; if i < argv.len() { transport = argv[i].clone(); } }
            _ => {}
        }
        i += 1;
    }

    JoinArgs { name, intro, transport }
}

// ── Ask 流程 ─────────────────────────────────────

async fn handle_ask_flow(
    to: &str, questions: &[QuestionArgs],
    _is_single: bool, _select_only: bool, output_json: bool,
) -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;

    for (i, q) in questions.iter().enumerate() {
        let choices: Vec<String> = q.options.iter().map(|(t, _)| t.clone()).collect();
        let prefix = if questions.len() > 1 { format!("# Q{}\n", i + 1) } else { String::new() };
        let body = format!("{}{}", prefix, q.message);

        if output_json {
            eprintln!("[agtalk] 等待人类回复: {}", q.message);
        }

        let resp = cli.ask(to, &body, &choices, 300).await?;

        let resp_str = serde_json::to_string_pretty(&resp)?;
        if output_json {
            println!("{}", resp_str);
        } else {
            print_ask_result(&resp_str);
        }
    }

    Ok(())
}

fn print_ask_result(json: &str) {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    match v.get("type").and_then(|t| t.as_str()) {
        Some("ask_response") => {
            let choice = v.get("choice").and_then(|c| c.as_str()).unwrap_or("?");
            println!("[已选择] {}", choice);
            if let Some(reason) = v.get("reason").and_then(|r| r.as_str()) {
                if !reason.is_empty() {
                    println!("[原因] {}", reason);
                }
            }
        }
        Some("ask_timeout") => {
            println!("[超时] 未在规定时间内收到人类回复");
        }
        _ => {
            if let Some(choice) = v
                .get("data")
                .and_then(|d| d.get("choice"))
                .and_then(|c| c.as_str())
            {
                println!("[已选择] {}", choice);
            }
        }
    }
}

fn print_agent_help() {
    println!("agtalk — Agent 与人类/Agent 通信的本地命令。");
    println!();
    println!("常用:");
    println!("  agtalk human \"是否继续？\" -o approve -o reject --output json");
    println!("  agtalk agent \"请 review\" -n reviewer -s \"代码评审\"");
    println!("  agtalk join agent-x --intro \"CLI agent\" --transport terminal");
    println!("  agtalk inbox");
    println!("  agtalk chats");
    println!("  agtalk peers");
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn print_peers(resp: &crate::ipc::ServerMsg) -> Result<()> {
    let crate::ipc::ServerMsg::Ok { data } = resp else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };
    let Some(items) = data.as_array() else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["name", "type", "transport", "status"]);
    for item in items {
        table.add_row(vec![
            item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            item.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            item.get("transport").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            item.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        ]);
    }
    anstream::println!("{}", table);
    Ok(())
}

fn print_chats(resp: &crate::ipc::ServerMsg) -> Result<()> {
    let crate::ipc::ServerMsg::Ok { data } = resp else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };
    let Some(items) = data.as_array() else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["id", "kind", "participants", "unread", "last"]);
    for item in items {
        let participants = item
            .get("participants")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let last = item
            .get("last_message")
            .and_then(|v| v.as_object())
            .and_then(|m| m.get("body"))
            .and_then(|v| v.as_str())
            .map(|s| truncate(s, 36))
            .unwrap_or_default();
        table.add_row(vec![
            item.get("id").and_then(|v| v.as_str()).map(short_id).unwrap_or_default(),
            item.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            participants,
            item.get("unread_count").map(|v| v.to_string()).unwrap_or_default(),
            last,
        ]);
    }
    anstream::println!("{}", table);
    Ok(())
}

// ── 各 handler ────────────────────────────────────

async fn handle_agent(args: AgentArgs) -> Result<()> {
    if let Some(msg_id) = args.done {
        let participant = current_participant_name();
        return handle_done(&msg_id, &participant).await;
    }

    let to = args.name.ok_or_else(|| anyhow!("agtalk agent 发送消息需要 -n <name>"))?;
    let metadata = json!({
        "subject": args.subject,
        "notify": args.notify,
    });
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.send(
        &to,
        &args.message,
        None,
        args.reply_to.as_deref(),
        None,
        Some("text"),
        Some(metadata),
    ).await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_inbox(participant: &str) -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.inbox(participant, None, 50).await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_join(args: &JoinArgs) -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;
    let transport_config = match &args.intro {
        Some(intro) => json!({"intro": intro}).to_string(),
        None => "{}".into(),
    };
    let resp = cli.register(&args.name, "agent", "", &args.transport, &transport_config).await?;
    write_current_participant(&args.name)?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_leave() -> Result<()> {
    let name = current_participant_name();
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.unregister(&name).await?;
    remove_current_participant()?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_me() -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.whoami().await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_peers() -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.list_participants(None).await?;
    print_peers(&resp)?;
    Ok(())
}

async fn handle_chats(participant: Option<&str>) -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.list_conversations(participant).await?;
    print_chats(&resp)?;
    Ok(())
}

async fn handle_done(msg_id: &str, participant: &str) -> Result<()> {
    let mut cli = Client::connect(&socket_path()).await?;
    let resp = cli.done(msg_id, participant).await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_init() -> Result<()> {
    anstream::println!("{} 初始化 agtalk 环境...", label("INFO", anstyle::AnsiColor::Cyan));
    let dir = crate::paths::config_dir();
    std::fs::create_dir_all(&dir)?;
    println!("  配置目录: {:?}", dir);
    match Client::connect(&socket_path()).await {
        Ok(mut cli) => {
            let _ = cli.ping().await?;
            anstream::println!("  daemon: {}", label("运行中", anstyle::AnsiColor::Green));
        }
        Err(_) => anstream::println!("  daemon: {}", label("未运行", anstyle::AnsiColor::Yellow)),
    }
    anstream::println!("{} 初始化完成", label("OK", anstyle::AnsiColor::Green));
    Ok(())
}

async fn handle_daemon(argv: &[String]) -> Result<()> {
    let cmd = argv.first().map(|s| s.as_str()).unwrap_or("status");
    let pid_path = crate::paths::pid_path();
    match cmd {
        "start" => {
            anstream::println!("{} 启动 daemon...", label("INFO", anstyle::AnsiColor::Cyan));
            let exe = std::env::current_exe().unwrap_or_default();
            std::process::Command::new(&exe)
                .arg("__daemon")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            for _ in 0..30 {
                if pid_path.exists() { break; }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            anstream::println!("{} daemon 已启动", label("OK", anstyle::AnsiColor::Green));
            Ok(())
        }
        "stop" => {
            match std::fs::read_to_string(&pid_path) {
                Ok(pid) => {
                    let pid = pid.trim();
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM").arg(pid)
                        .status();
                    for _ in 0..30 {
                        if !pid_path.exists() { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    let _ = std::fs::remove_file(&pid_path);
                    let _ = std::fs::remove_file(socket_path());
                    anstream::println!("{} daemon 已停止", label("OK", anstyle::AnsiColor::Green));
                }
                Err(_) => anstream::println!("{} daemon 未运行（无 PID 文件）", label("WARN", anstyle::AnsiColor::Yellow)),
            }
            Ok(())
        }
        "restart" => {
            if let Ok(pid) = std::fs::read_to_string(&pid_path) {
                let _ = std::process::Command::new("kill")
                    .arg("-TERM").arg(pid.trim())
                    .status();
                for _ in 0..30 {
                    if !pid_path.exists() { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            let _ = std::fs::remove_file(&pid_path);
            let _ = std::fs::remove_file(socket_path());
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let exe = std::env::current_exe().unwrap_or_default();
            std::process::Command::new(&exe)
                .arg("__daemon")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            for _ in 0..30 {
                if pid_path.exists() { break; }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            anstream::println!("{} daemon 已重启", label("OK", anstyle::AnsiColor::Green));
            Ok(())
        }
        "status" => {
            let pid_alive = match std::fs::read_to_string(&pid_path) {
                Ok(pid) => std::process::Command::new("kill")
                    .arg("-0").arg(pid.trim())
                    .status().map(|s| s.success()).unwrap_or(false),
                Err(_) => false,
            };
            match Client::connect(&socket_path()).await {
                Ok(mut cli) => {
                    let _ = cli.ping().await?;
                    anstream::println!("daemon: {} ({})", label("运行中", anstyle::AnsiColor::Green), crate::paths::socket_path());
                }
                Err(_) if pid_alive => {
                    anstream::println!("daemon: {}", label("启动中（PID 存在但未就绪）", anstyle::AnsiColor::Yellow));
                }
                Err(e) => anstream::println!("daemon: {} ({})", label("未运行", anstyle::AnsiColor::Yellow), e),
            }
            Ok(())
        }
        _ => {
            eprintln!("用法: agtalk daemon start|stop|restart|status");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_agent_args, parse_human_args, parse_join_args};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_human_message_with_options() {
        let (questions, _, _, output_json) =
            parse_human_args(&args(&["是否部署？", "-o", "approve", "-o", "reject", "--output", "json"]));
        assert!(output_json);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].message, "是否部署？");
        assert_eq!(questions[0].options, vec![
            ("approve".to_string(), false),
            ("reject".to_string(), false),
        ]);
    }

    #[test]
    fn parse_human_explicit_question() {
        let (questions, _, _, _) = parse_human_args(&args(&["-q", "是否部署？", "-o", "approve"]));
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].message, "是否部署？");
        assert_eq!(questions[0].options, vec![("approve".to_string(), false)]);
    }

    #[test]
    fn parse_human_multiple_questions() {
        let (questions, _, _, _) = parse_human_args(&args(&[
            "-q", "部署环境？", "-o", "staging", "-o!", "production",
            "-q", "是否清缓存？", "-o", "yes", "-o", "no",
        ]));
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].options, vec![
            ("staging".to_string(), false),
            ("production".to_string(), true),
        ]);
        assert_eq!(questions[1].options, vec![
            ("yes".to_string(), false),
            ("no".to_string(), false),
        ]);
    }

    #[test]
    fn parse_human_select_only_without_options_keeps_empty_options_for_validation() {
        let (questions, _, select_only, _) = parse_human_args(&args(&["是否部署？", "--select-only"]));
        assert!(select_only);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].message, "是否部署？");
        assert!(questions[0].options.is_empty());
    }

    #[test]
    fn parse_agent_send() {
        let parsed = parse_agent_args(&args(&[
            "请", "review", "-n", "reviewer", "-s", "代码评审", "-r", "msg-1", "-i",
        ])).unwrap();
        assert_eq!(parsed.message, "请 review");
        assert_eq!(parsed.name.as_deref(), Some("reviewer"));
        assert_eq!(parsed.subject.as_deref(), Some("代码评审"));
        assert_eq!(parsed.reply_to.as_deref(), Some("msg-1"));
        assert!(parsed.notify);
    }

    #[test]
    fn parse_agent_done() {
        let parsed = parse_agent_args(&args(&["-d", "msg-1"])).unwrap();
        assert_eq!(parsed.done.as_deref(), Some("msg-1"));
    }

    #[test]
    fn parse_join() {
        let parsed = parse_join_args(&args(&["agent-x", "--intro", "CLI agent", "--transport", "popup"]));
        assert_eq!(parsed.name, "agent-x");
        assert_eq!(parsed.intro.as_deref(), Some("CLI agent"));
        assert_eq!(parsed.transport, "popup");
    }
}
