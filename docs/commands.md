# agtalk 命令参考

> 状态：NG 命令面（已实现）
> 目标：agent-first、命令克制、知识通用、避免旧版功能膨胀
> 约束：路由只认 UUID；name 只用于本地身份选择与 lookup 消歧

---

## 1. 设计目标

agtalk 是本地 Agent 对话总线。命令面必须服务 agent 的稳定心智模型，而不是为每个功能新增一个顶层命令。

新版命令面只保留少数稳定领域，加一个高频编排入口：

```text
daemon   daemon 生命周期
id       身份与寻址
msg      消息与询问
mem      计划、上下文与长期记忆
tool     运行时辅助
config   配置
run      发送/协作 spec 优先入口
```

`run` 是 agent 优先使用的发送/协作入口，不限单次或多次发送。只要消息发送需要模板化、复用或记录发送意图，就可以优先用 `run`。它只执行 agtalk 内部白名单动作，不执行任意 shell。

---

## 2. 全局规则

### 2.1 路由规则

- 消息投递只认 address UUID。
- name 不唯一，只用于展示、本地身份选择、lookup 消歧。
- 不提供按 name 发送消息的能力。

### 2.2 身份规则

身份在当前 workspace 的文件系统中：

```text
.agtalk/
  agents.json
  <agent-name>/
    session.json
    history.jsonl
    relations.json
    memory/
      plan.md
      context.md
      status.json
      entries.jsonl
```

- `session.json` 是身份/认证锚点，只保存低频身份字段。
- `history.jsonl` 与 `relations.json` 由消息收发自动维护，是 agent 私有的协作视图。
- `memory/` 是 agent 自己的知识与工作现场，不能参与认证、路由、PID 校验。

daemon SQLite 里只保存全局可见的 mem 注册/索引，不是长期记忆仓库：

- agent 上线（`id join`）后，daemon 才从 `.agtalk/<agent-name>/memory/` 注册它的 mem。
- agent 下线或 `id leave` 后，daemon 从 SQLite 移除该 agent 的 mem 注册/索引。
- SQLite 中的 mem 记录是派生视图，可重建；agent 自己的长期记录以文件系统为准。
- 其他 agent 只能看到在线 agent 已注册的公开计划、状态和允许公开的索引。

### 2.3 全局参数

```bash
agtalk --as <name> <command>
agtalk --json <command>
AGTALK_NAME=<name> agtalk <command>
```

- `--as` 和 `AGTALK_NAME` 只选择本地 session，不参与路由。
- `--json` 输出稳定 JSON，供 agent 和脚本消费。
- 人类文本输出只是辅助，不作为 agent 的稳定解析接口。

---

## 3. 顶层命令

```bash
agtalk daemon ...
agtalk id ...
agtalk msg ...
agtalk mem ...
agtalk tool ...
agtalk config ...
agtalk run [spec-name|path.yaml]
```

不再新增这些顶层命令：

```text
agent human ask plan browser peers me chats attachment poll-inbox gui settings
```

这些语义必须归入 `daemon`、`id`、`msg`、`mem`、`tool`、`config` 或 `run`。

---

## 4. daemon：daemon 生命周期

```bash
agtalk daemon start
agtalk daemon stop
agtalk daemon status
agtalk daemon restart
```

daemon 是 agtalk 的唯一真相来源，不放进 `tool`。人类或启动脚本通常只需要执行 `agtalk daemon start`。

---

## 5. id：身份与寻址

### 5.1 id join

```bash
agtalk id join [name] [--intro <text>] [--notify <channel>]
```

创建或复用当前工作目录下的 agent 身份。

- 如果 `.agtalk/<name>/session.json` 已存在，复用原 address，只重新绑定当前进程/会话锚点。
- 如果不存在，创建新 mailbox、session、agents.json 记录。
- `--intro` 传入时更新展示元数据；未传入则保留旧值。
- `--notify` 默认 `auto`。
- 上线成功后注册或刷新 `.agtalk/<name>/memory/` 到 SQLite 的在线 mem 索引。

notify channel：

```text
auto              由 CLI 依次尝试 plugin:zellij / plugin:tmux discover，否则 none
plugin:<name>     由 CLI 调用外部 notify 插件 discover（如 zellij、tmux、macos、webhook）
none              关闭打扰，仅 pull
```

zellij/tmux 已迁出 agtalk core，项目提供对应 shell 脚本插件，复制到 `<config_dir>/plugins/` 即可：

