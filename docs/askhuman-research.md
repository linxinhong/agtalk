# askhuman-research.md — AskHuman 架构调研（agtalk Human Messaging 借鉴）

调研对象：`~/projects/AskHuman`（Tauri 2 + Rust + Vue 3 的 human-in-the-loop 工具：
agent 经 `AskHuman` CLI 提问，本地 popup + Telegram/Slack/DingTalk/Feishu 多通道并行竞速，
首个回复胜出，结果经 stdout 返回 agent）。**只读调研，未修改 AskHuman 任何文件，
不作为 agtalk 运行时依赖。**

调研目的：为 agtalk Human Messaging Phase 2（popup 已实现）/ Phase 3（Feishu surface）提供借鉴。

---

## 1. AskHuman 架构总览（模块证据）

### 进程形态：单二进制 + daemon + 弹窗 helper 三进程

- argv 分派唯一入口 `src-tauri/src/cli/mod.rs:29`：`--settings` / `--popup` / `daemon <sub>` / 默认 ask。
- CLI 是**瘦客户端**（`src-tauri/src/client/mod.rs:316` `run_ask`）：`ensure_running()` 自启 daemon →
  Hello 握手 → `Submit` → 流式读 `Final{stdout, exit_code}` → 写 stdout 退出。
- IPC 是 NDJSON over Unix socket（`src-tauri/src/ipc/mod.rs:3`），socket `~/.askhuman/daemon.sock`
  chmod 0600（`ipc/transport.rs:36`），`PROTOCOL_VERSION = 2`。
- 弹窗 helper：daemon spawn `AskHuman --popup --endpoint <sock> --token <一次性token>`
  （`src-tauri/src/daemon/mod.rs:7321`）；另有预热池保持 1 个热实例（`daemon/mod.rs:7345,7397-7430`）。

### 请求模型与竞速仲裁

- `RequestRegistry`（`daemon/request.rs:200-293`）：每 submit 分配 UUIDv4 request_id + 一次性 GUI
  token + 独立 Coordinator，`by_id`/`by_token` 双 HashMap；并发提问 = 多个独立弹窗进程，互不干扰。
- `FirstTerminalGate<T>`（`src-tauri/src/app/terminal_gate.rs:5-32`）：`Mutex<Option<T>>`，首个
  `try_set` 胜出——这就是「首个回复胜出」的原语（单进程内存态）。
- `Coordinator::submit`（`src-tauri/src/app/coordinator.rs:189-263`）：首胜 → interrupt 落败渠道 →
  **收尾窗口**（事件驱动 + 5s 上限 `FINALIZE_TIMEOUT`，等落败端把卡片改成终态）→ 渲染 stdout →
  旁路写历史。
- Confirm 变体（`app/confirm_coordinator.rs:53-67`）：**先校验再夺 gate，校验失败不关 gate**
  （无效提交不浪费仲裁机会）；surface 上报的 action_id 永不采纳，只收 choice_index 经 daemon
  ledger 解析（`ipc/mod.rs:281-288`）。
- CLI 断连 = 整请求取消（`daemon/mod.rs:1459-1474`）；helper 断开未作答 = cancel（`daemon/mod.rs:1622-1626`）。

### Feishu 通道（Phase 3 重点参考）

- `src-tauri/src/feishu/`：纯协议层。长连接**自研 protobuf 帧**（`feishu/ws.rs:49-69`，仅 20 行
  prost 定义，不用 lark-oapi SDK）；建连 `POST /callback/ws/endpoint` 取 wss URL（`ws.rs:317-352`）；
  ping/pong 心跳 + 分片重组 + 断线重连（最多 5 次递增退避，`ws.rs:286-374`）；**3 秒 ACK 契约**
  （`ws.rs:82-88`）；业务路由以回包 `header.event_type` 为权威（`ws.rs:203-218`，实测卡片回调可能
  以 `type=event` 投递——官方文档没说的坑）。
- `tenant_access_token` 进程内缓存（`feishu/token.rs:26-78`，过期前 60s 余量）；**全链路只用
  tenant token + 单聊 open_id，没有 user_access_token**。
