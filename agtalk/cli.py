# agtalk/cli.py — Typer + Rich 命令实现
import os
import sys
import json
from datetime import datetime
from collections import defaultdict
from typing import Optional

from rich.live import Live
from rich.text import Text

import typer

from agtalk.main import app
from agtalk.console import console
from agtalk import db, registry, messenger
from agtalk.factory import get, get_agent_name
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
    """返回 (emoji, rich color)。"""
    return {
        "done":      ("✅", "green"),
        "sent":      ("📤", "blue"),
        "pending":   ("⏳", "yellow"),
        "delivered": ("📬", "cyan"),
        "read":      ("👁", "white"),
        "failed":    ("❌", "red"),
    }.get(status, ("•", "white"))


# ─── 初始化 ──────────────────────────────────────────
@app.command()
def init():
    """初始化环境：检查终端环境、初始化数据库、确保 FIFO 存在。"""
    errors = []
    mux = get()

    with console.status("[bold green]正在初始化...", spinner="dots"):
        try:
            session = mux.get_current_session()
            pane_id = mux.get_current_pane_id()
            console.print(
                f"✅ 终端环境: session=[cyan]{session}[/cyan], "
                f"pane_id=[cyan]{pane_id}[/cyan]"
            )
        except EnvironmentError as e:
            errors.append(f"终端环境: {e}")

        import shutil
        if shutil.which("zellij"):
            console.print("✅ zellij CLI 可用")
        else:
            errors.append("zellij CLI 未找到")

        try:
            db.init_db()
            console.print(f"✅ 数据库: [dim]{db.get_db_path()}[/dim]")
        except Exception as e:
            errors.append(f"数据库初始化: {e}")

        try:
            _ensure_fifo()
            from agtalk.delivery import FIFO_PATH
            console.print(f"✅ FIFO: [dim]{FIFO_PATH}[/dim]")
        except Exception as e:
            errors.append(f"FIFO 创建: {e}")

    if errors:
        for err in errors:
            console.print(f"❌ {err}")
        sys.exit(1)
    else:
        console.print("\n✅ 初始化完成")


# ─── 注册 ────────────────────────────────────────────
@app.command()
def register(
    agent_name: str,
    role: str = "",
    capabilities: str = "",
    bio: str = "",
):
    """注册当前 pane 为 Agent。"""
    try:
        info = registry.register(agent_name, role, capabilities, bio)
    except EnvironmentError as e:
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)
    os.environ["AGTALK_AGENT_NAME"] = agent_name

    console.print(f"\n✅ 注册成功: {agent_name}")
    console.print(f"  Session: {info['session']} | Pane: {info['pane_id']}")
    if role:
        console.print(f"  Role: {role}")
    if capabilities:
        console.print(f"  Capabilities: {capabilities}")
    if bio:
        console.print(f"  Bio: {bio}")


@app.command()
def unregister(agent_name: str):
    """注销 Agent。"""
    registry.unregister(agent_name)
    console.print(f"[yellow]🗑 已注销:[/yellow] {agent_name}")


@app.command("list")
def list_agents(
    as_json: bool = False,
    capabilities: bool = False,
):
    """列出所有注册 Agent。"""
    with db.get_conn() as conn:
        if capabilities:
            rows = conn.execute(
                "SELECT agent_name, session, pane_id, role, capabilities, last_seen_at FROM agents"
            ).fetchall()
        else:
            rows = conn.execute(
                "SELECT agent_name, session, pane_id, role, last_seen_at FROM agents"
            ).fetchall()

    if as_json:
        console.print(json.dumps([dict(r) for r in rows], indent=2))
        return

    if not rows:
        console.print("暂无注册的 Agent")
        return

    console.print(f"{'Agent':<25} {'Role':<12} {'Session':<10} {'Pane':<5}")
    console.print("-" * 55)
    for r in rows:
        cap = f" | {r['capabilities']}" if capabilities and r["capabilities"] else ""
        console.print(
            f"{r['agent_name']:<25} {r['role'] or '-':<12} "
            f"{r['session']:<10} {r['pane_id']:<5}{cap}"
        )


