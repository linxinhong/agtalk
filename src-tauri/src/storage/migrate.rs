use rusqlite::Connection;

pub const CURRENT_VERSION: u32 = 8;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS _migrations (
    version INTEGER PRIMARY KEY,
    applied_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE TABLE IF NOT EXISTS system_mailboxes (
    role TEXT PRIMARY KEY,
    address TEXT NOT NULL UNIQUE REFERENCES mailboxes(address)
);

CREATE TABLE IF NOT EXISTS mailboxes (
    address TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    intro TEXT NOT NULL DEFAULT '',
    workspace TEXT NOT NULL DEFAULT '',
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE TABLE IF NOT EXISTS event_sequences (
    address TEXT PRIMARY KEY REFERENCES mailboxes(address),
    last_event_id INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    to_address TEXT NOT NULL REFERENCES mailboxes(address),
    from_address TEXT NOT NULL REFERENCES mailboxes(address),
    body TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'text',
    reply_to_id TEXT REFERENCES messages(id),
    metadata TEXT NOT NULL DEFAULT '{}',
    event_id INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec')),
    UNIQUE(to_address, event_id)
);

CREATE TABLE IF NOT EXISTS message_status_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL REFERENCES messages(id),
    old_status TEXT,
    new_status TEXT NOT NULL,
    changed_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE INDEX IF NOT EXISTS idx_messages_to_address ON messages(to_address);
CREATE INDEX IF NOT EXISTS idx_messages_event_id ON messages(to_address, event_id);
CREATE INDEX IF NOT EXISTS idx_messages_reply_to ON messages(reply_to_id);
"#;

const MIGRATE_V2: &str = r#"
ALTER TABLE mailboxes ADD COLUMN left_at REAL DEFAULT NULL;
CREATE INDEX IF NOT EXISTS idx_mailboxes_left_at ON mailboxes(left_at);
"#;

const MIGRATE_V3: &str = r#"
ALTER TABLE messages ADD COLUMN to_name TEXT NOT NULL DEFAULT '';
ALTER TABLE messages ADD COLUMN from_name TEXT NOT NULL DEFAULT '';
"#;

const MIGRATE_V4: &str = r#"
CREATE TABLE IF NOT EXISTS browser_sessions (
    address TEXT PRIMARY KEY REFERENCES mailboxes(address),
    token TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);
CREATE INDEX IF NOT EXISTS idx_browser_sessions_token ON browser_sessions(token);
"#;

const MIGRATE_V5: &str = r#"
CREATE TABLE IF NOT EXISTS mem_index (
    address TEXT PRIMARY KEY REFERENCES mailboxes(address),
    name TEXT NOT NULL,
    workspace TEXT NOT NULL DEFAULT '',
    memory_path TEXT NOT NULL,
    plan_updated_at REAL NOT NULL DEFAULT 0,
    status_summary TEXT NOT NULL DEFAULT '',
    public_topics TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_mem_index_name ON mem_index(name);
CREATE INDEX IF NOT EXISTS idx_mem_index_workspace ON mem_index(workspace);
"#;

const MIGRATE_V6: &str = r#"
ALTER TABLE mailboxes ADD COLUMN notify_channel TEXT NOT NULL DEFAULT '';
ALTER TABLE mailboxes ADD COLUMN notify_target TEXT NOT NULL DEFAULT '{}';
"#;

const MIGRATE_V7: &str = r#"
ALTER TABLE messages ADD COLUMN subject TEXT DEFAULT NULL;
"#;

const MIGRATE_V8: &str = r#"
ALTER TABLE mailboxes ADD COLUMN workspace_root TEXT NOT NULL DEFAULT '';
"#;

pub fn run(conn: &mut Connection) -> Result<(), super::StorageError> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (\
         version INTEGER PRIMARY KEY, \
         applied_at REAL NOT NULL DEFAULT (unixepoch('subsec')))",
    )?;

    let version: u32 = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if version < 1 {
        tx.execute_batch(SCHEMA_V1)?;
    }
    if version < 2 {
        tx.execute_batch(MIGRATE_V2)?;
    }
    if version < 3 {
        tx.execute_batch(MIGRATE_V3)?;
    }
    if version < 4 {
        tx.execute_batch(MIGRATE_V4)?;
    }
    if version < 5 {
        tx.execute_batch(MIGRATE_V5)?;
    }
    if version < 6 {
        tx.execute_batch(MIGRATE_V6)?;
    }
    if version < 7 {
        tx.execute_batch(MIGRATE_V7)?;
    }
    if version < 8 {
        tx.execute_batch(MIGRATE_V8)?;
    }

    tx.execute(
        "INSERT OR REPLACE INTO _migrations(version) VALUES (?1)",
        [CURRENT_VERSION],
    )?;

    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_columns(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("PRAGMA table_info(messages)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn max_migration(conn: &Connection) -> u32 {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
    }

    #[test]
    fn v6_to_v7_adds_subject_column_and_preserves_rows() {
        let mut conn = Connection::open_in_memory().unwrap();

        // 构造 v6 时代的数据库：SCHEMA_V1 + MIGRATE_V2..V6，但不包含 MIGRATE_V7。
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(MIGRATE_V2).unwrap();
        conn.execute_batch(MIGRATE_V3).unwrap();
        conn.execute_batch(MIGRATE_V4).unwrap();
        conn.execute_batch(MIGRATE_V5).unwrap();
        conn.execute_batch(MIGRATE_V6).unwrap();
        conn.execute("INSERT OR REPLACE INTO _migrations(version) VALUES (6)", [])
            .unwrap();

        // 一条 v6 时代的 mailbox + message（不带 subject 列）。
        conn.execute(
            "INSERT INTO mailboxes (address, name) VALUES ('a1', 'alice')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, to_address, from_address, body, event_id) \
             VALUES ('m1', 'a1', 'a1', 'hello', 1)",
            [],
        )
        .unwrap();

        assert!(
            !message_columns(&conn).iter().any(|c| c == "subject"),
            "迁移前 messages 表不应存在 subject 列"
        );
        assert_eq!(max_migration(&conn), 6);

        run(&mut conn).unwrap();

        assert_eq!(max_migration(&conn), CURRENT_VERSION);
        assert!(
            message_columns(&conn).iter().any(|c| c == "subject"),
            "迁移后 messages 表应新增 subject 列"
        );

        // 旧行的 subject 应为 NULL（列新增、不强制回填）。
        let subject: Option<String> = conn
            .query_row("SELECT subject FROM messages WHERE id = 'm1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(subject.is_none(), "旧 message 行的 subject 应保持 NULL");
    }

    fn mailbox_columns(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("PRAGMA table_info(mailboxes)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn v7_to_v8_adds_workspace_root_column_with_empty_default() {
        let mut conn = Connection::open_in_memory().unwrap();

        // 构造 v7 时代的数据库：SCHEMA_V1 + MIGRATE_V2..V7，但不包含 MIGRATE_V8。
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(MIGRATE_V2).unwrap();
        conn.execute_batch(MIGRATE_V3).unwrap();
        conn.execute_batch(MIGRATE_V4).unwrap();
        conn.execute_batch(MIGRATE_V5).unwrap();
        conn.execute_batch(MIGRATE_V6).unwrap();
        conn.execute_batch(MIGRATE_V7).unwrap();
        conn.execute("INSERT OR REPLACE INTO _migrations(version) VALUES (7)", [])
            .unwrap();

        conn.execute(
            "INSERT INTO mailboxes (address, name) VALUES ('a1', 'alice')",
            [],
        )
        .unwrap();

        assert!(
            !mailbox_columns(&conn).iter().any(|c| c == "workspace_root"),
            "迁移前 mailboxes 表不应存在 workspace_root 列"
        );
        assert_eq!(max_migration(&conn), 7);

        run(&mut conn).unwrap();

        assert_eq!(max_migration(&conn), 8);
        assert!(
            mailbox_columns(&conn).iter().any(|c| c == "workspace_root"),
            "迁移后 mailboxes 表应新增 workspace_root 列"
        );

        // 旧行的 workspace_root 应为空串（NOT NULL DEFAULT ''）。
        let root: String = conn
            .query_row(
                "SELECT workspace_root FROM mailboxes WHERE address = 'a1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(root, "", "旧 mailbox 行的 workspace_root 应为空串");
    }
}