- `FsRouter`（`feishu/router.rs`）：**进程内独占一条长连接**，`open_message_id → route` 卡片精确
  路由 + `open_id → route` 聊天松散路由（`router.rs:37-47`）；`RoutedFs` Drop 即注销（`:169-176`）；
  卡片回调 oneshot 等会话裁决，上限 2.5s（< 飞书 3s 窗口），超时/孤儿 → 空 ACK（`:196-213`）。
- 卡片（`feishu/card.rs`，1506 行）：JSON 2.0 直接下发无需后台模板；3 秒窗口内同步回包换卡
  （`callback_update_card`，按钮 Loading 直接变终态）；被抢答收尾 best-effort `patch_card`
  改「已在 X 回答」（`channels/feishu.rs:383-406`）；`send_card` 失败自动回退纯文本编号方案（`:237-249`）。
- Channel 抽象两层：`trait Channel { id/start/interrupt }`（`channels/mod.rs:70-77` 抢答层）+
  `trait MessagingChannel`（`channels/conversation.rs:91-198` 传输层），`run_conversation` 公共
  驱动多题逐条，编排与传输彻底解耦，四 IM 通道共用。

### 配置与安全

- 配置目录 `~/.askhuman`（`src-tauri/src/paths.rs:12-25`，`ASKHUMAN_HOME` 可覆盖隔离）。
- secret 存 OS keychain（`src-tauri/src/secrets.rs`，`keyring` crate），config.json 不落明文；
  钥匙串不可用时回退明文 0600 + 告警（`config.rs:456-494` 含明文残留自动迁移）。
- **两条读取路径**：`load()` vs `load_without_secrets()`（`config.rs:389/426`）——无关命令不碰
  钥匙串避免弹框。每次 load 自愈 0600/0700 权限，且仅 mode 不同才 chmod（防 watcher 风暴，`config.rs:513-515`）。
- doctor 一屏体检（`src-tauri/src/cli/doctor.rs`）：daemon 状态 + 每渠道 enabled/configured/connected
  三态；`connected` 在 daemon 未运行时显式 null 而非误报 false。
- detect 两步式 open_id 自动发现（`cli/channel_cmd.rs:382-499`）：发 4 位识别码 → 用户私聊机器人
  回发 → 取 `sender.open_id` 回填，120s 超时。
- **Aqua 会话修正**（`daemon/spawn.rs:1-74`）：非 GUI 会话（SSH）拉起的 daemon 读不到登录钥匙串 →
  经 `launchctl bootstrap gui/<uid>` 让 daemon 落在 Aqua 会话。macOS 钥匙串 + daemon 的真实坑。
- 日志脱敏：`config show` 密钥显示 `●●●`（`cli/cfgio.rs:116-141`）；secret 绝不进 argv
  （env/file/stdin 三选一）。
- 失败降级：GUI 不可用 + 有 IM 渠道 → headless；都不可用 → 退出码 3（`app/mod.rs:234-273`）；
  Router 「连接已死则重连」+ 配置热更惰性失效重建（`daemon/mod.rs:2136,7558`）。

---

## 2. 对照 agtalk 红线检查

| 红线 | AskHuman 做法 | 结论 |
|---|---|---|
| 路由只认 UUID | 平台 id（open_message_id/open_id/message_ts）直接做会话路由键 | **不可照搬**：平台 id 只能做 surface 投递句柄，落库时记为外部元数据，回写时反查 |
| SSE 唯一推送 | 私有 NDJSON 长连接 + IM 各自长连接 + Telegram 长轮询 | **不可照搬**：agtalk surface 订阅统一 SSE；IM 长连接只是入站通道 |
| 消息先持久化（at-least-once） | 纯内存 Coordinator，daemon 重启在途请求全丢（drain 缓解） | **不可照搬**：agtalk 仲裁已在 DB 单事务，更强，不要退化为内存 gate |
| human 唯一 mailbox/消息库/消息 ID | 独立 request_id 体系 + append-only history.jsonl（第二历史库） | **不可照搬数据模型**：只借鉴机制 |
| daemon 唯一真相源 | 自带 daemon + 第二 socket 协议 + 非 Unix 单进程回退 | **不可引入第二 daemon**：Feishu Router 模式必须搬进 agtalk daemon 内部 |

---

## 3. 对 agtalk 的建议

### 3.1 可借鉴（机制层，推荐采纳）

