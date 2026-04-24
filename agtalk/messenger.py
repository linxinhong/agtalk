# agtalk/messenger.py — 消息模块
import uuid
import json

from .db import get_conn
from .registry import lookup
from .factory import get_agent_name
from .delivery import _deliver

STUCK_DELIVERED_TIMEOUT = 30  # 秒


def _new_msg_id() -> str:
    return str(uuid.uuid4())


def send(
    to_agent: str,
    body: str,
    from_agent: str = None,
    subject: str = "",
    msg_type: str = "text",
    priority: int = 5,
    reply_to: str = None,
    metadata: dict = None,
    send_enter: bool = True,
    deliver: bool = False,
) -> str:
    """
    发送消息给指定 agent。
    返回 msg_id。
    deliver=False（默认）: 只写 messages 表（inbox），不弹 pane，不入 queue
    deliver=True: 直接投递到 pane + 离线入 queue（遗留模式，一般不再使用）
    send_enter=False 时，不自动发送 Enter，接收方需手动按 Enter 查看。
    """
    if from_agent is None:
        from_agent = get_agent_name()
    msg_id = _new_msg_id()
    meta_str = json.dumps(metadata or {})

    with get_conn() as conn:
        conn.execute("""
            INSERT INTO messages
                (msg_id, to_agent, from_agent, subject, body, msg_type,
                 priority, status, reply_to_msg_id, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)
        """, (msg_id, to_agent, from_agent, subject, body, msg_type,
              priority, reply_to, meta_str))
        _log_event(conn, msg_id, "sent", from_agent)

    if not deliver:
        return msg_id

    # 尝试立即投递
    agent_info = lookup(to_agent)
    if agent_info:
        _deliver(msg_id, to_agent, from_agent, body, agent_info, send_enter=send_enter)
    else:
        # 加入离线队列
        with get_conn() as conn:
            conn.execute("""
                INSERT INTO offline_queue (msg_id, to_agent)
                VALUES (?, ?)
            """, (msg_id, to_agent))

    return msg_id


def inbox(agent_name: str, status: str = "pending,delivered", limit: int = 20) -> list[dict]:
    """查看 inbox，返回消息列表"""
    statuses = tuple(status.split(","))
    placeholders = ",".join("?" * len(statuses))
    with get_conn() as conn:
        rows = conn.execute(f"""
            SELECT msg_id, from_agent, subject, body, msg_type, priority,
                   status, created_at, reply_to_msg_id
            FROM messages
            WHERE to_agent = ? AND status IN ({placeholders})
            ORDER BY priority ASC, created_at ASC
            LIMIT ?
        """, (agent_name, *statuses, limit)).fetchall()
        return [dict(r) for r in rows]


def _resolve_msg_id(conn, msg_id: str, agent_name: str) -> str:
    """支持短 msg_id 前缀匹配（如 7a8b9335 → 完整 UUID）"""
    if len(msg_id) >= 36:
        return msg_id
    row = conn.execute(
        "SELECT msg_id FROM messages WHERE msg_id LIKE ? AND to_agent=?",
        (msg_id + "%", agent_name)
    ).fetchone()
    if not row:
        raise ValueError(f"找不到消息: {msg_id}")
    return row["msg_id"]


def mark_read(msg_id: str, agent_name: str):
    with get_conn() as conn:
        full_id = _resolve_msg_id(conn, msg_id, agent_name)
        conn.execute("""
            UPDATE messages SET status='read', read_at=unixepoch('now','subsec')
            WHERE msg_id=? AND to_agent=?
        """, (full_id, agent_name))
        _log_event(conn, full_id, "read", agent_name)


def mark_done(msg_id: str, agent_name: str):
    with get_conn() as conn:
        full_id = _resolve_msg_id(conn, msg_id, agent_name)
        conn.execute("""
            UPDATE messages SET status='done', done_at=unixepoch('now','subsec')
            WHERE msg_id=? AND to_agent=?
        """, (full_id, agent_name))
        _log_event(conn, full_id, "done", agent_name)


def broadcast(body: str, from_agent: str = None, exclude: list = None) -> list[str]:
    """广播给所有在线 agent（写入 inbox，不直接投递 pane）"""
    exclude = exclude or []
    from_agent = from_agent or get_agent_name()
    with get_conn() as conn:
        rows = conn.execute("SELECT agent_name FROM agents").fetchall()
    msg_ids = []
    for row in rows:
        name = row["agent_name"]
        if name not in exclude and name != from_agent:
            mid = send(name, body, from_agent=from_agent, deliver=False)
            msg_ids.append(mid)
    return msg_ids


def _log_event(conn, msg_id: str, event: str, agent: str, session: str = "", pane_id=None, note: str = ""):
    """在 message_log 中记录事件"""
    conn.execute("""
        INSERT INTO message_log (msg_id, event, agent, session, pane_id, note)
        VALUES (?, ?, ?, ?, ?, ?)
    """, (msg_id, event, agent, session, pane_id, note))


def check_stuck_messages(timeout_sec: int = STUCK_DELIVERED_TIMEOUT) -> int:
    """将卡在 delivered 超过 timeout_sec 的消息标记为 failed"""
    cutoff = f"unixepoch('now','subsec') - {timeout_sec}"
    with get_conn() as conn:
        rows = conn.execute(f"""
            SELECT msg_id FROM messages
            WHERE status = 'delivered'
              AND delivered_at < {cutoff}
              AND retry_count < max_retries
        """).fetchall()
        count = 0
        for row in rows:
            conn.execute("""
                UPDATE messages
                SET status = 'failed', retry_count = retry_count + 1
                WHERE msg_id = ?
            """, (row["msg_id"],))
            _log_event(conn, row["msg_id"], "stuck", "system")
            count += 1
        return count


def retry_message(msg_id: str) -> bool:
    """将 failed 消息重新置为 pending，等待下次投递"""
    with get_conn() as conn:
        row = conn.execute(
            "SELECT to_agent FROM messages WHERE msg_id = ? AND status = 'failed'",
            (msg_id,)
        ).fetchone()
        if not row:
            return False
        conn.execute("""
            UPDATE messages SET status = 'pending', delivered_at = NULL
            WHERE msg_id = ?
        """, (msg_id,))
        _log_event(conn, msg_id, "retry", row["to_agent"])
        return True
