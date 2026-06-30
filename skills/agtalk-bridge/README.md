# agtalk-bridge skill

给 AI agent（Kimi / codex / claude code / 任何 CLI agent）用的 agtalk 通信手册。教会 agent 如何通过 agtalk 总线：发消息给其他 agent、向人类请求审批、接收回复、以及在 context compaction 后恢复身份。

## 这是什么

`SKILL.md` 是 skill 主体。它告诉 agent：

- **身份在文件系统，不在脑子里**——`.agtalk/<name>/session.json` 承载身份，`agtalk whoami` 随时恢复，compact 压不到磁盘。
- **路由只认 UUID**——发消息前先 `lookup` 拿到收件人 UUID，绝不能按 name 发。
- **收消息默认 pull**（`inbox` / `detail -`），SSE 只用于秒级命中且必带超时——这是 Kimi/codex/claude code 三个 agent 实际验证后的结论。
- **没有阻塞 wait 命令**——长阻塞会被 agent 执行框架超时强杀，agent 自己用 pull 循环或 `curl --max-time` 控制。

## 设计依据

skill 内容与以下文档一致，互相对账：
- `docs/design.md` §4（接收消息两条路径）
- `docs/commands.md`（命令参考 + 设计原则）
- 三个 agent 的反馈：Kimi（不能原生 SSE，用 pull）、codex（可 curl SSE）、claude code（必须带 max-time，curl 比 fetch 顺）

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
