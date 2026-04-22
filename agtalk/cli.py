# agtalk/cli.py — 薄 CLI 入口
import os
import sys
import json
import random
import click

from . import db, registry, messenger
from .factory import get, get_agent_name
from .delivery import notify as _fifo_notify, watch_until_done, _ensure_fifo, notify_agent

_PRESET_NAMES = ["Alex", "Bob", "Chris", "David", "Emma", "Frank", "Grace", "Henry", "Iris", "Jack", "Kate", "Liam", "Mary", "Nick", "Olivia", "Paul", "Quinn", "Rose", "Sam", "Tom", "Uma", "Victor", "Wendy", "Xin", "Yale", "Zoe"]


def _unescape_body(body: str) -> str:
    """将字面转义字符替换为真实控制字符。

    支持 \\n → \n, \\t → \t, \\r → \r, \\\\ → \
    按反序处理避免冲突。
    """
    body = body.replace("\\\\", "\x00")  # 临时占位
    body = body.replace("\\n", "\n")
    body = body.replace("\\t", "\t")
    body = body.replace("\\r", "\r")
    body = body.replace("\x00", "\\")
    return body


# 命令分组
_COMMAND_GROUPS = {
    "注册管理": ["register", "unregister", "list"],
    "消息通信": ["send", "notify", "broadcast", "multicast", "key-enter", "inbox", "done"],
    "系统工具": ["init", "prune", "check-stuck", "health", "memory"],
}


def _format_help(self, ctx, formatter):
    """自定义帮助格式，按分组显示命令"""
    for group_name, commands in _COMMAND_GROUPS.items():
        formatter.write_heading(f"{group_name}")
        for cmd_name in commands:
            cmd = ctx.command.get_command(ctx, cmd_name)
            if cmd:
                help_text = cmd.get_short_help_str()
                formatter.write(f"  {cmd_name:<20} {help_text}\n")
        formatter.write("\n")


class GroupWithHelp(click.Group):
    """带分组的自定义 Group"""
    def format_help(self, ctx, formatter):
        # 输出标题和用法
        formatter.write("\n  agtalk — agent talk\n\n")
        formatter.write("  Usage: agtalk COMMAND [OPTIONS]\n\n")
        # 输出分组命令列表
        _format_help(self, ctx, formatter)
        # 输出选项说明
        formatter.write("\nOptions:\n")
        formatter.write("  --help                      Show this message and exit.\n")


@click.group(cls=GroupWithHelp)
def cli():
    """agtalk — agent talk"""
    pass


# ─── 初始化 ──────────────────────────────────────────
@cli.command()
def init():
    """初始化环境：检查终端环境、初始化数据库、确保 FIFO 存在"""
    errors = []
    mux = get()

    # Step 1: 检查终端环境变量
    try:
        session = mux.get_current_session()
        pane_id = mux.get_current_pane_id()
        click.echo(f"✅ 终端环境: session={session}, pane_id={pane_id}")
    except EnvironmentError as e:
        errors.append(f"❌ 终端环境: {e}")

    # Step 2: 检查底层 CLI 可执行
    import shutil
    if shutil.which("zellij"):
        click.echo("✅ zellij CLI 可用")
    else:
        errors.append("❌ zellij CLI 未找到")

    # Step 3: 初始化数据库
    try:
        db.init_db()
        click.echo(f"✅ 数据库: {db.get_db_path()}")
    except Exception as e:
        errors.append(f"❌ 数据库初始化: {e}")

    # Step 4: 确保 FIFO 存在
    try:
        _ensure_fifo()
        from .delivery import FIFO_PATH
        click.echo(f"✅ FIFO: {FIFO_PATH}")
    except Exception as e:
        errors.append(f"❌ FIFO 创建: {e}")

    if errors:
        for err in errors:
            click.echo(err, err=True)
        sys.exit(1)


