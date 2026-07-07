# agtalk-notify-zellij

agtalk 的 zellij notify 插件。把 daemon 的"有消息"信号转换成 zellij pane 内的文本提示。

`discover` 在 `agtalk id join` 时由 CLI 在当前 shell 执行，`send` 由 daemon 在收到消息时执行。

## 命令

```bash
agtalk-notify-zellij discover        # 输出当前 zellij endpoint JSON
agtalk-notify-zellij send            # 从 stdin 读取 notify payload，执行提醒
agtalk-notify-zellij send --dry-run  # 只验证 endpoint 可达
```

## 安装

```bash
cargo build -p agtalk-notify-zellij --release
mkdir -p ~/.config/agtalk2/plugins
cp target/release/agtalk-notify-zellij ~/.config/agtalk2/plugins/
chmod +x ~/.config/agtalk2/plugins/agtalk-notify-zellij
```

## 使用

在 zellij session 内：

```bash
agtalk id join my-agent --notify plugin:zellij
```

## 协议

见 `src/protocol.rs` 和项目根目录 `docs/notify-plugin.md`。
