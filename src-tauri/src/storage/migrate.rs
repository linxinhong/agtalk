use rusqlite::Connection;

pub const CURRENT_VERSION: u32 = 3;

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

    tx.execute(
        "INSERT OR REPLACE INTO _migrations(version) VALUES (?1)",
        [CURRENT_VERSION],
    )?;

    tx.commit()?;
    Ok(())
}
