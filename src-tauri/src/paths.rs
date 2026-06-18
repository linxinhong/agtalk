//! 统一的配置路径。避免 macOS 上 dirs::config_dir() 返回 ~/Library/Application Support/
//! 带来的权限问题，统一使用 ~/.config/agtalk2。

use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    // 优先用 XDG_CONFIG_HOME，否则用 ~/.config
    if let Ok(dir) = std::env::var("AGTALK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("agtalk2");
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("agtalk2")
}

pub fn db_path() -> PathBuf {
    config_dir().join("talk.db")
}

pub fn socket_path() -> String {
    config_dir().join("daemon.sock").to_string_lossy().to_string()
}

pub fn pid_path() -> std::path::PathBuf {
    config_dir().join("daemon.pid")
}

pub fn current_participant_path() -> std::path::PathBuf {
    config_dir().join("current_participant")
}
