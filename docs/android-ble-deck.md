# agtalk Android Deck 与 BLE Transport 设计

> 版本：v0.1
> 状态：设计草案，待实现
> 定位：将 Android 手机作为 agtalk 的移动控制台与 Vibe Coding Deck，通过 BLE 与本机 agtalk daemon 直连。

---

## 1. 目标

Android Deck 让备用 Android 手机成为 agtalk 的移动 participant：

```text
Android APK
  -> BLE
agtalk daemon
  -> agent / human / notify / inbox / routing
```

核心能力：

- 手机通过 BLE 连接 agtalk daemon。
- 手机显示 Deck 按钮、Inbox、Notify 和轻量状态。
- 手机点击按钮发送 `deck.trigger`。
- agtalk 展开本地按钮模板并发送普通 agtalk message。
- agtalk notify 通过 BLE 推送到手机。
- 手机支持 Reply / Done。

非目标：

- v0.1 不做蓝牙 HID 键盘。
- v0.1 不做大文件传输或超长 Prompt 传输。
- v0.1 不允许 Android 执行 shell、git push、配置修改等高风险动作。
- v0.1 不把 BLE 连接状态写进核心 `messages` 表。
- Android APK 不是完整桌面端替代品。

---

## 2. 总体架构

```text
Android APK                         agtalk daemon
------------                        -------------
Deck / Inbox / Notify               transport/ble
Reply / Done                        scanner / connection
BLE GATT Server / Advertiser   ->   protocol / codec / security
                                    routing / msg / notify / storage
```

角色划分：

| 组件 | BLE 角色 | agtalk 角色 | 职责 |
|---|---|---|---|
| Android APK | Peripheral / GATT Server | mobile participant | UI、按钮触发、通知展示、快速回复 |
| agtalk daemon | Central / GATT Client | 本地总线 | 扫描连接、协议转换、消息投递、notify 推送 |
| agents | 无 | 普通 mailbox | 接收和回复 agtalk message |

关键决策：

- Android 广播 `agtalk-deck` 并提供 GATT Server。
- agtalk daemon 扫描并连接 Android。
- BLE 是 transport，不新增业务消息模型。
- Deck 模板只存在 agtalk 本地，Android 只发送 `deck_id + button_id`。
- 连接配置、配对、设备管理、Deck 调试统一走 `agtalk config gui`，不新增 `agtalk mobile ...` 或 `agtalk deck ...` 顶层命令。

---

## 3. Android APK

项目目录：

```text
android/
  settings.gradle.kts
  build.gradle.kts
  app/
    build.gradle.kts
    src/main/AndroidManifest.xml
    src/main/java/.../MainActivity.kt
```

技术栈：

```text
Kotlin
Jetpack Compose
BluetoothGattServer
BluetoothLeAdvertiser
Coroutine / Flow
DataStore
```

模块：

```text
ui/deck        Deck 网格、按钮状态
ui/inbox       消息列表、详情、Reply、Done
ui/notify      通知历史、系统通知入口
ui/device      配对、连接状态、Debug 信息
ble/           GATT Server、Advertiser、Protocol、Chunker
data/          DeviceStore、DeckCache、SettingsStore
domain/        TriggerDeckButton、SendReply、MarkDone、HandleAgtalkEvent
```

Android 本地只保存：

- `device_id`
- pairing token
- 最近连接状态
- Deck UI 布局缓存
- 最近通知缓存

真实 conversation、message、notify 仍以 agtalk daemon 为准。

### 3.1 页面

v0.1 页面：

- Deck：按钮网格，点击发送 `deck.trigger`。
- Inbox：展示待处理消息，支持 Reply / Done。
- Notify：展示 `message.new` / `notify.agent_waiting`，触发系统通知和震动。
- Device：显示连接状态、pairing code、协议版本、最近错误。
- Settings：基础设置和低电量/震动开关。

### 3.2 权限

`AndroidManifest.xml`：

```xml
<uses-permission android:name="android.permission.BLUETOOTH" />
<uses-permission android:name="android.permission.BLUETOOTH_ADMIN" />
<uses-permission android:name="android.permission.BLUETOOTH_SCAN" />
<uses-permission android:name="android.permission.BLUETOOTH_CONNECT" />
<uses-permission android:name="android.permission.BLUETOOTH_ADVERTISE" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE" />

<uses-feature
    android:name="android.hardware.bluetooth_le"
    android:required="true" />
```

