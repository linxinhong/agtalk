# ══════════════════════════════════════════════════════════════
# agtalk/migrations/versions/005_add_agent_workdir.py
# ══════════════════════════════════════════════════════════════
import sqlite3


def up(conn: sqlite3.Connection):
    """
    给 agents 表增加 workdir 字段，记录 Agent 的工作目录。
    """
    conn.execute("ALTER TABLE agents ADD COLUMN workdir TEXT DEFAULT ''")
    conn.execute("UPDATE agents SET workdir = '' WHERE workdir IS NULL")


def down(conn: sqlite3.Connection):
    # SQLite 不支持 DROP COLUMN，回滚需重建表（开发环境可忽略）
    pass
