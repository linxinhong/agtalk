# agtalk notify plugin 协议

版本：v2

定位：agtalk core 只负责把"有消息"信号交给本地可执行插件；具体如何提醒（zellij、tmux、系统通知、webhook、BLE 等）由插件实现。插件不是消息传输通道，不读取消息正文，不持有 agtalk token。

---

## 1. 核心原则

1. **只推信号，不推正文**：插件只收到轻量 payload，不含消息 body、subject、attachment、token。
2. **插件不是 transport**：消息可靠投递仍走 daemon -> SQLite -> SSE / `agtalk msg read`。
3. **插件失败不影响消息**：notify 失败只记录日志，消息已落库、SSE 已触发。
4. **不经 shell**：daemon 用参数数组执行插件，插件从 stdin 读 JSON。
5. **discover + send 两阶段**：`id join` 时由 CLI 在当前 shell 调用插件 `discover` 缓存 endpoint；daemon `send` 失败时可自动重新 `discover` 刷新。
6. **CLI 侧 discover**：`agtalk id join --notify plugin:<name>` 必须在能访问目标终端/session 的 shell 中执行，失败时直接报错，不自动降级。
7. **不限语言**：插件可以是 shell、Python、Go、Rust 等任何可执行文件，只要支持 `discover` / `send` 协议。

---

## 2. 通道选择

```bash
agtalk id join coder --notify none              # 不打扰
agtalk id join coder --notify plugin:zellij     # zellij 终端提示
agtalk id join coder --notify plugin:tmux       # tmux 终端提示
agtalk id join coder --notify plugin:macos      # 自定义插件
agtalk id join coder --notify auto              # 自动尝试 plugin:zellij -> plugin:tmux -> none
```

**注意**：v2 已移除 core 内置 `zellij` / `tmux` 通道。要使用终端通知，必须把对应 plugin 安装到 `<config_dir>/plugins/`（通常为 `~/.config/agtalk2/plugins/`），或在 PATH 中提供 `agtalk-notify-<name>`。

---

## 3. 插件二进制发现

`plugin:<name>` 按以下顺序查找可执行文件：

1. 全局配置 `notify.plugins.<name>.path`
   - 绝对路径：直接使用。
   - 相对路径/纯文件名：解析为 `<config_dir>/plugins/<path>`，禁止 `..` 逃逸。
2. 若配置未指定，在 PATH 中查找 `agtalk-notify-<name>`。

**推荐做法**：直接放入 `<config_dir>/plugins/`，无需在 config.json 中登记：

```bash
mkdir -p ~/.config/agtalk2/plugins
cp agtalk-notify-zellij ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-zellij
# 无需配置，agtalk 自动在 ~/.config/agtalk2/plugins/ 发现
```

也支持显式配置（例如插件不在 `<config_dir>/plugins/` 而是在其他目录）：

```bash
agtalk config set notify.plugins.macos.path /opt/agtalk/plugins/agtalk-notify-macos
```

或安装到 PATH 作为备选：

```bash
cp agtalk-notify-zellij ~/.local/bin/
chmod +x ~/.local/bin/agtalk-notify-zellij
```

---

## 4. 插件协议

插件必须支持两个子命令：

```bash
<plugin> discover        # stdout 输出 JSON endpoint
<plugin> send [--dry-run] # stdin 读 JSON payload，执行提醒或验证
```

### 4.1 discover 输出

```json
{
  "version": 1,
  "type": "notify_endpoint",
  "channel": "zellij",
  "ready": true,
  "endpoint": {
    "session": "agtalk",
    "pane": "1"
  },
  "message": "zellij session agtalk, pane 1"
}
```

字段：

| 字段 | 必填 | 说明 |
|---|---:|---|
| `version` | 是 | 固定 `1` |
| `type` | 是 | 固定 `"notify_endpoint"` |
| `channel` | 是 | 插件标识，必须等于插件名 |
| `ready` | 是 | 当前环境是否可通知 |
| `endpoint` | 否 | 插件自定义定位对象，core 透传 |
| `message` | 否 | 人类可读说明 |

### 4.2 send 输入

```json
{
  "version": 1,
  "type": "notify",
  "endpoint": {
    "session": "agtalk",
    "pane": "1"
  },
  "from_name": "nora",
  "read_command": "agtalk --as coder msg read",
  "read_args": ["--as", "coder", "msg", "read"],
  "binary_path": "/usr/local/bin/agtalk",
  "agent_name": "coder",
  "agent_address": "550e8400-e29b-41d4-a716-446655440000",
  "message_id": "msg-123",
  "text": "[agtalk:msg-123] | from nora | exec: agtalk --as coder msg read"
}
```

字段：

| 字段 | 说明 |
|---|---|
| `version` | 必填，当前固定 `1`；插件收到其他版本应报错退出（拒绝未知协议） |
| `type` | 固定 `"notify"` |
| `endpoint` | discover 返回的 endpoint 对象 |
| `from_name` | 发送方展示名 |
| `read_command` | 人类可读取信命令 |
| `read_args` | 插件可直接执行的安全参数数组 |
| `binary_path` | 当前 agtalk 二进制路径 |
| `agent_name` | 目标 agent 名称 |
| `agent_address` | 目标 agent address UUID |
| `message_id` | 触发本次 notify 的消息 ID |
| `text` | daemon 组装好的注入文本，插件默认直接透传 |

