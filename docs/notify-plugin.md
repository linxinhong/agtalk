# agtalk notify 插件机制设计

版本：v0.1

定位：notify 插件是 agtalk 打扰层的扩展机制。daemon 有新消息时，插件把“有消息”这个轻量信号转换成当前 agent 运行环境可感知的提醒，例如系统通知、HTTP webhook、BLE 推送、文件 flag、IDE 通知等。

notify 插件不是消息传输通道，不读取消息正文，不持有 agtalk token。消息仍以 daemon + SQLite + SSE / pull 为唯一可靠投递路径。

---

## 1. 背景与目标

agtalk 的消息模型是 pull-first：

```text
daemon 持久化消息
  -> agent 主动执行 agtalk msg read / msg wait
  -> agent 获取正文
```

但 agent 经常不会主动查收件箱。notify 的价值是把“有消息”这个事实主动推到 agent 的执行环境，让 agent 注意到并执行 `agtalk msg read`。

现有内置通道：

```text
zellij -> zellij action write-chars
tmux   -> tmux send-keys
none   -> 不打扰
auto   -> 自动检测 zellij / tmux，否则 none
```

这些通道无法覆盖所有运行环境。插件机制用于支持：

1. macOS / Windows / Linux 系统通知。
2. IDE / 编辑器通知。
3. HTTP webhook。
4. Android / BLE / mobile deck。
5. 后台 agent 自定义唤醒机制。
6. 文件 flag / socket / named pipe 等本地协作方式。

v1 目标：

1. 支持通过本地可执行文件扩展 notify。
2. 保持安全边界清晰。
3. 不影响消息入库和 SSE 推送。
4. 不把 webhook、BLE、GUI 等都做成内置通道。
5. 让 `tool doctor` 能诊断插件配置，但不触发插件。

---

## 2. 核心原则

### 2.1 只推信号，不推正文

notify 插件只收到“有新消息”的轻量 payload，不包含消息正文。

允许传递：

```text
from_name
read_command
binary_path
workspace
agent_name
agent_address
```

禁止传递：

```text
message body
subject
attachments
full metadata
browser token
session secret
high entropy auth token
```

### 2.2 插件不是 transport

插件不负责消息可靠投递。它只负责提醒。

```text
消息传输：daemon -> DB -> SSE / msg read
打扰提醒：daemon -> notify plugin -> agent 注意到 -> agent 自己 msg read
```

### 2.3 插件失败不影响消息

notify 失败只能降级为纯 pull：

```text
message 已入库
SSE 已触发
notify plugin 失败
  -> 记录 debug log
  -> 不回滚消息
  -> 不让 msg send / reply / ask 失败
```

### 2.4 不经 shell

daemon 必须使用参数数组执行插件：

```rust
Command::new(path)
```

禁止：

```text
sh -c
eval
字符串拼 shell 命令
```

### 2.5 插件路径必须绝对

插件配置中的 `path` 必须是绝对路径。这样可以避免不同工作目录、PATH 污染和同名程序劫持。

---

## 3. 通道模型

notify channel 分三类：

| 类型 | 示例 | 说明 |
|---|---|---|
| 禁用 | `none` | 不打扰，仅 pull |
| 内置 | `zellij` / `tmux` | daemon 内置实现 |
| 插件 | `plugin:macos` | 本地可执行文件 |

用户注册身份时选择通道：

```bash
agtalk id join coder --notify zellij
agtalk id join coder --notify tmux
agtalk id join coder --notify none
agtalk id join coder --notify plugin:macos
```

`auto` 的 v1 规则：

```text
zellij -> tmux -> none
```

第一版 `auto` 不自动选择插件。插件必须显式使用 `plugin:<name>`，避免 daemon 在不明确的环境里错误调用外部程序。

后续可以扩展：

```text
auto -> zellij -> tmux -> plugin auto-match -> none
```

但 plugin auto-match 需要额外设计匹配规则，不进入 v1。

---

## 4. 插件配置

