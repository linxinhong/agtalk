//! CLI 命令分派：agtalk 单一二进制入口。

use super::client::Client;
use super::daemon;
use super::help;
use crate::ipc::ServerMsg;
use crate::{identity, notify, session, workspace};
use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
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
    #[command(about = "从 YAML 文件执行 agtalk 命令")]
    Run(RunCommand),
    #[command(about = "回复审批请求")]
    Reply(ReplyCommand),
    #[command(about = "加入本地通信网络")]
    Join(JoinCommand),
    #[command(about = "接管已有 peer 身份")]
    Attach(AttachCommand),
    #[command(about = "离开本地通信网络")]
    Leave(LeaveCommand),
    #[command(about = "清理已退役 session")]
    Cleanup(CleanupCommand),
    #[command(about = "查看 Agent 自己的信息")]
    Me,
    #[command(about = "列出所有在线参与者")]
    Peers(PeersArgs),
    #[command(about = "查看待处理消息（待办中心）")]
    Inbox(InboxArgs),
    #[command(about = "查看消息详情")]
    Detail(DetailArgs),
    #[command(about = "等待审批结果")]
    Wait(WaitArgs),
    #[command(about = "查看附件全文")]
    Attachment(AttachmentArgs),
    #[command(about = "查看对话列表")]
    Chats,
    #[command(about = "管理全局配置")]
    Config(ConfigArgs),
    #[command(about = "初始化环境")]
    Init,
    #[command(about = "打开设置界面")]
    Settings,
    #[command(about = "管理后台服务")]
    Daemon(DaemonCommand),
    #[command(about = "长期知识库", subcommand)]
    Mem(MemCommand),
}

#[derive(Debug, Subcommand)]
enum MemCommand {
    #[command(about = "Topic 管理", subcommand)]
    Topic(MemTopicCommand),
    #[command(about = "添加 memory")]
    Add(MemAddArgs),
    #[command(about = "查看 memory 详情")]
    Show(MemShowArgs),
    #[command(about = "更新 memory")]
    Update(MemUpdateArgs),
    #[command(about = "归档 memory")]
    Archive(MemArchiveArgs),
    #[command(about = "从消息/附件提升为 memory")]
    Promote(MemPromoteArgs),
    #[command(about = "搜索 memory")]
    Search(MemSearchArgs),
    #[command(about = "生成 Memory Pack")]
    Pack(MemPackArgs),
    #[command(about = "列出 memory")]
    List(MemListArgs),
}

#[derive(Debug, Subcommand)]
enum MemTopicCommand {
    #[command(about = "添加 topic")]
    Add(MemTopicAddArgs),
    #[command(about = "列出 topics")]
    List(MemTopicListArgs),
    #[command(about = "查看 topic")]
    Show(MemTopicShowArgs),
    #[command(about = "更新 topic")]
    Update(MemTopicUpdateArgs),
}

#[derive(Debug, Args)]
struct MemTopicAddArgs {
    slug: String,
    #[arg(short = 't', long = "title", help = "topic 标题")]
    title: String,
    #[arg(short = 's', long = "summary", help = "摘要")]
    summary: Option<String>,
    #[arg(short = 'a', long = "alias", help = "别名，可多次使用")]
    alias: Vec<String>,
    #[arg(short = 'p', long = "priority", default_value = "3", help = "优先级 1-5")]
    priority: i32,
}

#[derive(Debug, Args)]
struct MemTopicListArgs {
    #[arg(long, help = "包含已归档")]
    all: bool,
}

#[derive(Debug, Args)]
struct MemTopicShowArgs {
    slug: String,
}

#[derive(Debug, Args)]
struct MemTopicUpdateArgs {
    slug: String,
    #[arg(short = 't', long = "title", help = "topic 标题")]
    title: Option<String>,
    #[arg(short = 's', long = "summary", help = "摘要")]
    summary: Option<String>,
    #[arg(short = 'a', long = "alias", help = "别名，可多次使用")]
    alias: Vec<String>,
    #[arg(short = 'p', long = "priority", help = "优先级 1-5")]
    priority: Option<i32>,
    #[arg(long, help = "归档 topic")]
    archive: bool,
}

#[derive(Debug, Args)]
struct MemAddArgs {
    content: String,
    #[arg(short = 't', long = "type", default_value = "fact", help = "记忆类型: fact|decision|rule|procedure|issue|snippet|preference|summary|note|context")]
    item_type: String,
    #[arg(short = 'T', long = "title", help = "标题；省略时使用内容前 50 字")]
    title: Option<String>,
    #[arg(short = 's', long = "summary", help = "摘要")]
    summary: Option<String>,
    #[arg(short = 'p', long = "topic", help = "topic slug，可多次使用")]
    topic: Vec<String>,
    #[arg(short = 'g', long = "tags", help = "标签，逗号分隔")]
    tags: Option<String>,
    #[arg(short = 'i', long = "importance", default_value = "3", help = "重要性 1-5")]
    importance: i32,
    #[arg(short = 'c', long = "confidence", default_value = "medium", help = "low|medium|high")]
    confidence: String,
    #[arg(
        short = 'S',
        long = "scope",
        default_value = "workspace",
        value_parser = ["global", "workspace"],
        help = "global|workspace"
    )]
    scope: String,
}

#[derive(Debug, Args)]
struct MemShowArgs {
    mem_id: String,
}

#[derive(Debug, Args)]
struct MemUpdateArgs {
    mem_id: String,
    #[arg(short = 'c', long = "content", help = "内容")]
    content: Option<String>,
    #[arg(short = 'T', long = "title", help = "标题")]
    title: Option<String>,
    #[arg(short = 's', long = "summary", help = "摘要")]
    summary: Option<String>,
    #[arg(short = 'p', long = "topic", help = "topic slug，可多次使用")]
    topic: Vec<String>,
    #[arg(short = 'g', long = "tags", help = "标签，逗号分隔")]
    tags: Option<String>,
    #[arg(short = 'i', long = "importance", help = "重要性 1-5")]
    importance: Option<i32>,
    #[arg(short = 'S', long = "status", help = "active|archived|superseded|draft")]
    status: Option<String>,
}

#[derive(Debug, Args)]
struct MemArchiveArgs {
    mem_id: String,
}

#[derive(Debug, Args)]
struct MemPromoteArgs {
    source_ref: String,
    #[arg(short = 'y', long = "source-type", default_value = "message", help = "message|artifact")]
    source_type: String,
    #[arg(short = 't', long = "type", default_value = "fact", help = "记忆类型: fact|decision|rule|procedure|issue|snippet|preference|summary|note|context")]
    item_type: String,
    #[arg(short = 'T', long = "title", help = "标题")]
    title: Option<String>,
    #[arg(short = 's', long = "summary", help = "摘要")]
    summary: Option<String>,
    #[arg(short = 'p', long = "topic", help = "topic slug，可多次使用")]
    topic: Vec<String>,
    #[arg(short = 'g', long = "tags", help = "标签，逗号分隔")]
    tags: Option<String>,
    #[arg(short = 'i', long = "importance", default_value = "3", help = "重要性 1-5")]
    importance: i32,
    #[arg(short = 'c', long = "confidence", default_value = "medium", help = "low|medium|high")]
    confidence: String,
}

#[derive(Debug, Args)]
struct MemSearchArgs {
    query: Option<String>,
    #[arg(short = 'p', long = "topic", help = "topic slug，可多次使用")]
    topic: Vec<String>,
    #[arg(short = 't', long = "type", help = "记忆类型: fact|decision|rule|procedure|issue|snippet|preference|summary|note|context")]
    item_type: Option<String>,
    #[arg(
        short = 'S',
        long = "scope",
        value_parser = ["global", "workspace"],
        help = "global|workspace"
    )]
    scope: Option<String>,
    #[arg(short = 'l', long = "limit", default_value = "20", help = "返回条数")]
    limit: u32,
}

#[derive(Debug, Args)]
struct MemPackArgs {
    topic_slug: String,
    #[arg(short = 'l', long = "limit", default_value = "10", help = "包含条数")]
    limit: u32,
}

#[derive(Debug, Args)]
struct MemListArgs {
    #[arg(short = 'p', long = "topic", help = "按 topic slug 过滤")]
    topic: Option<String>,
    #[arg(short = 't', long = "type", help = "按记忆类型过滤")]
    item_type: Option<String>,
    #[arg(
        short = 'S',
        long = "scope",
        value_parser = ["global", "workspace"],
        help = "global|workspace"
    )]
    scope: Option<String>,
    #[arg(
        short = 's',
        long = "status",
        default_value = "active",
        value_parser = ["active", "archived", "all"],
        help = "active|archived|all"
    )]
    status: String,
    #[arg(short = 'l', long = "limit", default_value = "20", help = "返回条数")]
    limit: u32,
}

#[derive(Debug, Args)]
struct HumanCommand {
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "消息正文与选项"
    )]
    args: Vec<String>,
}

#[derive(Debug, Args)]
struct RunCommand {
    #[arg(help = "YAML runner 文件路径；省略时读取 .agtalk/runs/<当前agent-name>.yaml")]
    file: Option<String>,
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
    #[arg(short = 'f', long = "file", help = "附件路径，可多次使用")]
    file: Vec<String>,
    #[arg(short = 'i', long = "notify", help = "提醒 Agent 查收消息")]
    notify: bool,
    #[arg(long = "no-enter", help = "提醒时不自动发送回车")]
    no_enter: bool,
    #[arg(long = "with-mem", help = "注入指定 topic 的 Memory Pack")]
    with_mem: Option<String>,
    #[arg(long = "with-mem-limit", default_value = "5", help = "Memory Pack 最大条数")]
    with_mem_limit: u32,
    #[arg(help = "消息正文")]
    message: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ReplyCommand {
    pub(crate) msg_id: String,
    pub(crate) choice: String,
    #[arg(short = 'r', long = "reason", help = "附带说明")]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Args)]
struct JoinCommand {
    name: String,
    #[arg(long = "intro", help = "Agent 自我介绍")]
    intro: Option<String>,
    #[arg(long = "role", default_value = "agent", help = "Agent 角色")]
    role: String,
    #[arg(
        long = "transport",
        default_value = "terminal",
        help = "Agent 的通知方式"
    )]
    transport: String,
    #[arg(long = "takeover", help = "接管同 endpoint 上的旧 active session")]
    takeover: bool,
    #[arg(long = "print-env", help = "只输出 export AGTALK_NAME=...")]
    print_env: bool,
}

#[derive(Debug, Args)]
struct AttachCommand {
    name: String,
    #[arg(long = "takeover", help = "接管同 endpoint 上的旧 active session")]
    takeover: bool,
    #[arg(long = "print-env", help = "只输出 export AGTALK_NAME=...")]
    print_env: bool,
}

#[derive(Debug, Args)]
struct LeaveCommand {
    #[arg(long = "purge", help = "同时删除本地 session 凭证文件")]
    purge: bool,
}

#[derive(Debug, Args)]
struct CleanupCommand {
    #[arg(long = "dry-run", help = "仅列出会被清理的 session，不实际删除")]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct DaemonCommand {
    #[arg(default_value = "status", value_parser = ["start", "stop", "restart", "status"])]
    action: String,
}

// ── mem 长期知识库：CLI 与 YAML Runner 共用 ─────────────

async fn with_mem_client<F, Fut>(f: F) -> Result<()>
where
    F: FnOnce(Option<String>, Client) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let (workspace_id, session_id, token) = load_workspace_and_session()?;
    let client = Client::connect_and_auth(&socket_path(), &session_id, &token).await?;
    f(workspace_id, client).await
}

