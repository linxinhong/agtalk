use rusqlite::Connection;

pub const CURRENT_VERSION: u32 = 11;

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

const MIGRATE_V9: &str = r#"
CREATE TABLE IF NOT EXISTS human_deliveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL REFERENCES messages(id),
    surface TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    external_ref TEXT DEFAULT NULL,
    error TEXT DEFAULT NULL,
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec')),
    updated_at REAL NOT NULL DEFAULT (unixepoch('subsec')),
    UNIQUE(message_id, surface)
);
CREATE INDEX IF NOT EXISTS idx_human_deliveries_surface_status ON human_deliveries(surface, status);

CREATE TABLE IF NOT EXISTS human_action_receipts (
    surface TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    message_id TEXT DEFAULT NULL,
    action TEXT NOT NULL,
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec')),
    PRIMARY KEY (surface, external_event_id)
);

CREATE TABLE IF NOT EXISTS approval_resolutions (
    request_message_id TEXT PRIMARY KEY REFERENCES messages(id),
    resolved_by TEXT NOT NULL,
    resolution TEXT NOT NULL,
    selected_choice TEXT DEFAULT NULL,
    response_message_id TEXT NOT NULL REFERENCES messages(id),
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);
"#;

// V10：图工程运行时（见 docs/design_graph.md §5.2）。
// 幂等键 graph_run_id+node_key+attempt 用 UNIQUE 约束保证；version 为乐观锁。
const MIGRATE_V10: &str = r#"
CREATE TABLE IF NOT EXISTS graph_runs (
    id TEXT PRIMARY KEY,
    goal TEXT NOT NULL,
    spec_snapshot TEXT NOT NULL,
    compiled_graph TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    repository TEXT,
    base_revision TEXT,
    integration_target TEXT,
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec')),
    started_at REAL,
    completed_at REAL,
    failure_reason TEXT
);

CREATE TABLE IF NOT EXISTS node_runs (
    id TEXT PRIMARY KEY,
    graph_run_id TEXT NOT NULL REFERENCES graph_runs(id),
    node_key TEXT NOT NULL,
    node_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt INTEGER NOT NULL DEFAULT 1,
    participant_id TEXT,
    workspace_id TEXT,
    lease_token TEXT,
    lease_expires_at REAL,
    input_artifact_ids TEXT NOT NULL DEFAULT '[]',
    output_artifact_ids TEXT NOT NULL DEFAULT '[]',
    started_at REAL,
    heartbeat_at REAL,
    completed_at REAL,
    failure_type TEXT,
    failure_detail TEXT,
    verification_summary TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    UNIQUE(graph_run_id, node_key, attempt)
);
CREATE INDEX IF NOT EXISTS idx_node_runs_graph ON node_runs(graph_run_id);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    graph_run_id TEXT NOT NULL REFERENCES graph_runs(id),
    producer_node_run_id TEXT REFERENCES node_runs(id),
    artifact_type TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    uri TEXT NOT NULL,
    checksum TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);
CREATE INDEX IF NOT EXISTS idx_artifacts_run ON artifacts(graph_run_id);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    graph_run_id TEXT NOT NULL REFERENCES graph_runs(id),
    owner_node_run_id TEXT REFERENCES node_runs(id),
    repository TEXT,
    base_revision TEXT,
    branch TEXT,
    path TEXT,
    status TEXT NOT NULL DEFAULT 'creating',
    dirty INTEGER NOT NULL DEFAULT 0,
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec')),
    released_at REAL
);

CREATE TABLE IF NOT EXISTS verifications (
    id TEXT PRIMARY KEY,
    node_run_id TEXT NOT NULL REFERENCES node_runs(id),
    verifier_type TEXT NOT NULL,
    rule TEXT,
    expected TEXT,
    actual TEXT,
    status TEXT NOT NULL,
    evidence_artifact_id TEXT REFERENCES artifacts(id),
    started_at REAL,
    completed_at REAL
);
CREATE INDEX IF NOT EXISTS idx_verifications_node ON verifications(node_run_id);

