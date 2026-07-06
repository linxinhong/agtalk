//! mem_index 在线索引：daemon 侧按 address/name/workspace 快速查找 agent 记忆元信息。

use crate::storage::Storage;
use rusqlite::OptionalExtension;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct MemIndexRow {
    pub address: String,
    pub name: String,
    pub workspace: String,
    pub memory_path: String,
    pub plan_updated_at: f64,
    pub status_summary: String,
    pub public_topics: String,
}

impl MemIndexRow {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            address: row.get("address")?,
            name: row.get("name")?,
            workspace: row.get("workspace")?,
            memory_path: row.get("memory_path")?,
            plan_updated_at: row.get("plan_updated_at")?,
            status_summary: row.get("status_summary")?,
            public_topics: row.get("public_topics")?,
        })
    }
}

/// 注册或覆盖一条 mem_index 记录。
pub fn register(storage: &Storage, address: &str, name: &str, workspace: &str, memory_path: &Path) {
    let memory_path = memory_path.to_string_lossy().into_owned();
    let _ = storage.conn().execute(
        "INSERT OR REPLACE INTO mem_index \
         (address, name, workspace, memory_path, plan_updated_at, status_summary, public_topics) \
         VALUES (?1, ?2, ?3, ?4, 0, '', '')",
        rusqlite::params![address, name, workspace, memory_path],
    );
}

/// 刷新状态摘要与公开 topic 列表，并更新 plan_updated_at 为当前时间。
pub fn refresh(storage: &Storage, address: &str, status_summary: &str, public_topics: &[String]) {
    let topics = public_topics.join(",");
    let _ = storage.conn().execute(
        "UPDATE mem_index SET status_summary = ?2, public_topics = ?3, \
         plan_updated_at = unixepoch('subsec') WHERE address = ?1",
        rusqlite::params![address, status_summary, topics],
    );
}

/// 移除 mem_index 记录。
pub fn remove(storage: &Storage, address: &str) {
    let _ = storage
        .conn()
        .execute("DELETE FROM mem_index WHERE address = ?1", [address]);
}

/// 按 address 查找单条记录。
pub fn lookup_by_address(storage: &Storage, address: &str) -> Option<MemIndexRow> {
    storage
        .conn()
        .query_row(
            "SELECT * FROM mem_index WHERE address = ?1",
            [address],
            MemIndexRow::from_row,
        )
        .optional()
        .ok()
        .flatten()
}

/// 按 name 查找记录（name 不唯一，可能返回多条）。
pub fn lookup_by_name(storage: &Storage, name: &str) -> Vec<MemIndexRow> {
    let conn = storage.conn();
    let mut stmt = match conn.prepare("SELECT * FROM mem_index WHERE name = ?1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([name], MemIndexRow::from_row);
    rows.map(|iter| iter.filter_map(Result::ok).collect())
        .unwrap_or_default()
}
