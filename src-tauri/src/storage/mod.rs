//! SQLite 存储层：只负责连接与迁移，业务查询分散到各领域模块。

use crate::paths::{db_path, ensure_config_dir, set_permissions_0600, PathsError};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;

pub mod migrate;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("路径错误: {0}")]
    Paths(#[from] PathsError),
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("迁移失败: {0}")]
    Migrate(String),
}

#[derive(Clone)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
}

impl Storage {
    /// 打开 ~/.config/agtalk2/agtalk.db
    pub fn open() -> Result<Self, StorageError> {
        Self::open_with_path(&db_path()?)
    }

    /// 内存数据库，仅用于测试。
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.migrate()?;
        Ok(storage)
    }

    fn open_with_path<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        ensure_config_dir()?;
        let conn = Connection::open(path.as_ref())?;
        set_permissions_0600(path.as_ref())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        migrate::run(&mut conn)?;
        Ok(())
    }

    /// 获取数据库连接锁；调用方负责尽快释放。
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_db_migrates() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(tables.contains(&"mailboxes".to_string()));
        assert!(tables.contains(&"messages".to_string()));
        assert!(tables.contains(&"_migrations".to_string()));
    }

    #[test]
    fn migration_version_recorded() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        let version: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, migrate::CURRENT_VERSION);
    }

    #[test]
    fn messages_have_subject_column() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(messages)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|c| c == "subject"));
    }
}