# ─── 发消息 ──────────────────────────────────────────
@app.command()
def send(
    agent: str,
    body: str,
    subject: str = "",
    msg_type: str = "text",
    priority: int = 5,
    wait_done: bool = False,
    timeout: int = 120,
    reply_to: Optional[str] = None,
    notify: bool = False,
    no_enter: bool = False,
):
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
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    preview = body[:50] + ("..." if len(body) > 50 else "")
    console.print(
        f"[blue]📨[/blue] [bold]{msg_id[:8]}...[/bold] → [cyan]{agent}[/cyan]  [dim]{preview}[/dim]"
    )

    if notify:
        agent_info = registry.lookup(agent)
        try:
            from_agent = get_agent_name()
        except Exception:
            from_agent = "unknown"
        if agent_info:
            try:
                notify_agent(agent, from_agent, agent_info, send_enter=not no_enter, msg_id=msg_id)
                console.print(f"[yellow]🔔 已提醒[/yellow] {agent} 查收")
            except Exception as e:
                console.print(f"[red]🔔 提醒发送失败:[/red] {e}")
        else:
            console.print(f"[dim]🔔 {agent} 离线，无法提醒[/dim]")

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
):
    """向指定 Agent 的 pane 发送提醒通知（不写 inbox）。"""
    body = _unescape_body(body)
    agent_info = registry.lookup(agent)
    if not agent_info:
        console.print(f"[red]❌ Agent {agent} 未找到或已离线[/red]")
        return
    try:
        from_agent = get_agent_name()
    except EnvironmentError as e:
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    if msg_id:
        with db.get_conn() as conn:
            row = conn.execute(
                "SELECT status FROM messages WHERE msg_id LIKE ?",
                (msg_id + "%",),
            ).fetchone()
        if row and row["status"] in ("done", "failed", "read"):
            console.print(f"[dim]⏭ 消息已 {row['status']}，无需提醒[/dim]")
            return

    custom_text = body if body else None
    notify_agent(agent, from_agent, agent_info, send_enter=not no_enter,
                 custom_text=custom_text, msg_id=msg_id)
    console.print(f"[yellow]🔔 已提醒[/yellow] {agent}")


@app.command()
def broadcast(
    body: str,
    exclude: str = "",
    notify: bool = False,
    no_enter: bool = False,
):
    """广播给所有 Agent（仅写入 inbox）。"""
    body = _unescape_body(body)
    excludes = [e.strip() for e in exclude.split(",") if e.strip()]
    try:
        ids = messenger.broadcast(body, exclude=excludes)
    except EnvironmentError as e:
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)
    console.print(f"[blue]📢 广播完成[/blue]，共 [bold]{len(ids)}[/bold] 条消息")

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
                        notify_agent(to_agent, from_agent, agent_info,
                                     send_enter=not no_enter, msg_id=mid)
                    except Exception:
                        pass


@app.command()
def multicast(
    agents: str,
    body: str,
    notify: bool = False,
    no_enter: bool = False,
):
    """多播给指定 Agents (逗号分隔，仅写入 inbox)。"""
    body = _unescape_body(body)
    agent_list = [a.strip() for a in agents.split(",") if a.strip()]
    if not agent_list:
        console.print("[red]❌ 未指定 agents[/red]")
        return
    try:
        from_agent = get_agent_name()
    except EnvironmentError as e:
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)

    sent = {}
    for agent in agent_list:
        try:
            mid = messenger.send(to_agent=agent, body=body)
            sent[agent] = mid
            console.print(f"  [green]✅[/green] [cyan]{agent}[/cyan]: {mid[:8]}...")
        except Exception as e:
            console.print(f"  [red]❌[/red] [cyan]{agent}[/cyan]: {e}")

    if notify:
        for agent, mid in sent.items():
            agent_info = registry.lookup(agent)
            if agent_info:
                try:
                    notify_agent(agent, from_agent, agent_info,
                                 send_enter=not no_enter, msg_id=mid)
                except Exception:
                    pass