pub(crate) async fn run_mem_topic_add(
    slug: String,
    title: String,
    summary: Option<String>,
    aliases: Vec<String>,
    priority: i32,
) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let resp = client
            .mem_topic_add(
                workspace_id.as_deref(),
                &slug,
                &title,
                summary.as_deref(),
                &aliases,
                priority,
            )
            .await?;
        print_json_or_table(resp, |topic: crate::storage::MemTopic| {
            anstream::println!("已创建 topic: {}", topic.slug);
            anstream::println!("标题: {}", topic.title);
            if let Some(summary) = &topic.summary {
                anstream::println!("摘要: {}", summary);
            }
            if !topic.aliases.is_empty() {
                anstream::println!("别名: {}", topic.aliases.join(", "));
            }
        });
        Ok(())
    })
    .await
}

pub(crate) async fn run_mem_topic_list(all: bool) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let status = if all { None } else { Some("active") };
        let resp = client.mem_topic_list(workspace_id.as_deref(), status).await?;
        print_json_or_table(resp, |topics: Vec<crate::storage::MemTopic>| {
            if topics.is_empty() {
                anstream::println!("暂无 topic");
                return;
            }
            let mut table = Table::new();
            table.set_header(vec!["slug", "标题", "状态", "优先级"]);
            for t in topics {
                table.add_row(vec![t.slug, t.title, t.status, t.priority.to_string()]);
            }
            anstream::println!("{}", table);
        });
        Ok(())
    })
    .await
}

pub(crate) async fn run_mem_topic_show(slug: String) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let resp = client.mem_topic_show(workspace_id.as_deref(), &slug).await?;
        print_json_or_table(resp, |topic: crate::storage::MemTopic| {
            anstream::println!("slug: {}", topic.slug);
            anstream::println!("标题: {}", topic.title);
            if let Some(summary) = &topic.summary {
                anstream::println!("摘要: {}", summary);
            }
            anstream::println!("状态: {}", topic.status);
            anstream::println!("优先级: {}", topic.priority);
            if !topic.aliases.is_empty() {
                anstream::println!("别名: {}", topic.aliases.join(", "));
            }
        });
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_mem_topic_update(
    slug: String,
    title: Option<String>,
    summary: Option<String>,
    aliases: Vec<String>,
    priority: Option<i32>,
    archive: bool,
) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let status = if archive { Some("archived".to_string()) } else { None };
        let aliases = if aliases.is_empty() { None } else { Some(aliases) };
        let resp = client
            .mem_topic_update(
                workspace_id.as_deref(),
                &slug,
                title.as_deref(),
                summary.as_deref(),
                aliases,
                priority,
                status.as_deref(),
            )
            .await?;
        print_json_or_table(resp, |topic: crate::storage::MemTopic| {
            anstream::println!("已更新 topic: {}", topic.slug);
        });
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_mem_add(
    content: String,
    item_type: String,
    title: Option<String>,
    summary: Option<String>,
    topics: Vec<String>,
    tags: Option<String>,
    importance: i32,
    confidence: String,
    _scope: String,
) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let title = title.unwrap_or_else(|| {
            let mut t = content.clone();
            if t.chars().count() > 50 {
                t = t.chars().take(50).collect::<String>() + "...";
            }
            t
        });
        let tags = parse_tags(tags.as_deref());
        let resp = client
            .mem_add(
                workspace_id.as_deref(),
                &item_type,
                &title,
                &content,
                summary.as_deref(),
                &topics,
                &tags,
                importance,
                &confidence,
                "manual",
                "manual",
            )
            .await?;
        print_json_or_table(resp, |item: crate::storage::MemItem| {
            anstream::println!("已创建 memory: {}", item.id);
        });
        Ok(())
    })
    .await
}

pub(crate) async fn run_mem_show(mem_id: String) -> Result<()> {
    with_mem_client(|_workspace_id, mut client| async move {
        let resp = client.mem_show(&mem_id).await?;
        print_json_or_table(resp, |item: crate::storage::MemItem| {
            anstream::println!("id: {}", item.id);
            anstream::println!("type: {}", item.item_type);
            anstream::println!("title: {}", item.title);
            anstream::println!("scope: {}", item.scope);
            anstream::println!("status: {}", item.status);
            anstream::println!("confidence: {}", item.confidence);
            anstream::println!("importance: {}", item.importance);
            if !item.topics.is_empty() {
                anstream::println!(
                    "topics: {}",
                    item.topics.iter().map(|t| t.slug.as_str()).collect::<Vec<_>>().join(", ")
                );
            }
            if !item.tags.is_empty() {
                anstream::println!("tags: {}", item.tags.join(", "));
            }
            if !item.sources.is_empty() {
                anstream::println!("sources:");
                for s in &item.sources {
                    anstream::println!("  - type: {} / ref: {}", s.source_type, s.source_ref);
                    if let Some(quote) = &s.quote {
                        anstream::println!("    quote: {}", quote);
                    }
                }
            }
            if let Some(summary) = &item.summary {
                anstream::println!("summary:\n{}", summary);
            }
            anstream::println!("content:\n{}", item.content);
        });
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_mem_update(
    mem_id: String,
    content: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    topics: Vec<String>,
    tags: Option<String>,
    importance: Option<i32>,
    status: Option<String>,
) -> Result<()> {
    with_mem_client(|_workspace_id, mut client| async move {
        let tags = tags.map(|s| parse_tags(Some(&s)));
        let topic_slugs = if topics.is_empty() { None } else { Some(topics) };
        let resp = client
            .mem_update(
                &mem_id,
                title.as_deref(),
                content.as_deref(),
                summary.as_deref(),
                topic_slugs,
                tags,
                importance,
                status.as_deref(),
            )
            .await?;
        print_json_or_table(resp, |item: crate::storage::MemItem| {
            anstream::println!("已更新 memory: {}", item.id);
        });
        Ok(())
    })
    .await
}

pub(crate) async fn run_mem_archive(mem_id: String) -> Result<()> {
    with_mem_client(|_workspace_id, mut client| async move {
        let resp = client.mem_archive(&mem_id).await?;
        print_json_or_table(resp, |item: crate::storage::MemItem| {
            anstream::println!("已归档 memory: {}", item.id);
        });
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_mem_promote(
    source_ref: String,
    source_type: String,
    item_type: String,
    title: Option<String>,
    summary: Option<String>,
    topics: Vec<String>,
    tags: Option<String>,
    importance: i32,
    confidence: String,
) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let title = title.unwrap_or_else(|| format!("从 {} 提升", source_ref));
        let tags = parse_tags(tags.as_deref());
        let resp = client
            .mem_promote(
                &source_type,
                &source_ref,
                workspace_id.as_deref(),
                &item_type,
                &title,
                summary.as_deref(),
                &topics,
                &tags,
                importance,
                &confidence,
            )
            .await?;
        print_json_or_table(resp, |item: crate::storage::MemItem| {
            anstream::println!("已提升为 memory: {}", item.id);
        });
        Ok(())
    })
    .await
}

pub(crate) async fn run_mem_search(
    query: Option<String>,
    topics: Vec<String>,
    item_type: Option<String>,
    scope: Option<String>,
    limit: u32,
) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let resp = client
            .mem_search(
                workspace_id.as_deref(),
                query.as_deref(),
                &topics,
                item_type.as_deref(),
                scope.as_deref(),
                limit,
            )
            .await?;
        print_json_or_table(resp, |results: Vec<crate::storage::MemSearchResult>| {
            if results.is_empty() {
                anstream::println!("未找到 memory");
                return;
            }
            let mut table = Table::new();
            table.set_header(vec!["id", "type", "title", "scope", "importance"]);
            for r in results {
                table.add_row(vec![
                    r.item.id,
                    r.item.item_type,
                    r.item.title,
                    r.item.scope,
                    r.item.importance.to_string(),
                ]);
            }
            anstream::println!("{}", table);
        });
        Ok(())
    })
    .await
}

pub(crate) fn format_mem_pack(pack: &crate::storage::MemPack) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Memory Pack: {}\n\n", pack.topic.slug));
    out.push_str("## Topic Summary\n");
    if let Some(summary) = &pack.topic.summary {
        out.push_str(summary);
    } else {
        out.push_str(&pack.topic.title);
    }
    out.push_str("\n\n");
    let mut by_type: std::collections::HashMap<String, Vec<&crate::storage::MemItem>> =
        std::collections::HashMap::new();
    for item in &pack.items {
        by_type.entry(item.item_type.clone()).or_default().push(item);
    }
    for (typ, items) in by_type {
        out.push_str(&format!("## {}\n", typ));
        for item in items {
            out.push_str(&format!("- **{}**\n", item.title));
            if let Some(summary) = &item.summary {
                out.push_str(&format!("  {}\n", summary));
            }
        }
        out.push('\n');
    }
    out
}

pub(crate) async fn run_mem_pack(topic_slug: String, limit: u32) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let resp = client.mem_pack(workspace_id.as_deref(), &topic_slug, limit).await?;
        print_json_or_table(resp, |pack: crate::storage::MemPack| {
            anstream::println!("{}", format_mem_pack(&pack));
        });
        Ok(())
    })
    .await
}

pub(crate) async fn run_mem_list(
    topic: Option<String>,
    item_type: Option<String>,
    scope: Option<String>,
    status: String,
    limit: u32,
) -> Result<()> {
    with_mem_client(|workspace_id, mut client| async move {
        let resp = client
            .mem_list(
                workspace_id.as_deref(),
                topic.as_deref(),
                item_type.as_deref(),
                scope.as_deref(),
                &status,
                limit,
            )
            .await?;
        print_json_or_table(resp, |items: Vec<crate::storage::MemItem>| {
            if items.is_empty() {
                anstream::println!("未找到 memory");
                return;
            }
            let mut table = Table::new();
            table.set_header(vec![
                "id",
                "type",
                "title",
                "topic",
                "status",
                "importance",
                "updated_at",
            ]);
            for item in items {
                let topic_slugs: Vec<String> =
                    item.topics.iter().map(|t| t.slug.clone()).collect();
                let short_id = if item.id.len() > 8 {
                    item.id[..8].to_string()
                } else {
                    item.id.clone()
                };
                table.add_row(vec![
                    short_id,
                    item.item_type,
                    item.title,
                    topic_slugs.join(", "),
                    item.status,
                    item.importance.to_string(),
                    format_timestamp(item.updated_at),
                ]);
            }
            anstream::println!("{}", table);
        });
        Ok(())
    })
    .await
}

fn format_timestamp(ts: f64) -> String {
    let secs = ts.trunc() as i64;
    let nanos = ((ts - ts.trunc()) * 1_000_000_000.0) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}

async fn handle_mem(cmd: MemCommand) -> Result<()> {
    match cmd {
        MemCommand::Topic(topic_cmd) => match topic_cmd {
            MemTopicCommand::Add(args) => {
                run_mem_topic_add(args.slug, args.title, args.summary, args.alias, args.priority)
                    .await
            }
            MemTopicCommand::List(args) => run_mem_topic_list(args.all).await,
            MemTopicCommand::Show(args) => run_mem_topic_show(args.slug).await,
            MemTopicCommand::Update(args) => {
                run_mem_topic_update(
                    args.slug,
                    args.title,
                    args.summary,
                    args.alias,
                    args.priority,
                    args.archive,
                )
                .await
            }
        },
        MemCommand::Add(args) => {
            run_mem_add(
                args.content,
                args.item_type,
                args.title,
                args.summary,
                args.topic,
                args.tags,
                args.importance,
                args.confidence,
                args.scope,
            )
            .await
        }
        MemCommand::Show(args) => run_mem_show(args.mem_id).await,
        MemCommand::Update(args) => {
            run_mem_update(
                args.mem_id,
                args.content,
                args.title,
                args.summary,
                args.topic,
                args.tags,
                args.importance,
                args.status,
            )
            .await
        }
        MemCommand::Archive(args) => run_mem_archive(args.mem_id).await,
        MemCommand::Promote(args) => {
            run_mem_promote(
                args.source_ref,
                args.source_type,
                args.item_type,
                args.title,
                args.summary,
                args.topic,
                args.tags,
                args.importance,
                args.confidence,
            )
            .await
        }
        MemCommand::Search(args) => {
            run_mem_search(
                args.query,
                args.topic,
                args.item_type,
                args.scope,
                args.limit,
            )
            .await
        }
        MemCommand::Pack(args) => run_mem_pack(args.topic_slug, args.limit).await,
        MemCommand::List(args) => {
            run_mem_list(
                args.topic,
                args.item_type,
                args.scope,
                args.status,
                args.limit,
            )
            .await
        }
    }
}

