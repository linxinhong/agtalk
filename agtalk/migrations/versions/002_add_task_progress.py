# ══════════════════════════════════════════════════════════════
# agtalk/migrations/versions/002_add_task_progress.py
# ══════════════════════════════════════════════════════════════
import sqlite3


def up(conn: sqlite3.Connection):
    """
    创建 task_progress 表，记录每个消息任务的进度快照。
    设计原则：只追加，不修改历史，完整保留每次进度变更记录。
    """

    # 1. 主表
    conn.execute("""
        CREATE TABLE IF NOT EXISTS task_progress (
            id         INTEGER  PRIMARY KEY AUTOINCREMENT,
            msg_id     TEXT     NOT NULL
                                REFERENCES messages(msg_id) ON DELETE CASCADE,
            percent    INTEGER  NOT NULL
                                CHECK (percent BETWEEN 0 AND 100),
            note       TEXT,
            created_at TEXT     NOT NULL
                                DEFAULT (datetime('now', 'localtime'))
        )
    """)

    # 2. 索引
    conn.execute("""
        CREATE INDEX IF NOT EXISTS idx_progress_msg_id
        ON task_progress (msg_id)
    """)
    conn.execute("""
        CREATE INDEX IF NOT EXISTS idx_progress_created_at
        ON task_progress (created_at DESC)
    """)


def down(conn: sqlite3.Connection):
    """回滚：删除 task_progress 表及相关索引。"""
    conn.execute("DROP INDEX IF EXISTS idx_progress_created_at")
    conn.execute("DROP INDEX IF EXISTS idx_progress_msg_id")
    conn.execute("DROP TABLE  IF EXISTS task_progress")
