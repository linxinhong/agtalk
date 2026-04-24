# agtalk/registry.py — Agent 注册与查询
import os
import re

from .db import get_conn
from .factory import get, detect_name
from .delivery import _flush_offline_queue

AGENT_NAME_PATTERN = r'^[a-z]+_[a-z]+_[A-Z][a-zA-Z0-9]+$'


def register(agent_name: str, role: str = "", capabilities: str = "", bio: str = "", workdir: str = "", mux: str = "") -> dict:
    """
    注册当前 pane 为 agent。
    必须在终端多路复用器环境内执行。
    """
    if not re.match(AGENT_NAME_PATTERN, agent_name):
        raise ValueError(f"agent_name 不符合规范: {agent_name}\n"
                         f"格式应为 {{tool}}_{{role}}_{{Name}}, 如 claude_coder_Alex")
    multiplexer = get()
    session = multiplexer.get_current_session()
    pane_id = multiplexer.get_current_pane_id()
    pid = os.getppid()  # agent 所在 shell 的 pid，用于进程级健康检查

    with get_conn() as conn:
        conn.execute("""
            INSERT INTO agents (agent_name, session, pane_id, pid, role, capabilities, bio, workdir, mux, last_seen_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, unixepoch('now','subsec'))
            ON CONFLICT(agent_name) DO UPDATE SET
                session=excluded.session,
                pane_id=excluded.pane_id,
                pid=excluded.pid,
                role=excluded.role,
                capabilities=excluded.capabilities,
                bio=excluded.bio,
                workdir=excluded.workdir,
                mux=excluded.mux,
                last_seen_at=unixepoch('now','subsec')
        """, (agent_name, session, pane_id, pid, role, capabilities, bio, workdir, mux))

    # 投递离线消息
    _flush_offline_queue(agent_name, session, pane_id)
    
    # 将当前 pane 标题改为 agent_name，方便识别
    multiplexer.rename_pane(session, pane_id, agent_name)
    
    return {"agent_name": agent_name, "session": session, "pane_id": pane_id, "pid": pid, "workdir": workdir, "mux": mux}


def unregister(agent_name: str):
    with get_conn() as conn:
        conn.execute("DELETE FROM agents WHERE agent_name = ?", (agent_name,))


def lookup(agent_name: str) -> dict | None:
    """查询 agent 的 session + pane_id，同时验活"""
    mux = get()
    with get_conn() as conn:
        row = conn.execute(
            "SELECT * FROM agents WHERE agent_name = ?", (agent_name,)
        ).fetchone()
        if not row:
            return None
        if not mux.pane_is_alive(row["session"], row["pane_id"]):
            conn.execute("DELETE FROM agents WHERE agent_name = ?", (agent_name,))
            return None
        return dict(row)


def prune(dry_run: bool = False) -> list[str]:
    """清理 pane 已关闭的 agent"""
    dead = []
    mux = get()
    with get_conn() as conn:
        rows = conn.execute("SELECT agent_name, session, pane_id, pid, mux FROM agents").fetchall()
        if not rows:
            return dead

        # 一次获取所有 pane 信息
        session_panes = {}
        for row in rows:
            session = row["session"]
            if session not in session_panes:
                session_panes[session] = mux.list_panes(session)

        alive_pane_ids = {p["id"] for panes in session_panes.values() for p in panes if not p.get("exited", False)}

        for row in rows:
            if row["mux"] not in ("zellij", "tmux"):
                continue  # 非终端多路复用器的 Agent（web/office/remote 等）不检查 pane 存活
            pane_alive = row["pane_id"] in alive_pane_ids
            if pane_alive:
                continue  # pane 存活即认为 agent 存活
            # pane 已死，再确认进程是否也死了
            proc_alive = _proc_alive(row["pid"]) if row["pid"] else False
            if not proc_alive:
                dead.append(row["agent_name"])
                if not dry_run:
                    conn.execute("DELETE FROM agents WHERE agent_name = ?", (row["agent_name"],))
    return dead


def health(agent_name: str = "") -> bool:
    """健康检查：DB、多路复用器、pane、agent、进程"""
    # 1. DB 可写
    try:
        with get_conn() as conn:
            conn.execute("INSERT INTO message_log (msg_id, event, agent, session) VALUES ('test', 'health_check', 'system', '')")
            conn.execute("DELETE FROM message_log WHERE msg_id='test'")
    except Exception:
        return False

    if not agent_name:
        return True

    # 2. Agent 注册状态
    info = lookup(agent_name)
    if not info:
        return False

    # 3. 进程存活检查
    if info.get("pid"):
        return _proc_alive(info["pid"])
    return True


def _proc_alive(pid: int) -> bool:
    """通过 os.kill(pid, 0) 检查进程是否存活"""
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False
