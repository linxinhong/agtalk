//! mailbox 数据模型与 DB 查询。

use super::IdentityError;
use crate::config::HumanConfig;
use crate::storage::Storage;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mailbox {
    pub address: String,
    pub name: String,
    pub intro: String,
    pub workspace: String,
    pub workspace_root: String,
    pub notify_channel: String,
    pub notify_target: serde_json::Value,
    pub created_at: f64,
    pub left_at: Option<f64>,
}

impl Mailbox {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        let notify_target_str: String = row.get("notify_target")?;
        let notify_target = serde_json::from_str(&notify_target_str).unwrap_or_default();
        Ok(Self {
            address: row.get("address")?,
            name: row.get("name")?,
            intro: row.get("intro")?,
            workspace: row.get("workspace")?,
            workspace_root: row.get("workspace_root")?,
            notify_channel: row.get("notify_channel")?,
            notify_target,
            created_at: row.get("created_at")?,
            left_at: row.get("left_at")?,
        })
    }
}

/// 创建 mailbox，返回 address UUID。
pub fn create(
    storage: &Storage,
    name: &str,
    intro: &str,
    workspace: &str,
) -> Result<String, IdentityError> {
    create_with_notify(
        storage,
        name,
        intro,
        workspace,
        "none",
        &serde_json::Value::Null,
        "",
    )
}

/// 创建 mailbox 并同时设置 notify 配置。
#[allow(clippy::too_many_arguments)]
pub fn create_with_notify(
    storage: &Storage,
    name: &str,
    intro: &str,
    workspace: &str,
    notify_channel: &str,
    notify_target: &serde_json::Value,
    workspace_root: &str,
) -> Result<String, IdentityError> {
    let address = Uuid::new_v4().to_string();
    let mut conn = storage.conn();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO mailboxes (address, name, intro, workspace, notify_channel, notify_target, workspace_root) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            address,
            name,
            intro,
            workspace,
            notify_channel,
            notify_target.to_string(),
            workspace_root,
        ],
    )?;
    tx.execute(
        "INSERT INTO event_sequences (address, last_event_id) VALUES (?1, 0)",
        [&address],
    )?;
    tx.commit()?;
    Ok(address)
}

/// 按 address 查询活跃 mailbox（未 leave）。
pub fn get_by_address(storage: &Storage, address: &str) -> Result<Option<Mailbox>, IdentityError> {
    let conn = storage.conn();
    let mut stmt =
        conn.prepare("SELECT * FROM mailboxes WHERE address = ?1 AND left_at IS NULL")?;
    let mb = stmt.query_row([address], Mailbox::from_row).optional()?;
    Ok(mb)
}

/// 按 address 查询 mailbox，包含已 leave。
pub fn get_including_left(
    storage: &Storage,
    address: &str,
) -> Result<Option<Mailbox>, IdentityError> {
    let conn = storage.conn();
    let mut stmt = conn.prepare("SELECT * FROM mailboxes WHERE address = ?1")?;
    let mb = stmt.query_row([address], Mailbox::from_row).optional()?;
    Ok(mb)
}

/// 按 name 过滤查询活跃 mailbox；name 为空时返回全部活跃。
pub fn list(storage: &Storage, name_filter: Option<&str>) -> Result<Vec<Mailbox>, IdentityError> {
    let conn = storage.conn();
    let rows: Vec<Mailbox> = if let Some(name) = name_filter {
        let mut stmt = conn.prepare(
            "SELECT * FROM mailboxes WHERE name = ?1 AND left_at IS NULL ORDER BY created_at",
        )?;
        let mapped = stmt.query_map([name], Mailbox::from_row)?;
        mapped.collect::<Result<_, _>>()?
    } else {
        let mut stmt =
            conn.prepare("SELECT * FROM mailboxes WHERE left_at IS NULL ORDER BY created_at")?;
        let mapped = stmt.query_map([], Mailbox::from_row)?;
        mapped.collect::<Result<_, _>>()?
    };
    Ok(rows)
}

/// 返回所有 mailbox（含已 leave），用于 cleanup 等管理操作。
pub fn list_including_left(storage: &Storage) -> Result<Vec<Mailbox>, IdentityError> {
    let conn = storage.conn();
    let mut stmt = conn.prepare("SELECT * FROM mailboxes ORDER BY created_at")?;
    let mapped = stmt.query_map([], Mailbox::from_row)?;
    Ok(mapped.collect::<Result<_, _>>()?)
}

