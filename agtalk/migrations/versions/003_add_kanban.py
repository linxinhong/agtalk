# ══════════════════════════════════════════════════════════════
# agtalk/migrations/versions/003_add_kanban.py
# ══════════════════════════════════════════════════════════════
import sqlite3


def up(conn: sqlite3.Connection):
    """
    创建看板卡片表和评论表。
    """

    conn.execute("""
        CREATE TABLE IF NOT EXISTS kanban_cards (
            card_id     TEXT PRIMARY KEY,
            title       TEXT NOT NULL,
            body        TEXT NOT NULL,
            author      TEXT DEFAULT '',
            status      TEXT NOT NULL DEFAULT 'open',
            created_at  REAL NOT NULL DEFAULT (unixepoch('now','subsec')),
            updated_at  REAL NOT NULL DEFAULT (unixepoch('now','subsec'))
        )
    """)

    conn.execute("""
        CREATE TABLE IF NOT EXISTS kanban_comments (
            comment_id   TEXT PRIMARY KEY,
            card_id      TEXT NOT NULL REFERENCES kanban_cards(card_id) ON DELETE CASCADE,
            author       TEXT DEFAULT '',
            body         TEXT NOT NULL,
            created_at   REAL NOT NULL DEFAULT (unixepoch('now','subsec'))
        )
    """)

    conn.execute("""
        CREATE INDEX IF NOT EXISTS idx_kanban_status ON kanban_cards(status)
    """)
    conn.execute("""
        CREATE INDEX IF NOT EXISTS idx_kanban_comments ON kanban_comments(card_id)
    """)


def down(conn: sqlite3.Connection):
    conn.execute("DROP INDEX IF EXISTS idx_kanban_comments")
    conn.execute("DROP INDEX IF EXISTS idx_kanban_status")
    conn.execute("DROP TABLE IF EXISTS kanban_comments")
    conn.execute("DROP TABLE IF EXISTS kanban_cards")
