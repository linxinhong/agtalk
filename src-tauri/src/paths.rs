//! 全局路径解析：~/.config/agtalk2

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathsError {
    #[error("无法定位用户配置目录")]
    NoConfigDir,
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

const CONFIG_DIR_ENV: &str = "AGTALK_CONFIG_DIR";

/// 全局配置目录：~/.config/agtalk2
/// 可通过环境变量 `AGTALK_CONFIG_DIR` 覆盖，用于测试或多实例。
pub fn config_dir() -> Result<PathBuf, PathsError> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let dir = dirs::config_dir()
        .ok_or(PathsError::NoConfigDir)?
        .join("agtalk2");
    Ok(dir)
}

/// 确保配置目录存在，权限 0700
pub fn ensure_config_dir() -> Result<PathBuf, PathsError> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// 数据库路径：~/.config/agtalk2/agtalk.db
pub fn db_path() -> Result<PathBuf, PathsError> {
    Ok(config_dir()?.join("agtalk.db"))
}

/// 配置文件路径：~/.config/agtalk2/config.json
pub fn config_path() -> Result<PathBuf, PathsError> {
    Ok(config_dir()?.join("config.json"))
}

/// daemon PID 锁文件路径：~/.config/agtalk2/daemon.pid
pub fn daemon_pid_path() -> Result<PathBuf, PathsError> {
    Ok(config_dir()?.join("daemon.pid"))
}

/// 设置文件权限 0600
pub fn set_permissions_0600(path: &Path) -> Result<(), PathsError> {
    #[cfg(unix)]
    {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// 设置目录权限 0700
pub fn set_permissions_0700(path: &Path) -> Result<(), PathsError> {
    #[cfg(unix)]
    {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_agtalk2() {
        let dir = config_dir().unwrap();
        assert_eq!(dir.file_name().unwrap(), "agtalk2");
    }
}
