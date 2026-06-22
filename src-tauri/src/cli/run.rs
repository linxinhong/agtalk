//! `agtalk run <file.yaml>` 通用 YAML Runner。
//!
//! Runner 只执行 agtalk 内部命令，不执行任意 shell。
//! YAML 中的相对路径按 YAML 文件所在目录解析。

use super::dispatch::{
    handle_agent, handle_ask_flow, handle_attachment, handle_chats, handle_detail, handle_inbox,
    handle_me, handle_peers, handle_reply, handle_wait, run_mem_add, run_mem_archive,
    run_mem_pack, run_mem_promote, run_mem_search, run_mem_show, run_mem_topic_add,
    run_mem_topic_list, run_mem_topic_show, run_mem_topic_update, run_mem_update, AgentArgs,
    AttachmentArgs, DetailArgs, InboxArgs, PeersArgs, QuestionArgs, ReplyCommand, WaitArgs,
};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const SUPPORTED_VERSION: i64 = 1;

fn default_output() -> String {
    "text".to_string()
}

fn default_timeout() -> u64 {
    300
}

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Deserialize)]
struct RunSpec {
    version: i64,
    #[serde(flatten)]
    command: RunCommandSpec,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum RunCommandSpec {
    Agent(AgentSpec),
    Human(HumanSpec),
    Reply(ReplySpec),
    Wait(WaitSpec),
    Inbox(InboxSpec),
    Detail(DetailSpec),
    Attachment(AttachmentSpec),
    Chats,
    Peers(PeersSpec),
    Me,
    Mem(Box<MemSpec>),
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AgentSpec {
    name: Option<String>,
    subject: Option<String>,
    message: String,
    reply_to: Option<String>,
    done: Option<String>,
    notify: bool,
    no_enter: bool,
    files: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct HumanSpec {
    message: Option<String>,
    single: bool,
    select_only: bool,
    #[serde(default = "default_output")]
    output: String,
    questions: Vec<QuestionSpec>,
}

#[derive(Debug, Deserialize)]
struct QuestionSpec {
    text: String,
    #[serde(default)]
    options: Vec<OptionSpec>,
}

#[derive(Debug, Deserialize)]
struct OptionSpec {
    text: String,
    #[serde(default)]
    recommended: bool,
}

#[derive(Debug, Deserialize)]
struct ReplySpec {
    msg_id: String,
    choice: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct WaitSpec {
    msg_id: String,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default = "default_output")]
    output: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct InboxSpec {
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    peek: bool,
}

#[derive(Debug, Deserialize)]
struct DetailSpec {
    msg_id: String,
}

#[derive(Debug, Deserialize)]
struct AttachmentSpec {
    attachment_id: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PeersSpec {
    verbose: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct MemSpec {
    mem_command: String,
    slug: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    aliases: Vec<String>,
    priority: Option<i32>,
    archive: bool,
    all: bool,
    content: Option<String>,
    #[serde(rename = "type")]
    item_type: Option<String>,
    topics: Vec<String>,
    tags: Option<String>,
    importance: Option<i32>,
    confidence: Option<String>,
    scope: Option<String>,
    status: Option<String>,
    mem_id: Option<String>,
    source_ref: Option<String>,
    #[serde(rename = "source_type")]
    source_type: Option<String>,
    query: Option<String>,
    topic_slug: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
}

/// 解析并执行 YAML Runner 文件。
pub async fn handle_run(file: &str) -> Result<()> {
    let path = Path::new(file);
    let base_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let yaml = std::fs::read_to_string(file)
        .with_context(|| format!("无法读取 YAML 文件: {}", file))?;

    let spec: RunSpec = serde_yaml::from_str(&yaml)
        .with_context(|| format!("解析 YAML 失败: {} (请检查字段名与 command/mem_command 是否匹配)", file))?;

    if spec.version != SUPPORTED_VERSION {
        anyhow::bail!(
            "{}: 不支持的 version: {}，仅支持 {}",
            file,
            spec.version,
            SUPPORTED_VERSION
        );
    }

    match spec.command {
        RunCommandSpec::Agent(s) => run_agent(&base_dir, s, file).await,
        RunCommandSpec::Human(s) => run_human(s, file).await,
        RunCommandSpec::Reply(s) => run_reply(s, file).await,
        RunCommandSpec::Wait(s) => run_wait(s, file).await,
        RunCommandSpec::Inbox(s) => run_inbox(s, file).await,
        RunCommandSpec::Detail(s) => run_detail(s, file).await,
        RunCommandSpec::Attachment(s) => run_attachment(s, file).await,
        RunCommandSpec::Chats => handle_chats().await,
        RunCommandSpec::Peers(s) => handle_peers(&PeersArgs { verbose: s.verbose }).await,
        RunCommandSpec::Me => handle_me().await,
        RunCommandSpec::Mem(s) => run_mem(*s).await,
    }
}

async fn run_mem(spec: MemSpec) -> Result<()> {
    let item_type = spec.item_type.clone().unwrap_or_else(|| "fact".to_string());
    let confidence = spec.confidence.clone().unwrap_or_else(|| "medium".to_string());
    let scope = spec.scope.clone().unwrap_or_else(|| "project".to_string());
    let source_type = spec.source_type.clone().unwrap_or_else(|| "message".to_string());
    let priority = spec.priority.unwrap_or(3);
    let importance = spec.importance.unwrap_or(3);

    match spec.mem_command.as_str() {
        "topic_add" => {
            let slug = spec.slug.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem topic_add 缺少必填字段 'slug'"))?;
            let title = spec.title.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem topic_add 缺少必填字段 'title'"))?;
            run_mem_topic_add(slug, title, spec.summary, spec.aliases, priority).await
        }
        "topic_list" => run_mem_topic_list(spec.all).await,
        "topic_show" => {
            let slug = spec.slug.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem topic_show 缺少必填字段 'slug'"))?;
            run_mem_topic_show(slug).await
        }
        "topic_update" => {
            let slug = spec.slug.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem topic_update 缺少必填字段 'slug'"))?;
            run_mem_topic_update(
                slug,
                spec.title,
                spec.summary,
                spec.aliases,
                spec.priority,
                spec.archive,
            )
            .await
        }
        "add" => {
            let content = spec.content.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem add 缺少必填字段 'content'"))?;
            run_mem_add(
                content,
                item_type,
                spec.title,
                spec.summary,
                spec.topics,
                spec.tags,
                importance,
                confidence,
                scope,
            )
            .await
        }
        "show" => {
            let mem_id = spec.mem_id.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem show 缺少必填字段 'mem_id'"))?;
            run_mem_show(mem_id).await
        }
        "update" => {
            let mem_id = spec.mem_id.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem update 缺少必填字段 'mem_id'"))?;
            run_mem_update(
                mem_id,
                spec.content,
                spec.title,
                spec.summary,
                spec.topics,
                spec.tags,
                spec.importance,
                spec.status,
            )
            .await
        }
        "archive" => {
            let mem_id = spec.mem_id.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem archive 缺少必填字段 'mem_id'"))?;
            run_mem_archive(mem_id).await
        }
        "promote" => {
            let source_ref = spec.source_ref.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem promote 缺少必填字段 'source_ref'"))?;
            run_mem_promote(
                source_ref,
                source_type,
                item_type,
                spec.title,
                spec.summary,
                spec.topics,
                spec.tags,
                importance,
                confidence,
            )
            .await
        }
        "search" => {
            run_mem_search(spec.query, spec.topics, spec.item_type.clone(), spec.scope.clone(), spec.limit).await
        }
        "pack" => {
            let topic_slug = spec.topic_slug.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("mem pack 缺少必填字段 'topic_slug'"))?;
            run_mem_pack(topic_slug, spec.limit).await
        }
        other => anyhow::bail!("不支持的 mem_command: {}", other),
    }
}

/// 当前 Agent 的固定 YAML Runner 路径。
///
/// Agent 可反复覆盖该文件，再运行 `agtalk run`。这样命令入口保持不变，
/// 适合需要对固定命令做授权的执行环境。
pub fn default_run_file() -> Result<PathBuf> {
    let identity = crate::identity::resolve_identity(None)?;
    crate::paths::run_yaml_path(&identity.participant_name)
}

async fn run_agent(base_dir: &Path, spec: AgentSpec, file: &str) -> Result<()> {
    if spec.done.is_none() && spec.name.is_none() {
        anyhow::bail!("{}: agent 命令在 done 为空时缺少必填字段 'name'", file);
    }
    if spec.done.is_none() && spec.message.is_empty() {
        anyhow::bail!(
            "{}: agent 命令在 done 为空时缺少非空字段 'message'",
            file
        );
    }

    let files = resolve_files(&spec.files, base_dir)?;

    let args = AgentArgs {
        message: spec.message,
        name: spec.name,
        subject: spec.subject,
        reply_to: spec.reply_to,
        done: spec.done,
        files,
        notify: spec.notify,
        no_enter: spec.no_enter,
    };
    handle_agent(args).await
}

async fn run_human(spec: HumanSpec, file: &str) -> Result<()> {
    let mut questions: Vec<QuestionArgs> = Vec::new();

    for (i, q) in spec.questions.iter().enumerate() {
        if q.text.is_empty() {
            anyhow::bail!(
                "{}: questions[{}].text 不能为空",
                file,
                i
            );
        }
        questions.push(QuestionArgs {
            message: q.text.clone(),
            options: q
                .options
                .iter()
                .map(|o| (o.text.clone(), o.recommended))
                .collect(),
        });
    }

    if questions.is_empty() {
        let message = spec
            .message
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("{}: human 命令缺少 message 或 questions", file))?;
        questions.push(QuestionArgs {
            message: message.to_string(),
            options: vec![],
        });
    }

    if spec.select_only && questions.iter().any(|q| q.options.is_empty()) {
        anyhow::bail!(
            "{}: human 命令 select_only=true 要求每题至少有一个 option",
            file
        );
    }

    let output_json = spec.output == "json";
    handle_ask_flow(&questions, spec.single, spec.select_only, output_json).await
}

async fn run_reply(spec: ReplySpec, file: &str) -> Result<()> {
    if spec.msg_id.is_empty() {
        anyhow::bail!("{}: reply 命令缺少必填字段 'msg_id'", file);
    }
    if spec.choice.is_empty() {
        anyhow::bail!("{}: reply 命令缺少必填字段 'choice'", file);
    }
    handle_reply(&ReplyCommand {
        msg_id: spec.msg_id,
        choice: spec.choice,
        reason: spec.reason,
    })
    .await
}

async fn run_wait(spec: WaitSpec, file: &str) -> Result<()> {
    if spec.msg_id.is_empty() {
        anyhow::bail!("{}: wait 命令缺少必填字段 'msg_id'", file);
    }
    handle_wait(&WaitArgs {
        msg_id: spec.msg_id,
        timeout: spec.timeout,
        output: spec.output,
    })
    .await
}

async fn run_inbox(spec: InboxSpec, file: &str) -> Result<()> {
    let (unread, pending, action_required, all) = match spec.status.as_deref() {
        None | Some("") => (false, false, false, false),
        Some("unread") => (true, false, false, false),
        Some("pending") => (false, true, false, false),
        Some("action_required") => (false, false, true, false),
        Some("all") => (false, false, false, true),
        Some(other) => anyhow::bail!(
            "{}: inbox 命令 status 不支持 '{}',
            可选值: unread/pending/action_required/all",
            file,
            other
        ),
    };
    handle_inbox(&InboxArgs {
        peek: spec.peek,
        unread,
        pending,
        action_required,
        all,
        limit: spec.limit,
    })
    .await
}

async fn run_detail(spec: DetailSpec, file: &str) -> Result<()> {
    if spec.msg_id.is_empty() {
        anyhow::bail!("{}: detail 命令缺少必填字段 'msg_id'", file);
    }
    handle_detail(&DetailArgs { msg_id: spec.msg_id }).await
}

async fn run_attachment(spec: AttachmentSpec, file: &str) -> Result<()> {
    if spec.attachment_id.is_empty() {
        anyhow::bail!(
            "{}: attachment 命令缺少必填字段 'attachment_id'",
            file
        );
    }
    handle_attachment(&AttachmentArgs {
        attachment_id: spec.attachment_id,
    })
    .await
}

/// 将 YAML 中的文件路径按 base_dir 解析为规范化的绝对路径或相对路径。
fn resolve_files(files: &[String], base_dir: &Path) -> Result<Vec<String>> {
    let mut resolved = Vec::with_capacity(files.len());
    for f in files {
        let p = Path::new(f);
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            base_dir.join(p)
        };
        resolved.push(normalize_path(&full).to_string_lossy().to_string());
    }
    Ok(resolved)
}

/// 简单路径规范化：去掉 `.` 并回退 `..`。
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::Normal(_) => out.push(comp),
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::RootDir => out.push(comp),
            std::path::Component::CurDir => {}
            std::path::Component::Prefix(p) => out.push(p.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_yaml(content: &str) -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("run.yaml");
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        let path_str = file.to_string_lossy().to_string();
        (dir, file, path_str)
    }

    #[test]
    fn parse_agent_yaml() {
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: agent
name: reviewer
subject: "TASK"
message: "请 review"
reply_to: msg-1
done: null
notify: true
no_enter: false
files:
  - ./note.md
"#,
        );
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(spec.version, 1);
        let RunCommandSpec::Agent(a) = spec.command else {
            panic!("expected agent");
        };
        assert_eq!(a.name.as_deref(), Some("reviewer"));
        assert_eq!(a.subject.as_deref(), Some("TASK"));
        assert_eq!(a.message, "请 review");
        assert_eq!(a.reply_to.as_deref(), Some("msg-1"));
        assert!(a.notify);
        assert!(!a.no_enter);
        assert_eq!(a.files, vec!["./note.md"]);
    }

