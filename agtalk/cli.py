# agtalk/cli.py — Typer + Rich 命令实现
import os
import sys
import json
from datetime import datetime
from collections import defaultdict
from typing import Optional

from rich.live import Live
from rich.text import Text
from rich.panel import Panel
from rich import box

import typer

from agtalk.main import app
from agtalk.console import console
from agtalk import db, registry, messenger
from agtalk.factory import get, get_agent_name, detect_name, get_by_name
from agtalk.delivery import notify as _fifo_notify, watch_until_done, _ensure_fifo, notify_agent

_PRESET_NAMES = [
    "Alex", "Bob", "Chris", "David", "Emma", "Frank", "Grace",
    "Henry", "Iris", "Jack", "Kate", "Liam", "Mary", "Nick",
    "Olivia", "Paul", "Quinn", "Rose", "Sam", "Tom", "Uma",
    "Victor", "Wendy", "Xin", "Yale", "Zoe",
]


def _unescape_body(body: str) -> str:
    """将字面转义字符替换为真实控制字符。"""
    body = body.replace("\\\\", "\x00")
    body = body.replace("\\n", "\n")
    body = body.replace("\\t", "\t")
    body = body.replace("\\r", "\r")
    body = body.replace("\x00", "\\")
    return body


def _fmt_time(ts: float) -> str:
    """时间戳 → 人类可读时间。"""
    return datetime.fromtimestamp(ts).strftime("%H:%M:%S")


def _fmt_ts_full(ts: float) -> str:
    return datetime.fromtimestamp(ts).strftime("%m-%d %H:%M:%S")


def _status_style(status: str) -> tuple[str, str]:
    """返回 (label, rich color)。"""
    return {
        "done":        ("DONE", "green"),
        "sent":        ("SEND", "blue"),
        "pending":     ("WAIT", "yellow"),
        "delivered":   ("RECV", "cyan"),
        "read":        ("READ", "white"),
        "failed":      ("FAIL", "red"),
        "progress":    ("PROGRESS", "yellow"),
        "open":        ("OPEN", "yellow"),
        "in_progress": ("WORK", "blue"),
        "resolved":    ("DONE", "green"),
        "closed":      ("CLOSE", "dim"),
    }.get(status, ("UNKN", "white"))


_prefix_colors = {
    "[TASK]":     "green",
    "[REPLY]":    "cyan",
    "[DONE]":     "green",
    "[ACK]":      "blue",
    "[INFO]":     "white",
    "[FILE]":     "magenta",
    "[QUESTION]": "yellow",
    "[REMINDER]": "yellow",
    "[ISSUE]":    "red",
}


def _colorize_prefix(text: str) -> str:
    """为消息前缀标注颜色。"""
    for prefix, color in _prefix_colors.items():
        if text.startswith(prefix):
            return f"[{color}]{prefix}[/{color}]{text[len(prefix):]}"
    return text


def _short_id() -> str:
    """生成 8 位短 ID。"""
    import uuid
    return uuid.uuid4().hex[:8]