@app.command("key-enter")
def key_enter(agent_name: str):
    """向 agent pane 发送 Enter 键。"""
    info = registry.lookup(agent_name)
    if not info:
        console.print(f"[red]❌ Agent {agent_name} 未找到[/red]")
        return
    mux = get()
    mux.send_keys(info["session"], info["pane_id"], "Enter")
    console.print(f"[green]✅ 已向[/green] {agent_name} 发送 Enter")


# ─── 收消息 ──────────────────────────────────────────
@app.command()
def inbox(
    agent_name: str,
    show_all: bool = typer.Option(False, "--all", "--show-all", help="包含已读消息"),
    as_json: bool = False,
):
    """查看 inbox。"""
    if not agent_name:
        console.print("[red]❌ 请指定 agent 名字: agtalk inbox <your_name>[/red]")
        return

    status = "pending,delivered,read,done,failed" if show_all else "pending,delivered"
    messages = messenger.inbox(agent_name, status=status)

    if as_json:
        console.print(json.dumps(messages, indent=2))
        return

    prefix_style = {
        "[TASK]":  "bold yellow",
        "[REPLY]": "bold green",
        "[DONE]":  "bold green",
        "[ACK]":   "bold blue",
        "[INFO]":  "bold white",
        "[FILE]":  "bold magenta",
    }

    console.print(f"\n📬 {agent_name} 的收件箱 ({len(messages)} 条)\n")
    if not messages:
        console.print("  (empty)")
        return

    for m in messages:
        emoji, _color = _status_style(m["status"])
        body_preview = m["body"].replace("\n", " ")[:40]
        if len(m["body"]) > 40:
            body_preview += "..."
        line = f"  [{m['msg_id'][:8]}] {emoji} {m['from_agent']:<25} | {body_preview}"
        console.print(line, no_wrap=True, overflow="ellipsis")


@app.command()
def done(msg_id: str):
    """标记消息为已完成 (done)。"""
    try:
        agent_name = get_agent_name()
    except EnvironmentError as e:
        console.print(f"[red]❌ {e}[/red]")
        sys.exit(1)
    messenger.mark_done(msg_id, agent_name)
    _fifo_notify()
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
):
    """更新任务进度（0-100），或用 --watch 实时监控所有任务进度。"""
    if watch:
        _watch_all_progress()
        return

    if not (0 <= percent <= 100):
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
        conn.execute(
            "INSERT INTO task_progress (msg_id, percent, note) VALUES (?, ?, ?)",
            (msg_id, percent, note),
        )

    bar_filled = int(percent / 5)
    bar = "█" * bar_filled + "░" * (20 - bar_filled)
    color = "green" if percent == 100 else "yellow" if percent >= 50 else "cyan"

    console.print(
        f"[bold]{msg_id[:8]}[/bold]  [{color}]{bar}[/{color}]  "
        f"[bold {color}]{percent:3d}%[/bold {color}]"
        + (f"  [dim]{note}[/dim]" if note else "")
    )


@app.command("progress-list")
def progress_list(watch: bool = False):
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
    else:
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
            f"  {r['msg_id'][:8]}  {bar}  {pct:3d}%  {from_to:<30} | {note}"
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
def prune(dry_run: bool = False):
    """清理僵尸 Agent。"""
    dead = registry.prune(dry_run)
    if dead:
        verb = "将清理" if dry_run else "已清理"
        console.print(f"[yellow]🧹 {verb}:[/yellow] {', '.join(dead)}")
    else:
        console.print("[green]✅ 无需清理[/green]")


@app.command("check-stuck")
def check_stuck():
    """检查并标记超时的 delivered 消息为 failed。"""
    count = messenger.check_stuck_messages()
    console.print(f"[yellow]🚨 标记了[/yellow] [bold]{count}[/bold] 条卡死消息为 failed")


