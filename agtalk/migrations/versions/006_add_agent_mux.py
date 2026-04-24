# ══════════════════════════════════════════════════════════════
# agtalk/migrations/versions/006_add_agent_mux.py
# ══════════════════════════════════════════════════════════════
import sqlite3


def up(conn: sqlite3.Connection):
    """
    给 agents 表增加 mux 字段，记录 Agent 注册时的多路复用器类型。
    """
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")]
    if "agents" not in tables:
        return
    cols = [r[1] for r in conn.execute("PRAGMA table_info(agents)").fetchall()]
    if "mux" not in cols:
        conn.execute("ALTER TABLE agents ADD COLUMN mux TEXT DEFAULT ''")
        conn.execute("UPDATE agents SET mux = '' WHERE mux IS NULL")


def down(conn: sqlite3.Connection):
    pass
