# agtalk/db.py — 数据库层（含 Schema Migration）
import sqlite3
import os
from pathlib import Path
from contextlib import contextmanager

_CURRENT_VERSION = 1


def get_db_path() -> Path:
    custom = os.environ.get("AGTALK_DB_PATH")
    if custom:
        return Path(custom)
    return Path.home() / ".config" / "agtalk" / "talk.db"


@contextmanager
def get_conn():
    db_path = get_db_path()
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path), timeout=10, isolation_level=None)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.row_factory = sqlite3.Row
    try:
        yield conn
    finally:
        conn.close()


def _get_schema_sql() -> str:
    return """
-- ① Agent 注册表
CREATE TABLE IF NOT EXISTS agents (
    agent_name      TEXT PRIMARY KEY,
    session         TEXT NOT NULL,
    pane_id         INTEGER NOT NULL,
    pid             INTEGER,
    role            TEXT NOT NULL DEFAULT '',
    capabilities    TEXT NOT NULL DEFAULT '',
    bio             TEXT NOT NULL DEFAULT '',
    registered_at   REAL NOT NULL DEFAULT (unixepoch('now','subsec')),
    last_seen_at    REAL NOT NULL DEFAULT (unixepoch('now','subsec'))
);

-- ② 消息 inbox
CREATE TABLE IF NOT EXISTS messages (
    msg_id          TEXT PRIMARY KEY,
    to_agent        TEXT NOT NULL,
    from_agent      TEXT NOT NULL,
    subject         TEXT NOT NULL DEFAULT '',
    body            TEXT NOT NULL,
    msg_type        TEXT NOT NULL DEFAULT 'text',
    priority        INTEGER NOT NULL DEFAULT 5,
    status          TEXT NOT NULL DEFAULT 'pending',
    retry_count     INTEGER NOT NULL DEFAULT 0,
    max_retries     INTEGER NOT NULL DEFAULT 3,
    created_at      REAL NOT NULL DEFAULT (unixepoch('now','subsec')),
    delivered_at    REAL,
    read_at         REAL,
    done_at         REAL,
    reply_to_msg_id TEXT,
    metadata        TEXT DEFAULT '{}'
);

-- ③ 消息事件日志
CREATE TABLE IF NOT EXISTS message_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    msg_id          TEXT NOT NULL,
    event           TEXT NOT NULL,
    agent           TEXT NOT NULL,
    session         TEXT NOT NULL,
    pane_id         INTEGER,
    note            TEXT DEFAULT '',
    created_at      REAL NOT NULL DEFAULT (unixepoch('now','subsec'))
);

-- ④ 离线消息队列
CREATE TABLE IF NOT EXISTS offline_queue (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    msg_id          TEXT NOT NULL,
    to_agent        TEXT NOT NULL,
    next_retry_at   REAL NOT NULL DEFAULT (unixepoch('now','subsec')),
    created_at      REAL NOT NULL DEFAULT (unixepoch('now','subsec'))
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_messages_to_agent  ON messages(to_agent, status);
CREATE INDEX IF NOT EXISTS idx_messages_from_agent ON messages(from_agent);
CREATE INDEX IF NOT EXISTS idx_offline_queue_agent ON offline_queue(to_agent);
"""


def _migrate_v1(conn):
    """Initial v1 schema — 初次建表"""
    conn.executescript(_get_schema_sql())


def _ensure_schema():
    """检查并执行 schema migration"""
    with get_conn() as conn:
        cur = conn.execute("PRAGMA user_version").fetchone()[0]
        if cur < 1:
            _migrate_v1(conn)
        conn.execute(f"PRAGMA user_version = {_CURRENT_VERSION}")


def init_db():
    """兼容旧接口，内部调用 _ensure_schema"""
    _ensure_schema()
