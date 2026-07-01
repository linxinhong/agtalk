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
    pub created_at: f64,
    pub left_at: Option<f64>,
}

impl Mailbox {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            address: row.get("address")?,
            name: row.get("name")?,
            intro: row.get("intro")?,
            workspace: row.get("workspace")?,
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
    let address = Uuid::new_v4().to_string();
    let mut conn = storage.conn();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO mailboxes (address, name, intro, workspace) VALUES (?1, ?2, ?3, ?4)",
        params![address, name, intro, workspace],
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

/// 将 mailbox 标记为已离开（软删除），保留历史消息。
pub fn mark_left(storage: &Storage, address: &str) -> Result<(), IdentityError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE mailboxes SET left_at = unixepoch('subsec') WHERE address = ?1 AND left_at IS NULL",
        [address],
    )?;
    Ok(())
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
        let cfg = HumanConfig {
            name: "human".to_string(),
            intro: "人类收件箱".to_string(),
        };
        let a = ensure_human(&storage, &cfg).unwrap();
        let b = ensure_human(&storage, &cfg).unwrap();
        assert_eq!(a, b);
    }
}
