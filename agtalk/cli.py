# agtalk/cli.py — Typer + Rich 命令实现
import os
import sys
import json
from datetime import datetime
from collections import defaultdict
from typing import Optional

from rich.table import Table
from rich.panel import Panel
from rich.live import Live
from rich import box

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
            console.print(f"[red]❌ {err}[/red]")
        sys.exit(1)
    else:
        console.print(Panel("[bold green]初始化完成 🎉[/bold green]", expand=False))


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

    table = Table(box=box.ROUNDED, show_header=False, padding=(0, 1))
    table.add_column("Key", style="dim")
    table.add_column("Value", style="bold")
    table.add_row("Agent", agent_name)
    table.add_row("Session", info["session"])
    table.add_row("Pane", str(info["pane_id"]))
    if role:
        table.add_row("Role", role)
    if capabilities:
        table.add_row("Capabilities", capabilities)
    if bio:
        table.add_row("Bio", bio)
    console.print(Panel(table, title="[green]✅ 注册成功[/green]", expand=False))


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
        console.print("[dim]暂无注册的 Agent[/dim]")
        return

    table = Table(box=box.ROUNDED, show_header=True, header_style="bold cyan")
    table.add_column("Agent", style="bold")
    table.add_column("Role", style="yellow")
    table.add_column("Session")
    table.add_column("Pane", justify="center")
    if capabilities:
        table.add_column("Capabilities", style="dim")

    for r in rows:
        row_data = [
            r["agent_name"],
            r["role"] or "—",
            r["session"],
            str(r["pane_id"]),
        ]
        if capabilities:
            row_data.append(r["capabilities"] or "—")
        table.add_row(*row_data)

    console.print(table)


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
    show_all: bool = False,
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

    console.print(f"\n[bold]📬 {agent_name} 的收件箱[/bold]\n")
    if not messages:
        console.print("  [dim](empty)[/dim]")
        return

    table = Table(
        box=box.ROUNDED, show_header=True, header_style="bold cyan",
        expand=True, padding=(0, 1),
    )
    table.add_column("ID", style="dim", width=10)
    table.add_column("发件方", style="bold", width=25)
    table.add_column("状态", justify="center", width=8)
    table.add_column("消息摘要")

    for m in messages:
        emoji, color = _status_style(m["status"])
        body_preview = m["body"][:60] + ("..." if len(m["body"]) > 60 else "")
        for prefix, style in prefix_style.items():
            if body_preview.startswith(prefix):
                body_preview = f"[{style}]{prefix}[/{style}]" + body_preview[len(prefix):]
                break
        table.add_row(
            m["msg_id"][:8],
            m["from_agent"],
            f"[{color}]{emoji}[/{color}]",
            body_preview,
        )
    console.print(table)


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
            return Panel(
                "[dim]暂无进度数据，请先使用 agtalk progress <msg_id> <percent>[/dim]",
                title="📋 任务进度", expand=False,
            )

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
        return "[dim](暂无进度记录)[/dim]"

    table = Table(
        box=box.SIMPLE_HEAD, header_style="bold cyan",
        show_header=True, expand=True, padding=(0, 1),
        row_styles=["", "dim"],
    )
    table.add_column("消息 ID", style="dim", width=8, no_wrap=True)
    table.add_column("发起方 → 执行方", width=24, no_wrap=True, overflow="ellipsis")
    table.add_column("进度", width=20, no_wrap=True)
    table.add_column("%", justify="right", width=4, no_wrap=True)
    table.add_column("备注", overflow="ellipsis", no_wrap=True)

    for r in rows:
        pct = r["percent"]
        bar_filled = int(pct / 5)
        bar = "█" * bar_filled + "░" * (20 - bar_filled)
        color = "green" if pct == 100 else "yellow" if pct >= 50 else "cyan"
        body_preview = (r["body"] or "")[:20] + ("..." if r["body"] and len(r["body"]) > 20 else "")
        from_to = f"{r['from_agent'] or '?'} → {r['to_agent'] or '?'}"

        table.add_row(
            r["msg_id"][:8],
            from_to,
            f"[{color}]{bar}[/{color}]",
            f"[bold {color}]{pct}%[/bold {color}]",
            r["note"] or body_preview or "—",
        )

    return table


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
    """时间线视角：Rich Table。"""
    console.print(f"\n[bold]📜 消息历史[/bold] [dim](最近 {len(rows)} 条)[/dim]\n")

    table = Table(
        box=box.ROUNDED, header_style="bold cyan",
        show_header=True, padding=(0, 0),
        row_styles=["", "dim"],
    )
    table.add_column("时间", style="dim", width=8, no_wrap=True)
    table.add_column("", justify="center", width=2, no_wrap=True)
    table.add_column("发件方 → 收件方", width=22, no_wrap=True, overflow="ellipsis")
    table.add_column("消息摘要", width=40, overflow="ellipsis", no_wrap=True)

    prefix_style = {
        "[TASK]": "bold yellow", "[REPLY]": "bold green",
        "[DONE]": "bold green",  "[ACK]": "bold blue",
        "[INFO]": "bold white",  "[FILE]": "bold magenta",
    }

    for r in rows:
        emoji, color = _status_style(r["event"])
        from_to = f"{r['from_agent'] or '?'} → {r['to_agent'] or r['agent']}"
        # 将换行符替换为空格，防止表格行内换行
        body_text = (r["body"] or r["note"] or "").replace("\n", " ")
        body_preview = body_text[:35]
        if len(body_text) > 35:
            body_preview += "..."

        for prefix, style in prefix_style.items():
            if body_preview.startswith(prefix):
                body_preview = f"[{style}]{prefix}[/{style}]" + body_preview[len(prefix):]
                break

        table.add_row(
            _fmt_time(r["created_at"]),
            f"[{color}]{emoji}[/{color}]",
            from_to,
            body_preview or "[dim]—[/dim]",
        )

    console.print(table)