# ─── 注册 ────────────────────────────────────────────
@cli.command()
@click.argument("agent_name")
@click.option("--role", default="", help="主要职责 coder/reviewer/planner/tester")
@click.option("--capabilities", default="", help="能力列表，逗号分隔")
@click.option("--bio", default="", help="自我介绍")
def register(agent_name, role, capabilities, bio):
    """注册当前 pane 为 Agent"""
    try:
        info = registry.register(agent_name, role, capabilities, bio)
    except EnvironmentError as e:
        click.echo(f"❌ {e}", err=True)
        sys.exit(1)
    os.environ["AGTALK_AGENT_NAME"] = agent_name
    click.echo(f"✅ 已注册: {agent_name} @ {info['session']}:{info['pane_id']}")
    if bio:
        click.echo(f"📝 Bio: {bio}")


@cli.command()
@click.argument("agent_name")
def unregister(agent_name):
    """注销 Agent"""
    registry.unregister(agent_name)
    click.echo(f"🗑 已注销: {agent_name}")


@cli.command("list")
@click.option("--json", "as_json", is_flag=True)
@click.option("--capabilities", is_flag=True)
def list_agents(as_json, capabilities):
    """列出所有注册 Agent"""
    with db.get_conn() as conn:
        if capabilities:
            rows = conn.execute("SELECT agent_name, session, pane_id, role, capabilities, last_seen_at FROM agents").fetchall()
        else:
            rows = conn.execute("SELECT agent_name, session, pane_id, role, last_seen_at FROM agents").fetchall()

    if as_json:
        click.echo(json.dumps([dict(r) for r in rows], indent=2))
    else:
        if not rows:
            click.echo("暂无注册的 Agent")
            return
        click.echo(f"{'Agent Name':<30} {'Session':<15} {'Pane ID':<10} {'Role':<15}")
        click.echo("-" * 70)
        for r in rows:
            cap = f" ({r['capabilities']})" if capabilities and r['capabilities'] else ""
            click.echo(f"{r['agent_name']:<30} {r['session']:<15} {r['pane_id']:<10} {r['role']}{cap}")


# ─── 发消息 ──────────────────────────────────────────
@cli.command()
@click.argument("agent")
@click.argument("body")
@click.option("--subject", default="", help="消息主题")
@click.option("--type", "msg_type", default="text", help="text/task/file")
@click.option("--priority", default=5, type=int, help="优先级 1-9")
@click.option("--wait-done", is_flag=True, help="等待对方 mark done")
@click.option("--timeout", default=120, type=int, help="等待超时秒数")
@click.option("--reply-to", default=None, help="回复指定的消息 ID")
@click.option("--notify", is_flag=True, help="发送后自动提醒对方查收")
@click.option("--no-enter", is_flag=True, help="提醒不自动发送 Enter")
def send(agent, body, subject, msg_type, priority, wait_done, timeout, reply_to, notify, no_enter):
    """发送消息给指定 Agent（仅写入 inbox）。支持 \\n 转义换行。"""
    body = _unescape_body(body)
    try:
        msg_id = messenger.send(
            to_agent=agent,
            body=body,
            subject=subject,
            msg_type=msg_type,
            priority=priority,
            reply_to=reply_to,
            deliver=False,
        )
    except EnvironmentError as e:
        click.echo(f"❌ {e}", err=True)
        sys.exit(1)
    click.echo(f"📨 消息已存入 inbox: {msg_id[:8]}... → {agent}")

    if notify:
        agent_info = registry.lookup(agent)
        try:
            from_agent = get_agent_name()
        except Exception:
            from_agent = "unknown"
        if agent_info:
            try:
                notify_agent(agent, from_agent, agent_info, send_enter=not no_enter, msg_id=msg_id)
                click.echo(f"🔔 已提醒 {agent} 查收")
            except Exception as e:
                click.echo(f"🔔 提醒发送失败: {e}")
        else:
            click.echo(f"🔔 {agent} 离线，无法提醒")

    if wait_done:
        click.echo(f"等待对方完成... (超时 {timeout}s)")
        status = watch_until_done(msg_id, timeout)
        if status == "done":
            click.echo("✅ 对方已标记完成")
        elif status == "failed":
            click.echo("❌ 消息处理失败")
        else:
            click.echo("⏰ 等待超时")