Android 12 及以上需要运行时请求 Nearby devices 相关权限。

---

## 4. agtalk BLE Transport

Cargo feature：

```toml
[features]
ble = ["btleplug"]

[dependencies]
btleplug = { version = "0.12", optional = true }
```

模块：

```text
src-tauri/src/transport/ble/
  mod.rs
  scanner.rs
  connection.rs
  protocol.rs
  codec.rs
  chunk.rs
  pairing.rs
  security.rs
  device_registry.rs
```

职责：

- `scanner.rs`：初始化 adapter，扫描 name prefix / service UUID。
- `connection.rs`：连接、发现 service、读写 characteristic、重连。
- `protocol.rs`：Envelope、命令、事件、协议版本。
- `codec.rs`：JSON encode/decode、payload hash、字段校验。
- `chunk.rs`：分片、重组、超时清理、重复分片去重。
- `pairing.rs`：pairing mode、配对码、token 下发。
- `security.rs`：token 校验、权限等级、审计。
- `device_registry.rs`：trusted device 查询与更新。

BLE transport 只做协议转换：

```text
deck.trigger    -> msg.send
message.reply   -> msg.reply
message.done    -> msg.done
agtalk notify   -> BLE event
```

禁止：

- BLE 直接写业务表。
- BLE 绕过 UUID 路由。
- BLE 触发 shell。
- BLE 直接修改 agtalk config。

---

## 5. 配置与 GUI

配置：

```toml
[transport.ble]
enabled = false
scan_name_prefix = "agtalk-deck"
auto_reconnect = true
max_devices = 1
connect_timeout_ms = 8000
heartbeat_interval_ms = 5000
heartbeat_miss_limit = 3

[transport.ble.security]
pairing_required = true
device_whitelist = true
command_token_required = true

[mobile.deck]
default_deck = "coding"
allow_broadcast = false
allow_shell = false
```

配置入口：

```bash
agtalk config gui
```

GUI 提供：

- BLE enable / disable。
- 扫描附近 `agtalk-deck`。
- 开启 pairing mode。
- 显示配对码。
- 信任/移除设备。
- 连接状态、last seen、协议版本。
- Deck list/show/edit/trigger 调试。
- BLE Debug 日志与最近 command ack/error。

CLI 只保留底层配置：

```bash
agtalk config get transport.ble.enabled
agtalk config set transport.ble.enabled true
```

不新增公开顶层命令：

```text
agtalk mobile ...
agtalk deck ...
```

---

## 6. BLE 协议

### 6.1 GATT Service

```text
Service name: AGTALK_DECK_SERVICE
Service UUID: b8f0a000-7f3b-4c2f-9d2a-a70000000001
```

Characteristics：

| Name | UUID | Direction | Property |
|---|---|---|---|
| `mobile_event_notify` | `b8f0a001-7f3b-4c2f-9d2a-a70000000001` | Android -> agtalk | Notify |
| `daemon_event_write` | `b8f0a002-7f3b-4c2f-9d2a-a70000000001` | agtalk -> Android | Write / Write Without Response |
| `device_info` | `b8f0a003-7f3b-4c2f-9d2a-a70000000001` | agtalk reads Android | Read |
| `heartbeat` | `b8f0a004-7f3b-4c2f-9d2a-a70000000001` | 双向 | Write / Notify |

说明：

- Android 是 GATT Server，所以 Android -> agtalk 使用 characteristic notify。
- agtalk -> Android 使用 central write 到 `daemon_event_write`；Android 收到写入后更新 UI、系统通知和震动。

### 6.2 Envelope

v0.1 使用 JSON：

```json
{
  "id": "cmd_001",
  "version": 1,
  "type": "deck.trigger",
  "timestamp": 1760000000000,
  "device_id": "android_001",
  "token": "<pairing-token>",
  "payload": {}
}
```

Android -> agtalk：

- `pair.confirm`
- `deck.trigger`
- `message.reply`
- `message.done`
- `heartbeat`

agtalk -> Android：

- `ack`
- `error`
- `message.new`
- `notify.agent_waiting`
- `inbox.snapshot`
- `heartbeat`

### 6.3 Command Examples

`deck.trigger`：

```json
{
  "id": "cmd_001",
  "version": 1,
  "type": "deck.trigger",
  "timestamp": 1760000000000,
  "device_id": "android_001",
  "token": "<pairing-token>",
  "payload": {
    "deck_id": "coding",
    "button_id": "ask_kimi_review"
  }
}
```

