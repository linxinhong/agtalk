# agtalk-bridge skill

给 AI agent（Kimi / codex / claude code / 任何 CLI agent）用的 agtalk 通信手册。教会 agent 如何通过 agtalk 总线：发消息给其他 agent、向人类请求审批、接收回复、以及在 context compaction 后恢复身份。

## 这是什么

`SKILL.md` 是 skill 主体。它告诉 agent：

- **身份在文件系统，不在脑子里**——`.agtalk/<name>/session.json` 承载身份，`agtalk whoami` 随时恢复，compact 压不到磁盘。
- **路由只认 UUID**——发消息前先 `lookup` 拿到收件人 UUID，绝不能按 name 发。
- **收消息默认 pull**（`inbox` / `msg read`），SSE 只用于短期等待——`agtalk wait <msg-id> --timeout`。
- **`join` 是幂等的**——重复执行会复用同一 address，agent 可以放心在每轮任务前调用。
- **`--as <name>` / `AGTALK_NAME` 是身份选择器**——只用于选择本地 session，不参与消息路由。

## 设计依据

skill 内容与以下文档一致，互相对账：
- `docs/design.md` §10（Agent-First 便利层）
- `docs/design.md` §4（接收消息两条路径）
- `docs/commands.md`（命令参考 + 设计原则）

## 安装（让 agent 能发现这个 skill）

skill 需要在 agent 的 skill 目录下才能被发现。软链接最简单（保持与仓库同步）：

```bash
ln -s ~/projects/agtalk/skills/agtalk-bridge ~/.agents/skills/agtalk-bridge
```

或在 agent 配置里把 `~/projects/agtalk/skills` 加入 skill 搜索路径。

## 验证

agent 安装后，可让它执行一次自检（skill 里 Step 0–1）：

```
请用 agtalk-bridge skill：检查 daemon 状态、确认你是谁（whoami）、列出可用 agent（lookup）。
```

如果三条命令都正常返回，说明 agent 已能正确使用 agtalk 通信。