```bash
mkdir -p ~/.config/agtalk2/plugins

cp plugins/agtalk-notify-zellij ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-zellij
agtalk id join coder --notify plugin:zellij

cp plugins/agtalk-notify-tmux ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-tmux
agtalk id join coder --notify plugin:tmux
```

如果插件放在其他目录，再用 `agtalk config set notify.plugins.<name>.path <abs-path>` 显式登记。

### 5.2 id show

```bash
agtalk id show
agtalk --json id show
```

显示当前身份。

JSON 输出：

```json
{
  "type": "identity",
  "address": "550e8400-e29b-41d4-a716-446655440000",
  "name": "codex-coder-Alex",
  "intro": "代码实现 agent"
}
```

### 5.3 id lookup

```bash
agtalk id lookup [name]
agtalk --json id lookup [name]
```

查询候选 mailbox，供调用方消歧。

- 无参：列出全部活跃 mailbox。
- 有参：按 name 过滤。
- 返回 address、name、intro、notify、notify_ready。
- `notify` 是目标 agent 注册时声明的打扰通道摘要：plugin:<name>、none、unknown。
- `notify_ready=true` 表示目标有可用 notify 通道；agent 可据此决定是否发送后接 `wait`。
- 不做按 name 路由。

### 5.4 id leave

```bash
agtalk id leave [--purge]
```

注销当前身份。

- 标记 mailbox left。
- 删除 `.agtalk/<name>/`。
- 移除当前 PID/session 锚点。
- 移除 SQLite 中该 agent 的在线 mem 索引。
- `--purge` 保留给“即使 session 已失效也删除本地凭证”的场景。

### 5.5 id cleanup

```bash
agtalk id cleanup [--execute]
```

批量清理当前工作目录下无效或未激活的 agent 身份。默认 dry-run，只列出会被清理的项；加 `--execute` 才真正执行。

清理范围：

- **stale_mailbox**：DB 中有 mailbox 记录，但 `.agtalk/<name>/session.json` 缺失或 address 不匹配。
- **stale_session**：`.agtalk/<name>/session.json` 存在，但 DB 中无对应活跃 mailbox。
- **stale_pid_anchor**：`agents.json` 中指向的 session 已不存在。

被清理的 mailbox 会标记为 `left`（保留历史消息），session 目录会被删除，stale pid anchor 会从 `agents.json` 移除。

文本输出示例（dry-run）：

```text
dry run: the following items would be removed

  - reviewer (550e8400-...): stale_mailbox
  - orphan (00000000-...): stale_session
  - reviewer (pid 99999): stale_pid_anchor
```

---

## 6. msg：消息与询问

### 6.1 msg send

```bash
agtalk msg send <address> <body> [--subject <text>] [--file <path> ...] [--notify] [--more]
```

按 UUID 发送消息。

- `<address>` 必须是目标 mailbox UUID。
- `--subject` 是简短任务标题，会随消息持久化，并出现在 `inbox` / `read` / `wait` / `--json` / `history.jsonl` 中；trim 后为空视为未设置。
- `msg reply` 不新增 `--subject`，会自动继承被回复消息的 subject，方便按同一任务标题扫描历史。
- `--notify` 只提醒对方查收，不传正文，也不传 subject。
- `--more` 表示后续还有同一逻辑消息的下一段。

### 6.2 msg reply

```bash
agtalk msg reply <msg-id> <body> [--file <path> ...] [--notify]
```

回复指定消息，形成 reply chain。

### 6.3 msg done

```bash
agtalk msg done <msg-id> [--body <text>] [--file <path> ...]
```

标记消息完成。可附带结果说明或附件。

`done` 是显式状态动作，避免消息长期停留在待办列表。

### 6.4 msg ask

```bash
agtalk msg ask <message> [options]
```

向 human mailbox 发送普通询问或审批请求。

常用选项：

```text
-q, --question <text>        提出问题，可多次出现
-o, --option <text>          为当前问题添加选项
--recommended <text>         添加推荐选项
--single                     单选
--select-only                禁止自由文本
--no-wait                    发送后不等待回复（默认阻塞等待，超时 300 秒）
--timeout <sec>              等待回复的超时秒数（默认 300）
```

示例：

```bash
agtalk msg ask "要继续部署吗？" --option 继续 --recommended 停止 --single --timeout 60
```

行为：