fn parse_tags(tags: Option<&str>) -> Vec<String> {
    tags.unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn load_workspace_and_session() -> Result<(Option<String>, String, String)> {
    let workspace_id = crate::workspace::read_workspace()
        .ok()
        .flatten()
        .map(|w| w.workspace.id);
    let (session_id, token) = load_session()?;
    Ok((workspace_id, session_id, token))
}

fn load_session() -> Result<(String, String)> {
    let resolved = crate::identity::resolve_identity(None)?;
    Ok((resolved.session_id, resolved.token))
}

fn print_json_or_table<T, F>(resp: ServerMsg, printer: F)
where
    T: serde::de::DeserializeOwned,
    F: FnOnce(T),
{
    match resp {
        ServerMsg::Ok { data } => {
            if std::env::var("AGTALK_OUTPUT").unwrap_or_default() == "json" {
                anstream::println!("{}", serde_json::to_string_pretty(&data).unwrap_or_default());
            } else {
                match serde_json::from_value::<T>(data.clone()) {
                    Ok(v) => printer(v),
                    Err(_) => {
                        // 回退到原始 JSON
                        anstream::println!("{}", serde_json::to_string_pretty(&data).unwrap_or_default());
                    }
                }
            }
        }
        ServerMsg::Error { code, message } => {
            eprintln!("错误 [{}]: {}", code, message);
            std::process::exit(1);
        }
        _ => {
            eprintln!("异常响应");
            std::process::exit(1);
        }
    }
}

fn socket_path() -> String {
    crate::paths::socket_path()
}

pub fn print_help() {
    anstream::println!("agtalk 0.1.0 — Agent 与 Agent，Agent 与人协作的本地通信工具");
    anstream::println!();
    anstream::println!("{}", help::section("用法"));
    anstream::println!("{}", help::cmd("agtalk <命令> [参数]", ""));
    anstream::println!(
        "{}",
        help::cmd(
            "agtalk agent <消息> [选项]",
            "最常用：给 Agent 发任务 / 回复（AI 见 --agent-help）"
        )
    );
    anstream::println!(
        "{}",
        help::cmd(
            "agtalk reply <msg-id> <choice>",
            "回复审批请求（-r/--reason 附加说明）"
        )
    );
    anstream::println!(
        "{}",
        help::cmd(
            "agtalk run [file.yaml]",
            "从 YAML 文件执行 agtalk 命令；省略时读取 .agtalk/runs/<当前agent-name>.yaml"
        )
    );
    anstream::println!();
    anstream::println!("{}", help::section("Agent 对话（发任务 / 回复）"));
    anstream::println!("{}", help::cmd("agtalk agent <消息> [选项]", ""));
    anstream::println!("{}", help::opt("-n, --name <name>", "指定目标 Agent"));
    anstream::println!("{}", help::opt("-s, --subject <标题>", "消息主题"));
    anstream::println!("{}", help::opt("-r, --reply-to <msg-id>", "回复指定消息"));
    anstream::println!("{}", help::opt("-d, --done <msg-id>", "标记消息已完成"));
    anstream::println!("{}", help::opt("-f, --file <path>", "附件路径，可多次添加"));
    anstream::println!("{}", help::opt("-i, --notify", "提醒 Agent 查收"));
    anstream::println!("{}", help::opt("--with-mem <topic-slug>", "注入指定 topic 的 Memory Pack"));
    anstream::println!("{}", help::opt("--with-mem-limit <n>", "Memory Pack 最大条数，默认 5"));
    anstream::println!();
    anstream::println!("{}", help::section("人类对话（向人提问 / 收集回应）"));
    anstream::println!("{}", help::cmd("agtalk human <消息> [选项]", ""));
    anstream::println!(
        "{}",
        help::opt("-q, --question <text>", "提出问题，可多次出现")
    );
    anstream::println!("{}", help::opt("-o, --option <text>", "添加预定义回答选项"));
    anstream::println!(
        "{}",
        help::opt("-o!, --option! <text>", "同 -o，并标记为推荐答案")
    );
    anstream::println!("{}", help::opt("--single", "单选（默认多选）"));
    anstream::println!(
        "{}",
        help::opt("--select-only", "严格选择：禁用自由文本（每题必须有选项）")
    );
    anstream::println!(
        "{}",
        help::opt("--output <text|json>", "输出格式（默认 text）")
    );
    anstream::println!();
    anstream::println!("{}", help::section("参与者"));
    anstream::println!(
        "{}",
        help::cmd("join <name> [--intro ... --transport ...]", "加入网络")
    );
    anstream::println!(
        "{}",
        help::cmd("attach <name>", "接管已有 peer 身份")
    );
    anstream::println!("{}", help::opt("--takeover", "强制接管同 endpoint 上的旧 session"));
    anstream::println!(
        "{}",
        help::cmd("leave [--purge]", "离开网络；--purge 同时删除本地凭证")
    );
    anstream::println!(
        "{}",
        help::cmd("cleanup [--dry-run]", "清理当前 workspace 已退役的 session 记录与本地文件")
    );
    anstream::println!();
    anstream::println!("  shell / 无 endpoint 的 join 不参与 takeover 冲突桶。");
    anstream::println!("  takeover 为原子操作：创建新 session 失败时旧 session 仍保持 active。");
    anstream::println!("  leave --purge 在 session 已被接管或失效时仍可删除本地凭证。");
    anstream::println!(
        "{}",
        help::cmd("me / peers", "自己信息 / 在线列表")
    );
    anstream::println!();
    anstream::println!("{}", help::section("收件箱与对话"));
    anstream::println!(
        "{}",
        help::cmd("inbox [选项]", "查看待处理消息（待办中心）")
    );
    anstream::println!("{}", help::opt("--peek", "只查看，不标记已读"));
    anstream::println!("{}", help::opt("--unread", "仅显示未读消息"));
    anstream::println!("{}", help::opt("--pending", "仅显示待处理消息"));
    anstream::println!("{}", help::opt("--action-required", "仅显示需要回应的消息"));
    anstream::println!("{}", help::opt("--all", "显示全部消息（包括已完成）"));
    anstream::println!("{}", help::cmd("detail <msg-id>", "查看消息详情"));
    anstream::println!("{}", help::cmd("wait <msg-id> [--timeout <秒>] [--output json]", "等待审批结果"));
    anstream::println!("{}", help::cmd("attachment <att-id>", "查看附件全文"));
    anstream::println!("{}", help::cmd("chats", "查看对话列表"));
    anstream::println!(
        "{}",
        help::cmd("run [file.yaml]", "从 YAML 文件执行 agtalk 命令")
    );
    anstream::println!(
        "  省略 file.yaml 时读取 .agtalk/runs/<当前agent-name>.yaml；适合固定授权入口"
    );
    anstream::println!();
    anstream::println!("{}", help::section("长期知识库"));
    anstream::println!("{}", help::cmd("mem topic add <slug> --title <title>", "添加 topic"));
    anstream::println!("{}", help::cmd("mem topic list [--all]", "列出 topics"));
    anstream::println!("{}", help::cmd("mem topic show <slug>", "查看 topic"));
    anstream::println!("{}", help::cmd("mem topic update <slug> [--title ...]", "更新 topic"));
    anstream::println!("{}", help::cmd("mem add <content> --topic <slug> --type <type>", "添加 memory"));
    anstream::println!("{}", help::cmd("mem show <mem-id|prefix>", "查看 memory，支持短 ID 前缀"));
    anstream::println!("{}", help::cmd("mem update <mem-id|prefix> [--content ...]", "更新 memory，支持短 ID 前缀"));
    anstream::println!("{}", help::cmd("mem archive <mem-id|prefix>", "归档 memory，支持短 ID 前缀"));
    anstream::println!("{}", help::cmd("mem promote <msg-id> --topic <slug> --type <type>", "从消息提升为 memory"));
    anstream::println!("{}", help::cmd("mem search <query> [--topic <slug>]", "搜索 memory"));
    anstream::println!("{}", help::cmd("mem list [--topic <slug>] [--status active|archived|all]", "列出 memory"));
    anstream::println!("{}", help::cmd("mem pack <topic-slug>", "生成 Memory Pack"));
    anstream::println!();
    anstream::println!("{}", help::section("环境"));
    anstream::println!("{}", help::cmd("init", "初始化环境"));
    anstream::println!("{}", help::cmd("settings", "打开 GUI 设置"));
    anstream::println!(
        "{}",
        help::cmd("config <get|set|list> [key] [value]", "管理全局配置")
    );
    anstream::println!(
        "{}",
        help::cmd("gui", "启动 GUI（开发：pnpm tauri dev -- gui）")
    );
    anstream::println!(
        "{}",
        help::cmd("daemon <start|stop|restart|status>", "管理后台 daemon")
    );
    anstream::println!();
    anstream::println!("{}", help::section("帮助"));
    anstream::println!("{}", help::cmd("--agent-help", "面向 AI 的精简提问用法"));
    anstream::println!("{}", help::cmd("<命令> --help", "子命令详细用法"));
    anstream::println!("{}", help::cmd("--help, -h", "显示此帮助"));
    anstream::println!("{}", help::cmd("--version, -V", "显示版本"));
}

pub fn dispatch(argv: &[String]) {
    // 顶层 --help / -h / 无参：打印结构化帮助（比 clap 默认更全面）
    let wants_help =
        argv.len() < 2 || (argv.len() >= 2 && matches!(argv[1].as_str(), "--help" | "-h"));
    if wants_help {
        print_help();
        return;
    }

    let cli = Cli::try_parse_from(argv).unwrap_or_else(|err| err.exit());

    if cli.agent_help {
        print_agent_help();
        return;
    }

    let Some(command) = cli.command else {
        print_help();
        return;
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
            rt.block_on(handle_ask_flow(
                &questions,
                is_single,
                select_only,
                output_json,
            ))
        }
        Commands::Agent(cmd) => {
            let args = AgentArgs {
                message: cmd.message.join(" "),
                name: cmd.name,
                subject: cmd.subject,
                reply_to: cmd.reply_to,
                done: cmd.done,
                files: cmd.file,
                notify: cmd.notify,
                no_enter: cmd.no_enter,
                with_mem: cmd.with_mem,
                with_mem_limit: cmd.with_mem_limit,
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
        Commands::Run(cmd) => {
            let file = match cmd.file {
                Some(file) => file,
                None => match super::run::default_run_file() {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        eprintln!("错误: 无法确定默认 YAML runner 文件: {}", e);
                        exit(1);
                    }
                },
            };
            rt.block_on(super::run::handle_run(&file))
        }
        Commands::Reply(cmd) => rt.block_on(handle_reply(&cmd)),
        Commands::Join(cmd) => {
            let args = JoinArgs {
                name: cmd.name,
                intro: cmd.intro,
                role: cmd.role,
                transport: cmd.transport,
                takeover: cmd.takeover,
                print_env: cmd.print_env,
            };
            rt.block_on(handle_join(&args))
        }
        Commands::Attach(cmd) => {
            let args = AttachArgs {
                name: cmd.name,
                takeover: cmd.takeover,
                print_env: cmd.print_env,
            };
            rt.block_on(handle_attach(&args))
        }
        Commands::Leave(cmd) => rt.block_on(handle_leave(cmd.purge)),
        Commands::Cleanup(cmd) => rt.block_on(handle_cleanup(cmd.dry_run)),
        Commands::Me => rt.block_on(handle_me()),
        Commands::Peers(args) => rt.block_on(handle_peers(&args)),
        Commands::Inbox(args) => rt.block_on(handle_inbox(&args)),
        Commands::Detail(args) => rt.block_on(handle_detail(&args)),
        Commands::Wait(args) => rt.block_on(handle_wait(&args)),
        Commands::Attachment(args) => rt.block_on(handle_attachment(&args)),
        Commands::Chats => rt.block_on(handle_chats()),
        Commands::Config(args) => rt.block_on(handle_config(&args)),
        Commands::Init => rt.block_on(handle_init()),
        Commands::Settings => match std::env::current_exe() {
            Ok(exe) => std::process::Command::new(exe)
                .arg("gui")
                .spawn()
                .map(|_| ())
                .map_err(Into::into),
            Err(e) => Err(e.into()),
        },
        Commands::Daemon(cmd) => {
            let args = vec![cmd.action];
            rt.block_on(handle_daemon(&args))
        }
        Commands::Mem(cmd) => rt.block_on(handle_mem(cmd)),
    };

    if let Err(e) = result {
        eprintln!("错误: {}", e);
        exit(1);
    }
}

// ── 参数解析 ──────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct QuestionArgs {
    pub(crate) message: String,
    pub(crate) options: Vec<(String, bool)>, // (text, recommended)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AgentArgs {
    pub(crate) message: String,
    pub(crate) name: Option<String>,
    pub(crate) subject: Option<String>,
    pub(crate) reply_to: Option<String>,
    pub(crate) done: Option<String>,
    pub(crate) files: Vec<String>,
    pub(crate) notify: bool,
    pub(crate) no_enter: bool,
    pub(crate) with_mem: Option<String>,
    pub(crate) with_mem_limit: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct JoinArgs {
    name: String,
    intro: Option<String>,
    role: String,
    transport: String,
    takeover: bool,
    print_env: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct AttachArgs {
    name: String,
    takeover: bool,
    print_env: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct InboxArgs {
    #[arg(long, help = "只查看，不标记已读")]
    pub(crate) peek: bool,
    #[arg(long)]
    pub(crate) unread: bool,
    #[arg(long)]
    pub(crate) pending: bool,
    #[arg(long = "action-required")]
    pub(crate) action_required: bool,
    #[arg(long)]
    pub(crate) all: bool,
    #[arg(long, default_value = "50", help = "返回条数上限")]
    pub(crate) limit: u32,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PeersArgs {
    #[arg(long, short, help = "显示详细排障信息")]
    pub(crate) verbose: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct DetailArgs {
    pub(crate) msg_id: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct WaitArgs {
    pub(crate) msg_id: String,
    #[arg(short = 't', long = "timeout", default_value = "300", help = "最长等待秒数")]
    pub(crate) timeout: u64,
    #[arg(long = "output", default_value = "text", help = "输出格式：text / json")]
    pub(crate) output: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct AttachmentArgs {
    pub(crate) attachment_id: String,
}

#[derive(Debug, clap::Args)]
struct ConfigArgs {
    #[arg(value_parser = ["get", "set", "list"])]
    action: String,
    key: Option<String>,
    value: Option<String>,
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
                if i + 1 >= argv.len() {
                    break;
                }
                i += 1;
                questions.push(QuestionArgs {
                    message: argv[i].clone(),
                    options: vec![],
                });
            }
            "-o" | "--option" | "-o!" | "--option!" => {
                if i + 1 >= argv.len() {
                    break;
                }
                i += 1;
                let recommended = arg.ends_with('!');
                if questions.is_empty() {
                    questions.push(QuestionArgs {
                        message: String::new(),
                        options: vec![],
                    });
                }
                if let Some(q) = questions.last_mut() {
                    q.options.push((argv[i].clone(), recommended));
                }
            }
            "--single" => {
                is_single = true;
            }
            "--select-only" => {
                select_only = true;
            }
            "--output" => {
                if i + 1 >= argv.len() {
                    break;
                }
                i += 1;
                output_json = argv[i] == "json";
            }
            _ => {
                if !saw_question {
                    if message_text.is_empty() {
                        message_text = arg.clone();
                    } else {
                        message_text = format!("{} {}", message_text, arg);
                    }
                }
            }
        }
        i += 1;
    }

    if !message_text.is_empty() && (questions.is_empty() || questions[0].message.is_empty()) {
        if questions.is_empty() {
            questions.push(QuestionArgs {
                message: message_text,
                options: vec![],
            });
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
    let mut files: Vec<String> = Vec::new();
    let mut notify = false;
    let mut no_enter = false;
    let mut with_mem = None;
    let mut with_mem_limit: u32 = 5;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-n" | "--name" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--name 缺少参数"));
                }
                name = Some(argv[i].clone());
            }
            "-s" | "--subject" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--subject 缺少参数"));
                }
                subject = Some(argv[i].clone());
            }
            "-r" | "--reply-to" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--reply-to 缺少参数"));
                }
                reply_to = Some(argv[i].clone());
            }
            "-d" | "--done" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--done 缺少参数"));
                }
                done = Some(argv[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--file 缺少参数"));
                }
                files.push(argv[i].clone());
            }
            "-i" | "--notify" => {
                notify = true;
            }
            "--no-enter" => {
                no_enter = true;
            }
            "--with-mem" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--with-mem 缺少参数"));
                }
                with_mem = Some(argv[i].clone());
            }
            "--with-mem-limit" => {
                i += 1;
                if i >= argv.len() {
                    return Err(anyhow!("--with-mem-limit 缺少参数"));
                }
                with_mem_limit = argv[i].parse().context("--with-mem-limit 必须是正整数")?;
            }
            arg => {
                message_parts.push(arg.to_string());
            }
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
        files,
        notify,
        no_enter,
        with_mem,
        with_mem_limit,
    })
}