`message.reply`：

```json
{
  "id": "cmd_002",
  "version": 1,
  "type": "message.reply",
  "timestamp": 1760000000000,
  "device_id": "android_001",
  "token": "<pairing-token>",
  "payload": {
    "message_id": "msg_456",
    "content": "继续，但只修改必要文件，不要重构。"
  }
}
```

`message.done`：

```json
{
  "id": "cmd_003",
  "version": 1,
  "type": "message.done",
  "timestamp": 1760000000000,
  "device_id": "android_001",
  "token": "<pairing-token>",
  "payload": {
    "message_id": "msg_456"
  }
}
```

`message.new`：

```json
{
  "id": "evt_001",
  "version": 1,
  "type": "message.new",
  "timestamp": 1760000001000,
  "payload": {
    "message_id": "msg_789",
    "from": "kimi-coder",
    "subject": "完成摘要",
    "preview": "本次修改已完成，验证结果如下..."
  }
}
```

---

## 7. 分片与可靠性

BLE 只传短命令。长 Prompt、模板和上下文保存在 agtalk 本地。

分片格式：

```json
{
  "frame": {
    "message_id": "cmd_001",
    "seq": 1,
    "total": 3
  },
  "data": "base64..."
}
```

策略：

| 场景 | 策略 |
|---|---|
| 命令未 ack | Android 3 秒后提示失败 |
| 分片缺失 | agtalk 等待 5 秒后丢弃 |
| 心跳丢失 | 连续 3 次丢失则断开 |
| 断线重连 | Android 继续广播，agtalk 自动重连 |
| 重复命令 | 根据 command `id` 返回上次结果 |

断线重连后，agtalk 推送当前未读 `inbox.snapshot`。

---

## 8. Pairing / Security / Audit

安全目标：

- 未授权手机不能控制 agtalk。
- 配对必须由本机用户确认。
- 所有手机触发动作必须审计。
- BLE 默认关闭。
- v0.1 不开放高风险动作。

配对流程：

```text
1. 用户打开 agtalk config gui
2. GUI 开启 BLE pairing mode
3. Android 广播 agtalk-deck service
4. agtalk 扫描到设备并显示配对码
5. Android 输入配对码
6. 双方建立 device token
7. agtalk 写入 trusted device
8. 后续命令携带 token 鉴权
```

新增表：

```sql
CREATE TABLE mobile_devices (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  platform TEXT NOT NULL,
  ble_address_hash TEXT,
  public_key TEXT,
  token_hash TEXT NOT NULL,
  trusted INTEGER NOT NULL DEFAULT 0,
  protocol_version INTEGER NOT NULL DEFAULT 1,
  paired_at REAL NOT NULL,
  last_seen_at REAL
);

CREATE TABLE mobile_action_logs (
  id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  action_type TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  result TEXT NOT NULL,
  error TEXT,
  created_at REAL NOT NULL
);

CREATE TABLE mobile_command_cache (
  command_id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  result_json TEXT NOT NULL,
  created_at REAL NOT NULL
);
```

权限：

| Level | 动作 | v0.1 策略 |
|---|---|---|
| L1 | `deck.trigger` / `message.reply` / `message.done` | trusted device 允许 |
| L2 | `message.forward` / `broadcast` / 读取完整 inbox | 默认关闭 |
| L3 | shell / git push / config set / 高风险 approval | 禁止 |

---

## 9. Deck 模型

新增表：

```sql
CREATE TABLE decks (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  scope TEXT NOT NULL DEFAULT 'workspace',
  created_at REAL NOT NULL,
  updated_at REAL NOT NULL
);

CREATE TABLE deck_buttons (
  id TEXT PRIMARY KEY,
  deck_id TEXT NOT NULL,
  title TEXT NOT NULL,
  icon TEXT,
  color TEXT,
  action_type TEXT NOT NULL,
  action_payload_json TEXT NOT NULL,
  order_index INTEGER NOT NULL,
  created_at REAL NOT NULL,
  updated_at REAL NOT NULL
);
```

按钮示例：

```json
{
  "id": "ask_kimi_review",
  "deck_id": "coding",
  "title": "让 Kimi 评审",
  "icon": "review",
  "color": "blue",
  "action_type": "message.send",
  "action_payload_json": {
    "to": "550e8400-e29b-41d4-a716-446655440000",
    "subject": "代码评审",
    "content": "请评审当前实现，重点关注架构风险、边界条件、是否过度设计。"
  }
}
```