    #[test]
    fn parse_agent_to_args_matches_cli() {
        let (dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: agent
name: reviewer
subject: TASK
message: 请 review
reply_to: msg-1
notify: true
files:
  - ./note.md
"#,
        );
        let base_dir = Path::new(&path).parent().unwrap().to_path_buf();
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let RunCommandSpec::Agent(a) = spec.command else {
            panic!("expected agent");
        };
        let files = resolve_files(&a.files, &base_dir).unwrap();
        let args = AgentArgs {
            message: a.message,
            name: a.name,
            subject: a.subject,
            reply_to: a.reply_to,
            done: a.done,
            files,
            notify: a.notify,
            no_enter: a.no_enter,
        };
        assert_eq!(args.message, "请 review");
        assert_eq!(args.name.as_deref(), Some("reviewer"));
        assert_eq!(args.subject.as_deref(), Some("TASK"));
        assert_eq!(args.reply_to.as_deref(), Some("msg-1"));
        assert!(args.notify);
        assert_eq!(args.files.len(), 1);
        assert!(args.files[0].starts_with(dir.path().to_string_lossy().as_ref()));
        assert!(args.files[0].ends_with("note.md"));
    }

    #[test]
    fn parse_human_yaml() {
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: human
message: "部署前确认"
single: true
select_only: true
output: json
questions:
  - text: "是否继续？"
    options:
      - text: "继续"
        recommended: true
      - text: "停止"
"#,
        );
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let RunCommandSpec::Human(h) = spec.command else {
            panic!("expected human");
        };
        assert_eq!(h.message.as_deref(), Some("部署前确认"));
        assert!(h.single);
        assert!(h.select_only);
        assert_eq!(h.output, "json");
        assert_eq!(h.questions.len(), 1);
        assert_eq!(h.questions[0].text, "是否继续？");
        assert_eq!(h.questions[0].options.len(), 2);
        assert_eq!(h.questions[0].options[0].text, "继续");
        assert!(h.questions[0].options[0].recommended);
        assert!(!h.questions[0].options[1].recommended);

