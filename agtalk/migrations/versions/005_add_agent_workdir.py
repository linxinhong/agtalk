# ══════════════════════════════════════════════════════════════
# agtalk/migrations/versions/005_add_agent_workdir.py
# ══════════════════════════════════════════════════════════════
import sqlite3


def up(conn: sqlite3.Connection):
    """
    给 agents 表增加 workdir 字段，记录 Agent 的工作目录。
    """
    # 检查 agents 表是否存在（新数据库中 _migrate_v1 尚未执行）
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")]
    if "agents" not in tables:
        return  # 表不存在，跳过（_migrate_v1 会创建含 workdir 的表）
    cols = [r[1] for r in conn.execute("PRAGMA table_info(agents)").fetchall()]
    if "workdir" not in cols:
        conn.execute("ALTER TABLE agents ADD COLUMN workdir TEXT DEFAULT ''")
        conn.execute("UPDATE agents SET workdir = '' WHERE workdir IS NULL")


def down(conn: sqlite3.Connection):
    # SQLite 不支持 DROP COLUMN，回滚需重建表（开发环境可忽略）
    pass