/// 将 mailbox 标记为已离开（软删除），保留历史消息。
pub fn mark_left(storage: &Storage, address: &str) -> Result<(), IdentityError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE mailboxes SET left_at = unixepoch('subsec') WHERE address = ?1 AND left_at IS NULL",
        [address],
    )?;
    Ok(())
}

/// 更新活跃 mailbox 的展示元数据。
pub fn update(
    storage: &Storage,
    address: &str,
    name: &str,
    intro: &str,
    workspace: &str,
) -> Result<(), IdentityError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE mailboxes SET name = ?2, intro = ?3, workspace = ?4 WHERE address = ?1 AND left_at IS NULL",
        params![address, name, intro, workspace],
    )?;
    Ok(())
}

/// 单独更新 mailbox 的 notify 配置。
pub fn set_notify(
    storage: &Storage,
    address: &str,
    notify_channel: &str,
    notify_target: &serde_json::Value,
) -> Result<(), IdentityError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE mailboxes SET notify_channel = ?2, notify_target = ?3 WHERE address = ?1",
        params![address, notify_channel, notify_target.to_string()],
    )?;
    Ok(())
}

/// 恢复一个已 leave 或可能缺失的 mailbox。
/// 如果 address 不存在则创建；如果存在但 left_at 不为空则清空 left_at 并更新元数据；
/// 如果存在且活跃则仅更新元数据。同时确保 event_sequences 存在。
pub fn revive(
    storage: &Storage,
    address: &str,
    name: &str,
    intro: &str,
    workspace: &str,
) -> Result<(), IdentityError> {
    revive_with_notify(
        storage,
        address,
        name,
        intro,
        workspace,
        "none",
        &serde_json::Value::Null,
        "",
    )
}

/// 恢复 mailbox 并同时设置 notify 配置。
#[allow(clippy::too_many_arguments)]
pub fn revive_with_notify(
    storage: &Storage,
    address: &str,
    name: &str,
    intro: &str,
    workspace: &str,
    notify_channel: &str,
    notify_target: &serde_json::Value,
    workspace_root: &str,
) -> Result<(), IdentityError> {
    let mut conn = storage.conn();
    let tx = conn.transaction()?;

    let exists: bool = tx
        .query_row(
            "SELECT 1 FROM mailboxes WHERE address = ?1",
            [address],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);

    if exists {
        tx.execute(
            "UPDATE mailboxes SET name = ?2, intro = ?3, workspace = ?4, left_at = NULL, notify_channel = ?5, notify_target = ?6, workspace_root = ?7 WHERE address = ?1",
            params![
                address,
                name,
                intro,
                workspace,
                notify_channel,
                notify_target.to_string(),
                workspace_root,
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO mailboxes (address, name, intro, workspace, notify_channel, notify_target, workspace_root) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                address,
                name,
                intro,
                workspace,
                notify_channel,
                notify_target.to_string(),
                workspace_root,
            ],
        )?;
    }

    tx.execute(
        "INSERT OR IGNORE INTO event_sequences (address, last_event_id) VALUES (?1, 0)",
        [address],
    )?;

    tx.commit()?;
    Ok(())
}

/// 统计未 leave 的 mailbox 数量。
pub fn count_active(storage: &Storage) -> Result<i64, IdentityError> {
    let conn = storage.conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM mailboxes WHERE left_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(count)
}

/// 物理删除 mailbox（仅应在无历史消息关联时使用，如测试清理）。
pub fn delete(storage: &Storage, address: &str) -> Result<(), IdentityError> {
    let conn = storage.conn();
    conn.execute("DELETE FROM mailboxes WHERE address = ?1", [address])?;
    Ok(())
}