### 4.3 注入文本风格

参考实现直接透传 `text`：

```text
[agtalk:c3384a71] | from nora | exec: agtalk --as coder msg read
```

- 前缀 `[agtalk:<短 id>]` 为 `message_id` 的前 8 位，方便 agent 直接识别是哪条消息（完整 UUID 见 `agtalk msg read --json`）。
- `exec:` 后接可直接执行的取信命令，`--as <agent>` 指定本地身份。
- 插件默认不自己组装文本，只读取 `text` 注入终端；如需自定义格式，由 daemon 端调整。

### 4.4 send --dry-run

只验证 `endpoint` 是否可达，不真正打扰用户。用于 `agtalk tool doctor`。

---

## 5. session.json 持久化

```json
{
  "notify_channel": "plugin:zellij",
  "notify_target": {
    "type": "plugin",
    "name": "zellij",
    "endpoint": {
      "session": "agtalk",
      "pane": "1"
    }
  }
}
```

`endpoint` 由插件 discover 输出，agtalk core 透传并在 send 失败时重新 discover 刷新。

---

## 6. 参考实现

项目提供两个参考插件，均为 shell 脚本：

- `plugins/agtalk-notify-zellij`
- `plugins/agtalk-notify-tmux`

插件不限语言，只要实现 `discover` / `send [--dry-run]` 协议即可。

### 6.1 zellij 安装

```bash
mkdir -p ~/.config/agtalk2/plugins
cp plugins/agtalk-notify-zellij ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-zellij
agtalk id join coder --notify plugin:zellij
```

> 备注：`id join` 时 core 会传入 `AGTALK_NOTIFY_NAME=<agent>`；该环境变量非空时，zellij 参考插件会把当前 pane 重命名为 agent 名，方便在多 pane 中识别身份。这是有意副作用，仅在显式 `id join --notify plugin:zellij` 时发生；`send` 阶段不会改名。

### 6.2 tmux 安装

```bash
mkdir -p ~/.config/agtalk2/plugins
cp plugins/agtalk-notify-tmux ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-tmux
agtalk id join coder --notify plugin:tmux
```

---

## 7. 自定义插件示例

### macOS 系统通知

```python
#!/usr/bin/env python3
import json
import subprocess
import sys

cmd = sys.argv[1] if len(sys.argv) > 1 else ""

if cmd == "discover":
    print(json.dumps({
        "version": 1,
        "type": "notify_endpoint",
        "channel": "macos",
        "ready": True,
        "endpoint": {},
        "message": "macOS notification center",
    }))
    sys.exit(0)

if cmd == "send":
    payload = json.load(sys.stdin)
    from_name = payload.get("from_name", "agent")
    read_command = payload.get("read_command", "agtalk msg read")
    subprocess.run([
        "/usr/bin/osascript",
        "-e",
        f'display notification "New message from {from_name}. Run: {read_command}" with title "agtalk"'
    ], check=True)
    sys.exit(0)

print("usage: agtalk-notify-macos discover | send [--dry-run]", file=sys.stderr)
sys.exit(1)
```

---

## 8. doctor 检查

`agtalk tool doctor` 对 plugin 通道执行：

1. 查找插件二进制（config path 或 PATH）。
2. 调用 `discover`。
3. 若 `ready`，调用 `send --dry-run`。

check ID：

| ID | 说明 |
|---|---|
| `notify.environment` | auto 检测是否发现可用 plugin |
| `notify.channel` | 当前 session 的 notify channel |
| `notify.target` | 当前 session 的 notify target |
| `notify.plugin.binary` | 插件二进制是否可用 |
| `notify.plugin.discover` | discover 是否 ready |
| `notify.plugin.endpoint_stale` | session 中缓存的 endpoint 是否和当前环境一致 |
| `notify.plugin.dry_run` | send --dry-run 是否成功 |

---

## 9. 安全边界

1. 插件永远收不到消息正文。
2. 插件不接收 browser token、session secret。
3. daemon 不经 shell 执行插件。
4. 插件 timeout 默认 1000ms，最大 10000ms，超时被 kill。
5. 相对路径禁止 `..`，canonicalize 后必须在 `<config_dir>/plugins/` 内。

---

## 10. 错误降级

| 错误 | 处理 |
|---|---|
| 插件二进制缺失 | CLI `id join` 报错；无 CLI discover 时 daemon 侧兜底降级为 none；doctor 报 error |
| discover 返回 not ready | CLI `id join` 报错；无 CLI discover 时 daemon 侧兜底降级为 none；doctor 报 error |
| send 失败 | 自动重新 discover 一次并重试；仍失败则记录 warn，不影响消息 |
| send 超时 | kill 插件进程，记录 warn，不影响消息 |
| endpoint 过期 | doctor 报 warn，提示在当前终端环境内重新 join |

---

## 11. 一句话总结

```text
notify plugin 把 agtalk 的"有消息"信号交给用户自定义本地程序处理；
core 只负责 discover 缓存、send 触发、doctor 检查；
插件只收轻量 payload，不收正文和 secret；
失败时消息仍可靠留在 daemon 中，agent 仍可通过 msg read 拉取。
```