@cli.command()
@click.argument("agent")
@click.argument("body", default="")
@click.option("--no-enter", is_flag=True, help="不自动发送 Enter")
@click.option("--msg-id", default=None, help="关联的消息 ID，使用标准提醒格式")
def notify(agent, body, no_enter, msg_id):
    """向指定 Agent 的 pane 发送提醒通知（不写 inbox）。"""
    body = _unescape_body(body)
    agent_info = registry.lookup(agent)
    if not agent_info:
        click.echo(f"❌ Agent {agent} 未找到或已离线", err=True)
        return

    try:
        from_agent = get_agent_name()
    except EnvironmentError as e:
        click.echo(f"❌ {e}", err=True)
        sys.exit(1)

    # 若关联了 msg-id，检查消息是否已处理
    if msg_id:
        with db.get_conn() as conn:
            row = conn.execute(
                "SELECT status FROM messages WHERE msg_id LIKE ?",
                (msg_id + "%",)
            ).fetchone()
        if row and row["status"] in ("done", "failed", "read"):
            click.echo(f"⏭ 消息已 {row['status']}，无需提醒")
            return

    custom_text = body if body else None
    notify_agent(agent, from_agent, agent_info, send_enter=not no_enter, custom_text=custom_text, msg_id=msg_id)
    click.echo(f"🔔 已提醒 {agent}")


@cli.command()
@click.argument("body")
@click.option("--exclude", default="", help="排除的 agent，逗号分隔")
@click.option("--notify", is_flag=True, help="发送后自动提醒对方查收")
@click.option("--no-enter", is_flag=True, help="提醒不自动发送 Enter")
def broadcast(body, exclude, notify, no_enter):
    """广播给所有 Agent（仅写入 inbox）"""
    body = _unescape_body(body)
    excludes = [e.strip() for e in exclude.split(",") if e.strip()]
    try:
        ids = messenger.broadcast(body, exclude=excludes)
    except EnvironmentError as e:
        click.echo(f"❌ {e}", err=True)
        sys.exit(1)
    click.echo(f"📢 广播完成，共 {len(ids)} 条消息")

    if notify:
        try:
            from_agent = get_agent_name()
        except Exception:
            from_agent = "unknown"
        for mid in ids:
            with db.get_conn() as conn:
                row = conn.execute(
                    "SELECT to_agent FROM messages WHERE msg_id = ?", (mid,)
                ).fetchone()
            if row:
                to_agent = row["to_agent"]
                agent_info = registry.lookup(to_agent)
                if agent_info:
                    try:
                        notify_agent(to_agent, from_agent, agent_info, send_enter=not no_enter, msg_id=mid)
                    except Exception:
                        pass


@cli.command()
@click.argument("agents")
@click.argument("body")
@click.option("--notify", is_flag=True, help="发送后自动提醒对方查收")
@click.option("--no-enter", is_flag=True, help="提醒不自动发送 Enter")
def multicast(agents, body, notify, no_enter):
    """多播给指定 Agents (逗号分隔，仅写入 inbox)"""
    body = _unescape_body(body)
    agent_list = [a.strip() for a in agents.split(",") if a.strip()]
    if not agent_list:
        click.echo("❌ 未指定 agents", err=True)
        return
    try:
        from_agent = get_agent_name()
    except EnvironmentError as e:
        click.echo(f"❌ {e}", err=True)
        sys.exit(1)

    sent = {}
    for agent in agent_list:
        try:
            mid = messenger.send(to_agent=agent, body=body)
            sent[agent] = mid
            click.echo(f"✅ {agent}: {mid[:8]}...")
        except Exception as e:
            click.echo(f"❌ {agent}: {e}", err=True)

    if notify:
        for agent, mid in sent.items():
            agent_info = registry.lookup(agent)
            if agent_info:
                try:
                    notify_agent(agent, from_agent, agent_info, send_enter=not no_enter, msg_id=mid)
                except Exception:
                    pass