插件注册在全局配置中：

```json
{
  "notify": {
    "default": "auto",
    "plugins": {
      "macos": {
        "path": "agtalk-notify-macos",
        "timeout_ms": 1000
      },
      "webhook": {
        "path": "agtalk-notify-webhook",
        "timeout_ms": 2000
      }
    }
  }
}
```

字段说明：

| 字段 | 必填 | 说明 |
|---|---:|---|
| `notify.plugins.<name>.path` | 是 | 插件可执行文件的路径 |
| `notify.plugins.<name>.timeout_ms` | 否 | 超时时间，默认 1000ms |

路径解析规则：

1. **约定目录**：插件默认放在 `<config_dir>/plugins/`（macOS/Linux 通常为 `~/.config/agtalk2/plugins/`）。
2. 如果 `path` 是**绝对路径**，按原样使用。
3. 如果 `path` 是**相对路径或纯文件名**，自动解析为 `<config_dir>/plugins/<path>`。
4. 文件必须存在且可执行。
5. `timeout_ms` 必须是有限值，建议 500-3000ms。
6. 插件名称只作为本地配置 key，不参与路由。

示例配置命令：

```bash
# 将插件放入约定目录
mkdir -p ~/.config/agtalk2/plugins
cp agtalk-notify-macos ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-macos

# 配置时只需写文件名，会自动解析到 plugins 目录
agtalk config set notify.plugins.macos.path agtalk-notify-macos
agtalk config set notify.plugins.macos.timeout_ms 1000
agtalk id join coder --notify plugin:macos
```

也支持绝对路径（旧写法仍兼容）：

```bash
agtalk config set notify.plugins.macos.path /Users/me/.config/agtalk2/plugins/agtalk-notify-macos
```

---

## 5. session.json 持久化

agent join 后，session 记录当前 notify 通道。

内置 zellij 示例：

```json
{
  "notify_channel": "zellij",
  "notify_target": {
    "type": "zellij",
    "session": "dev",
    "pane": "3"
  }
}
```

插件示例：

```json
{
  "notify_channel": "plugin:macos",
  "notify_target": {
    "type": "plugin",
    "name": "macos"
  }
}
```

插件 target 只保存插件名，不保存 path。path 来自全局 config，这样用户可以替换插件实现而不需要重写所有 session。

---

## 6. 插件执行协议

daemon 调用插件：

```text
Command::new(<absolute_plugin_path>)
  stdin:  JSON payload
  stdout: ignored
  stderr: captured on failure
  shell:  no
```

成功 / 失败约定：

| 情况 | 行为 |
|---|---|
| exit code 0 | notify 成功 |
| exit code 非 0 | notify 失败，记录 stderr |
| timeout | kill 插件进程，按失败处理 |
| path 非绝对 | 配置错误 |
| 文件不存在 | 配置错误 |
| 文件不可执行 | 配置错误 |

插件必须：

1. 从 stdin 读取完整 JSON。
2. 快速返回。
3. 不等待交互式输入。
4. 不自行调用 `agtalk msg read`，除非用户明确把插件写成这种行为。
5. 不要求 daemon 传 secret。

插件可以：

1. 发送系统通知。
2. POST webhook。
3. 写本地 flag 文件。
4. 推送 BLE / mobile 事件。
5. 调用 IDE / 编辑器 API。

---

## 7. stdin payload

v1 payload：

```json
{
  "version": 1,
  "type": "notify",
  "from_name": "nora",
  "read_command": "/Users/me/projects/agtalk/target/debug/agtalk msg read",
  "binary_path": "/Users/me/projects/agtalk/target/debug/agtalk",
  "workspace": "agtalk",
  "agent_name": "codex",
  "agent_address": "550e8400-e29b-41d4-a716-446655440000"
}
```

字段说明：