#[allow(dead_code)]
fn parse_join_args(argv: &[String]) -> JoinArgs {
    let name = argv.first().cloned().unwrap_or_default();
    let mut intro = None;
    let mut role = "agent".to_string();
    let mut transport = "terminal".to_string();
    let mut takeover = false;
    let mut print_env = false;

    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--intro" => {
                i += 1;
                if i < argv.len() {
                    intro = Some(argv[i].clone());
                }
            }
            "--role" => {
                i += 1;
                if i < argv.len() {
                    role = argv[i].clone();
                }
            }
            "--transport" => {
                i += 1;
                if i < argv.len() {
                    transport = argv[i].clone();
                }
            }
            "--takeover" => {
                takeover = true;
            }
            "--print-env" => {
                print_env = true;
            }
            _ => {}
        }
        i += 1;
    }

    JoinArgs {
        name,
        intro,
        role,
        transport,
        takeover,
        print_env,
    }
}

// ── Ask 结果打印 ──────────────────────────────────

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
            let msg_id = v.get("msg_id").and_then(|m| m.as_str()).unwrap_or("?");
            println!("[超时] 未在规定时间内收到人类回复: {}", msg_id);
        }
        Some("ask_dismissed") => {
            println!("[已关闭] 人类关闭了弹窗，未作出选择");
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
    anstream::println!("agtalk —— Agent 本地协作、向人类提问与长期知识库（面向 AI / 自动化）。");
    anstream::println!();
    anstream::println!("{}", help::section("调用方式"));
    anstream::println!(
        "{}",
        help::cmd(
            "agtalk human \"<消息>\" [-q \"<问题>\" [-o \"<选项>\" ...] ...] [--output json]",
            ""
        )
    );
    anstream::println!();
    anstream::println!("{}", help::section("参数"));
    anstream::println!(
        "{}",
        help::opt("<消息>", "共享描述（可选；无 -q 时提升为唯一问题）")
    );
    anstream::println!(
        "{}",
        help::opt("-q, --question <text>", "提出问题，可多次出现")
    );
    anstream::println!(
        "{}",
        help::opt("-o, --option <text>", "跟随问题后添加预定义选项")
    );
    anstream::println!(
        "{}",
        help::opt("-o!, --option! <text>", "同 -o，并标记为你的推荐答案")
    );
    anstream::println!("{}", help::opt("--single", "单选（默认多选）"));
    anstream::println!(
        "{}",
        help::opt("--select-only", "严格选择：禁用自由文本（每题必须有 -o）")
    );
    anstream::println!(
        "{}",
        help::opt("--output <text|json>", "输出格式（默认 text）")
    );
    anstream::println!();
    anstream::println!("{}", help::section("人类回应"));
    anstream::println!(
        "{}",
        help::cmd("[已选择] <选项>", "text 格式：用户勾选的选项")
    );
    anstream::println!("{}", help::cmd("[原因] <文本>", "若人类附带了说明"));
    anstream::println!(
        "{}",
        help::cmd(
            "{\"type\":\"ask_response\",...}",
            "json 格式（--output json）"
        )
    );
    anstream::println!("{}", help::cmd("[超时]", "未在规定时间内收到回复"));
    anstream::println!();
    anstream::println!("{}", help::section("多问题"));
    anstream::println!(
        "{}",
        help::cmd("", "每题以「# Qn」前缀分组，逐题阻塞等待回应")
    );
    anstream::println!();
    anstream::println!("{}", help::section("示例"));
    anstream::println!("  agtalk human \"要继续部署吗？\" -o! 继续 -o 停止");
    anstream::println!("  agtalk human -q \"部署目标？\" -o staging -o! production --single --select-only --output json");
    anstream::println!(
        "  agtalk human -q \"保留日志？\" -o 保留 -o 清除 -q \"开启缓存？\" -o 开 -o 关"
    );
    anstream::println!();
    anstream::println!("{}", help::section("Agent 命名建议"));
    anstream::println!("  # 推荐格式：<agent-type>-<role>-<随机名称>");
    anstream::println!("  #   agent-type 可以是 codex、claude、kimi 等你当前运行 agent 的类别");
    anstream::println!("  #   随机名称可以是 Alex、Bob、Cathy 等便于区分的名字");
    anstream::println!("  #   例如：codex-coder-Alex、claude-reviewer-Bob、kimi-planner-Cathy");
    anstream::println!("  # 保留名不能注册：me、human");
    anstream::println!();
    anstream::println!("  # 先查看当前有哪些在线 Agent");
    anstream::println!("  agtalk peers");
    anstream::println!();
    anstream::println!("{}", help::section("Agent 间协作（注册 + 对话）"));
    anstream::println!("  # 给 Agent 发普通消息 / 回复时建议加 -i 提醒对方查收；标记完成不需要 -i");
    anstream::println!();
    anstream::println!("  # agent-a 终端：join 后 session 保持 active，后续命令自动识别该身份");
    anstream::println!("  agtalk join codex-coder-Alex --intro \"代码生成 Agent\" --role coder");
    anstream::println!("  # 普通消息（带多附件）");
    anstream::println!("  agtalk agent \"请 review PR #42\" -n claude-reviewer-Bob -s \"代码评审\" -i -f ./src/main.rs -f ./README.md");
    anstream::println!("  # 注入长期知识库上下文（Memory Pack）后再发任务");
    anstream::println!("  agtalk agent \"按项目规范重构这段代码\" -n codex-coder-Alex --with-mem project-setup -i");
    anstream::println!();
    anstream::println!("  # 复杂请求（长正文/多附件/多选项）建议写 YAML 一次执行，见文末「YAML Runner」");
    anstream::println!();
    anstream::println!("  # agent-b 终端：join 后同样自动识别");
    anstream::println!("  agtalk join claude-reviewer-Bob --intro \"代码评审 Agent\" --role reviewer");
    anstream::println!("  # 同一 endpoint 已存在 active session 时会提示冲突；可加 --takeover 强制接管");
    anstream::println!("  # takeover 为原子操作；shell / 无 endpoint 的 join 不会冲突");
    anstream::println!("  agtalk join codex-coder-Alex --takeover");
    anstream::println!("  # 在新终端接管已有 peer 身份（不修改 role/intro/transport）");
    anstream::println!("  agtalk attach codex-coder-Alex");
    anstream::println!("  agtalk attach codex-coder-Alex --takeover");
    anstream::println!("  # 离开并保留凭证（默认）；--purge 删除本地 session.json，即使 session 已失效");
    anstream::println!("  agtalk leave");
    anstream::println!("  agtalk leave --purge");
    anstream::println!("  # 清理当前 workspace 已退役 session 记录与本地文件");
    anstream::println!("  agtalk cleanup");
    anstream::println!("  agtalk inbox");
    anstream::println!("  # 回复消息（带附件）");
    anstream::println!("  agtalk agent \"已通过，可合并\" -n codex-coder-Alex -r <msg-id> -i -f ./result.log");
    anstream::println!("  # 标记消息完成（带附件，无需 -i）");
    anstream::println!("  agtalk agent -d <msg-id> -f ./result.log");
    anstream::println!();
    anstream::println!("  # 若一个终端有多个 active session，可为命令指定身份");
    anstream::println!("  AGTALK_NAME=codex-coder-Alex agtalk me");
    anstream::println!();
    anstream::println!("  # 列出在线 Agent / 查看自己");
    anstream::println!("  agtalk peers");
    anstream::println!("  agtalk me");
    anstream::println!();
    anstream::println!("{}", help::section("长期知识库 (mem)"));
    anstream::println!("  # 把关键事实、决策、偏好沉淀为可搜索的 memory");
    anstream::println!("  # type: fact|decision|rule|procedure|issue|snippet|preference|summary|note|context");
    anstream::println!("  # confidence: low|medium|high；scope: global|workspace，默认 workspace");
    anstream::println!("  agtalk mem topic add project-setup --title \"项目环境配置\" --summary \"开发环境、依赖与构建命令\"");
    anstream::println!("  agtalk mem add \"使用 pnpm + vite；构建命令 pnpm build\" --type fact --title \"构建方式\" --topic project-setup --confidence high --scope workspace");
    anstream::println!("  agtalk mem add \"优先使用 Result/Option 显式错误处理\" --type rule --title \"Rust 错误处理规范\" --topic project-setup --tags \"rust,style\" --confidence high");
    anstream::println!("  agtalk mem search \"error handling\" --topic project-setup");
    anstream::println!("  agtalk mem list --topic project-setup --status active");
    anstream::println!("  agtalk mem pack project-setup");
    anstream::println!("  # 从消息提升为 memory（source_ref 为 msg_id）；topic 不存在会报错，不会自动创建");
    anstream::println!("  agtalk mem promote <msg-id> --type fact --title \"从对话提取的关键结论\" --topic project-setup --confidence medium");
    anstream::println!();
    anstream::println!("{}", help::section("YAML Runner（复杂指令）"));
    anstream::println!(
        "{}",
        help::cmd("agtalk run [file.yaml]", "复杂请求一次执行，免去 shell 长命令与引号")
    );
    anstream::println!("  # Runner 只执行 agtalk 内部命令，不执行任意 shell");
    anstream::println!("  # YAML 中的相对路径按 YAML 文件所在目录解析");
    anstream::println!("  # version 必须为 1；command 支持 11 种 snake_case 命令");
    anstream::println!("  #");
    anstream::println!("  # 建议：把复杂指令固定写入 .agtalk/runs/<当前agent-name>.yaml");
    anstream::println!("  # 每次只需覆盖同一文件再执行；agtalk run 不带参数时会读取该文件");
    anstream::println!("  # 例：cat > .agtalk/runs/codex-coder-Alex.yaml <<'YAML' && AGTALK_NAME=codex-coder-Alex agtalk run");
    anstream::println!();
    anstream::println!("  version: 1");
    anstream::println!(
        "  command: agent | human | reply | wait | inbox | detail | attachment | chats | peers | me | mem"
    );
    anstream::println!();
    anstream::println!("  # agent —— 给 Agent 发任务 / 回复 / 标记完成");
    anstream::println!("{}", help::opt("name", "目标 Agent（done 为空时必填） -> -n"));
    anstream::println!("{}", help::opt("subject", "消息主题 -> -s"));
    anstream::println!(
        "{}",
        help::opt("message", "正文（done 为空时必填非空）")
    );
    anstream::println!("{}", help::opt("reply_to", "回复某消息 -> -r"));
    anstream::println!(
        "{}",
        help::opt("done", "标记完成（有值时可省略 name/message） -> -d")
    );
    anstream::println!("{}", help::opt("notify", "提醒对方查收 -> -i"));
    anstream::println!("{}", help::opt("no_enter", "提醒时不自动发回车 -> --no-enter"));
    anstream::println!(
        "{}",
        help::opt("files", "附件数组（相对路径按 YAML 目录解析） -> 多个 -f")
    );
    anstream::println!("{}", help::opt("with_mem", "注入指定 topic 的 Memory Pack -> --with-mem"));
    anstream::println!("{}", help::opt("with_mem_limit", "Memory Pack 最大条数 -> --with-mem-limit"));
    anstream::println!("  示例:");
    anstream::println!("    version: 1");
    anstream::println!("    command: agent");
    anstream::println!("    name: kimi-coder-Kimi");
    anstream::println!("    subject: \"TASK: 实现功能 X\"");
    anstream::println!("    message: |");
    anstream::println!("      请阅读附件并实现，重点关注：");
    anstream::println!("      1. session 校验逻辑");
    anstream::println!("      2. 错误处理");
    anstream::println!("    notify: true");
    anstream::println!("    files:");
    anstream::println!("      - ./src/main.rs");
    anstream::println!("      - ./docs/spec.md");
    anstream::println!();
    anstream::println!("  # human —— 向人类提问");
    anstream::println!(
        "{}",
        help::opt("message", "共享描述；questions 为空时作为唯一问题")
    );
    anstream::println!("{}", help::opt("single", "单选 -> --single"));
    anstream::println!(
        "{}",
        help::opt("select_only", "严格选择（每题需至少一个 option） -> --select-only")
    );
    anstream::println!("{}", help::opt("output", "text | json（默认 text）"));
    anstream::println!("{}", help::opt("questions[].text", "问题文本 -> -q"));
    anstream::println!(
        "{}",
        help::opt("questions[].options[].text", "选项 -> -o")
    );
    anstream::println!(
        "{}",
        help::opt(
            "questions[].options[].recommended",
            "推荐选项 -> -o!"
        )
    );
    anstream::println!();
    anstream::println!("  # 其余命令摘要：");
    anstream::println!("  #   reply      msg_id / choice / reason");
    anstream::println!("  #   wait       msg_id / timeout（默认 300s） / output");
    anstream::println!(
        "  #   inbox      status: unread | pending | action_required | all / limit（默认 50） / peek"
    );
    anstream::println!("  #   detail     msg_id");
    anstream::println!("  #   attachment attachment_id");
    anstream::println!("  #   chats      （无字段）");
    anstream::println!("  #   peers      verbose");
    anstream::println!("  #   me         （无字段）");
    anstream::println!("  #   mem        mem_command + 对应字段");
    anstream::println!();
    anstream::println!("  # mem 字段说明：");
    anstream::println!("  #   mem_command: topic_add | topic_list | topic_show | topic_update | add | show | update | archive | promote | search | list | pack");
    anstream::println!("  #   slug / title / summary / aliases / priority / archive / all");
    anstream::println!("  #   content / item_type / confidence / importance / scope / topics / tags / mem_id / source_ref / source_type / query / limit / status");
    anstream::println!();
    anstream::println!("  # 完整字段与示例见 docs/commands.md");
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// 从 stdin 读取 y/n 确认。
fn read_yes_no() -> Option<bool> {
    use std::io::Write;
    let mut input = String::new();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let _ = stdout.flush();
    if stdin.read_line(&mut input).is_ok() {
        let trimmed = input.trim().to_ascii_lowercase();
        if trimmed == "y" || trimmed == "yes" {
            return Some(true);
        }
        if trimmed == "n" || trimmed == "no" || trimmed.is_empty() {
            return Some(false);
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn print_peers(resp: &crate::ipc::ServerMsg, verbose: bool) -> Result<()> {
    let crate::ipc::ServerMsg::Ok { data } = resp else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };
    let Some(items) = data.as_array() else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };

    let endpoint_for = |item: &serde_json::Value| -> String {
        item.get("sessions")
            .and_then(|v| v.as_array())
            .map(|sessions| {
                let mut parts: Vec<String> = sessions
                    .iter()
                    .filter_map(|s| {
                        let notify = s.get("notify_config").and_then(|v| v.as_str())?;
                        let cfg: serde_json::Value = serde_json::from_str(notify).ok()?;
                        let plugin = cfg.get("plugin").and_then(|v| v.as_str())?;
                        let ep = cfg.get("endpoint").and_then(|v| v.as_object())?;
                        let session = ep.get("session").and_then(|v| v.as_str())?;
                        let pane_id = ep.get("pane_id").and_then(|v| v.as_str())?;
                        Some(format!("{}:{}:{}", plugin, session, pane_id))
                    })
                    .collect();
                parts.sort();
                parts.dedup();
                if parts.is_empty() {
                    "-".to_string()
                } else {
                    parts.join(", ")
                }
            })
            .unwrap_or_else(|| "-".to_string())
    };

    let ts_str = |item: &serde_json::Value, key: &str| -> String {
        item.get(key)
            .and_then(|v| v.as_f64())
            .map(format_iso)
            .unwrap_or_else(|| "-".to_string())
    };

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    if verbose {
        table.set_header(vec![
            "#",
            "name",
            "type",
            "role",
            "endpoint",
            "load",
            "last_seen",
            "last_sent",
            "last_read",
            "session_id",
        ]);
    } else {
        table.set_header(vec!["#", "name", "role", "intro", "endpoint", "load", "last_seen"]);
    }
    for (idx, item) in items.iter().enumerate() {
        let idx_str = (idx + 1).to_string();
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let participant_type = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let role = item
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = if participant_type == "web" {
            format!("[web] {}", name)
        } else {
            name
        };
        let intro = item
            .get("intro")
            .and_then(|v| v.as_str())
            .map(|s| truncate(s, 24))
            .unwrap_or_default();
        let endpoint = endpoint_for(item);
        let unread = item.get("unread").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let pending = item.get("pending").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let load = format!("{}/{}", unread, pending);
        let last_seen = ts_str(item, "last_seen_at");
        let last_sent = ts_str(item, "last_sent_at");
        let last_read = ts_str(item, "last_read_at");
        let session_id = item
            .get("sessions")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|s| s.get("session_id").and_then(|v| v.as_str()).map(short_id))
            .unwrap_or_default();

        if verbose {
            table.add_row(vec![
                idx_str,
                name,
                participant_type,
                role,
                endpoint,
                load,
                last_seen,
                last_sent,
                last_read,
                session_id,
            ]);
        } else {
            table.add_row(vec![idx_str, name, role, intro, endpoint, load, last_seen]);
        }
    }
    anstream::println!("{}", table);
    Ok(())
}

fn format_iso(ts: f64) -> String {
    let secs = ts.trunc() as i64;
    let nanos = ((ts - ts.trunc()) * 1_000_000_000.0) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
        .unwrap_or_default()
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
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
    table.set_header(vec!["id", "kind", "peers", "unread", "pending", "last"]);
    for item in items {
        let peers = item
            .get("peers")
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
        let (unread, pending) = item
            .get("counts")
            .and_then(|v| v.as_object())
            .map(|c| {
                (
                    c.get("unread")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        .to_string(),
                    c.get("pending")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        .to_string(),
                )
            })
            .unwrap_or_default();
        table.add_row(vec![
            item.get("id")
                .and_then(|v| v.as_str())
                .map(short_id)
                .unwrap_or_default(),
            item.get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            peers,
            unread,
            pending,
            last,
        ]);
    }
    anstream::println!("{}", table);
    Ok(())
}

fn print_inbox(
    me_name: &str,
    stats_resp: &crate::ipc::ServerMsg,
    resp: &crate::ipc::ServerMsg,
) -> Result<()> {
    let crate::ipc::ServerMsg::Ok { data } = resp else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };
    let Some(items) = data.as_array() else {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    };

    // 基于 all 状态响应统计消息数量
    let (total, read) = if let crate::ipc::ServerMsg::Ok { data: stats_data } = stats_resp {
        if let Some(all_items) = stats_data.as_array() {
            let total = all_items.len();
            let read = all_items
                .iter()
                .filter(|item| {
                    item.get("delivery")
                        .and_then(|v| v.as_object())
                        .and_then(|d| d.get("status"))
                        .and_then(|v| v.as_str())
                        == Some("read")
                })
                .count();
            (total, read)
        } else {
            (items.len(), 0)
        }
    } else {
        (items.len(), 0)
    };
    let unread = total.saturating_sub(read);

    anstream::println!("me: {}", me_name);
    anstream::println!("消息统计: 共 {} 条, 已读 {}, 未读 {}", total, read, unread);
    anstream::println!();

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        "id", "kind", "priority", "from", "mode", "preview", "status", "actions",
    ]);
    for item in items {
        let from = item
            .get("from")
            .and_then(|v| v.as_object())
            .and_then(|s| s.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mode = item
            .get("content")
            .and_then(|v| v.as_object())
            .and_then(|c| c.get("mode"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let preview = item
            .get("content")
            .and_then(|v| v.as_object())
            .and_then(|c| c.get("body"))
            .and_then(|v| v.as_str())
            .map(|s| truncate(s, 36))
            .unwrap_or_default();
        let truncated = item
            .get("content")
            .and_then(|v| v.as_object())
            .and_then(|c| c.get("truncated"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let preview = if truncated && mode != "full" {
            format!("{}…", preview)
        } else {
            preview
        };
        let status = item
            .get("delivery")
            .and_then(|v| v.as_object())
            .and_then(|d| d.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let actions = item
            .get("actions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        table.add_row(vec![
            item.get("id")
                .and_then(|v| v.as_str())
                .map(short_id)
                .unwrap_or_default(),
            item.get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            item.get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            from.to_string(),
            mode.to_string(),
            preview,
            status.to_string(),
            actions,
        ]);
    }
    anstream::println!("{}", table);
    Ok(())
}

// ── 各 handler ────────────────────────────────────

pub(crate) async fn handle_ask_flow(
    questions: &[QuestionArgs],
    _is_single: bool,
    _select_only: bool,
    output_json: bool,
) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;

    for (i, q) in questions.iter().enumerate() {
        let choices: Vec<String> = q.options.iter().map(|(t, _)| t.clone()).collect();
        let prefix = if questions.len() > 1 {
            format!("# Q{}\n", i + 1)
        } else {
            String::new()
        };
        let body = format!("{}{}", prefix, q.message);

        if output_json {
            eprintln!("[agtalk] 等待人类回复: {}", q.message);
        }

        let resp = cli.ask("human", &body, &choices, 300).await?;

        let resp_str = serde_json::to_string_pretty(&resp)?;
        if output_json {
            println!("{}", resp_str);
        } else {
            print_ask_result(&resp_str);
        }
    }

    Ok(())
}

pub(crate) async fn handle_agent(args: AgentArgs) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;

    let attachments = build_send_attachments(&args.files)?;

    // 1. 标记完成（-d）
    if let Some(ref msg_id) = args.done {
        let resp = cli
            .done(msg_id, &identity.participant_name, attachments.clone())
            .await?;
        println!("{}", serde_json::to_string_pretty(&resp)?);
    }

    // 2. 发送消息（-n + 正文），notify/send_enter 由 daemon 处理
    if args.name.is_some() && !args.message.is_empty() {
        let to = args.name.unwrap();

        // 若指定 --with-mem，先获取 Memory Pack 并注入消息正文
        let mut message = args.message;
        if let Some(topic_slug) = args.with_mem {
            let resp = cli
                .mem_pack(Some(&identity.workspace_id), &topic_slug, args.with_mem_limit)
                .await?;
            match resp {
                ServerMsg::Ok { data } => {
                    let pack: crate::storage::MemPack =
                        serde_json::from_value(data).context("解析 Memory Pack 失败")?;
                    if pack.items.is_empty() {
                        anstream::println!(
                            "[agtalk] 提示：topic '{}' 的 Memory Pack 为空，将只发送原消息。",
                            topic_slug
                        );
                        message = format!(
                            "<agtalk_memory_pack topic=\"{}\" empty=\"true\" />\n\n<user_message>{}</user_message>",
                            topic_slug, message
                        );
                    } else {
                        let pack_text = format_mem_pack(&pack);
                        message = format!(
                            "<agtalk_memory_pack topic=\"{}\">\n{}</agtalk_memory_pack>\n\n<user_message>{}</user_message>",
                            topic_slug,
                            pack_text.trim_end(),
                            message
                        );
                    }
                }
                ServerMsg::Error { code, message: err } => {
                    anyhow::bail!(
                        "无法加载 topic '{}' 的 Memory Pack（{}: {}），消息未发送。",
                        topic_slug,
                        code,
                        err
                    );
                }
                _ => {}
            }
        }

        let metadata = json!({
            "subject": args.subject,
        });
        let send_enter = if args.no_enter { Some(false) } else { None };
        let resp = cli
            .send(
                &to,
                &message,
                None,
                args.reply_to.as_deref(),
                None,
                Some("text"),
                Some(metadata),
                args.notify,
                send_enter,
                attachments.clone(),
            )
            .await?;
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    // 只有 -d 没有发送消息时已经处理完
    if args.done.is_some() {
        return Ok(());
    }

    if args.name.is_none() {
        anyhow::bail!("agtalk agent 发送消息需要 -n <name>");
    }
    anyhow::bail!("agtalk agent 缺少消息正文")
}

pub(crate) async fn handle_reply(args: &ReplyCommand) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let reason = args.reason.clone().unwrap_or_default();
    let resp = cli.reply(&args.msg_id, &args.choice, &reason).await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

/// 校验 -f 指定的文件并构建 SendAttachment 列表。
fn build_send_attachments(paths: &[String]) -> Result<Vec<crate::ipc::SendAttachment>> {
    let mut attachments = Vec::with_capacity(paths.len());
    for p in paths {
        let path = std::path::Path::new(p);
        if !path.exists() {
            anyhow::bail!("附件不存在: {}", p);
        }
        if !path.is_file() {
            anyhow::bail!("附件必须是文件: {}", p);
        }
        let meta = std::fs::metadata(path)
            .with_context(|| format!("无法读取附件元数据: {}", p))?;
        let abs = std::fs::canonicalize(path)
            .with_context(|| format!("无法解析附件绝对路径: {}", p))?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let content_type = infer_content_type(&filename);
        attachments.push(crate::ipc::SendAttachment {
            path: abs.to_string_lossy().to_string(),
            filename,
            content_type,
            size: meta.len() as usize,
        });
    }
    Ok(attachments)
}

fn infer_content_type(filename: &str) -> String {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "txt" | "md" | "markdown" => "text/plain",
        "rs" => "text/rust",
        "py" => "text/x-python",
        "js" => "text/javascript",
        "ts" => "text/typescript",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "xml" => "text/xml",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "wav" => "audio/wav",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub(crate) async fn handle_inbox(args: &InboxArgs) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let status = if args.all {
        Some("all")
    } else if args.action_required {
        Some("action_required")
    } else if args.pending {
        Some("pending")
    } else if args.unread {
        Some("unread")
    } else {
        None // default: all non-done items
    };

    // 获取当前身份名称
    let me_name = match cli.whoami().await? {
        crate::ipc::ServerMsg::Ok { data } => data
            .get("participant")
            .and_then(|v| v.as_str())
            .unwrap_or(&identity.participant_name)
            .to_string(),
        _ => identity.participant_name.clone(),
    };

    let limit = args.limit.max(1);

    // 额外 peek 一次 all 状态用于统计，不修改消息状态
    let stats_resp = cli
        .inbox(&identity.participant_name, Some("all"), limit, true)
        .await?;

    let resp = cli
        .inbox(&identity.participant_name, status, limit, args.peek)
        .await?;
    print_inbox(&me_name, &stats_resp, &resp)?;
    Ok(())
}

pub(crate) async fn handle_detail(args: &DetailArgs) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let msg_id = if args.msg_id == "-" {
        resolve_detail_dash(&mut cli, &identity.participant_name).await?
    } else {
        args.msg_id.clone()
    };
    let resp = cli.detail(&msg_id).await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

pub(crate) async fn handle_wait(args: &WaitArgs) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let resp = cli.wait(&args.msg_id, args.timeout).await?;
    let output_json = args.output == "json";
    match resp {
        ServerMsg::WaitResult {
            msg_id,
            status,
            choice,
            reason,
            timed_out,
        } => {
            if output_json {
                let json = serde_json::json!({
                    "type": "wait_result",
                    "msg_id": msg_id,
                    "status": status,
                    "choice": choice,
                    "reason": reason,
                    "timed_out": timed_out,
                });
                println!("{}", serde_json::to_string_pretty(&json)?);
            } else if timed_out || status == "timed_out" {
                println!("[超时] 未在规定时间内收到人类回复: {}", msg_id);
            } else if choice == "__dismissed__" {
                println!("[已关闭] 人类关闭了弹窗，未作出选择");
            } else {
                println!("[已回复] {}: {}", msg_id, choice);
                if !reason.is_empty() {
                    println!("[原因] {}", reason);
                }
            }
            Ok(())
        }
        ServerMsg::Error { code, message } => Err(anyhow!("等待失败 [{}]: {}", code, message)),
        other => Err(anyhow!("等待返回异常: {:?}", other)),
    }
}

async fn resolve_detail_dash(cli: &mut Client, participant_name: &str) -> Result<String> {
    if let Some(msg_id) = latest_inbox_id(cli, participant_name, Some("unread")).await? {
        return Ok(msg_id);
    }
    latest_inbox_id(cli, participant_name, Some("all"))
        .await?
        .ok_or_else(|| anyhow!("当前 inbox 没有可查看的消息"))
}

async fn latest_inbox_id(
    cli: &mut Client,
    participant_name: &str,
    status: Option<&str>,
) -> Result<Option<String>> {
    match cli.inbox(participant_name, status, 1, true).await? {
        ServerMsg::Ok { data } => Ok(data
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(|id| id.as_str())
            .map(str::to_string)),
        ServerMsg::Error { code, message } => {
            Err(anyhow!("查询 inbox 失败: {}: {}", code, message))
        }
        other => Err(anyhow!(
            "查询 inbox 返回异常: {}",
            serde_json::to_string(&other)?
        )),
    }
}

pub(crate) async fn handle_attachment(args: &AttachmentArgs) -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let resp = cli.attachment(&args.attachment_id).await?;
    let value = match &resp {
        ServerMsg::Ok { data } => data.clone(),
        _ => {
            println!("{}", serde_json::to_string_pretty(&resp)?);
            return Ok(());
        }
    };
    if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
        println!("{}", content);
    } else {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    }
    Ok(())
}