@cli.command("key-enter")
@click.argument("agent_name")
def key_enter(agent_name):
    """向 agent pane 发送 Enter 键（配合 notify --no-enter 使用）"""
    info = registry.lookup(agent_name)
    if not info:
        click.echo(f"❌ Agent {agent_name} 未找到", err=True)
        return
    mux = get()
    mux.send_keys(info["session"], info["pane_id"], "Enter")
    click.echo(f"✅ 已向 {agent_name} 发送 Enter")


# ─── 收消息 ──────────────────────────────────────────
@cli.command()
@click.argument("agent_name", default="")
@click.option("--all", "show_all", is_flag=True, help="包含已读消息")
@click.option("--json", "as_json", is_flag=True)
def inbox(agent_name, show_all, as_json):
    """查看 inbox"""
    if not agent_name:
        click.echo("❌ 请指定 agent 名字: agtalk inbox <your_name>", err=True)
        return

    status = "pending,delivered,read,done,failed" if show_all else "pending,delivered"
    messages = messenger.inbox(agent_name, status=status)

    if as_json:
        click.echo(json.dumps(messages, indent=2))
    else:
        click.echo(f"📬 {agent_name} 的收件箱\n")
        if not messages:
            click.echo("  (empty)")
            return
        for m in messages:
            click.echo(f"[{m['msg_id'][:8]}] {m['from_agent']} → {m['status']:>10}")
            click.echo(f"  {m['body'][:60]}{'...' if len(m['body']) > 60 else ''}")
            click.echo()


@cli.command()
@click.argument("msg_id")
def done(msg_id):
    """标记消息为已完成 (done)"""
    try:
        agent_name = get_agent_name()
    except EnvironmentError as e:
        click.echo(f"❌ {e}", err=True)
        sys.exit(1)

    messenger.mark_done(msg_id, agent_name)
    _fifo_notify()
    click.echo(f"✅ 消息 {msg_id[:8]}... 已标记完成")


# ─── 工具 ──────────────────────────────────────────
@cli.command()
@click.option("--dry-run", is_flag=True)
def prune(dry_run):
    """清理僵尸 Agent"""
    dead = registry.prune(dry_run)
    if dead:
        verb = "将清理" if dry_run else "已清理"
        click.echo(f"🧹 {verb}: {', '.join(dead)}")
    else:
        click.echo("✅ 无需清理")


@cli.command()
def check_stuck():
    """检查并标记超时的 delivered 消息为 failed"""
    count = messenger.check_stuck_messages()
    click.echo(f"🚨 标记了 {count} 条卡死消息为 failed")


@cli.command()
@click.option("--agent", default="", help="指定 agent")
@click.option("--last", default=20, type=int)
@click.option("--json", "as_json", is_flag=True)
def memory(agent, last, as_json):
    """查询消息历史"""
    with db.get_conn() as conn:
        if agent:
            rows = conn.execute("""
                SELECT msg_id, event, agent, session, note, created_at
                FROM message_log
                WHERE agent = ?
                ORDER BY created_at DESC
                LIMIT ?
            """, (agent, last)).fetchall()
        else:
            rows = conn.execute("""
                SELECT msg_id, event, agent, session, note, created_at
                FROM message_log
                ORDER BY created_at DESC
                LIMIT ?
            """, (last,)).fetchall()

    if as_json:
        click.echo(json.dumps([dict(r) for r in rows], indent=2))
    else:
        if not rows:
            click.echo("📭 无消息历史")
            return
        click.echo(f"📜 消息历史 (最近 {len(rows)} 条)\n")
        for r in rows:
            click.echo(f"[{r['created_at']:.2f}] {r['event']:>10} {r['agent']} ({r['session']})")
            if r['note']:
                click.echo(f"  {r['note']}")