CREATE TABLE IF NOT EXISTS graph_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    graph_run_id TEXT NOT NULL REFERENCES graph_runs(id),
    event_type TEXT NOT NULL,
    node_key TEXT,
    payload TEXT NOT NULL DEFAULT '{}',
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);
CREATE INDEX IF NOT EXISTS idx_graph_events_run ON graph_events(graph_run_id);

CREATE TABLE IF NOT EXISTS graph_node_assignments (
    id TEXT PRIMARY KEY,
    node_run_id TEXT NOT NULL REFERENCES node_runs(id),
    message_id TEXT REFERENCES messages(id),
    kind TEXT NOT NULL,
    created_at REAL NOT NULL DEFAULT (unixepoch('subsec'))
);
CREATE INDEX IF NOT EXISTS idx_assignments_node ON graph_node_assignments(node_run_id);
"#;

// V11：workspaces 记录提交哈希（P1-2 worktree commit/merge 后可审计）。
const MIGRATE_V11: &str = r#"
ALTER TABLE workspaces ADD COLUMN commit_hash TEXT DEFAULT NULL;
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
    if version < 9 {
        tx.execute_batch(MIGRATE_V9)?;
    }
    if version < 10 {
        tx.execute_batch(MIGRATE_V10)?;
    }
    if version < 11 {
        tx.execute_batch(MIGRATE_V11)?;
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

        assert_eq!(max_migration(&conn), CURRENT_VERSION);
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

    fn table_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        count > 0
    }

    #[test]
    fn v8_to_v9_creates_human_tables_with_unique_constraints() {
        let mut conn = Connection::open_in_memory().unwrap();

        // 构造 v8 时代的数据库：SCHEMA_V1 + MIGRATE_V2..V8，但不包含 MIGRATE_V9。
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(MIGRATE_V2).unwrap();
        conn.execute_batch(MIGRATE_V3).unwrap();
        conn.execute_batch(MIGRATE_V4).unwrap();
        conn.execute_batch(MIGRATE_V5).unwrap();
        conn.execute_batch(MIGRATE_V6).unwrap();
        conn.execute_batch(MIGRATE_V7).unwrap();
        conn.execute_batch(MIGRATE_V8).unwrap();
        conn.execute("INSERT OR REPLACE INTO _migrations(version) VALUES (8)", [])
            .unwrap();

        conn.execute(
            "INSERT INTO mailboxes (address, name) VALUES ('h1', 'human')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, to_address, from_address, body, event_id) \
             VALUES ('m1', 'h1', 'h1', 'hello', 1)",
            [],
        )
        .unwrap();

        assert!(!table_exists(&conn, "human_deliveries"));
        assert!(!table_exists(&conn, "human_action_receipts"));
        assert!(!table_exists(&conn, "approval_resolutions"));
        assert_eq!(max_migration(&conn), 8);

        run(&mut conn).unwrap();

        assert_eq!(max_migration(&conn), CURRENT_VERSION);
        assert!(table_exists(&conn, "human_deliveries"));
        assert!(table_exists(&conn, "human_action_receipts"));
        assert!(table_exists(&conn, "approval_resolutions"));

        // human_deliveries: message_id + surface 唯一
        conn.execute(
            "INSERT INTO human_deliveries (message_id, surface) VALUES ('m1', 'popup')",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO human_deliveries (message_id, surface) VALUES ('m1', 'popup')",
            [],
        );
        assert!(dup.is_err(), "message_id + surface 应唯一");

        // human_action_receipts: surface + external_event_id 主键去重
        conn.execute(
            "INSERT INTO human_action_receipts (surface, external_event_id, action) \
             VALUES ('feishu', 'evt-1', 'reply')",
            [],
        )
        .unwrap();
        let dup_receipt = conn.execute(
            "INSERT INTO human_action_receipts (surface, external_event_id, action) \
             VALUES ('feishu', 'evt-1', 'reply')",
            [],
        );
        assert!(dup_receipt.is_err(), "surface + external_event_id 应唯一");

        // approval_resolutions: request_message_id 主键（跨端首个有效审批胜出）
        conn.execute(
            "INSERT INTO approval_resolutions (request_message_id, resolved_by, resolution, response_message_id) \
             VALUES ('m1', 'popup', 'text', 'm1')",
            [],
        )
        .unwrap();
        let dup_resolution = conn.execute(
            "INSERT INTO approval_resolutions (request_message_id, resolved_by, resolution, response_message_id) \
             VALUES ('m1', 'feishu', 'text', 'm1')",
            [],
        );
        assert!(dup_resolution.is_err(), "request_message_id 应唯一");
    }

    #[test]
    fn v9_to_v10_creates_graph_tables_with_idempotency_key() {
        let mut conn = Connection::open_in_memory().unwrap();

        // 构造 v9 时代的数据库：SCHEMA_V1 + MIGRATE_V2..V9，但不包含 MIGRATE_V10。
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(MIGRATE_V2).unwrap();
        conn.execute_batch(MIGRATE_V3).unwrap();
        conn.execute_batch(MIGRATE_V4).unwrap();
        conn.execute_batch(MIGRATE_V5).unwrap();
        conn.execute_batch(MIGRATE_V6).unwrap();
        conn.execute_batch(MIGRATE_V7).unwrap();
        conn.execute_batch(MIGRATE_V8).unwrap();
        conn.execute_batch(MIGRATE_V9).unwrap();
        conn.execute("INSERT OR REPLACE INTO _migrations(version) VALUES (9)", [])
            .unwrap();

        conn.execute(
            "INSERT INTO mailboxes (address, name) VALUES ('a1', 'alice')",
            [],
        )
        .unwrap();

        for t in [
            "graph_runs",
            "node_runs",
            "artifacts",
            "workspaces",
            "verifications",
            "graph_events",
            "graph_node_assignments",
        ] {
            assert!(!table_exists(&conn, t), "迁移前 {t} 表不应存在");
        }
        assert_eq!(max_migration(&conn), 9);

        run(&mut conn).unwrap();

        assert_eq!(max_migration(&conn), CURRENT_VERSION);
        for t in [
            "graph_runs",
            "node_runs",
            "artifacts",
            "workspaces",
            "verifications",
            "graph_events",
            "graph_node_assignments",
        ] {
            assert!(table_exists(&conn, t), "迁移后 {t} 表应存在");
        }

        // 幂等键：graph_run_id + node_key + attempt 唯一。
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph) \
             VALUES ('g1', 'goal', '{}', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO node_runs (id, graph_run_id, node_key, node_type) \
             VALUES ('n1', 'g1', 'impl', 'executor')",
            [],
        )
        .unwrap();
        // 同一节点 attempt=2 是合法的新执行尝试。
        conn.execute(
            "INSERT INTO node_runs (id, graph_run_id, node_key, node_type, attempt) \
             VALUES ('n2', 'g1', 'impl', 'executor', 2)",
            [],
        )
        .unwrap();
        // 完全相同的幂等键必须被拒绝。
        let dup = conn.execute(
            "INSERT INTO node_runs (id, graph_run_id, node_key, node_type, attempt) \
             VALUES ('n3', 'g1', 'impl', 'executor', 1)",
            [],
        );
        assert!(
            dup.is_err(),
            "graph_run_id+node_key+attempt 应唯一（幂等键）"
        );

        // 乐观锁默认 version=1。
        let version: i64 = conn
            .query_row("SELECT version FROM node_runs WHERE id='n1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 1, "node_runs.version 默认应为 1");

        // graph_events 单调自增。
        conn.execute(
            "INSERT INTO graph_events (graph_run_id, event_type) VALUES ('g1', 'graph_created')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO graph_events (graph_run_id, event_type) VALUES ('g1', 'graph_validated')",
            [],
        )
        .unwrap();
        let (e1, e2): (i64, i64) = conn
            .query_row("SELECT MIN(id), MAX(id) FROM graph_events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert!(e2 > e1, "graph_events.id 应单调递增");
    }
}
