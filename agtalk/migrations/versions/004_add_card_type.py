# ══════════════════════════════════════════════════════════════
# agtalk/migrations/versions/004_add_card_type.py
# ══════════════════════════════════════════════════════════════
import sqlite3


def up(conn: sqlite3.Connection):
    """
    给 kanban_cards 增加 type 字段，用于区分卡片(card)和公告(announcement)。
    """
    conn.execute("ALTER TABLE kanban_cards ADD COLUMN type TEXT DEFAULT 'card'")
    conn.execute("UPDATE kanban_cards SET type = 'card' WHERE type IS NULL")
    conn.execute("CREATE INDEX IF NOT EXISTS idx_kanban_type ON kanban_cards(type)")


def down(conn: sqlite3.Connection):
    conn.execute("DROP INDEX IF EXISTS idx_kanban_type")
    # SQLite 不支持 DROP COLUMN，回滚需重建表（开发环境可忽略）