def _render_memory_task_view(rows):
    """任务视角：按 msg_id 分组，展示任务生命周期。"""
    tasks = defaultdict(list)
    for r in rows:
        tasks[r["msg_id"]].append(r)

    console.print(f"\n[bold]📋 任务视图[/bold] [dim]({len(tasks)} 个任务)[/dim]\n")

    table = Table(
        box=box.SIMPLE_HEAD, header_style="bold cyan",
        show_header=True, expand=True, padding=(0, 1),
        row_styles=["", "dim"],
    )
    table.add_column("消息 ID", style="dim", width=8, no_wrap=True)
    table.add_column("发起方 → 执行方", width=24, no_wrap=True, overflow="ellipsis")
    table.add_column("事件流", width=20, no_wrap=True, overflow="ellipsis")
    table.add_column("耗时", justify="right", width=6, no_wrap=True)

    for msg_id, events in tasks.items():
        events_sorted = sorted(events, key=lambda x: x["created_at"])
        first = events_sorted[0]
        last_event = events_sorted[-1]

        elapsed = last_event["created_at"] - first["created_at"]
        elapsed_str = f"{elapsed:.0f}s" if elapsed < 60 else f"{elapsed/60:.1f}m"

        body_preview = (first["body"] or "")[:30] + ("..." if len(first["body"] or "") > 30 else "")
        from_to = f"{first['from_agent'] or '?'} → {first['to_agent'] or '?'}"

        # 事件流：用 emoji 串联
        event_chain = " ".join(
            _status_style(e["event"])[0] for e in events_sorted
        )

        _, color = _status_style(last_event["event"])

        table.add_row(
            msg_id[:8],
            from_to,
            f"[{color}]{body_preview}[/{color}]  {event_chain}",
            f"[bold]{elapsed_str}[/bold]",
        )

    console.print(table)


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

    table = Table(box=box.ROUNDED, show_header=False, padding=(0, 1))
    table.add_column("Key", style="dim")
    table.add_column("Value", style="bold")
    table.add_row("Agent", name)
    table.add_row("Role", info.get("role") or "—")
    table.add_row("Bio", info.get("bio") or "—")
    table.add_row("Capabilities", info.get("capabilities") or "—")
    table.add_row("Session", info["session"])
    table.add_row("Pane", str(info["pane_id"]))
    console.print(Panel(table, title="[bold]👤 当前 Agent[/bold]", expand=False))


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

    table = Table(box=box.ROUNDED, show_header=False, padding=(0, 1))
    table.add_column("", width=3)
    table.add_column("检查项", style="bold")
    table.add_column("详情", style="dim")

    passed = 0
    total = 0
    for name_str, ok, detail in checks:
        if ok is None:
            icon = "[dim]—[/dim]"
        elif ok:
            icon = "[green]✅[/green]"
            passed += 1
            total += 1
        else:
            icon = "[red]❌[/red]"
            total += 1
        table.add_row(icon, name_str, detail)

    score_color = "green" if passed == total else "yellow" if passed > 0 else "red"
    console.print(Panel(
        table,
        title="[bold]🏥 健康检查[/bold]",
        subtitle=f"[{score_color}]健康分数: {passed}/{total}[/{score_color}]",
        expand=False,
    ))