1. **收尾窗口**（coordinator.rs:22,245-262）：首个回复胜出后，给落败 surface ≤5s 事件驱动窗口
   把卡片/界面定格为终态（「已由 X 回答」），而不是永远停在「待答」。**Phase 3 Feishu 必做**。
2. **校验失败不关 gate**（confirm_coordinator.rs:62-64）：无效提交（非法 choice）不消耗仲裁机会。
   agtalk 当前实现已符合（approval.rs 校验在仲裁 INSERT 之前），保持。
3. **surface 上报只收索引/结果，由 daemon 解释**（ipc/mod.rs:281-288）：GUI 给的 action_id 永不
   采纳。agtalk popup 目前直接回 choice 字符串，approval.rs 已用 choices 白名单校验，边界等价；
   Feishu 卡片回调必须坚持「卡片 value 存 agtalk message UUID + choice index，不存业务结论」。
4. **卡片状态回写双路径**：3 秒窗口内同步回包换卡（丝滑）+ 窗口外 OpenAPI `patch_card`
   （best-effort 收尾）。Phase 3 直接复用该时序。
5. **FsRouter 形态**：daemon 全局独占一条长连接，`外部消息 id → 内部会话` 路由表，句柄 Drop 即注销。
   agtalk 版：daemon 持全局 FeishuRouter，入站事件**先落 human mailbox DB 再走 SSE/仲裁**，
   外部 id 与 agtalk message UUID 的映射存 DB（如 delivery.external_ref）。
6. **tenant token 缓存**（token.rs：app_id 键、60s 余量）与**自研 20 行 protobuf 帧**：照抄形态，
   避免 lark-oapi 重依赖，与 agtalk 单 crate 风格一致。
7. **detect 两步式 open_id 发现**（识别码私聊回填）：`agtalk config` 飞书向导可复用此交互。
8. **secret 两层读取路径 + 自愈 0600 + 钥匙串回退明文告警**：agtalk 引入 keychain 存
   Feishu appSecret 时照搬；`load_without_secrets()` 模式避免无关命令触发钥匙串弹框。
9. **doctor 三态**（enabled/configured/connected，daemon 未运行时 connected=null）：比 agtalk
   当前 doctor 更结构化，`tool doctor` 可对齐。
10. **Aqua 会话修正**：agtalk daemon 若需读钥匙串必踩此坑，有现成解（launchctl bootstrap gui/<uid>）。
11. **弹窗防白闪时序**：`visible(false)` 建窗 → 前端 pull 初值 → 绘制完成 → show + focus
    （app/mod.rs:183,1024-1064）。agtalk `__popup` 冷 spawn 可先做「延迟 show」消除白闪，
    预热池暂不需要。
12. **一次性 GUI token**（request.rs:371-386，关联即注销）：agtalk `__popup <msg-id>` 目前靠
    命令行 msg-id，防伪造弹窗进程可考虑 daemon 下发一次性 token（优先级低）。
13. **CLI 断连 = 取消 / helper 断开 = dismissed**：agtalk popup 的 ChildMonitor 语义已对齐
    （关闭且无回复 = dismissed），措辞与日志可对齐。

### 3.2 必须改造才能借鉴（不能直接使用）

- **入站事件接收侧**（ws.rs/router.rs）：AskHuman 收到飞书事件直接喂内存 Coordinator，不持久化、
  无去重表（靠 3s ACK + 路由注销防重推）。agtalk 版必须：事件 → **先落 human mailbox DB**
  （含外部 event_id 进 `human_action_receipts` 幂等）→ SSE → 仲裁。崩溃窗口（平台已 ACK 但
  agtalk 未落库）要用 receipt 同事务语义兜住（Phase 1 已建）。
- **竞速仲裁**：内存 `FirstTerminalGate` 不能替代 agtalk 的单事务仲裁（跨独立进程 surface）。
  但「首胜 → 通知落败 surface 收尾」的**流程**可借鉴：approval 仲裁胜出后，daemon 应向其它
  surface 的 delivery 标记终态并触发卡片回写。
- **会话归属**：飞书文字回复按 open_id 归「最新活动会话」有并发歧义（AskHuman 自己也承认，
  router.rs:41）。agtalk 不做自由文字归属猜测：入站文字要么引用具体消息（卡片按钮带 UUID），
  要么作为 human→agent 的新消息处理。

### 3.3 明确不应复用