def _j(data: dict | list) -> None:
    """输出 JSON（Agent 默认格式），时间戳自动转 ISO 8601，msg_id 自动截短。"""
    from datetime import datetime, timezone

    def _normalize(obj):
        if isinstance(obj, dict):
            for k, v in list(obj.items()):
                if k in ("created_at", "last_seen_at", "registered_at", "updated_at") and isinstance(v, (int, float)):
                    obj[k] = datetime.fromtimestamp(v, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
                elif k == "msg_ids" and isinstance(v, list):
                    obj[k] = [item[:8] if isinstance(item, str) and len(item) > 8 else item for item in v]
                elif k == "sent" and isinstance(v, dict):
                    for sk, sv in list(v.items()):
                        if isinstance(sv, str) and len(sv) > 8:
                            v[sk] = sv[:8]
                elif k in ("msg_id", "reply_to_msg_id") and isinstance(v, str) and len(v) > 8:
                    obj[k] = v[:8]
                else:
                    _normalize(v)
        elif isinstance(obj, list):
            for item in obj:
                _normalize(item)

    _normalize(data)
    console.print(json.dumps(data, indent=2, ensure_ascii=False, default=str))


# ─── 初始化 ──────────────────────────────────────────
@app.command()
def init(
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """初始化环境：检查终端环境、初始化数据库、确保 FIFO 存在。"""
    errors = []
    warnings = []
    results = {}

    with console.status("[bold green]正在初始化...", spinner="dots"):
        # 检测 multiplexer（非 multiplexer 环境下降级为警告）
        try:
            mux = get()
            session = mux.get_current_session()
            pane_id = mux.get_current_pane_id()
            results["session"] = session
            results["pane_id"] = pane_id
            results["mux"] = detect_name()
        except EnvironmentError as e:
            warnings.append(f"终端环境: {e}")
            results["mux"] = "unknown"

        import shutil
        has_zellij = bool(shutil.which("zellij"))
        has_tmux = bool(shutil.which("tmux"))
        results["zellij"] = has_zellij
        results["tmux"] = has_tmux
        if not has_zellij and not has_tmux:
            warnings.append("未找到 zellij 或 tmux CLI")

        try:
            db.init_db()
            results["db"] = str(db.get_db_path())
        except Exception as e:
            errors.append(f"数据库初始化: {e}")

        try:
            _ensure_fifo()
            from agtalk.delivery import FIFO_PATH
            results["fifo"] = str(FIFO_PATH)
        except Exception as e:
            errors.append(f"FIFO 创建: {e}")

    if not view:
        if errors:
            _j({"ok": False, "errors": errors, "warnings": warnings, "results": results})
            sys.exit(1)
        else:
            _j({"ok": True, "warnings": warnings, **results})
            return

    if errors:
        for err in errors:
            console.print(f"[red]❌ {err}[/red]")
        sys.exit(1)

    if warnings:
        for w in warnings:
            console.print(f"[yellow]⚠ {w}[/yellow]")

    console.print(f"✅ 数据库: [dim]{results.get('db')}[/dim]")
    console.print(f"✅ FIFO: [dim]{results.get('fifo')}[/dim]")
    if results.get("session"):
        console.print(f"✅ 终端环境: session=[cyan]{results['session']}[/cyan], pane_id=[cyan]{results.get('pane_id')}[/cyan], mux=[cyan]{results['mux']}[/cyan]")
    console.print("\n✅ 初始化完成")


# ─── 注册 ────────────────────────────────────────────
@app.command()
def register(
    agent_name: str,
    role: str = "",
    capabilities: str = "",
    bio: str = "",
    workdir: str = typer.Option("", "--workdir", "-w", help="工作目录（默认当前目录）"),
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """注册当前 pane 为 Agent。"""
    if not workdir:
        workdir = os.getcwd()
    mux = detect_name()
    try:
        info = registry.register(agent_name, role, capabilities, bio, workdir, mux)
    except EnvironmentError as e:
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)
    os.environ["AGTALK_AGENT_NAME"] = agent_name

    if not view:
        with db.get_conn() as conn:
            row = conn.execute(
                "SELECT * FROM agents WHERE agent_name = ?", (agent_name,)
            ).fetchone()
        agent_info = dict(row) if row else {}
        _j({"ok": True, **agent_info})
        return

    console.print(f"\n✅ 注册成功: {agent_name}")
    console.print(f"  Session: {info['session']} | Pane: {info['pane_id']}")
    console.print(f"  Mux:     {mux}")
    if role:
        console.print(f"  Role: {role}")
    if capabilities:
        console.print(f"  Capabilities: {capabilities}")
    if bio:
        console.print(f"  Bio: {bio}")
    console.print(f"  Workdir: {workdir}")


@app.command()
def unregister(
    agent_name: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """注销 Agent。"""
    registry.unregister(agent_name)
    if not view:
        _j({"ok": True, "agent_name": agent_name})
        return
    console.print(f"[yellow]🗑 已注销:[/yellow] {agent_name}")


@app.command("list")
def list_agents(
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
    capabilities: bool = False,
):
    """列出所有注册 Agent。"""
    with db.get_conn() as conn:
        rows = conn.execute("SELECT * FROM agents").fetchall()

    if not view:
        _j([dict(r) for r in rows])
        return

    if not rows:
        console.print("暂无注册的 Agent")
        return

    # 批量验活（非多路复用器环境跳过）
    alive_pane_ids: set = set()
    try:
        mux = get()
        session_panes = {}
        for r in rows:
            session = r["session"]
            if session not in session_panes:
                try:
                    session_panes[session] = mux.list_panes(session)
                except Exception:
                    session_panes[session] = []
        alive_pane_ids = {
            p["id"]
            for panes in session_panes.values()
            for p in panes
            if not p.get("exited", False)
        }
        can_detect_mux = True
    except EnvironmentError:
        can_detect_mux = False

    home = os.path.expanduser("~")
    cards = []
    for r in rows:
        if can_detect_mux:
            is_alive = r["pane_id"] in alive_pane_ids
            border_color = "green" if is_alive else "dim"
            status_dot = "●" if is_alive else "○"
        else:
            border_color = "yellow"
            status_dot = "?"


        wd = r["workdir"] or "-"
        if wd.startswith(home + "/"):
            wd = "~" + wd[len(home):]
        elif wd == home:
            wd = "~"

        content = Text()
        content.append(f"角色: {r['role'] or '-'}\n")
        if r["capabilities"]:
            cap = r["capabilities"]
            if len(cap) > 22:
                cap = cap[:19] + "..."
            content.append(f"能力: {cap}\n")
        if r["bio"]:
            bio = r["bio"]
            if len(bio) > 22:
                bio = bio[:19] + "..."
            content.append(f"简介: {bio}\n")
        content.append(f"目录: {wd}\n")
        content.append(f"终端: {r['session']}:{r['pane_id']}")
        if r["mux"]:
            content.append(f" [{r['mux']}]", style="dim")

        title = Text()
        title.append(f"{status_dot} ", style=border_color)
        title.append(r["agent_name"], style="bold")

        panel = Panel(
            content,
            title=title,
            title_align="left",
            border_style=border_color,
            box=box.ROUNDED,
            padding=(0, 0),
        )
        cards.append(panel)

    console.print()
    card_width = max(40, int(console.width * 0.6))
    for card in cards:
        card.width = card_width
        console.print(card)
    console.print()


# ─── 发消息 ──────────────────────────────────────────
@app.command()
def send(
    agent: str = typer.Argument("", help="目标 Agent 名称（--file 模式下可省略）"),
    body: str = typer.Argument("", help="消息内容（--file 模式下可省略）"),
    subject: str = "",
    msg_type: str = "text",
    priority: int = 5,
    wait_done: bool = False,
    timeout: int = 120,
    reply_to: Optional[str] = None,
    notify: bool = True,
    no_notify: bool = typer.Option(False, "--no-notify", help="不发送 pane 提醒"),
    no_enter: bool = False,
    file: Optional[str] = typer.Option(None, "--file", "-f", help="从 JSON 文件读取消息配置（- 表示从 stdin 读取）"),
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """发送消息给指定 Agent（写入 inbox + 自动提醒）。支持 \\n 转义换行。"""
    if file:
        if file == "-":
            cfg = json.load(sys.stdin)
        else:
            with open(file, "r", encoding="utf-8") as f:
                cfg = json.load(f)
        agent = agent or cfg.get("agent", "")
        body = body or cfg.get("body", "")
        subject = subject or cfg.get("subject", "")
        msg_type = cfg.get("msg_type", msg_type)
        priority = cfg.get("priority", priority)
        wait_done = cfg.get("wait_done", wait_done)
        timeout = cfg.get("timeout", timeout)
        reply_to = reply_to or cfg.get("reply_to")
        if "notify" in cfg:
            notify = cfg["notify"]
        if "no_enter" in cfg:
            no_enter = cfg["no_enter"]

    if not agent:
        if not view:
            _j({"ok": False, "error": "请指定目标 Agent 名称，或使用 --file 提供配置"})
            sys.exit(1)
        console.print("[red]❌ 请指定目标 Agent 名称，或使用 --file 提供配置[/red]")
        sys.exit(1)
    if not body:
        if not view:
            _j({"ok": False, "error": "消息内容不能为空，或使用 --file 提供配置"})
            sys.exit(1)
        console.print("[red]❌ 消息内容不能为空，或使用 --file 提供配置[/red]")
        sys.exit(1)

    if body == "-":
        body = sys.stdin.read()
        if body.endswith("\n"):
            body = body[:-1]

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
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    preview = body[:50] + ("..." if len(body) > 50 else "")

    # 执行 notify（JSON 和 view 模式都需要）
    notified = False
    notify_error = ""
    if notify and not no_notify:
        agent_info = registry.lookup(agent)
        if agent_info:
            try:
                from_agent = get_agent_name()
            except Exception:
                from_agent = "unknown"
            try:
                notify_agent(agent, from_agent, agent_info, send_enter=not no_enter, msg_id=msg_id)
                notified = True
            except Exception as e:
                notify_error = str(e)

    if not view:
        result = {"ok": True, "msg_id": msg_id, "to_agent": agent, "preview": preview}
        if notified:
            result["notified"] = True
        if notify_error:
            result["notify_error"] = notify_error
        if wait_done:
            status = watch_until_done(msg_id, timeout)
            result["status"] = status
        _j(result)
        return

    console.print(
        f"[blue]📨[/blue] [dim]{msg_id[:8]}...[/dim] → [cyan]{agent}[/cyan]  [dim]{preview}[/dim]"
    )
    if notified:
        console.print(f"[yellow]🔔 已提醒[/yellow] {agent} 查收")
    elif notify_error:
        console.print(f"[red]🔔 提醒发送失败:[/red] {notify_error}")

    if wait_done:
        with console.status(f"[yellow]等待 {agent} 完成... (超时 {timeout}s)[/yellow]", spinner="dots"):
            status = watch_until_done(msg_id, timeout)
        if status == "done":
            console.print("[green]✅ 对方已标记完成[/green]")
        elif status == "failed":
            console.print("[red]❌ 消息处理失败[/red]")
        else:
            console.print("[yellow]⏰ 等待超时[/yellow]")


@app.command()
def notify(
    agent: str,
    body: str = "",
    no_enter: bool = False,
    msg_id: Optional[str] = None,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """向指定 Agent 的 pane 发送提醒通知（不写 inbox）。"""
    body = _unescape_body(body)
    agent_info = registry.lookup(agent)
    if not agent_info:
        if not view:
            _j({"ok": False, "error": f"Agent {agent} 未找到或已离线"})
            sys.exit(1)
        console.print(f"[red]❌ Agent {agent} 未找到或已离线[/red]")
        return
    try:
        from_agent = get_agent_name()
    except EnvironmentError as e:
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    if msg_id:
        with db.get_conn() as conn:
            row = conn.execute(
                "SELECT status FROM messages WHERE msg_id LIKE ?",
                (msg_id + "%",),
            ).fetchone()
        if row and row["status"] in ("done", "failed", "read"):
            if not view:
                _j({"ok": True, "skipped": True, "reason": f"消息已 {row['status']}"})
                return
            console.print(f"[dim]⏭ 消息已 {row['status']}，无需提醒[/dim]")
            return

    custom_text = body if body else None
    notify_agent(agent, from_agent, agent_info, send_enter=not no_enter,
                 custom_text=custom_text, msg_id=msg_id)
    if not view:
        _j({"ok": True, "agent": agent, "notified": True})
        return
    console.print(f"[yellow]🔔 已提醒[/yellow] {agent}")


@app.command()
def broadcast(
    body: str,
    exclude: str = "",
    notify: bool = True,
    no_notify: bool = typer.Option(False, "--no-notify", help="不发送 pane 提醒"),
    no_enter: bool = False,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """广播给所有 Agent（写入 inbox + 自动提醒）。"""
    body = _unescape_body(body)
    excludes = [e.strip() for e in exclude.split(",") if e.strip()]
    try:
        ids = messenger.broadcast(body, exclude=excludes)
    except EnvironmentError as e:
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    # 执行 notify（JSON 和 view 模式都需要）
    notified_count = 0
    if notify and not no_notify:
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
                        notify_agent(to_agent, from_agent, agent_info,
                                     send_enter=not no_enter, msg_id=mid)
                        notified_count += 1
                    except Exception:
                        pass

    if not view:
        result = {"ok": True, "count": len(ids), "msg_ids": ids}
        if notified_count:
            result["notified_count"] = notified_count
        _j(result)
        return
    console.print(f"[blue]📢 广播完成[/blue]，共 [bold]{len(ids)}[/bold] 条消息")
    if notified_count:
        console.print(f"[yellow]🔔 已提醒 {notified_count} 个 Agent[/yellow]")


@app.command()
def multicast(
    agents: str,
    body: str,
    notify: bool = True,
    no_notify: bool = typer.Option(False, "--no-notify", help="不发送 pane 提醒"),
    no_enter: bool = False,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """多播给指定 Agents (逗号分隔，写入 inbox + 自动提醒)。"""
    body = _unescape_body(body)
    agent_list = [a.strip() for a in agents.split(",") if a.strip()]
    if not agent_list:
        if not view:
            _j({"ok": False, "error": "未指定 agents"})
            sys.exit(1)
        console.print("[red]❌ 未指定 agents[/red]")
        return
    try:
        from_agent = get_agent_name()
    except EnvironmentError as e:
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    sent = {}
    failed = {}
    for agent in agent_list:
        try:
            mid = messenger.send(to_agent=agent, body=body)
            sent[agent] = mid
        except Exception as e:
            failed[agent] = str(e)

    # 执行 notify（JSON 和 view 模式都需要）
    notified_count = 0
    if notify and not no_notify:
        for agent, mid in sent.items():
            agent_info = registry.lookup(agent)
            if agent_info:
                try:
                    notify_agent(agent, from_agent, agent_info,
                                 send_enter=not no_enter, msg_id=mid)
                    notified_count += 1
                except Exception:
                    pass

    if not view:
        result = {"ok": True, "sent": sent, "failed": failed}
        if notified_count:
            result["notified_count"] = notified_count
        _j(result)
        return

    for agent, mid in sent.items():
        console.print(f"  [green]✅[/green] [cyan]{agent}[/cyan]: {mid[:8]}...")
    for agent, err in failed.items():
        console.print(f"  [red]❌[/red] [cyan]{agent}[/cyan]: {err}")
    if notified_count:
        console.print(f"[yellow]🔔 已提醒 {notified_count} 个 Agent[/yellow]")


@app.command("key-enter")
def key_enter(
    agent_name: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """向 agent pane 发送 Enter 键。"""
    info = registry.lookup(agent_name)
    if not info:
        if not view:
            _j({"ok": False, "error": f"Agent {agent_name} 未找到"})
            sys.exit(1)
        console.print(f"[red]❌ Agent {agent_name} 未找到[/red]")
        return
    if info.get("mux") not in ("zellij", "tmux"):
        if not view:
            _j({"ok": False, "error": f"Agent {agent_name} 不是终端 Agent，无法发送 Enter"})
            sys.exit(1)
        console.print(f"[yellow]⚠ Agent {agent_name} 不是终端 Agent，无法发送 Enter[/yellow]")
        return
    mux = get_by_name(info["mux"])
    mux.send_keys(info["session"], info["pane_id"], "Enter")
    if not view:
        _j({"ok": True, "agent": agent_name})
        return
    console.print(f"[green]✅ 已向[/green] {agent_name} 发送 Enter")


# ─── 收消息 ──────────────────────────────────────────
@app.command()
def inbox(
    agent_name: str,
    show_all: bool = typer.Option(False, "--all", "--show-all", help="包含已读消息"),
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """查看 inbox。"""
    if not agent_name:
        console.print("[red]❌ 请指定 agent 名字: agtalk inbox <your_name>[/red]")
        return

    status = "pending,delivered,read,done,failed" if show_all else "pending,delivered"
    messages = messenger.inbox(agent_name, status=status)

    if not view:
        _j(messages)
        return

    prefix_style = _prefix_colors

    console.print(f"\n📬 {agent_name} 的收件箱 ({len(messages)} 条)\n")
    if not messages:
        console.print("  (empty)")
        return

    console.print(
        "  提示：处理完 [TASK] 后用 agtalk done <msg_id> 标记完成；\n"
        '        需要回复时用 agtalk send <from_agent> "[REPLY] ..."',
        style="dim",
    )

    for m in messages:
        emoji, _color = _status_style(m["status"])
        body_preview = m["body"].replace("\n", " ")[:40]
        if len(m["body"]) > 40:
            body_preview += "..."
        line = f"  [dim]{m['msg_id'][:8]}[/dim] {emoji} {m['from_agent']:<25} | {_colorize_prefix(body_preview)}"
        console.print(line, no_wrap=True, overflow="ellipsis")


@app.command()
def done(
    msg_id: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """标记消息为已完成 (done)。"""
    try:
        agent_name = get_agent_name()
    except EnvironmentError as e:
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)
    messenger.mark_done(msg_id, agent_name)
    _fifo_notify()
    if not view:
        _j({"ok": True, "msg_id": msg_id, "status": "done"})
        return
    console.print(
        f"[green]✅ 消息[/green] [dim]{msg_id[:8]}...[/dim] [green]已标记完成[/green]"
    )


# ─── 进度管理 ──────────────────────────────────────────
@app.command()
def progress(
    msg_id: str,
    percent: int,
    note: str = "",
    watch: bool = False,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """更新任务进度（0-100），或用 --watch 实时监控所有任务进度。"""
    if watch:
        _watch_all_progress()
        return

    if not (0 <= percent <= 100):
        if not view:
            _j({"ok": False, "error": "进度必须在 0-100 之间"})
            return
        console.print("[red]❌ 进度必须在 0-100 之间[/red]")
        return

    with db.get_conn() as conn:
        conn.execute("""
            CREATE TABLE IF NOT EXISTS task_progress (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                msg_id      TEXT NOT NULL,
                percent     INTEGER NOT NULL,
                note        TEXT DEFAULT '',
                created_at  REAL NOT NULL DEFAULT (unixepoch('now','subsec'))
            )
        """)
        row = conn.execute(
            "SELECT msg_id FROM messages WHERE msg_id LIKE ?",
            (msg_id + "%",),
        ).fetchone()
        full_msg_id = row["msg_id"] if row else msg_id
        conn.execute(
            "INSERT INTO task_progress (msg_id, percent, note) VALUES (?, ?, ?)",
            (full_msg_id, percent, note),
        )

    if not view:
        _j({"ok": True, "msg_id": msg_id, "percent": percent, "note": note})
        return

    bar_filled = int(percent / 5)
    bar = "█" * bar_filled + "░" * (20 - bar_filled)
    color = "green" if percent == 100 else "yellow" if percent >= 50 else "cyan"

    console.print(
        f"[dim]{msg_id[:8]}[/dim]  [{color}]{bar}[/{color}]  "
        f"[bold {color}]{percent:3d}%[/bold {color}]"
        + (f"  [dim]{note}[/dim]" if note else "")
    )


@app.command("progress-list")
def progress_list(
    watch: bool = False,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """查看所有任务的进度总览。"""
    if watch:
        import time
        console.print("[dim]按 Ctrl+C 退出监控[/dim]")
        try:
            with Live(console=console, refresh_per_second=2) as live:
                while True:
                    live.update(_build_progress_table())
                    time.sleep(1)
        except KeyboardInterrupt:
            console.print("\n[dim]已退出监控[/dim]")
        return

    if not view:
        with db.get_conn() as conn:
            exists = conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='task_progress'"
            ).fetchone()
            if not exists:
                _j({"ok": True, "tasks": []})
                return
            rows = conn.execute("""
                SELECT tp.msg_id, tp.percent, tp.note, tp.created_at,
                       m.from_agent, m.to_agent, m.body, m.status
                FROM task_progress tp
                LEFT JOIN messages m ON m.msg_id = tp.msg_id
                WHERE tp.id IN (
                    SELECT MAX(id) FROM task_progress GROUP BY msg_id
                )
                ORDER BY tp.created_at DESC
            """).fetchall()
        tasks = [dict(r) for r in rows]
        _j({"ok": True, "tasks": tasks})
        return

    console.print(_build_progress_table())


def _build_progress_table():
    """构建进度总览 Table（供 Live 刷新使用）。"""
    with db.get_conn() as conn:
        exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='task_progress'"
        ).fetchone()
        if not exists:
            return "暂无进度数据，请先使用 agtalk progress <msg_id> <percent>"

        rows = conn.execute("""
            SELECT tp.msg_id, tp.percent, tp.note, tp.created_at,
                   m.from_agent, m.to_agent, m.body, m.status
            FROM task_progress tp
            LEFT JOIN messages m ON m.msg_id = tp.msg_id
            WHERE tp.id IN (
                SELECT MAX(id) FROM task_progress GROUP BY msg_id
            )
            ORDER BY tp.created_at DESC
        """).fetchall()

    if not rows:
        return "暂无进度记录"

    lines = ["📋 任务进度\n"]
    for r in rows:
        pct = r["percent"]
        bar_filled = int(pct / 5)
        bar = "█" * bar_filled + "░" * (20 - bar_filled)
        from_to = f"{r['from_agent'] or '?'} → {r['to_agent'] or '?'}"
        body_preview = (r["body"] or "").replace("\n", " ")[:30]
        if r["body"] and len(r["body"]) > 30:
            body_preview += "..."
        note = r["note"] or body_preview or "—"
        lines.append(
            f"  [dim]{r['msg_id'][:8]}[/dim]  {bar}  {pct:3d}%  {from_to:<30} | {_colorize_prefix(note)}"
        )

    return "\n".join(lines)


def _watch_all_progress():
    """--watch 模式：Live 实时刷新进度面板。"""
    import time
    console.print("[dim]按 Ctrl+C 退出监控[/dim]")
    try:
        with Live(console=console, refresh_per_second=2) as live:
            while True:
                live.update(_build_progress_table())
                time.sleep(1)
    except KeyboardInterrupt:
        console.print("\n[dim]已退出监控[/dim]")


# ─── 工具命令 ──────────────────────────────────────────
@app.command()
def prune(
    dry_run: bool = False,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """清理僵尸 Agent。"""
    dead = registry.prune(dry_run)
    if not view:
        _j({"ok": True, "pruned": dead, "dry_run": dry_run})
        return
    if dead:
        verb = "将清理" if dry_run else "已清理"
        console.print(f"[yellow]🧹 {verb}:[/yellow] {', '.join(dead)}")
    else:
        console.print("[green]✅ 无需清理[/green]")


@app.command("check-stuck")
def check_stuck(
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """检查并标记超时的 delivered 消息为 failed。"""
    count = messenger.check_stuck_messages()
    if not view:
        _j({"ok": True, "count": count})
        return
    console.print(f"[yellow]🚨 标记了[/yellow] [bold]{count}[/bold] 条卡死消息为 failed")


@app.command()
def memory(
    agent: str = typer.Option("", "--agent", help="指定 Agent 过滤"),
    last: int = typer.Option(20, "--last", help="最近 N 条"),
    msg_id: str = typer.Argument("", help="指定 msg_id 查看详情"),
    task_view: bool = typer.Option(False, "--group", help="按 msg_id 分组显示"),
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """查询消息历史（支持 --task 任务视角，支持指定 msg_id）。"""
    with db.get_conn() as conn:
        if msg_id:
            # 支持短 ID 前缀匹配
            full_id = conn.execute(
                "SELECT msg_id FROM messages WHERE msg_id LIKE ? LIMIT 1",
                (msg_id + "%",),
            ).fetchone()
            target_id = full_id["msg_id"] if full_id else msg_id
            rows = conn.execute("""
                SELECT l.msg_id, l.event, l.agent, l.session, l.note, l.created_at,
                       m.from_agent, m.to_agent, m.body
                FROM message_log l
                LEFT JOIN messages m ON m.msg_id = l.msg_id
                WHERE l.msg_id = ?
                ORDER BY l.created_at ASC
            """, (target_id,)).fetchall()
        elif agent:
            rows = conn.execute("""
                SELECT l.msg_id, l.event, l.agent, l.session, l.note, l.created_at,
                       m.from_agent, m.to_agent, m.body
                FROM message_log l
                LEFT JOIN messages m ON m.msg_id = l.msg_id
                WHERE l.agent = ?
                ORDER BY l.created_at DESC LIMIT ?
            """, (agent, last)).fetchall()
        else:
            rows = conn.execute("""
                SELECT l.msg_id, l.event, l.agent, l.session, l.note, l.created_at,
                       m.from_agent, m.to_agent, m.body
                FROM message_log l
                LEFT JOIN messages m ON m.msg_id = l.msg_id
                ORDER BY l.created_at DESC LIMIT ?
            """, (last,)).fetchall()

    if not rows:
        if not view:
            _j([])
        else:
            console.print("[dim]📭 无消息历史[/dim]")
        return

    if task_view or msg_id:
        if not view:
            _render_memory_task_json(rows)
        else:
            _render_memory_task_view(rows)
    else:
        if not view:
            _j([dict(r) for r in rows])
        else:
            _render_memory_timeline(rows)


def _render_memory_timeline(rows):
    """时间线视角：简洁列表。"""
    console.print(f"\n📜 消息历史 (最近 {len(rows)} 条)\n")

    for r in rows:
        emoji, _color = _status_style(r["event"])
        from_to = f"{r['from_agent'] or '?'} → {r['to_agent'] or r['agent']}"
        body_text = (r["body"] or r["note"] or "").replace("\n", " ")[:40]
        if len(r["body"] or "") > 40:
            body_text += "..."
        line = f"  [{_fmt_time(r['created_at'])}] {emoji} {from_to:<30} | {_colorize_prefix(body_text)}"
        console.print(line, no_wrap=True, overflow="ellipsis")


def _render_memory_task_json(rows):
    """任务视角 JSON：按 msg_id 分组，包含 progress。"""
    tasks = defaultdict(list)
    for r in rows:
        tasks[r["msg_id"]].append(dict(r))

    with db.get_conn() as conn:
        for msg_id in tasks:
            prog_rows = conn.execute("""
                SELECT percent, note, unixepoch(created_at) as created_at
                FROM task_progress
                WHERE msg_id = ?
                ORDER BY created_at
            """, (msg_id,)).fetchall()
            for pr in prog_rows:
                tasks[msg_id].append({
                    "event": "progress",
                    "percent": pr["percent"],
                    "note": pr["note"],
                    "created_at": pr["created_at"],
                    "agent": "",
                    "session": "",
                })

    result = []
    for msg_id, events in tasks.items():
        events_sorted = sorted(events, key=lambda x: x["created_at"])
        msg_events = [e for e in events_sorted if e["event"] != "progress"]
        first = msg_events[0] if msg_events else events_sorted[0]
        latest_progress = None
        for e in reversed(events_sorted):
            if e["event"] == "progress":
                latest_progress = {"percent": e["percent"], "note": e["note"]}
                break

        task = {
            "msg_id": msg_id,
            "from_agent": first.get("from_agent"),
            "to_agent": first.get("to_agent"),
            "body": first.get("body"),
            "events": [
                {
                    "event": e["event"],
                    "created_at": e["created_at"],
                    **({"note": e["note"]} if e.get("note") else {}),
                    **({"percent": e["percent"]} if e.get("percent") is not None else {}),
                }
                for e in events_sorted
            ],
        }
        if latest_progress:
            task["latest_progress"] = latest_progress
        result.append(task)

    _j(result)


def _render_memory_task_view(rows):
    """任务视角：按 msg_id 分组，展示任务生命周期。"""
    tasks = defaultdict(list)
    for r in rows:
        tasks[r["msg_id"]].append(dict(r))

    with db.get_conn() as conn:
        for msg_id in tasks:
            prog_rows = conn.execute("""
                SELECT percent, note, unixepoch(created_at) as created_at
                FROM task_progress
                WHERE msg_id = ?
                ORDER BY created_at
            """, (msg_id,)).fetchall()
            for pr in prog_rows:
                tasks[msg_id].append({
                    "event": "progress",
                    "percent": pr["percent"],
                    "note": pr["note"],
                    "created_at": pr["created_at"],
                    "agent": "",
                    "session": "",
                })

    console.print(f"\n📋 任务视图 ({len(tasks)} 个任务)\n")

    for msg_id, events in tasks.items():
        events_sorted = sorted(events, key=lambda x: x["created_at"])
        msg_events = [e for e in events_sorted if e["event"] != "progress"]
        first = msg_events[0] if msg_events else events_sorted[0]
        last_event = events_sorted[-1]

        elapsed = last_event["created_at"] - first["created_at"]
        if elapsed < 60:
            elapsed_str = f"{elapsed:.0f}s"
        elif elapsed < 3600:
            elapsed_str = f"{elapsed/60:.1f}m"
        else:
            elapsed_str = f"{elapsed/3600:.1f}h"

        from_to = f"{first['from_agent'] or '?'} → {first['to_agent'] or '?'}"

        # 树形结构
        console.print(
            f"[cyan]┌─[/cyan] [dim]{msg_id[:8]}[/dim]   {from_to}",
            no_wrap=True, overflow="ellipsis",
        )
        console.print("[cyan]│[/cyan]  [dim]────────[/dim]")
        console.print("[cyan]│[/cyan]  ")
        for line in (first["body"] or "").split("\n"):
            console.print(
                f"[cyan]│[/cyan]  {_colorize_prefix(line)}",
                no_wrap=True, overflow="ellipsis",
            )
        console.print("[cyan]│[/cyan]  ")
        console.print("[cyan]│[/cyan]  [dim]────────[/dim]")

        for e in events_sorted:
            e_emoji, e_color = _status_style(e["event"])
            note = e.get("note") or ""
            if e["event"] == "progress":
                note = f"{e['percent']}%  {note}".strip()
            line = (
                f"[cyan]│[/cyan]  [dim][{_fmt_time(e['created_at'])}][/dim]  "
                f"[{e_color}]{e_emoji}[/{e_color}]"
            )
            if note:
                line += f"  {note}"
            console.print(line, no_wrap=True, overflow="ellipsis")

        console.print("[cyan]│[/cyan]  ")
        console.print(
            f"[cyan]└─[/cyan] [耗时:[bold]{elapsed_str}[/bold]]",
            no_wrap=True, overflow="ellipsis",
        )
        console.print()


@app.command()
def whoami(
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """显示当前 agent 信息。"""
    try:
        name = get_agent_name()
    except EnvironmentError as e:
        if not view:
            _j({"ok": False, "error": str(e)})
            sys.exit(1)
        console.print(f"[red]❌ {e}[/red]")
        return

    info = registry.lookup(name)
    if not info:
        if not view:
            _j({"ok": True, "agent_name": name, "registered": False})
            return
        console.print(f"Agent: [bold]{name}[/bold]\n状态: [red]未注册[/red]")
        return

    if not view:
        _j({"ok": True, "agent_name": name, "registered": True, **{k: info.get(k) for k in ("role", "bio", "capabilities", "session", "pane_id", "workdir", "mux")}})
        return

    console.print(f"\n当前 Agent: {name}")
    console.print(f"  Role:         {info.get('role') or '-'}")
    console.print(f"  Bio:          {info.get('bio') or '-'}")
    console.print(f"  Capabilities: {info.get('capabilities') or '-'}")
    console.print(f"  Workdir:      {info.get('workdir') or '-'}")
    console.print(f"  Mux:          {info.get('mux') or '-'}")
    console.print(f"  Session:      {info['session']}")
    console.print(f"  Pane:         {info['pane_id']}")


@app.command()
def health(
    agent_name: str = "",
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """健康检查。"""
    checks = []

    with console.status("[bold]检查中...[/bold]", spinner="dots"):
        try:
            with db.get_conn() as conn:
                conn.execute(
                    "INSERT INTO message_log (msg_id, event, agent, session) "
                    "VALUES ('test', 'health_check', 'system', '')"
                )
                conn.execute("DELETE FROM message_log WHERE msg_id='test'")
            checks.append({"name": "DB 可写", "ok": True})
        except Exception as e:
            checks.append({"name": "DB 可写", "ok": False, "error": str(e)})

        import shutil
        ok = bool(shutil.which("zellij"))
        check = {"name": "zellij CLI", "ok": ok}
        if not ok:
            check["error"] = "未找到"
        checks.append(check)

        from agtalk.delivery import FIFO_PATH
        ok = FIFO_PATH.exists()
        check = {"name": "FIFO 存在", "ok": ok}
        if not ok:
            check["error"] = "不存在"
        checks.append(check)

        if agent_name:
            info = registry.lookup(agent_name)
            ok = bool(info)
            check = {"name": "Agent 注册", "ok": ok}
            if not ok:
                check["error"] = "未注册"
            checks.append(check)
            if info and info.get("pid"):
                alive = registry._proc_alive(info["pid"])
                check = {"name": "进程存活", "ok": alive}
                if not alive:
                    check["error"] = "进程已退出"
                checks.append(check)
        else:
            checks.append({"name": "Agent 检查", "ok": None, "error": "未指定"})

    passed = sum(1 for c in checks if c["ok"] is True)
    total = sum(1 for c in checks if c["ok"] is not None)

    if not view:
        _j({"ok": True, "checks": checks, "score": f"{passed}/{total}"})
        return

    console.print("\n🏥 健康检查")
    for c in checks:
        if c["ok"] is None:
            icon = "—"
        elif c["ok"]:
            icon = "✅"
        else:
            icon = "❌"
        detail_str = f" ({c['error']})" if c.get("error") else ""
        console.print(f"  {icon} {c['name']}{detail_str}")

    console.print(f"\n健康分数: {passed}/{total}")



# ─── 看板 ──────────────────────────────────────────────
kanban_app = typer.Typer()


def _kanban_author() -> str:
    """返回当前作者名（Agent 或空字符串）。"""
    try:
        return get_agent_name()
    except EnvironmentError:
        return ""


@kanban_app.command("list")
def _kanban_list(
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
    show_all: bool = typer.Option(False, "--all", help="包含已关闭/已过期条目"),
    announcements: bool = typer.Option(False, "--announcements", help="只看公告"),
):
    """查看看板卡片列表（默认只看卡片，--announcements 看公告）。"""
    card_type = "announcement" if announcements else "card"
    with db.get_conn() as conn:
        if show_all:
            rows = conn.execute("""
                SELECT c.card_id, c.title, c.author, c.status, c.created_at, c.type,
                       (SELECT COUNT(*) FROM kanban_comments WHERE card_id = c.card_id) as comment_count
                FROM kanban_cards c
                WHERE c.type = ?
                ORDER BY c.updated_at DESC
            """, (card_type,)).fetchall()
        else:
            rows = conn.execute("""
                SELECT c.card_id, c.title, c.author, c.status, c.created_at, c.type,
                       (SELECT COUNT(*) FROM kanban_comments WHERE card_id = c.card_id) as comment_count
                FROM kanban_cards c
                WHERE c.type = ? AND c.status != 'closed'
                ORDER BY c.updated_at DESC
            """, (card_type,)).fetchall()

    label = "公告" if announcements else "看板"
    if not view:
        board = {"open": [], "in_progress": [], "resolved": [], "closed": []}
        for r in rows:
            status = r["status"]
            if status not in board:
                status = "open"
            board[status].append({
                "card_id": r["card_id"],
                "title": r["title"],
                "author": r["author"] or None,
                "comment_count": r["comment_count"],
                "created_at": r["created_at"],
            })
        _j({"ok": True, "type": card_type, "board": board, "total": len(rows)})
        return

    if not rows:
        console.print(f"[dim]📭 暂无{label}内容[/dim]")
        return

    groups = {"open": [], "in_progress": [], "resolved": [], "closed": []}
    for r in rows:
        status = r["status"]
        if status not in groups:
            status = "open"
        groups[status].append(r)

    console.print(f"\n📋 {label} ({len(rows)} 条)\n")

    status_labels = {
        "open": "OPEN",
        "in_progress": "IN PROGRESS",
        "resolved": "RESOLVED",
        "closed": "CLOSED",
    }
    status_colors = {
        "open": "yellow",
        "in_progress": "blue",
        "resolved": "green",
        "closed": "dim",
    }

    for status in ("open", "in_progress", "resolved", "closed"):
        cards = groups[status]
        color = status_colors[status]
        label = status_labels[status]

        content = Text()
        if not cards:
            content.append("(empty)", style="dim")
        for c in cards:
            content.append(f"{c['card_id']} ")
            content.append(f"{c['title'][:28]}{'...' if len(c['title']) > 28 else ''}\n", style="bold")
            author_display = f"[{c['author']}]" if c["author"] else "[你]"
            content.append(f"    {author_display}  · {c['comment_count']}评论\n", style="dim")

        panel = Panel(
            content,
            title=f"[bold]{label}[/bold] ([bold]{len(cards)}[/bold])",
            border_style=color,
            box=box.ROUNDED,
            padding=(0, 2),
        )
        console.print(panel)


@kanban_app.command("post")
def post(
    title: str,
    body: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """发布看板卡片。"""
    author = _kanban_author()
    card_id = _short_id()
    with db.get_conn() as conn:
        conn.execute(
            "INSERT INTO kanban_cards (card_id, title, body, author, type) VALUES (?, ?, ?, ?, ?)",
            (card_id, title, body, author, "card"),
        )

    if not view:
        _j({"ok": True, "card_id": card_id, "title": title, "status": "open", "type": "card", "author": author or None})
        return

    console.print(f"[green]✅[/green] 已发布看板卡片 [{card_id}]: {title}")


@kanban_app.command("announce")
def announce(
    title: str,
    body: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """发布公告（全员可见）。"""
    author = _kanban_author()
    card_id = _short_id()
    with db.get_conn() as conn:
        conn.execute(
            "INSERT INTO kanban_cards (card_id, title, body, author, type) VALUES (?, ?, ?, ?, ?)",
            (card_id, title, body, author, "announcement"),
        )

    if not view:
        _j({"ok": True, "card_id": card_id, "title": title, "status": "open", "type": "announcement", "author": author or None})
        return

    console.print(f"[green]✅[/green] 已发布公告 [{card_id}]: {title}")


@kanban_app.command("show")
def show(
    card_id: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """查看看板卡片详情。"""
    with db.get_conn() as conn:
        card = conn.execute(
            "SELECT * FROM kanban_cards WHERE card_id = ?", (card_id,)
        ).fetchone()
        if not card:
            if not view:
                _j({"ok": False, "error": f"卡片 {card_id} 不存在"})
                sys.exit(1)
            console.print(f"[red]❌ 卡片 {card_id} 不存在[/red]")
            return

        comments = conn.execute("""
            SELECT comment_id, author, body, created_at
            FROM kanban_comments
            WHERE card_id = ?
            ORDER BY created_at ASC
        """, (card_id,)).fetchall()

    if not view:
        result = {
            "ok": True,
            "card_id": card["card_id"],
            "title": card["title"],
            "body": card["body"],
            "author": card["author"] or None,
            "status": card["status"],
            "comments": [
                {
                    "comment_id": c["comment_id"],
                    "author": c["author"] or None,
                    "body": c["body"],
                    "created_at": c["created_at"],
                }
                for c in comments
            ],
            "created_at": card["created_at"],
            "updated_at": card["updated_at"],
        }
        _j(result)
        return

    author_display = f"[magenta]{card['author']}[/magenta]" if card["author"] else "[green][你][/green]"
    console.print(f"\n┌─ 看板卡片 {card['card_id']} ─{'─' * 30}")
    console.print(f"│  {author_display}")
    console.print(f"│  [{_status_style(card['status'])[1]}]● {card['status'].upper()}[/{_status_style(card['status'])[1]}]")
    console.print(f"│")
    console.print(f"│  [bold]{card['title']}[/bold]")
    for line in (card["body"] or "").split("\n"):
        console.print(f"│  {line}")
    console.print(f"│")
    console.print(f"│  [dim]创建于 {_fmt_ts_full(card['created_at'])}[/dim]")
    console.print(f"└{'─' * 40}")

    if comments:
        console.print(f"\n💬 评论 ({len(comments)} 条)\n")
        for c in comments:
            c_author = f"[magenta]{c['author']}[/magenta]" if c["author"] else "[green][你][/green]"
            console.print(f"  {c_author}  {_fmt_time(c['created_at'])}")
            for line in c["body"].split("\n"):
                console.print(f"    {line}")
            console.print()
    else:
        console.print("\n  [dim](暂无评论)[/dim]\n")


@kanban_app.command("comment")
def comment(
    card_id: str,
    body: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """发表评论。"""
    author = _kanban_author()
    with db.get_conn() as conn:
        card = conn.execute(
            "SELECT card_id FROM kanban_cards WHERE card_id = ?", (card_id,)
        ).fetchone()
        if not card:
            if not view:
                _j({"ok": False, "error": f"卡片 {card_id} 不存在"})
                sys.exit(1)
            console.print(f"[red]❌ 卡片 {card_id} 不存在[/red]")
            return

        comment_id = _short_id()
        conn.execute(
            "INSERT INTO kanban_comments (comment_id, card_id, author, body) VALUES (?, ?, ?, ?)",
            (comment_id, card_id, author, body),
        )
        conn.execute(
            "UPDATE kanban_cards SET updated_at = unixepoch('now','subsec') WHERE card_id = ?",
            (card_id,),
        )

    if not view:
        _j({"ok": True, "comment_id": comment_id, "card_id": card_id, "author": author or None})
        return

    console.print(f"[green]✅[/green] 已评论卡片 {card_id}")


@kanban_app.command("move")
def move(
    card_id: str,
    status: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """移动卡片状态。"""
    valid_statuses = {"open", "in_progress", "resolved", "closed"}
    if status not in valid_statuses:
        if not view:
            _j({"ok": False, "error": f"无效状态: {status}，可选: {', '.join(valid_statuses)}"})
            sys.exit(1)
        console.print(f"[red]❌ 无效状态: {status}[/red]")
        return

    with db.get_conn() as conn:
        card = conn.execute(
            "SELECT card_id FROM kanban_cards WHERE card_id = ?", (card_id,)
        ).fetchone()
        if not card:
            if not view:
                _j({"ok": False, "error": f"卡片 {card_id} 不存在"})
                sys.exit(1)
            console.print(f"[red]❌ 卡片 {card_id} 不存在[/red]")
            return

        conn.execute(
            "UPDATE kanban_cards SET status = ?, updated_at = unixepoch('now','subsec') WHERE card_id = ?",
            (status, card_id),
        )

    if not view:
        _j({"ok": True, "card_id": card_id, "status": status})
        return

    console.print(f"[green]✅[/green] 卡片 {card_id} → [{_status_style(status)[1]}]{status.upper()}[/{_status_style(status)[1]}]")


@kanban_app.command("close")
def close(
    card_id: str,
    view: bool = typer.Option(False, "--view", "-v", help="人类可读格式"),
):
    """关闭卡片（快捷方式：移动到 closed）。"""
    with db.get_conn() as conn:
        card = conn.execute(
            "SELECT card_id FROM kanban_cards WHERE card_id = ?", (card_id,)
        ).fetchone()
        if not card:
            if not view:
                _j({"ok": False, "error": f"卡片 {card_id} 不存在"})
                sys.exit(1)
            console.print(f"[red]❌ 卡片 {card_id} 不存在[/red]")
            return

        conn.execute(
            "UPDATE kanban_cards SET status = 'closed', updated_at = unixepoch('now','subsec') WHERE card_id = ?",
            (card_id,),
        )

    if not view:
        _j({"ok": True, "card_id": card_id, "status": "closed"})
        return

    console.print(f"[green]✅[/green] 卡片 {card_id} → [dim]CLOSED[/dim]")


app.add_typer(kanban_app, name="kanban")