/// 获取/创建 human mailbox，返回 address。
pub fn ensure_human(storage: &Storage, cfg: &HumanConfig) -> Result<String, IdentityError> {
    let existing: Option<String> = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT address FROM system_mailboxes WHERE role = 'human'",
            [],
            |row| row.get(0),
        )
        .optional()?
    };

    if let Some(addr) = existing {
        return Ok(addr);
    }

    let address = Uuid::new_v4().to_string();
    let mut conn = storage.conn();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO mailboxes (address, name, intro) VALUES (?1, ?2, ?3)",
        params![address, cfg.name, cfg.intro],
    )?;
    tx.execute(
        "INSERT INTO event_sequences (address, last_event_id) VALUES (?1, 0)",
        [&address],
    )?;
    tx.execute(
        "INSERT INTO system_mailboxes (role, address) VALUES ('human', ?1)",
        [&address],
    )?;
    tx.commit()?;
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[test]
    fn create_and_get_mailbox() {
        let storage = Storage::open_in_memory().unwrap();
        let addr = create(&storage, "nora", "前端 review", "projA").unwrap();
        let mb = get_by_address(&storage, &addr).unwrap().unwrap();
        assert_eq!(mb.name, "nora");
        assert_eq!(mb.workspace, "projA");
    }

    #[test]
    fn list_filters_by_name() {
        let storage = Storage::open_in_memory().unwrap();
        create(&storage, "nora", "前端", "projA").unwrap();
        create(&storage, "nora", "后端", "projB").unwrap();
        create(&storage, "quinn", "设计", "projA").unwrap();
        let all = list(&storage, None).unwrap();
        assert_eq!(all.len(), 3);
        let noras = list(&storage, Some("nora")).unwrap();
        assert_eq!(noras.len(), 2);
    }

    #[test]
    fn ensure_human_creates_once() {
        let storage = Storage::open_in_memory().unwrap();
        let cfg = HumanConfig::default();
        let a = ensure_human(&storage, &cfg).unwrap();
        let b = ensure_human(&storage, &cfg).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn revive_creates_missing_mailbox() {
        let storage = Storage::open_in_memory().unwrap();
        revive(&storage, "addr-1", "nora", "前端", "projA").unwrap();
        let mb = get_by_address(&storage, "addr-1").unwrap().unwrap();
        assert_eq!(mb.name, "nora");
    }

    #[test]
    fn revive_restores_left_mailbox() {
        let storage = Storage::open_in_memory().unwrap();
        let addr = create(&storage, "nora", "前端", "projA").unwrap();
        mark_left(&storage, &addr).unwrap();
        assert!(get_by_address(&storage, &addr).unwrap().is_none());
        revive(&storage, &addr, "nora", "后端", "projB").unwrap();
        let mb = get_by_address(&storage, &addr).unwrap().unwrap();
        assert_eq!(mb.intro, "后端");
        assert_eq!(mb.workspace, "projB");
        assert!(mb.left_at.is_none());
    }

    #[test]
    fn update_changes_metadata() {
        let storage = Storage::open_in_memory().unwrap();
        let addr = create(&storage, "nora", "前端", "projA").unwrap();
        update(&storage, &addr, "nora", "后端", "projB").unwrap();
        let mb = get_by_address(&storage, &addr).unwrap().unwrap();
        assert_eq!(mb.intro, "后端");
        assert_eq!(mb.workspace, "projB");
    }

    #[test]
    fn create_with_notify_persists_workspace_root() {
        let storage = Storage::open_in_memory().unwrap();
        let addr = create_with_notify(
            &storage,
            "nora",
            "前端",
            "",
            "none",
            &serde_json::Value::Null,
            "/tmp/agentA/.agtalk",
        )
        .unwrap();
        let mb = get_by_address(&storage, &addr).unwrap().unwrap();
        assert_eq!(mb.workspace_root, "/tmp/agentA/.agtalk");
    }

    #[test]
    fn revive_with_notify_refreshes_workspace_root() {
        let storage = Storage::open_in_memory().unwrap();
        let addr = create_with_notify(
            &storage,
            "nora",
            "前端",
            "",
            "none",
            &serde_json::Value::Null,
            "/tmp/old/.agtalk",
        )
        .unwrap();
        revive_with_notify(
            &storage,
            &addr,
            "nora",
            "前端",
            "",
            "none",
            &serde_json::Value::Null,
            "/tmp/new/.agtalk",
        )
        .unwrap();
        let mb = get_by_address(&storage, &addr).unwrap().unwrap();
        assert_eq!(mb.workspace_root, "/tmp/new/.agtalk");
    }

    #[test]
    fn create_wrapper_defaults_workspace_root_empty() {
        let storage = Storage::open_in_memory().unwrap();
        let addr = create(&storage, "nora", "前端", "projA").unwrap();
        let mb = get_by_address(&storage, &addr).unwrap().unwrap();
        assert_eq!(mb.workspace_root, "");
    }
}