async fn handle_config(args: &ConfigArgs) -> Result<()> {
    use crate::config::AgConfig;
    match args.action.as_str() {
        "get" => {
            let key = args
                .key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("config get 需要 key"))?;
            let cfg = AgConfig::load().unwrap_or_default();
            match cfg.get(key)? {
                Some(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                None => println!("null"),
            }
        }
        "set" => {
            let key = args
                .key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("config set 需要 key"))?;
            let value = args
                .value
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("config set 需要 value"))?;
            let mut cfg = AgConfig::load().unwrap_or_default();
            cfg.set(key, value)?;
            cfg.save()?;
            anstream::println!(
                "{} 已设置 {} = {}",
                label("OK", anstyle::AnsiColor::Green),
                key,
                value
            );
            anstream::println!(
                "{} 重启 daemon 后新阈值生效",
                label("INFO", anstyle::AnsiColor::Cyan)
            );
        }
        "list" => {
            let cfg = AgConfig::load().unwrap_or_default();
            println!("{}", serde_json::to_string_pretty(&cfg)?);
        }
        _ => anyhow::bail!("未知 config 操作: {}", args.action),
    }
    Ok(())
}

async fn handle_join(args: &JoinArgs) -> Result<()> {
    // 1. 确定 workspace root
    let root = match crate::paths::find_agtalk_root() {
        Ok(r) => r,
        Err(_) => {
            // 自动以当前目录创建 workspace
            let cwd = std::env::current_dir().map_err(|e| anyhow!("无法获取当前目录: {}", e))?;
            let agtalk_dir = cwd.join(crate::paths::AGTALK_DIR_NAME);
            std::fs::create_dir_all(&agtalk_dir)?;
            cwd
        }
    };
    let root_str = root.to_string_lossy().to_string();
    let workspace_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".into());

    // 2. 自动捕获 zellij / tmux endpoint 和 runtime 信息
    let cfg = crate::config::AgConfig::load().unwrap_or_default();
    let notify_config = if let Some(endpoint) = notify::detect_zellij_endpoint() {
        notify::build_notify_config("zellij", &endpoint, cfg.notify.default_send_enter)
    } else if let Some(endpoint) = notify::detect_tmux_endpoint() {
        notify::build_notify_config("tmux", &endpoint, cfg.notify.default_send_enter)
    } else {
        serde_json::json!({})
    };

    let runtime_kind = if notify::detect_zellij_endpoint().is_some() {
        "zellij"
    } else if notify::detect_tmux_endpoint().is_some() {
        "tmux"
    } else {
        "shell"
    };
    let shell = std::env::var("SHELL")
        .ok()
        .and_then(|s| std::path::Path::new(&s).file_stem().and_then(|s| s.to_str()).map(String::from))
        .unwrap_or_else(|| "sh".to_string());
    let term = std::env::var("TERM").unwrap_or_default();
    let runtime_config = serde_json::json!({
        "kind": runtime_kind,
        "shell": shell,
        "term": term,
    });

    // 3. 连接 daemon 并请求 join
    let mut cli = Client::connect(&crate::paths::socket_path()).await?;
    let mut resp = cli
        .join(
            &root_str,
            &workspace_name,
            &args.name,
            &args.role,
            args.intro.as_deref().unwrap_or(""),
            &args.transport,
            notify_config.clone(),
            runtime_config.clone(),
            args.takeover,
        )
        .await?;

    // 4. 处理 session_conflict：显式确认后重试 takeover
    if let ServerMsg::Error { code, message } = &resp {
        if code == "session_conflict" {
            let confirm = if args.takeover {
                true
            } else {
                anstream::println!(
                    "{} {}",
                    label("WARN", anstyle::AnsiColor::Yellow),
                    message
                );
                anstream::println!(
                    "{} 是否接管同 endpoint 上的旧 session？(y/N)",
                    label("?", anstyle::AnsiColor::Cyan)
                );
                read_yes_no().unwrap_or(false)
            };
            if confirm {
                resp = cli
                    .join(
                        &root_str,
                        &workspace_name,
                        &args.name,
                        &args.role,
                        args.intro.as_deref().unwrap_or(""),
                        &args.transport,
                        notify_config.clone(),
                        runtime_config.clone(),
                        true,
                    )
                    .await?;
            }
        }
    }

    // 5. 解析响应
    let data = match &resp {
        ServerMsg::Ok { data } => data.clone(),
        ServerMsg::Error { code, message } => anyhow::bail!("join 失败 [{}]: {}", code, message),
        _ => anyhow::bail!("join 返回异常"),
    };

    let workspace_id = data
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = data
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = data
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 4. 构造 join 插件上下文（在 session_id/root_str 被 move 之前）
    let plugin_ctx = crate::join_plugin::JoinContext {
        name: args.name.clone(),
        role: args.role.clone(),
        session_id: session_id.clone(),
        workspace_root: root_str.clone(),
    };

    // 5. 写 workspace.json
    let now = chrono::Utc::now().to_rfc3339();
    let mut wf = workspace::WorkspaceFile {
        version: workspace::WORKSPACE_FILE_VERSION,
        workspace: workspace::WorkspaceMeta {
            id: workspace_id,
            name: workspace_name,
            root: root_str,
            detected_by: "cwd-scan".into(),
            created_at: now.clone(),
            updated_at: now,
        },
        daemon: workspace::DaemonMeta {
            profile: "default".into(),
            socket: Some(crate::paths::socket_path()),
        },
    };
    workspace::write_workspace(&mut wf)?;

    // 6. 写 session.json (v2)
    let notify_mirror = if notify_config.is_object()
        && notify_config
            .as_object()
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    {
        Some(notify_config)
    } else {
        None
    };
    let mut sf = session::SessionFile {
        version: session::SESSION_FILE_VERSION,
        name: args.name.clone(),
        session: session::SessionMeta {
            id: session_id,
            token,
            status: "active".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
        runtime: Some(runtime_config),
        notify: notify_mirror,
    };

    session::write_session(&args.name, &mut sf)?;

    // 7. 执行 join 生命周期插件（当前进程内，不重试、不影响 join 成功状态）
    crate::join_plugin::run_all(&crate::join_plugin::default_plugins(), &plugin_ctx).await;

    // 8. 输出
    if args.print_env {
        println!("export AGTALK_NAME={}", args.name);
    } else {
        anstream::println!(
            "{} joined as {}",
            label("OK", anstyle::AnsiColor::Green),
            args.name
        );
        anstream::println!("  workspace: {}", wf.workspace.root);
        anstream::println!("  session:   {}", sf.session.id);
        anstream::println!("\nTo use this identity:");
        anstream::println!("  # 单条命令指定身份：AGTALK_NAME={} agtalk <cmd>", args.name);
        anstream::println!("  # 单个 active session 时，后续命令会自动使用该身份");
    }

    Ok(())
}

async fn handle_attach(args: &AttachArgs) -> Result<()> {
    // 1. 确定 workspace root
    let root = match crate::paths::find_agtalk_root() {
        Ok(r) => r,
        Err(_) => {
            let cwd = std::env::current_dir().map_err(|e| anyhow!("无法获取当前目录: {}", e))?;
            let agtalk_dir = cwd.join(crate::paths::AGTALK_DIR_NAME);
            std::fs::create_dir_all(&agtalk_dir)?;
            cwd
        }
    };
    let root_str = root.to_string_lossy().to_string();
    let workspace_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".into());

    // 2. 自动捕获 zellij / tmux endpoint 和 runtime 信息
    let cfg = crate::config::AgConfig::load().unwrap_or_default();
    let notify_config = if let Some(endpoint) = notify::detect_zellij_endpoint() {
        notify::build_notify_config("zellij", &endpoint, cfg.notify.default_send_enter)
    } else if let Some(endpoint) = notify::detect_tmux_endpoint() {
        notify::build_notify_config("tmux", &endpoint, cfg.notify.default_send_enter)
    } else {
        serde_json::json!({})
    };

    let runtime_kind = if notify::detect_zellij_endpoint().is_some() {
        "zellij"
    } else if notify::detect_tmux_endpoint().is_some() {
        "tmux"
    } else {
        "shell"
    };
    let shell = std::env::var("SHELL")
        .ok()
        .and_then(|s| std::path::Path::new(&s).file_stem().and_then(|s| s.to_str()).map(String::from))
        .unwrap_or_else(|| "sh".to_string());
    let term = std::env::var("TERM").unwrap_or_default();
    let runtime_config = serde_json::json!({
        "kind": runtime_kind,
        "shell": shell,
        "term": term,
    });

    // 3. 连接 daemon 并请求 attach（复用 workspace/notify/runtime 逻辑，不修改 peer 属性）
    let mut cli = Client::connect(&crate::paths::socket_path()).await?;
    let mut resp = cli
        .attach(
            &root_str,
            &workspace_name,
            &args.name,
            notify_config.clone(),
            runtime_config.clone(),
            args.takeover,
        )
        .await?;

    // 4. 处理 session_conflict：显式确认后重试 takeover
    if let ServerMsg::Error { code, message } = &resp {
        if code == "session_conflict" {
            let confirm = if args.takeover {
                true
            } else {
                anstream::println!(
                    "{} {}",
                    label("WARN", anstyle::AnsiColor::Yellow),
                    message
                );
                anstream::println!(
                    "{} 是否接管同 endpoint 上的旧 session？(y/N)",
                    label("?", anstyle::AnsiColor::Cyan)
                );
                read_yes_no().unwrap_or(false)
            };
            if confirm {
                resp = cli
                    .attach(
                        &root_str,
                        &workspace_name,
                        &args.name,
                        notify_config.clone(),
                        runtime_config.clone(),
                        true,
                    )
                    .await?;
            }
        }
    }

    // 5. 解析响应
    let data = match &resp {
        ServerMsg::Ok { data } => data.clone(),
        ServerMsg::Error { code, message } => anyhow::bail!("attach 失败 [{}]: {}", code, message),
        _ => anyhow::bail!("attach 返回异常"),
    };

    let workspace_id = data
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = data
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = data
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 6. 写 workspace.json
    let now = chrono::Utc::now().to_rfc3339();
    let mut wf = workspace::WorkspaceFile {
        version: workspace::WORKSPACE_FILE_VERSION,
        workspace: workspace::WorkspaceMeta {
            id: workspace_id,
            name: workspace_name.clone(),
            root: root_str.clone(),
            detected_by: "cwd-scan".into(),
            created_at: now.clone(),
            updated_at: now,
        },
        daemon: workspace::DaemonMeta {
            profile: "default".into(),
            socket: Some(crate::paths::socket_path()),
        },
    };
    workspace::write_workspace(&mut wf)?;

    // 7. 写 session.json (v2)
    let notify_mirror = if notify_config.is_object()
        && notify_config
            .as_object()
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    {
        Some(notify_config)
    } else {
        None
    };
    let mut sf = session::SessionFile {
        version: session::SESSION_FILE_VERSION,
        name: args.name.clone(),
        session: session::SessionMeta {
            id: session_id,
            token,
            status: "active".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
        runtime: Some(runtime_config),
        notify: notify_mirror,
    };

    session::write_session(&args.name, &mut sf)?;

    // 8. 输出
    if args.print_env {
        println!("export AGTALK_NAME={}", args.name);
    } else {
        anstream::println!(
            "{} attached as {}",
            label("OK", anstyle::AnsiColor::Green),
            args.name
        );
        anstream::println!("  workspace: {}", wf.workspace.root);
        anstream::println!("  session:   {}", sf.session.id);
        anstream::println!("\nTo use this identity:");
        anstream::println!("  # 单条命令指定身份：AGTALK_NAME={} agtalk <cmd>", args.name);
        anstream::println!("  # 单个 active session 时，后续命令会自动使用该身份");
    }

    Ok(())
}

async fn handle_leave(purge: bool) -> Result<()> {
    // 尝试解析当前身份；purge 模式下允许 session 已被 takeover/失效。
    let identity = identity::resolve_identity(None);
    let mut leave_resp: Option<ServerMsg> = None;
    let target_name: Option<String>;

    match &identity {
        Ok(id) => {
            target_name = Some(id.participant_name.clone());
            match Client::connect_and_auth(&id.socket, &id.session_id, &id.token).await {
                Ok(mut cli) => {
                    leave_resp = Some(cli.leave(None).await?);
                }
                Err(e) => {
                    if !purge {
                        return Err(e);
                    }
                    // purge 模式：daemon 侧已失效也继续清理本地凭证
                }
            }
        }
        Err(e) => {
            if !purge {
                return Err(anyhow!("{}", e));
            }
            // purge 模式：从 AGTALK_NAME 或唯一 active session 推断目标
            target_name = std::env::var("AGTALK_NAME")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    session::list_active_sessions().ok().and_then(|v| {
                        if v.len() == 1 {
                            Some(v[0].clone())
                        } else {
                            None
                        }
                    })
                });
            if target_name.is_none() {
                return Err(anyhow!(
                    "无法确定要清理的 session：请设置 AGTALK_NAME，或确保只有唯一 active session"
                ));
            }
        }
    }

    // 处理本地 session 文件
    if let Some(name) = target_name {
        if purge {
            if let Err(e) = session::remove_session(&name) {
                let not_found = e
                    .downcast_ref::<std::io::Error>()
                    .map(|io| io.kind() == std::io::ErrorKind::NotFound)
                    .unwrap_or(false);
                if !not_found {
                    return Err(e);
                }
            }
            anstream::println!(
                "{} 已清除本地 session 凭证: {}",
                label("OK", anstyle::AnsiColor::Green),
                name
            );
        } else {
            // 本地将 session.json 标记为 left
            if let Ok(Some(mut sf)) = session::read_session(&name) {
                sf.session.status = "left".into();
                let _ = session::write_session(&name, &mut sf);
            }
        }
    }

    if let Some(resp) = leave_resp {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    }

    Ok(())
}