- 默认发送后经 SSE 阻塞等待人类回复，超时 300 秒；`--timeout` 覆盖；`--no-wait` 只发送不等待。
- 超时不取消 pending：询问仍在 human inbox，人类之后回复仍可通过 `msg read` / `msg wait <sent-msg-id>` 收到。
- `--json` 输出先打印 `AskResult`（message_id），等待结束后打印 `WaitResult`；超时返回稳定错误码 `timeout`。

`ask` 属于消息域，不提供顶层 `agtalk ask`。

### 6.5 msg inbox

```bash
agtalk msg inbox [--all]
```

查看当前身份的收件箱。

- 默认显示未完成消息。
- `--all` 显示全部消息，包括已完成。
- `--peek`、`--unread`、`--pending`、`--action-required`、`--limit` 是后续扩展，不进入第一阶段。

### 6.6 msg read

```bash
agtalk msg read
agtalk msg read <msg-id>
```

读取消息。

无参数时读取当前身份的所有未读消息，并标记为 read。这是 agent 工作循环的默认收信入口，不需要 `-`：

- 有未读消息：返回未读消息列表，按 event_id 升序。
- 没有未读消息：返回 `inbox_empty`。
- 不回退读取已读历史；需要看历史时用 `msg inbox --all` 或 `msg read <msg-id>`。

指定 `<msg-id>` 时读取单条消息详情，并标记为 read。

### 6.7 msg wait

```bash
agtalk msg wait [sent-msg-id] --timeout <sec> [--since <event-id>]
```

短期等待消息，底层使用 SSE。

- `<sent-msg-id>` 是自己刚发送出去的消息 ID（`msg send` 返回的 id 或 `msg ask` 返回的 message_id）。
- 无 `sent-msg-id`：收到下一条发给当前 agent 的消息即返回。
- 有 `sent-msg-id`：等待 `reply_to_id == sent-msg-id` 的回复。
- 必须有 timeout，避免占住 agent turn。

### 6.8 msg attachment

```bash
agtalk msg attachment <attachment-id>
```

查看附件全文。附件属于消息域，不作为顶层命令。

---

## 7. mem：计划、上下文与长期记忆

`mem` 是 agent 的知识与工作现场。它同时承载两类内容：

- 当前工作现场：计划、上下文、状态，其他 agent 可读，用于协作和恢复现场。
- 长期记忆：事实、决策、规则、偏好等沉淀，不默认向所有 agent 展示，通常通过 `pack` 注入上下文。

命令域叫 `mem`，磁盘目录叫 `memory`。这样命令保持短，文件系统语义保持清楚。

### 7.1 文件结构

每个 agent 自己的持久 memory 放在：

```text
.agtalk/<agent-name>/memory/
  plan.md
  context.md
  status.json
  entries.jsonl
```

- `plan.md`：当前目标、计划、进度、下一步、阻塞项。
- `context.md`：公开背景、约束、关键决策、协作注意事项。
- `status.json`：机器可读摘要。
- `entries.jsonl`：长期记忆条目，第一阶段使用简单 JSONL。

`plan.md`、`context.md`、`status.json` 是公开协作状态，不保存私密推理、token、敏感正文。`entries.jsonl` 用于长期知识沉淀。

### 7.2 SQLite 全局索引

SQLite 只保存在线 agent 的全局 mem 注册/索引：

```text
address
name
workspace
memory_path
plan_updated_at
status_summary
public_topics
```

规则：

- `id join` 成功后注册或刷新当前 agent 的 memory。
- `id leave`、session 失效、PID/start_time 校验失败后的惰性清理会移除该 agent 的 mem 索引。
- SQLite 不持久保存离线 agent 的长期 entries 正文。
- 其他 agent 的 `mem plan show/status` 只依赖这个在线索引；离线 agent 不可见。
- 后续如果要做跨 agent `mem search`，也必须遵守“只索引在线 agent，离线即移除”的规则。

### 7.3 mem plan show

```bash
agtalk mem plan show [--target <address-or-name>]
```

查看某个 agent 的公开计划和上下文。省略 `--target` 时查看自己。

- 优先按 address 查。
- 按 name 查询如果多匹配，返回候选并要求消歧。
- 只能查看在线 agent 已注册的公开 memory；离线 agent 不进入查询结果。
- remote agent 只能读取 target plan，不能写对方 plan。

### 7.4 mem plan update