| 字段 | 说明 |
|---|---|
| `version` | payload 版本，v1 固定为 `1` |
| `type` | 固定为 `notify` |
| `from_name` | 消息发送方展示名 |
| `read_command` | agent 应执行的取信命令 |
| `binary_path` | 当前 agtalk 二进制路径 |
| `workspace` | 目标 agent 的 workspace |
| `agent_name` | 目标 agent 名称 |
| `agent_address` | 目标 agent address UUID |

插件应优先展示 `read_command`，而不是自己推测命令。

示例提醒文本：

```text
[agtalk] New message from nora. Run: /Users/me/projects/agtalk/target/debug/agtalk msg read
```

---

## 8. 插件开发示例

### 8.1 macOS 系统通知插件

```python
#!/usr/bin/env python3
import json
import subprocess
import sys

payload = json.load(sys.stdin)

from_name = payload.get("from_name", "agent")
read_command = payload.get("read_command", "agtalk msg read")

title = "agtalk"
message = f"New message from {from_name}. Run: {read_command}"

subprocess.run(
    [
        "/usr/bin/osascript",
        "-e",
        f'display notification "{message}" with title "{title}"'
    ],
    check=True,
)
```

安装：

```bash
mkdir -p ~/.config/agtalk2/plugins
cp agtalk-notify-macos ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-macos
```

配置：

```bash
agtalk config set notify.plugins.macos.path agtalk-notify-macos
agtalk config set notify.plugins.macos.timeout_ms 1000
agtalk id join coder --notify plugin:macos
```

### 8.2 webhook 插件

webhook 不作为内置 notify 通道。用户可以用插件实现。

```python
#!/usr/bin/env python3
import json
import os
import sys
import urllib.request

payload = json.load(sys.stdin)

url = os.environ["AGTALK_NOTIFY_WEBHOOK_URL"]
data = json.dumps({
    "text": (
        f"[agtalk] New message from {payload.get('from_name')}. "
        f"Run: {payload.get('read_command')}"
    )
}).encode("utf-8")

req = urllib.request.Request(
    url,
    data=data,
    headers={"Content-Type": "application/json"},
    method="POST",
)

with urllib.request.urlopen(req, timeout=2) as resp:
    if resp.status >= 300:
        raise SystemExit(f"webhook failed: {resp.status}")
```

secret 由插件自己的环境变量或配置文件管理。agtalk 不传 webhook token。

### 8.3 fake plugin 测试脚本

```python
#!/usr/bin/env python3
import json
import os
import sys

payload = json.load(sys.stdin)
out = os.environ["AGTALK_NOTIFY_TEST_OUT"]

with open(out, "w", encoding="utf-8") as f:
    json.dump(payload, f, ensure_ascii=False)
```

测试用途：

```bash
AGTALK_NOTIFY_TEST_OUT=/tmp/agtalk-notify.json agtalk id join coder --notify plugin:test
```

---

## 9. doctor 检查

`agtalk tool doctor` 应检查插件配置，但不执行插件。

检查项：

| ID | 说明 |
|---|---|
| `notify.channel` | 当前 session 的 notify channel |
| `notify.target` | 当前 session 的 notify target |
| `notify.plugin` | 插件配置是否有效 |

对于 `plugin:<name>`，doctor 检查：

1. config 中是否存在该 plugin。
2. path 是否是绝对路径。
3. path 是否存在。
4. path 是否可执行。
5. timeout 是否合理。

doctor 不调用插件，避免诊断命令产生通知副作用。

示例输出：

```text
notify.channel        ok     plugin:macos
notify.target         ok     plugin macos
notify.plugin         ok     /Users/me/.config/agtalk2/plugins/agtalk-notify-macos
```

---

## 10. 安全边界

### 10.1 不传正文

插件永远不应收到消息正文。正文只能由 agent 主动执行 `agtalk msg read` 后读取。

### 10.2 不传认证 secret

插件不需要 browser token、session secret、高熵 token。它只知道目标 agent 的公开 address 和取信命令。

### 10.3 不经 shell

所有插件执行必须使用 `Command::new(path)` 和 stdin JSON。