@cli.command()
def whoami():
    """显示当前 agent 信息"""
    try:
        name = get_agent_name()
    except EnvironmentError as e:
        click.echo(f"❌ {e}")
        return
    
    info = registry.lookup(name)
    if not info:
        click.echo(f"Agent: {name}\n状态: 未注册")
        return
    
    click.echo(f"Agent: {name}")
    click.echo(f"Role: {info.get('role', '')}")
    click.echo(f"Bio: {info.get('bio', '')}")
    click.echo(f"Capabilities: {info.get('capabilities', '')}")
    click.echo(f"Session: {info['session']} | Pane: {info['pane_id']}")


@cli.command()
@click.argument("agent_name", default="")
def health(agent_name):
    """健康检查"""
    checks = []

    # 1. DB 可写
    try:
        with db.get_conn() as conn:
            conn.execute("INSERT INTO message_log (msg_id, event, agent, session) VALUES ('test', 'health_check', 'system', '')")
            conn.execute("DELETE FROM message_log WHERE msg_id='test'")
        checks.append(("DB 可写", True))
    except Exception as e:
        checks.append(("DB 可写", False, str(e)))

    # 2. 终端 CLI
    import shutil
    if shutil.which("zellij"):
        checks.append(("zellij CLI", True))
    else:
        checks.append(("zellij CLI", False, "未找到"))

    # 3. FIFO
    from .delivery import FIFO_PATH
    checks.append(("FIFO 存在", FIFO_PATH.exists()))

    # 4. Agent
    if agent_name:
        info = registry.lookup(agent_name)
        checks.append(("Agent 注册", bool(info)))
        if info and info.get("pid"):
            alive = registry._proc_alive(info["pid"])
            checks.append(("进程存活", alive))
        elif info:
            checks.append(("进程存活", True))
    else:
        checks.append(("Agent 检查", "跳过（未指定）"))

    click.echo("🏥 健康检查结果:\n")
    for check in checks:
        if len(check) == 2:
            name, ok = check
            status = "✅" if ok else "❌"
            click.echo(f"  {status} {name}")
        else:
            name, ok, detail = check
            status = "✅" if ok else "❌"
            click.echo(f"  {status} {name}: {detail}")

    failed = sum(1 for c in checks if len(c) == 2 and not c[1]) + sum(1 for c in checks if len(c) == 3 and not c[1])
    total = len([c for c in checks if len(c) == 2 or (len(c) == 3 and c[1] is not None)])
    click.echo(f"\n健康分数: {total - failed}/{total}")


# ─── 内部函数 ──────────────────────────────────────
def _write_skill_prompt(agent_name: str, session: str, pane_id: int):
    """向当前 pane write-chars 发送 SKILL 加载提示"""
    skill_content = f"""[ZAT-SKILL] zellij-agent-talk 已激活
你的 Agent 名: {agent_name}
Session: {session} | Pane: {pane_id}

通信标准:
  查看 inbox  : agtalk inbox <your_name>
  标记完成    : agtalk done <msg_id>        (标记完成, 不打扰对方)
  发消息      : agtalk send <agent> "<内容>"  (仅写入 inbox)
  提醒对方    : agtalk notify <agent> ["<精简内容>"]  (仅发 pane 提醒)
  列出 agents  : agtalk list

消息格式前缀: [TASK] [REPLY] [DONE] [ACK] [INFO] [FILE]
收到 [TASK] 消息时, 完成后请执行: agtalk done <msg_id>
"""
    mux = get()
    mux.write_chars_to_pane(session, pane_id, skill_content, send_enter=False)
    export_cmd = f"export AGTALK_AGENT_NAME={agent_name}"
    mux.write_chars_to_pane(session, pane_id, export_cmd, send_enter=True)


def main():
    cli()