async fn handle_cleanup(dry_run: bool) -> Result<()> {
    let workspace_id = match crate::workspace::read_workspace()? {
        Some(wf) => wf.workspace.id,
        None => anyhow::bail!("未找到 workspace.json，请在 agtalk workspace 内执行 cleanup"),
    };
    let mut cli = Client::connect(&crate::paths::socket_path()).await?;
    let resp = cli.cleanup(&workspace_id, dry_run).await?;
    let data = match &resp {
        ServerMsg::Ok { data } => data,
        ServerMsg::Error { code, message } => anyhow::bail!("cleanup 失败 [{}]: {}", code, message),
        _ => anyhow::bail!("cleanup 返回异常"),
    };
    let removed = data
        .get("removed")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if dry_run {
        anstream::println!(
            "{} 干跑模式，以下 participant 的 inactive session 将被清理：",
            label("INFO", anstyle::AnsiColor::Cyan)
        );
    } else {
        // 删除返回的本地 session 文件
        for name in &removed {
            if let Err(e) = session::remove_session(name) {
                anstream::println!(
                    "{} 无法删除 {} 的本地 session 文件: {}",
                    label("WARN", anstyle::AnsiColor::Yellow),
                    name,
                    e
                );
            }
        }
        anstream::println!(
            "{} 已清理 {} 个 inactive session",
            label("OK", anstyle::AnsiColor::Green),
            removed.len()
        );
    }
    for name in &removed {
        println!("  - {}", name);
    }
    Ok(())
}