### 10.4 插件是用户信任边界内代码

插件可以执行任意本地逻辑，因此配置插件等价于信任这个本地可执行文件。agtalk 只负责避免 shell 注入和避免传敏感消息内容。

### 10.5 插件不可阻塞 daemon

daemon 必须给插件设置 timeout。插件超时后按失败处理。

---

## 11. 错误与降级

notify plugin 错误不影响核心消息路径。

| 错误 | 处理 |
|---|---|
| 插件未配置 | 记录 debug，降级 pure pull |
| path 非绝对 | 记录 debug，降级 pure pull |
| 文件不存在 | 记录 debug，降级 pure pull |
| 文件不可执行 | 记录 debug，降级 pure pull |
| JSON 写入 stdin 失败 | 记录 debug，降级 pure pull |
| 插件返回非 0 | 记录 stderr，降级 pure pull |
| 插件超时 | kill 进程，降级 pure pull |

对于用户可见诊断，使用：

```bash
agtalk tool doctor
agtalk tool doctor --debug
```

---

## 12. 实现计划

### 12.1 数据结构

扩展 `NotifyTarget`：

```rust
Plugin {
    name: String,
}
```

新增 payload 结构：

```rust
NotifyPluginPayload {
    version: u32,
    type_: String,
    from_name: String,
    read_command: String,
    binary_path: String,
    workspace: String,
    agent_name: String,
    agent_address: String,
}
```

### 12.2 notify channel

新增模块：

```text
src-tauri/src/notify/plugin.rs
```

职责：

1. 根据 plugin name 读取 `AgConfig::load().notify.plugins`。
2. 校验 path。
3. 构造 payload。
4. 启动插件进程。
5. 写 stdin JSON。
6. 等待完成或 timeout。
7. 返回 `NotifyError`。

### 12.3 channel 解析

扩展 `resolve_channel(raw)`：

```text
none          -> disabled
auto          -> zellij / tmux / none
zellij        -> built-in zellij
tmux          -> built-in tmux
plugin:<name> -> plugin target
unknown       -> none 或明确错误
```

建议 v1 对未知通道降级为 none，保持 join 不阻塞；doctor 负责指出配置问题。

### 12.4 trigger 流程

当前流程：

```text
address -> session.json -> notify_channel -> channel.inject()
```

插件后流程：

```text
address -> session.json
  -> notify_channel == plugin:<name>
  -> plugin channel 读取 config
  -> 构造 payload
  -> 执行插件
```

### 12.5 doctor

补充插件检查，但不执行插件。

---

## 13. 测试计划

单元测试：

1. `resolve_channel("plugin:macos")` 返回 plugin target。
2. `id join --notify plugin:macos` 正确写入 session。
3. plugin path 非绝对路径时报错。
4. plugin 不存在时报错。
5. plugin 不可执行时报错。
6. plugin payload 不包含正文、subject、token。
7. plugin exit 0 时成功。
8. plugin exit 非 0 时返回 `CommandFailed`。
9. plugin timeout 后进程被终止。
10. zellij / tmux / none 行为保持不变。

集成测试：

1. 创建临时 fake plugin，读取 stdin 并写入临时文件。
2. 配置 `notify.plugins.test.path` 指向 fake plugin。
3. 创建 session 使用 `plugin:test`。
4. 调用 `notify::trigger`。
5. 断言 fake plugin 收到 payload。
6. 断言 payload 中没有正文。

doctor 测试：

1. `notify.channel = plugin:test` 时显示 plugin 通道。
2. 插件配置缺失时给出 warn。
3. path 非绝对时给出 error 或 warn。
4. path 有效时给出 ok。

---

## 14. 一句话总结

```text
notify 插件把 agtalk 的“有消息”信号交给用户自定义本地程序处理；
它只负责打扰，不负责传输；
它只收轻量 payload，不收正文和 secret；
失败时消息仍可靠留在 daemon 中，agent 仍可通过 msg read 拉取。
```
