//! CLI 子命令枚举定义（clap derive；与 mod.rs 拆分控制行数红线）。

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum Commands {
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
    /// 图工程（docs/design_graph.md）
    Graph {
        #[command(subcommand)]
        cmd: GraphCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum GraphCmd {
    /// 提交并运行图工程 spec（自动探测当前目录 git 的 repository/base_revision；
    /// 名字自动解析到 <cwd>/.agtalk/graph/<name>.yaml，也可传显式路径）
    Submit {
        /// spec 名字或 YAML 文件路径
        spec: PathBuf,
    },
    /// 列出 GraphRun
    List {
        #[arg(long)]
        status: Option<String>,
    },
    /// GraphRun 详情（节点状态/attempt/失败原因）
    Status { run_id: String },
    /// GraphRun 事件日志
    Logs {
        run_id: String,
        #[arg(long)]
        since: Option<i64>,
    },
    /// 取消运行
    Cancel { run_id: String },
    /// 图成本分析（docs/graph-engineering-survey.md §5）：本地评估，不建图
    Analyze {
        /// spec 名字或 YAML 文件路径（同 submit 解析规则）
        spec: PathBuf,
    },
    /// 以新 spec 打补丁更新图定义（仅 paused/ready 图；spec 名字解析同 submit）
    Patch {
        run_id: String,
        /// spec 名字或 YAML 文件路径
        spec: PathBuf,
    },
    /// 节点上报（participant 执行者用，docs/graph-participant-protocol.md §4）
    Node {
        #[command(subcommand)]
        cmd: GraphNodeCmd,
    },
    /// 拉起图工程管理界面（M4 预留）
    Gui { run_id: Option<String> },
}

#[derive(Subcommand)]
pub(crate) enum GraphNodeCmd {
    /// 心跳续租（长任务周期上报）
    Heartbeat {
        #[arg(long = "run")]
        run_id: String,
        #[arg(long = "node")]
        node_key: String,
        #[arg(long)]
        attempt: u32,
    },
    /// 提交候选结果（result.json 见 protocol §6）
    Result {
        #[arg(long = "run")]
        run_id: String,
        #[arg(long = "node")]
        node_key: String,
        #[arg(long)]
        attempt: u32,
        #[arg(long)]
        file: PathBuf,
    },
    /// 上报阻塞
    Blocker {
        #[arg(long = "run")]
        run_id: String,
        #[arg(long = "node")]
        node_key: String,
        #[arg(long)]
        attempt: u32,
        #[arg(long)]
        blocker: String,
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
    /// 生成身份接管提示词（3 行可复制文本；缺身份不自动 join，报错引导）
    Prompt {
        /// 目标身份名（省略时按 --as > AGTALK_NAME > 当前目录唯一 session 解析）
        name: Option<String>,
    },
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
        /// 发送后不等待回复（默认阻塞等待人类回复，超时 300 秒，可用 --timeout 覆盖）
        #[arg(long)]
        no_wait: bool,
        /// 等待回复的超时秒数（默认 300）
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
    List {
        /// 按 specialty 大小写不敏感精确匹配过滤
        #[arg(short, long)]
        specialty: Option<String>,
    },
    /// 查看某个 peer 的关系详情
    Show { name_or_address: String },
    /// 更新 peer 的手动字段（role / tags / note / specialties / preferred_for）
    Update {
        name_or_address: String,
        #[arg(short, long)]
        role: Option<String>,
        #[arg(short, long, value_delimiter = ',')]
        tag: Vec<String>,
        #[arg(short, long)]
        note: Option<String>,
        #[arg(short, long, value_delimiter = ',')]
        specialty: Vec<String>,
        #[arg(short = 'p', long, value_delimiter = ',')]
        preferred_for: Vec<String>,
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
    /// 打开 GUI 配置界面
    Gui,
}