        let questions: Vec<QuestionArgs> = h
            .questions
            .iter()
            .map(|q| QuestionArgs {
                message: q.text.clone(),
                options: q
                    .options
                    .iter()
                    .map(|o| (o.text.clone(), o.recommended))
                    .collect(),
            })
            .collect();
        assert_eq!(questions[0].options, vec![("继续".into(), true), ("停止".into(), false)]);
    }

    #[test]
    fn parse_reply_yaml() {
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: reply
msg_id: "12345678"
choice: "允许"
reason: "已确认"
"#,
        );
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let RunCommandSpec::Reply(r) = spec.command else {
            panic!("expected reply");
        };
        assert_eq!(r.msg_id, "12345678");
        assert_eq!(r.choice, "允许");
        assert_eq!(r.reason.as_deref(), Some("已确认"));
    }

    #[test]
    fn parse_wait_yaml() {
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: wait
msg_id: "12345678"
timeout: 60
output: json
"#,
        );
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let RunCommandSpec::Wait(w) = spec.command else {
            panic!("expected wait");
        };
        assert_eq!(w.msg_id, "12345678");
        assert_eq!(w.timeout, 60);
        assert_eq!(w.output, "json");
    }

    #[test]
    fn reject_unsupported_version() {
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 2
command: me
"#,
        );
        let result: Result<RunSpec, _> =
            serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap());
        // version 为 2 的 YAML 仍能解析，应由 handle_run 负责校验。
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.version, 2);
    }

    #[test]
    fn agent_missing_name_errors() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: agent