```bash
agtalk mem plan update [--plan <file|->] [--context <file|->] [--status <status>] [--summary <text>]
```

更新当前 agent 的公开计划、上下文和状态摘要。

- `--plan <file>` / `--context <file>` 在 CLI 边界读取文件内容；值为 `-` 时从 stdin 读取。同一次命令不允许 plan 与 context 同时从 stdin 读取。
- daemon 只接收内容，不根据客户端路径读取文件。
- `--status` 仅允许 `idle` / `working` / `waiting` / `blocked`；空或缺省保持兼容，非法值返回 `invalid_status`。
- `summary` 是自由短文本，用来描述当前工作、等待对象或阻塞原因。
- 写入使用临时文件 + rename，避免读到半截文件。
- 只能更新当前身份自己的 mem plan。
- 更新成功后刷新 SQLite 中当前 agent 的在线 mem 索引。

#### 协作状态约定

- 委派方发送任务后，把自己的 plan 更新为 `waiting`，并在 `summary` 写明等待哪个 agent / 什么结果。
- 接收方开始处理时把自己的 plan 更新为 `working`；完成后更新为 `idle`，并保留最近完成摘要。
- 其他 agent 用 `agtalk mem plan status --target <UUID-or-name>` 查看公开摘要，用 `mem plan show --target ...` 查看完整 plan/context。
- `msg send` / `run` 不会自动改写 plan；状态更新由 agent 工作流显式执行。

### 7.5 mem plan status

```bash
agtalk mem plan status [--target <address-or-name>]
agtalk --json mem plan status [--target <address-or-name>]
```

读取机器可读状态摘要。省略 `--target` 时读取自己。

示例：

```json
{
  "type": "mem_plan_status",
  "address": "550e8400-e29b-41d4-a716-446655440000",
  "name": "codex-coder-Alex",
  "updated_at": "2026-07-06T12:00:00Z",
  "status": "working",
  "summary": "正在实现命令面重构"
}
```

### 7.6 长期记忆

```bash
agtalk mem add <text> --topic <slug> --type <type> [--title <title>] [--tags <tags>]
agtalk mem search <query> [--topic <slug>] [--limit <n>]
agtalk mem show <mem-id|prefix>
agtalk mem list [--topic <slug>]
agtalk mem pack <topic> [--limit <n>]
```

type 建议：

```text
fact decision rule procedure issue snippet preference summary note context
```

约束：

- 不默认上向量库。
- 不在第一阶段设计复杂 topic 管理命令。
- 记忆文件优先使用 Markdown/JSONL。
- `mem add/search/show/list/pack` 默认只操作当前 agent 的 `.agtalk/<agent-name>/memory/entries.jsonl`。
- 长期项目知识优先沉淀进项目文档。
- `mem pack` 保留旧版价值：生成可注入 prompt/message 的通用 Markdown 上下文包。
- 内置全量使用指南：`agtalk --agent-guide`，源文件为 `docs/agent-usage.md`，编译时嵌入二进制，不写入任何 agent memory 或 agtalk.db。
- `agent-learning-handbook` 与 `agtalk mem pack agtalk/agent-guide` 不再作为 guide 入口。

### 7.7 协作关系索引

```bash
agtalk mem relation list [--specialty <text>]
agtalk mem relation show <name-or-address>
agtalk mem relation update <name-or-address> \
  --role <text> \
  --tag <tag1,tag2> \
  --note <text> \
  --specialty <a,b> \
  --preferred-for <a,b>
```

`relations.json` 是 agent 私有的协作关系索引，位于 `.agtalk/<agent-name>/relations.json`。

- `msg send` / `msg reply` / `msg ask` 成功后会自动按 peer 聚合发送/接收计数、最后联系时间、最后消息 ID。
- `role`、`tags`、`note`、`specialties`、`preferred_for` 是手动标注字段，自动更新不会覆盖。
- `--specialty` / `--preferred-for` 使用逗号分隔，自动 trim、去空、去重；`specialties` / `preferred_for` 按大小写规范化去重，保留首次输入的展示文本。
- `mem relation list --specialty <text>` 按 specialty 大小写不敏感精确匹配过滤。
- `mem relation show` 支持按 name 或 address（含短前缀）查找。
- 推荐协作流程：先 `mem relation list --specialty ...` 找已合作 peer；无匹配时用 `id lookup` 发现新 agent；发送时仍使用完整 UUID 路由。

---