- 数据模型：request_id 体系、history.jsonl 第二历史库、`/watch`/`interject.json` 等 IM 侧
  持久化状态文件（第二消息真相源）。
- 进程模型：第二 daemon、第二 socket 协议、非 Unix 单进程回退路径（同 app 多开长连接互抢的温床）。
- 推送协议：NDJSON 订阅推帧、Telegram 长轮询（违反 SSE 唯一推送）。
- stdout 结果区块契约（`[selected_options]` 等 marker）：与 agtalk 消息协议不同层。
- 工程结构：`daemon/mod.rs` 7773 行 god-file 是反面教材（agtalk 红线 800 行）。

---

## 4. 推荐实现顺序与风险

### Phase 2 收尾（popup，已实现基础上的小改进）

1. 弹窗防白闪：`__popup` 建窗 `visible(false)` → popup_load 完成 → 前端通知后端 show + focus。
2. （可选）daemon 下发一次性 token 关联弹窗进程，替代裸 msg-id 参数。
3. approval 仲裁胜出后，向同消息其它 delivery（未来 Feishu）广播「已处理」终态的钩子
   （当前 popup 是独立进程 per-message，无共享界面，影响小；为 Phase 3 铺垫）。

### Phase 3（Feishu surface）建议顺序

1. **配置与 secret**：`config.feishu {enabled, app_id, app_secret(keychain), open_id, base_url}`；
   secret 两层读取 + 0600 自愈；doctor 三态检查。
2. **协议层 `feishu/` 模块**：自研 protobuf 帧 + ws 长连接（心跳/重连/3s ACK）+ tenant token
   缓存 + OpenAPI client（send_card/patch_card/上传下载）。可先只读 `feishu/ws.rs`+`token.rs`+`client.rs`
   的形态重写，不引入 lark-oapi。
3. **daemon 全局 FeishuRouter**：单条长连接，`open_message_id → agtalk message UUID` 映射表
   （落库时写入 delivery.external_ref），入站事件 → receipt 幂等（飞书 header.event_id）→
   落 human mailbox → 走现有仲裁。
4. **出站投递**：fanout 到 feishu surface 时，deliver_via 渲染审批卡片（choices→按钮，
   recommended 高亮，select_only 禁输入），按钮 value 带 agtalk message UUID + choice index。
5. **仲裁收尾**：首胜后双路径回写（3s 内同步回包 / 窗口外 patch_card「已由 X 回答」），
   收尾窗口 ≤5s。
6. **detect 向导**：`agtalk config feishu detect` 两步式 open_id 发现。

### 风险清单

- **3 秒 ACK 窗口 vs 落库时序**：平台要求 3s 内 ACK，agtalk 要求先落库。实践上落库是毫秒级，
  窗口足够；但必须在「收到事件 → 落库 → ACK」串行路径上控制延迟，失败时飞书会重推，
  靠 receipt（event_id）幂等兜住。
- **open_id 单用户模型**：AskHuman 与 agtalk 都是单 human，兼容；多 human 是未来的坑。
- **钥匙串 + daemon 会话**：macOS 非 Aqua 会话读不到登录钥匙串（AskHuman 已踩），agtalk daemon
  由终端 `daemon start` 拉起时多半在 Aqua 会话，但 launchd/SSH 场景需 Aqua 修正方案。
- **卡片 value 体积**：飞书按钮 value 有长度限制，agtalk message UUID 36 字符 + choice index
  安全，但不要往 value 里塞正文。
- **重推风暴**：路由注销后的迟到回调要空 ACK 忽略，不能报错重试（AskHuman 实测经验）。

---

## 5. 未查清/后续专项

- `feishu/card.rs` 中段卡片 JSON 全结构（130-760 行）未逐行审，Phase 3 实现时需细读。
- `channels/confirm.rs`（审批确认卡 812 行）与 agtalk 审批流程的映射未深入。
- Slack/DingTalk/Telegram router 细节未逐一验证（agtalk 暂无计划）。
- AskHuman daemon 的 `/here`、`/watch` 等 IM 入站命令（人在飞书主动发消息给 agent）未深读；
  agtalk 若支持「human 经飞书主动发信」需专项调研。
- daemon.log 是否可能泄漏 token 仅抽查未见证据，未逐行审 7773 行文件。