规则：

- `to` 必须是 UUID address。
- name 只允许作为 GUI 展示，不进入路由。
- Android 只发送 `deck_id + button_id`。
- agtalk 本地展开模板，转换为现有 `msg.send`。

内置 Deck：

- Coding Deck
- Agent Control Deck
- Git Deck
- Personal Workflow Deck

---

## 10. 核心流程

### 10.1 手机点击按钮

```text
Android 点击按钮
  -> BLE deck.trigger
  -> agtalk 校验 trusted device/token
  -> command id 去重
  -> 查 deck button
  -> 展开本地模板
  -> 调 msg.send
  -> messages 落库
  -> SSE / notify 触发
  -> BLE ack 回 Android
```

### 10.2 Agent 回复

```text
Agent reply
  -> agtalk messages 落库
  -> SSE / notify
  -> BLE message.new 推送 Android
  -> Android 系统通知 / 震动 / Inbox 更新
```

### 10.3 Android Reply / Done

```text
Android Reply / Done
  -> BLE message.reply / message.done
  -> agtalk 校验权限
  -> 调现有 msg.reply / msg.done
  -> ack 回 Android
```

---

## 11. 测试计划

Rust：

- `protocol` / `codec` / `chunk` / `security` / `deck` 单元测试。
- fake BLE transport 集成测试：connect、pair、deck.trigger、reply、done、duplicate command。
- routing 测试确认 Deck 模板只按 UUID address 投递。
- notify 测试确认 BLE 推送失败不影响消息入库。
- `ble` feature 关闭时，常规 `cargo test -p agtalk` 仍通过。

Android：

- `BleProtocol` / `BleChunker` JVM tests。
- DataStore repository tests。
- Compose screen smoke tests。
- 手工 E2E：Android 广播 -> config GUI pair -> trusted -> trigger 发消息 -> agent reply -> 手机通知 -> 手机 Done。

验收：

- `agtalk config gui` 可开启 BLE 并扫描到 Android。
- GUI 配对后设备进入 trusted。
- 手机点击按钮能向指定 UUID agent 发消息。
- agent 回复后手机收到 notify。
- 手机 Reply / Done 能改变 agtalk message 状态。
- 重复点击不会重复发送。

---

## 12. MVP Milestones

### M0：协议与骨架

- agtalk `transport/ble` 模块骨架。
- Android APK 空壳。
- BLE Service UUID 与 protocol 定义。
- `agtalk config gui` 增加 BLE 设置页占位。

验收：daemon 在 `ble` feature 下能启动；Android APK 能开始广播。

### M1：连接

- Android GATT Server / Advertiser。
- agtalk 扫描、连接、读取 `device_info`。
- heartbeat。

验收：GUI 能看到 Android 设备。

### M2：配对与鉴权

- GUI pairing mode。
- 配对码。
- token 保存。
- trusted device。
- 未配对设备拒绝命令。

验收：未配对手机不能触发 `deck.trigger`；已配对手机可以触发。

### M3：Deck Trigger

- Android Deck 页面。
- `deck.trigger` 命令。
- agtalk 展开按钮模板并发送 `msg.send`。
- ack 回传。

验收：手机点击按钮后，agtalk 成功向指定 agent 发送消息。

### M4：Notify

- agtalk notify 转 BLE event。
- Android Notify 页面。
- Android 系统通知和震动。

验收：agent 回复后，Android 收到通知。

### M5：Inbox / Reply / Done

- Android Inbox 页面。
- `message.reply`。
- `message.done`。

验收：手机可以处理待办消息。

### M6：稳定化

- 分片。
- 重连。
- 幂等。
- 审计日志。
- Debug 页面。

验收：断线重连后不丢消息；重复点击不会重复发送。

---

## 13. 参考

- Android BLE overview: https://developer.android.com/develop/connectivity/bluetooth/ble/ble-overview
- Android `BluetoothLeAdvertiser`: https://developer.android.com/reference/android/bluetooth/le/BluetoothLeAdvertiser
- Android `BluetoothGattServer`: https://developer.android.com/reference/android/bluetooth/BluetoothGattServer
- Rust `btleplug`: https://docs.rs/btleplug/latest/btleplug/