## 8. tool：运行时辅助

`tool` 只放辅助能力，不承载核心协作语义。

允许：

```bash
agtalk tool doctor
agtalk tool version
agtalk tool path
```

禁止：

```text
tool ask
tool plan
tool browser
tool send
tool daemon
```

### 8.1 doctor

```bash
agtalk tool doctor
agtalk --json tool doctor
```

诊断当前环境，只诊断，不自动修复。

检查项：

- daemon 是否运行。
- config 是否可读。
- 当前身份是否可解析。
- `.agtalk/` 结构是否正常。
- DB 是否可达。
- 端口是否被占用。
- notify 环境是否可用。
- 当前执行的是哪个 agtalk 二进制。

如果发现问题，只输出建议命令，不自动执行修复。

### 8.2 version / path

```bash
agtalk tool version
agtalk tool path
```

用于排查多版本 agtalk 混用问题。

---

## 9. config：配置

```bash
agtalk config show
agtalk config get <key>
agtalk config set <key> <value>
agtalk config path
```

配置只管理全局设置，不做交互式向导。

常见 key：

```text
http_port
notify.default
human.name
human.intro
message.preview_limit_chars
message.inbox_inline_limit_bytes
```

配置文件权限保持 0600。

---

## 10. run：发送/协作 spec 优先入口

```bash
agtalk run              # 读取 .agtalk/<agent>/runs/default.yaml
agtalk run review       # 读取 .agtalk/<agent>/runs/review.yaml
agtalk run ./path/to/spec.yaml  # 使用显式路径
agtalk --json run [spec-name|path.yaml]
```

`run` 在 CLI 本进程解析 YAML 并逐步执行，不经过 REST API。

- `runs/` 只保存发送/协作 spec，不保存执行结果。
- 实际消息收发、正文、状态变化由 `.agtalk/<agent>/history.jsonl` 记录。
- 只要消息发送需要模板化、复用或记录发送意图，即使单次发送也优先用 `run`。

路径解析规则：

- 无参数：`.agtalk/<agent>/runs/default.yaml`
- `<name>`（不是现有文件路径）：`.agtalk/<agent>/runs/<name>.yaml`
- `<path/to/file.yaml>`（现有文件路径）：直接使用显式路径

约束：

- 只执行 agtalk 内部白名单动作。
- 不执行任意 shell。
- 任一步失败默认停止。
- 第一阶段没有变量替换：每个 step 的字段按字面量传给对应内部动作，不支持 `${}`、`{{ }}` 或任何模板语法。
- `--json` 输出稳定结构：

  ```json
  {
    "type": "run_result",
    "status": "ok",
    "file": ".agtalk/nora/runs/default.yaml",
    "stopped_at": null,
    "steps": [
      {
        "index": 1,
        "action": "msg.send",
        "status": "ok",
        "output": { "type": "ok", "id": "..." },
        "error": null
      }
    ]
  }
  ```

允许的 action：

```text
id.show
id.lookup
msg.send
msg.reply
msg.done
msg.ask
msg.wait
msg.read
msg.inbox
mem.plan.show
mem.plan.update
mem.plan.status
mem.pack
config.get
tool.doctor
```

第一阶段不允许：

```text
id.join
config.set
id.leave
shell
```

示例：

```yaml
version: 1
steps:
  - action: msg.send
    to: "550e8400-e29b-41d4-a716-446655440000"
    subject: "TASK: 实现命令面重构"
    body: |
      请根据 docs/commands.md 实现命令面重构。
    notify: true

  - action: msg.read
```

第一阶段不支持变量替换、条件、循环、失败回滚。后续如果要加变量语法，必须先定义转义、作用域和失败行为。

---

## 11. REST API

REST API 需要随 NG 命令面同步，但同步的是领域语义，不是一比一复制 CLI 命令。

原则：

- REST API 是 daemon 的资源接口，CLI / GUI / 浏览器扩展都是薄客户端。
- API 路径按 `id`、`msg`、`mem` 三个核心领域组织。
- `tool`、`config`、`run` 默认是 CLI 本地能力，不进入第一阶段 REST API。
- SSE 仍是唯一推送机制，长驻订阅走 events endpoint，不新增轮询 API。
- 所有写操作都使用 `POST` 或 `PATCH`，避免 `GET` 产生已读、完成等副作用。
- `/api` 原始 `ClientMsg` envelope 只作为内部兼容/调试入口；NG canonical API 使用下面的资源路径。
- REST API 不提供 `--as` 等价参数；身份来自 PID/start_time 认证头或浏览器 token。