pub(crate) async fn handle_me() -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let resp = cli.whoami().await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

pub(crate) async fn handle_peers(args: &PeersArgs) -> Result<()> {
    // peers 不需要认证，任何人都可以查看在线参与者列表
    let mut cli = Client::connect(&crate::paths::socket_path()).await?;
    let resp = cli.list_participants(None).await?;
    print_peers(&resp, args.verbose)?;
    Ok(())
}

pub(crate) async fn handle_chats() -> Result<()> {
    let identity = identity::resolve_identity(None)?;
    let mut cli =
        Client::connect_and_auth(&identity.socket, &identity.session_id, &identity.token).await?;
    let resp = cli
        .list_conversations(Some(&identity.participant_name))
        .await?;
    print_chats(&resp)?;
    Ok(())
}

async fn handle_init() -> Result<()> {
    anstream::println!(
        "{} 初始化 agtalk 环境...",
        label("INFO", anstyle::AnsiColor::Cyan)
    );
    let dir = crate::paths::config_dir();
    std::fs::create_dir_all(&dir)?;
    println!("  配置目录: {:?}", dir);

    // 生成默认 config.json（如果不存在）
    let cfg = crate::config::AgConfig::load()?;
    println!("  配置文件: {:?}", crate::paths::config_json_path());

    // 生成附件目录
    let attachment_dir = cfg.attachment_dir()?;
    std::fs::create_dir_all(&attachment_dir)?;
    println!("  附件目录: {:?}", attachment_dir);

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
    match cmd {
        "start" => daemon_start().await,
        "stop" => daemon_stop().await,
        "restart" => {
            daemon_stop().await?;
            daemon_start().await
        }
        "status" => daemon_status().await,
        _ => {
            eprintln!("用法: agtalk daemon start|stop|restart|status");
            Ok(())
        }
    }
}

