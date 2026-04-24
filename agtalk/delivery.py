# agtalk/delivery.py — 消息投递层（含 FIFO 通知）
import os
import select
import time
from pathlib import Path

from .db import get_conn
from .factory import get, get_by_name

FIFO_PATH = Path.home() / ".config" / "agtalk" / "notify.fifo"


def _ensure_fifo():
    """确保 FIFO 管道存在"""
    if not FIFO_PATH.exists():
        FIFO_PATH.parent.mkdir(parents=True, exist_ok=True)
        os.mkfifo(FIFO_PATH)


def notify():
    """向 FIFO 写入通知，唤醒所有 select 监听者。若无监听者，不阻塞。"""
    _ensure_fifo()
    try:
        fd = os.open(FIFO_PATH, os.O_WRONLY | os.O_NONBLOCK)
        with os.fdopen(fd, 'w') as f:
            f.write('\n')
    except (OSError, IOError):
        pass  # 无监听者，忽略


def _format_delivery_text(msg_id: str, from_agent: str, body: str) -> str:
    """投递到 pane 时的标准格式。使用 \r\n (CRLF) 提高终端兼容性。"""
    return (
        f"\r\n[agtalk:{msg_id[:8]}]\r\n"
        f"From: {from_agent}\r\n"
        f"---\r\n"
        f"{body}\r\n"
        f"---\r\n"
        f"回复: agtalk done {msg_id[:8]}\r\n"
    )


def _format_notification_text(to_agent: str, from_agent: str, msg_count: int = 1, msg_id: str = None) -> str:
    """生成 inbox 提醒通知文本，用于写入目标 pane。"""
    if msg_id:
        short_id = msg_id[:8]
        return (
            f"\r\n[agtalk:{short_id}] | exec: agtalk inbox {to_agent}\r\n"
        )
    if msg_count == 1:
        return (
            f"\r\n[agtalk] 新消息来自 {from_agent}\r\n"
            f"查收: agtalk inbox {to_agent}\r\n"
        )
    return (
        f"\r\n[agtalk] 你有 {msg_count} 条新消息（来自 {from_agent} 等）\r\n"
        f"查收: agtalk inbox {to_agent}\r\n"
    )


def notify_agent(to_agent: str, from_agent: str, agent_info: dict, send_enter: bool = True, custom_text: str = None, msg_id: str = None):
    """向目标 pane 发送 inbox 提醒通知（不投递消息全文）。

    优先级：custom_text > msg_id 标准格式 > 默认精简格式
    """
    if custom_text is not None:
        text = f"\r\n[agtalk-notify] {custom_text}\r\n"
    elif msg_id is not None:
        text = _format_notification_text(to_agent, from_agent, msg_id=msg_id)
    else:
        text = _format_notification_text(to_agent, from_agent)
    if agent_info.get("mux") in ("zellij", "tmux"):
        mux = get_by_name(agent_info["mux"])
        mux.write_chars_to_pane(
            session=agent_info["session"],
            pane_id=agent_info["pane_id"],
            text=text,
            send_enter=send_enter
        )


def _deliver(msg_id: str, to_agent: str, from_agent: str, body: str, agent_info: dict, send_enter: bool = False):
    """将消息写入目标 pane，并更新 DB 状态为 delivered"""
    text = _format_delivery_text(msg_id, from_agent, body)
    if agent_info.get("mux") in ("zellij", "tmux"):
        mux = get_by_name(agent_info["mux"])
        mux.write_chars_to_pane(
            session=agent_info["session"],
            pane_id=agent_info["pane_id"],
            text=text,
            send_enter=send_enter
        )
    with get_conn() as conn:
        conn.execute("""
            UPDATE messages SET status='delivered', delivered_at=unixepoch('now','subsec')
            WHERE msg_id=?
        """, (msg_id,))
        _log_event(conn, msg_id, "delivered", to_agent,
                   session=agent_info["session"], pane_id=agent_info["pane_id"])


def _flush_offline_queue(agent_name: str, session: str, pane_id: int, mux_name: str = ""):
    """agent 上线后发送离线消息提醒通知"""
    with get_conn() as conn:
        rows = conn.execute("""
            SELECT oq.id, oq.msg_id, m.from_agent
            FROM offline_queue oq
            JOIN messages m ON oq.msg_id = m.msg_id
            WHERE oq.to_agent = ?
            ORDER BY m.priority ASC, m.created_at ASC
        """, (agent_name,)).fetchall()

        if not rows:
            return

        # 汇总提醒：发送一条通知，告知有多少条离线消息
        msg_count = len(rows)
        unique_senders = ", ".join({r["from_agent"] for r in rows})
        text = _format_notification_text(agent_name, unique_senders, msg_count)
        if mux_name in ("zellij", "tmux"):
            mux = get_by_name(mux_name)
            mux.write_chars_to_pane(
                session=session,
                pane_id=pane_id,
                text=text,
                send_enter=True
            )

        # 清空离线队列（消息内容已在 messages 表中）
        for row in rows:
            conn.execute("DELETE FROM offline_queue WHERE id=?", (row["id"],))


def _log_event(conn, msg_id: str, event: str, agent: str, session: str = "", pane_id=None, note: str = ""):
    """在 message_log 中记录事件"""
    conn.execute("""
        INSERT INTO message_log (msg_id, event, agent, session, pane_id, note)
        VALUES (?, ?, ?, ?, ?, ?)
    """, (msg_id, event, agent, session, pane_id, note))


def watch_until_done(msg_id: str, timeout: int = 120) -> str | None:
    """通过 FIFO select 等待消息完成，超时返回 None"""
    _ensure_fifo()
    deadline = time.time() + timeout
    with open(FIFO_PATH) as fifo:
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                return None
            ready, _, _ = select.select([fifo], [], [], min(remaining, 1.0))
            if ready:
                fifo.read()
                with get_conn() as conn:
                    row = conn.execute(
                        "SELECT status FROM messages WHERE msg_id=?", (msg_id,)
                    ).fetchone()
                    if row and row["status"] in ("done", "failed"):
                        return row["status"]