@app.command()
def memory(
    agent: str = "",
    last: int = 20,
    task_view: bool = typer.Option(False, "--task", help="按任务线索分组显示"),
    as_json: bool = False,
):
    """查询消息历史（支持 --task 任务视角）。"""
    with db.get_conn() as conn:
        if agent:
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

    if as_json:
        console.print(json.dumps([dict(r) for r in rows], indent=2))
        return

    if not rows:
        console.print("[dim]📭 无消息历史[/dim]")
        return

    if task_view:
        _render_memory_task_view(rows)
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
        line = f"  [{_fmt_time(r['created_at'])}] {emoji} {from_to:<30} | {body_text}"
        console.print(line, no_wrap=True, overflow="ellipsis")


def _render_memory_task_view(rows):
    """任务视角：按 msg_id 分组，展示任务生命周期。"""
    tasks = defaultdict(list)
    for r in rows:
        tasks[r["msg_id"]].append(r)

    console.print(f"\n📋 任务视图 ({len(tasks)} 个任务)\n")

    for msg_id, events in tasks.items():
        events_sorted = sorted(events, key=lambda x: x["created_at"])
        first = events_sorted[0]
        last_event = events_sorted[-1]

        elapsed = last_event["created_at"] - first["created_at"]
        elapsed_str = f"{elapsed:.0f}s" if elapsed < 60 else f"{elapsed/60:.1f}m"

        body_preview = (first["body"] or "").replace("\n", " ")[:40]
        if len(first["body"] or "") > 40:
            body_preview += "..."
        from_to = f"{first['from_agent'] or '?'} → {first['to_agent'] or '?'}"

        event_chain = " → ".join(
            _status_style(e["event"])[0] for e in events_sorted
        )

        console.print(f"  {msg_id[:8]}  {body_preview}", no_wrap=True, overflow="ellipsis")
        console.print(f"    发起: {from_to} | {_fmt_time(first['created_at'])}", no_wrap=True, overflow="ellipsis")
        console.print(f"    流程: {event_chain} | 耗时 {elapsed_str}", no_wrap=True, overflow="ellipsis")
        console.print()


@app.command()
def whoami():
    """显示当前 agent 信息。"""
    try:
        name = get_agent_name()
    except EnvironmentError as e:
        console.print(f"[red]❌ {e}[/red]")
        return

    info = registry.lookup(name)
    if not info:
        console.print(f"Agent: [bold]{name}[/bold]\n状态: [red]未注册[/red]")
        return

    console.print(f"\n👤 当前 Agent: {name}")
    console.print(f"  Role:         {info.get('role') or '-'}")
    console.print(f"  Bio:          {info.get('bio') or '-'}")
    console.print(f"  Capabilities: {info.get('capabilities') or '-'}")
    console.print(f"  Session:      {info['session']}")
    console.print(f"  Pane:         {info['pane_id']}")


@app.command()
def health(agent_name: str = ""):
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
            checks.append(("DB 可写", True, ""))
        except Exception as e:
            checks.append(("DB 可写", False, str(e)))

        import shutil
        ok = bool(shutil.which("zellij"))
        checks.append(("zellij CLI", ok, "" if ok else "未找到"))

        from agtalk.delivery import FIFO_PATH
        checks.append(("FIFO 存在", FIFO_PATH.exists(), ""))

        if agent_name:
            info = registry.lookup(agent_name)
            checks.append(("Agent 注册", bool(info), ""))
            if info and info.get("pid"):
                alive = registry._proc_alive(info["pid"])
                checks.append(("进程存活", alive, ""))
        else:
            checks.append(("Agent 检查", None, "未指定"))

    console.print("\n🏥 健康检查")
    passed = 0
    total = 0
    for name_str, ok, detail in checks:
        if ok is None:
            icon = "—"
        elif ok:
            icon = "✅"
            passed += 1
            total += 1
        else:
            icon = "❌"
            total += 1
        detail_str = f" ({detail})" if detail else ""
        console.print(f"  {icon} {name_str}{detail_str}")

    console.print(f"\n健康分数: {passed}/{total}")