async fn daemon_start() -> Result<()> {
    anstream::println!("{} 检查 daemon 状态...", label("INFO", anstyle::AnsiColor::Cyan));
    let state = daemon::check_state().await;
    match &state {
        daemon::DaemonState::Running { pid } => {
            anstream::println!(
                "{} daemon 已运行 (pid {})",
                label("OK", anstyle::AnsiColor::Green),
                pid
            );
            return Ok(());
        }
        daemon::DaemonState::Starting { pid, reason, .. } => {
            anstream::println!(
                "{} 检测到 daemon 启动中 (pid {}): {}",
                label("WARN", anstyle::AnsiColor::Yellow),
                pid,
                reason
            );
        }
        daemon::DaemonState::Stale { reason, .. } => {
            anstream::println!(
                "{} 检测到 stale 文件: {}，准备清理",
                label("WARN", anstyle::AnsiColor::Yellow),
                reason
            );
            daemon::clean_stale_files()?;
        }
        daemon::DaemonState::NotRunning => {}
    }

    anstream::println!("{} 启动 daemon...", label("INFO", anstyle::AnsiColor::Cyan));
    let exe = std::env::current_exe().unwrap_or_default();
    let mut child_cmd = std::process::Command::new(&exe);
    child_cmd
        .arg("__daemon")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    daemon::apply_no_proxy(&mut child_cmd);
    let mut child = child_cmd.spawn().context("无法 spawn daemon 子进程")?;

    match daemon::wait_for_running(std::time::Duration::from_secs(5)).await {
        Ok(daemon::DaemonState::Running { pid }) => {
            anstream::println!(
                "{} daemon 已启动 (pid {})",
                label("OK", anstyle::AnsiColor::Green),
                pid
            );
            Ok(())
        }
        Ok(other) => {
            let _ = child.kill();
            daemon::clean_stale_files().ok();
            anyhow::bail!(
                "daemon 启动后状态异常: {}\n{}",
                other,
                daemon_diagnostic()
            );
        }
        Err(e) => {
            let _ = child.kill();
            daemon::clean_stale_files().ok();
            anyhow::bail!("{}\n{}", e, daemon_diagnostic());
        }
    }
}

async fn daemon_stop() -> Result<()> {
    let state = daemon::check_state().await;
    match &state {
        daemon::DaemonState::Running { pid } | daemon::DaemonState::Starting { pid, .. } => {
            anstream::println!(
                "{} 停止 daemon (pid {})...",
                label("INFO", anstyle::AnsiColor::Cyan),
                pid
            );
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if matches!(daemon::check_state().await, daemon::DaemonState::NotRunning) {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            daemon::clean_stale_files().ok();
            if matches!(daemon::check_state().await, daemon::DaemonState::NotRunning) {
                anstream::println!("{} daemon 已停止", label("OK", anstyle::AnsiColor::Green));
            } else {
                anstream::println!(
                    "{} daemon 未能优雅停止，已清理残留文件",
                    label("WARN", anstyle::AnsiColor::Yellow)
                );
            }
        }
        daemon::DaemonState::Stale { reason, .. } => {
            anstream::println!(
                "{} 检测到 stale 文件: {}，执行清理",
                label("WARN", anstyle::AnsiColor::Yellow),
                reason
            );
            daemon::clean_stale_files()?;
        }
        daemon::DaemonState::NotRunning => {
            anstream::println!(
                "{} daemon 未运行",
                label("WARN", anstyle::AnsiColor::Yellow)
            );
        }
    }
    Ok(())
}

async fn daemon_status() -> Result<()> {
    let state = daemon::check_state().await;
    match &state {
        daemon::DaemonState::Running { pid } => {
            anstream::println!(
                "daemon: {} (pid {}, {})",
                label("运行中", anstyle::AnsiColor::Green),
                pid,
                daemon::socket_path()
            );
        }
        daemon::DaemonState::Starting { pid, reason, .. } => {
            anstream::println!(
                "daemon: {} (pid {}, {})",
                label("启动中", anstyle::AnsiColor::Yellow),
                pid,
                reason
            );
        }
        daemon::DaemonState::Stale { reason, .. } => {
            anstream::println!(
                "daemon: {} ({})",
                label("残留状态", anstyle::AnsiColor::Red),
                reason
            );
            anstream::println!("  建议执行: agtalk daemon restart");
        }
        daemon::DaemonState::NotRunning => {
            anstream::println!(
                "daemon: {}",
                label("未运行", anstyle::AnsiColor::Yellow)
            );
        }
    }
    if daemon::has_network_proxy() {
        anstream::println!(
            "  代理环境: {}（agtalk 本地 IPC 使用 Unix socket，不应经过网络代理）",
            daemon::proxy_env_summary()
        );
    }
    Ok(())
}

fn daemon_diagnostic() -> String {
    let mut lines = vec![
        format!("socket 路径: {}", daemon::socket_path()),
        format!("pid 路径: {:?}", daemon::pid_path()),
        format!("代理环境: {}", daemon::proxy_env_summary()),
        "建议:".to_string(),
        "  1. 检查是否有其他 agtalk daemon 占用了 socket。".to_string(),
        "  2. 若刚升级 binary，执行 `agtalk daemon restart` 清理 stale 文件。".to_string(),
        "  3. 若使用代理工具，请确保 Unix socket 路径未被代理拦截。".to_string(),
    ];
    if daemon::has_network_proxy() {
        lines.push(format!(
            "  4. 启动 daemon 时已自动注入 NO_PROXY={}，如仍失败请检查代理工具是否覆盖。",
            daemon::no_proxy_value()
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{parse_agent_args, parse_human_args, parse_join_args};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_human_message_with_options() {
        let (questions, _, _, output_json) = parse_human_args(&args(&[
            "是否部署？",
            "-o",
            "approve",
            "-o",
            "reject",
            "--output",
            "json",
        ]));
        assert!(output_json);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].message, "是否部署？");
        assert_eq!(
            questions[0].options,
            vec![
                ("approve".to_string(), false),
                ("reject".to_string(), false),
            ]
        );
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
            "-q",
            "部署环境？",
            "-o",
            "staging",
            "-o!",
            "production",
            "-q",
            "是否清缓存？",
            "-o",
            "yes",
            "-o",
            "no",
        ]));
        assert_eq!(questions.len(), 2);
        assert_eq!(
            questions[0].options,
            vec![
                ("staging".to_string(), false),
                ("production".to_string(), true),
            ]
        );
        assert_eq!(
            questions[1].options,
            vec![("yes".to_string(), false), ("no".to_string(), false),]
        );
    }

    #[test]
    fn parse_human_select_only_without_options_keeps_empty_options_for_validation() {
        let (questions, _, select_only, _) =
            parse_human_args(&args(&["是否部署？", "--select-only"]));
        assert!(select_only);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].message, "是否部署？");
        assert!(questions[0].options.is_empty());
    }

    #[test]
    fn parse_agent_send() {
        let parsed = parse_agent_args(&args(&[
            "请",
            "review",
            "-n",
            "reviewer",
            "-s",
            "代码评审",
            "-r",
            "msg-1",
            "-i",
        ]))
        .unwrap();
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
    fn parse_agent_files() {
        let parsed = parse_agent_args(&args(&[
            "请 review",
            "-n",
            "reviewer",
            "-f",
            "./src/main.rs",
            "--file",
            "README.md",
        ]))
        .unwrap();
        assert_eq!(parsed.files, vec!["./src/main.rs", "README.md"]);
    }

    #[test]
    fn parse_join() {
        let parsed = parse_join_args(&args(&[
            "agent-x",
            "--intro",
            "CLI agent",
            "--transport",
            "popup",
        ]));
        assert_eq!(parsed.name, "agent-x");
        assert_eq!(parsed.intro.as_deref(), Some("CLI agent"));
        assert_eq!(parsed.transport, "popup");
    }
}