message: "hello"
"#,
        );
        let err = rt.block_on(handle_run(&path)).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("name"), "错误应包含字段名: {}", msg);
        assert!(msg.contains(&path), "错误应包含文件路径: {}", msg);
    }

    #[test]
    fn agent_missing_message_errors() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: agent
name: reviewer
"#,
        );
        let err = rt.block_on(handle_run(&path)).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("message"), "错误应包含字段名: {}", msg);
    }

    #[test]
    fn human_empty_message_and_questions_errors() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_dir, _file, path) = write_temp_yaml(
            r#"
version: 1
command: human
"#,
        );
        let err = rt.block_on(handle_run(&path)).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("message") || msg.contains("questions"), "{}", msg);
    }

    #[test]
    fn resolve_relative_files_from_yaml_dir() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let yaml = sub.join("run.yaml");
        let attachment = sub.join("data.txt");
        std::fs::File::create(&attachment).unwrap();
        std::fs::write(&yaml, "version: 1\ncommand: agent\nfiles:\n  - ./data.txt\n").unwrap();

        let base_dir = yaml.parent().unwrap().to_path_buf();
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&yaml).unwrap()).unwrap();
        let RunCommandSpec::Agent(a) = spec.command else {
            panic!("expected agent");
        };
        let files = resolve_files(&a.files, &base_dir).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], attachment.to_string_lossy().to_string());
    }

    #[test]
    fn absolute_files_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("abs.txt");
        std::fs::File::create(&file).unwrap();
        let yaml = dir.path().join("run.yaml");
        std::fs::write(
            &yaml,
            format!(
                "version: 1\ncommand: agent\nfiles:\n  - {}\n",
                file.to_string_lossy()
            ),
        )
        .unwrap();

        let base_dir = yaml.parent().unwrap().to_path_buf();
        let spec: RunSpec = serde_yaml::from_str(&std::fs::read_to_string(&yaml).unwrap()).unwrap();
        let RunCommandSpec::Agent(a) = spec.command else {
            panic!("expected agent");
        };
        let files = resolve_files(&a.files, &base_dir).unwrap();
        assert_eq!(files[0], file.to_string_lossy().to_string());
    }
}