### 11.1 id API

```text
POST /api/v1/id/join
POST /api/v1/id/leave
GET  /api/v1/id/me
GET  /api/v1/id/lookup?name=<name>
```

`POST /api/v1/id/join` 对应 `agtalk id join`：

- 创建或复用 session。
- 注册 PID/start_time。
- 捕获或刷新 notify target。
- 注册当前 agent 的在线 memory 索引。

`POST /api/v1/id/leave` 对应 `agtalk id leave`：

- 标记 mailbox left。
- 删除本地 session。
- 移除 PID/session 锚点。
- 移除在线 memory 索引。

### 11.2 msg API

```text
POST /api/v1/msg/send
POST /api/v1/msg/reply
POST /api/v1/msg/done
POST /api/v1/msg/ask
GET  /api/v1/msg/inbox?all=true|false
POST /api/v1/msg/read
POST /api/v1/msg/wait
GET  /api/v1/msg/attachment/:id
```

`POST /api/v1/msg/read` 请求体：

```json
{
  "message_id": null
}
```

- `message_id == null`：读取当前身份全部未读消息，并标记 read。
- `message_id != null`：读取指定消息详情，并标记 read。
- 不使用 `GET /read`，因为 read 会改变已读状态。

`POST /api/v1/msg/wait` 是短期 SSE 封装，必须带 timeout；常驻进程直接使用 events endpoint。

### 11.3 mem API

```text
GET   /api/v1/mem/plan?target=<address-or-name>
PATCH /api/v1/mem/plan
GET   /api/v1/mem/plan/status?target=<address-or-name>
```

第一阶段 REST 只暴露公开协作状态：

- `plan.md`
- `context.md`
- `status.json`

长期记忆 `entries.jsonl` 默认是 agent 本地能力，先不开放跨 agent REST 读写。后续如果需要跨 agent `mem search`，只能搜索在线 agent 的公开索引，离线即从 SQLite 移除。

### 11.4 events API

```text
GET /api/v1/events
```

也可以保留 `/events` 作为短路径别名，但 canonical endpoint 是 `/api/v1/events`。

认证头：

```text
X-AgTalk-Address: <uuid>
X-AgTalk-Pid: <pid>
X-AgTalk-Start-Time: <start_time>
Last-Event-ID: <event_id>
```

浏览器扩展域继续使用：

```text
X-AgTalk-Address: <uuid>
X-AgTalk-Browser-Token: <token>
Last-Event-ID: <event_id>
```

本机 human 客户端（popup/GUI）使用 human token，订阅 human mailbox 的统一 SSE（无需 `X-AgTalk-Address`）：

```text
X-AgTalk-Human-Token: <token>
Last-Event-ID: <event_id>
```

REST API 不新增轮询收信接口。需要“现在有什么”用 `msg inbox/read`，需要推送用 events。

### 11.5 human API（仅本机 human 客户端）

```text
GET  /api/v1/human/inbox?all=true|false
POST /api/v1/human/read
POST /api/v1/human/reply
POST /api/v1/human/done
GET  /api/v1/human/agents
POST /api/v1/human/send
```

认证：仅接受 `X-AgTalk-Human-Token`（daemon 启动时在 `<config_dir>/human/session.json` 颁发，0600），agent / browser 凭据一律拒绝。token 绝不暴露给 agent。详见 `docs/human-surfaces.md`。

- `POST /api/v1/human/reply`：请求体 `{message_id, body, choice?, surface?, external_event_id?}`。approval_request 首个有效回复原子胜出，后续返回 `already_resolved`；`select_only` 无 choice 返回 `select_only_requires_choice`。
- `reply` / `done` / `send` 均接受可选 `{surface, external_event_id}` 做跨端幂等：重复事件回放首次成功结果（`Ok{id}`，send 返回原 message id），不重复创建消息；中途崩溃的占位事件自动恢复。
- `GET /api/v1/human/agents`：返回在线 agent 列表（活跃 mailbox，排除 human 自身），人类主动发信只能从该列表选择。
- `POST /api/v1/human/send`：请求体 `{to, body, subject?, surface?, external_event_id?}`，`to` 必须是活跃 agent 的 UUID（拒绝 human 自身），复用 routing::send，触发目标 agent 的 SSE/notify。

