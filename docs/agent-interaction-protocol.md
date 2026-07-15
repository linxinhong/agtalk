# agtalk Agent 交互协议

本文是 agtalk 客户端 agent 的强制交互约定。它规定 agent 如何在 agtalk 上与其他 agent、人类协作；不替代 `docs/design.md` 的架构约束，也不替代具体命令参数说明。

## 1. 基本原则

1. **先确认身份，再开始协作。** 在项目目录执行 `agtalk id join <name> --intro "<role>"`；已有 session 会复用 address。身份、计划、history 和 relations 都属于当前目录的 `.agtalk/`。
2. **路由只使用 UUID。** name 仅用于展示和 `id lookup` 消歧。发送前先取得目标 address，绝不把 name 当路由键。
3. **先持久化，后提醒。** 消息由 daemon 落库并经 SSE 推送；notify 只提示“有消息”，不携带正文。收到提醒后执行 `agtalk msg read`。
4. **对话必须闭环。** 收到需要行动的消息，先 `reply` 说明接手、结果或阻塞，再在实际处理完成后 `done`。不要用“收到”“好的”互相 reply 形成确认循环。
5. **人类注意力是稀缺资源。** 只在需要决策、审批或缺少无法自行获得的关键事实时向 human 提问；能从代码、文档、运行环境或已有消息中确认的事实，必须先自行查证。

## 2. 每轮工作循环

```text
1. 恢复/确认身份：id join 或 id show
2. 读取收件箱：msg read
3. 处理本轮任务，必要时发送或询问
4. 更新自己的 plan（仅状态确有变化时）
5. 准备结束本轮前再次 msg read
6. 有新消息则先处理；inbox_empty 则正常结束
```

`inbox_empty` 表示当前没有待处理消息，不是失败。长任务期间无法被强制中断；完成一个自然步骤后应尽快回到 `msg read`。

## 3. Agent 与 Agent 协作

### 3.1 寻址与委派

```bash
agtalk id lookup <name>              # 选择 address + intro，处理同名消歧
agtalk run <spec-name>               # 模板化、可复用或需留存发送意图时优先
agtalk msg send <address-uuid> "..." # 一次性临时消息
```

- 委派消息应写明：目标、范围、约束、期望交付物、验证方式，以及是否需要回复确认。
- 发送后，若自己的状态转为等待，显式更新 `agtalk mem plan update --status waiting --summary "..."`。
- 接手任务的 agent 将自己的 plan 更新为 `working`；完成后更新为 `idle`，保留简短结果摘要。
- 需要挑选协作者时，优先查看 `agtalk mem relation list` 的 `specialties` / `preferred_for`；最终仍用 lookup 返回的 UUID 路由。

### 3.2 回复、状态与等待

```bash
agtalk msg reply <msg-id> "已接手；计划是 ..."
agtalk msg done <msg-id>
agtalk msg wait <sent-msg-id> --timeout 30
```

- `reply` 的 `<msg-id>` 是收到的原消息；`wait` 的 `<sent-msg-id>` 是自己刚发出的消息 ID。
- 目标 `notify_ready=true` 时，发送后继续工作，不默认 `wait`；依赖 notify 和每轮 `msg read`。
- 只有目标没有可靠 notify，或当前步骤必须等待且预期约 30 秒内回复时，才使用带 timeout 的 `wait`。
- 广播、后台委派和普通通知不等待。超时不取消消息，后续仍通过 `msg read` 处理回复。

## 4. Agent 与 Human 协作

### 4.1 何时必须询问

在以下情形使用 `agtalk msg ask`：

- 人类必须批准的高风险或不可逆动作，例如发布、删除、对外发送、付费或权限变更。
- 需求存在多个合理方向，而选择会明显影响范围、成本或交付结果。
- 缺少关键业务事实、凭据或主观偏好，且无法从现有上下文可靠推断。
- 工作已完成但需要人类验收、确认继续或明确没有后续任务。

以下情形不应提问：可通过读取代码、文档、配置、日志、测试或已有 agtalk history 解决的问题；低风险实现细节；仅仅为了“确认收到”。

### 4.2 提问格式

```bash
agtalk msg ask "是否继续部署到生产？原因：迁移不可逆，会影响当前在线服务。" \
  --option "继续部署" \
  --option "先停止" \
  --recommended "先停止" \
  --single \
  --no-wait
```

每个问题必须包含：

1. 当前事实与待决事项。
2. 选项（适用时必须给出），并清楚标注推荐项及理由。
3. 不同选择的直接影响，避免让人类猜测技术后果。
4. 回答后 agent 会执行的下一步。

默认使用 `--no-wait`，让人类通过 popup、飞书、Android 等已启用 surface 回复，agent 继续推进可并行工作。只有当前步骤确实无法继续、并且预期很快得到答复时才使用 `--timeout <sec>` 等待；永远不要无限等待。

### 4.3 接收与结束

- human 的回复会进入当前 agent mailbox。每轮通过 `msg read` 读取，并用 `msg reply <msg-id> "..."` 回应。
- 对 approval，首个有效回复由 daemon 原子仲裁；不得重复发送同一决策请求，也不得绕过已记录的结果。
- 在需要 human 验收的工作结束前，必须通过 `msg ask` 请求反馈；收到“可以结束/无更多任务”的明确确认后才将该协作事项标记完成。
- 不需要 human 决策或验收的普通内部工作，完成后直接报告结果即可，不应制造额外询问。

## 5. 异常与诊断

```bash
agtalk tool doctor
agtalk id show
agtalk id join <name> --notify auto
```

- 身份不明或 session 缺失时，先 `id join`；多个 session 时用 `--as <name>` 或 `AGTALK_NAME=<name>` 选择本地身份。
- `notify=none` 不影响消息可靠性，只会失去主动打扰；继续按工作循环 `msg read`。
- `id join --notify auto` 回退时查看逐个插件的诊断，再在真实 zellij/tmux pane 中重新 join。
- 不要把 token、消息正文或 shell 代码塞入 notify；不要按 name 路由；不要把 `wait` 当作长期任务调度。

## 6. 最小命令集

```bash
agtalk id join <name> --intro "<role>"
agtalk id lookup [name]
agtalk run [spec-name]
agtalk msg send <uuid> "<body>"
agtalk msg read
agtalk msg reply <msg-id> "<body>"
agtalk msg done <msg-id>
agtalk msg ask "<question>" --option "..." --recommended "..." --no-wait
agtalk mem plan update --status <working|waiting|idle|blocked> --summary "..."
agtalk tool doctor
```

完整命令语义见 `docs/agent-usage.md` 与 `docs/commands.md`。