### 11.6 旧 REST 路径映射

| 旧路径 | NG canonical |
|---|---|
| `POST /api/join` | `POST /api/v1/id/join` |
| `POST /api/leave` | `POST /api/v1/id/leave` |
| `GET /api/lookup` | `GET /api/v1/id/lookup` |
| `POST /api/send` | `POST /api/v1/msg/send` |
| `POST /api` + `Whoami` | `GET /api/v1/id/me` |
| `POST /api` + `Inbox` | `GET /api/v1/msg/inbox` |
| `POST /api` + `Detail` | `POST /api/v1/msg/read` |
| `POST /api` + `Reply` | `POST /api/v1/msg/reply` |
| `POST /api` + `Human` | `POST /api/v1/msg/ask` |
| `GET /events` | `GET /api/v1/events`，`/events` 可作为别名 |

---

## 12. agent 入口

agtalk 只向 agent 公开四个入口：

```bash
agtalk              # agent quick guide（文本）
agtalk --json       # agent quick guide（JSON，供脚本解析）
agtalk --agent-guide # 全量 agent 使用指南（Markdown）
agtalk --help       # 完整 CLI help
```

裸 `agtalk` 不是错误，而是 agent-first 默认入口：输出可直接执行的最小操作手册，按使用场景排序。

```text
agtalk agent quick guide

Rules:
  - Route only by UUID. Use id lookup to find address.
  - name is display only, not routing.
  - Before replying to user, run msg read.
  - inbox_empty means no message, not failure.
  - Use --json when parsing output.

1. Identity
  agtalk id show
  agtalk id join <name> --intro "<role>"

2. Find target
  agtalk id lookup [name]

3. Send / reply / done
  agtalk msg send <address-uuid> "<body>"
  agtalk msg reply <msg-id> "<body>"
  agtalk msg done [msg-id]

4. Read loop
  agtalk msg read
  # If inbox_empty: continue normal work.

5. Wait / ask human
  agtalk msg wait [sent-msg-id] --timeout 30
  agtalk msg ask "<question>" --option approve --option reject --timeout 60

6. Memory / plan
  agtalk mem plan show
  agtalk mem pack [topic]

7. Diagnose
  agtalk tool doctor

More:
  agtalk --agent-guide     full agent guide
  agtalk --help            full command tree
  agtalk <cmd> --help      command flags
```

- `--json` 输出结构化 recipe，不输出长文本。
- 完整说明通过 `agtalk --agent-guide` 获取。
- 不在这里解释 daemon / SSE / notify 原理。

---

## 13. 旧版到新版映射

| 旧版命令 | 新版位置 |
|---|---|
| `join` / `attach` | `id join` |
| `me` / `whoami` | `id show` |
| `peers` / `lookup` | `id lookup` |
| `leave` / `cleanup` | `id leave` / `id cleanup` |
| `agent` | `msg send` / `msg reply` / `msg done` |
| `human` | `msg ask` |
| `inbox` | `msg inbox` |
| `detail` | `msg read` |
| `wait` | `msg wait` |
| `attachment` | `msg attachment` |
| `plan` / `state` | `mem plan` |
| `mem add/search/pack` | `mem add/search/pack` |
| `daemon` | `daemon` |
| `config` | `config` |
| `poll-inbox` | 删除 |
| `gui` / `settings` / `chats` | 第一阶段不进入命令面 |

---

## 14. 稳定错误码

```text
identity_missing
identity_ambiguous
inbox_empty
message_not_found
timeout
daemon_unavailable
lookup_ambiguous
memory_unavailable
invalid_command
not_supported
already_resolved
select_only_requires_choice
invalid_choice
agent_not_found
```

`--json` 错误格式：

```json
{
  "type": "error",
  "code": "inbox_empty",
  "message": "当前 inbox 没有可查看的消息"
}
```

---

## 15. 命令治理规则

顶层命令固定为：

```text
daemon id msg mem tool config run
```

新增能力必须先回答：

1. 是 daemon 生命周期吗？放 `daemon`。
2. 是身份吗？放 `id`。
3. 是通信吗？放 `msg`。
4. 是计划、上下文、公开状态或长期记忆吗？放 `mem`。
5. 是运行时辅助吗？放 `tool`。
6. 是配置吗？放 `config`。
7. 是多步编排吗？放 `run`。

如果不能归类，先改设计文档讨论，不直接加命令。
