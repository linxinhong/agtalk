//! 统一的配置路径。避免 macOS 上 dirs::config_dir() 返回 ~/Library/Application Support/
//! 带来的权限问题，统一使用 ~/.config/agtalk。

use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    // 优先用 XDG_CONFIG_HOME，否则用 ~/.config
    if let Ok(dir) = std::env::var("AGTALK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("agtalk");
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("agtalk")
}

pub fn db_path() -> PathBuf {
    config_dir().join("agtalk.db")
}

pub fn config_json_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn socket_path() -> String {
    config_dir()
        .join("daemon.sock")
        .to_string_lossy()
        .to_string()
}

pub fn pid_path() -> std::path::PathBuf {
    config_dir().join("daemon.pid")
}

// ── 项目本地路径（.agtalk/ 凭证目录）──────────────────

pub const AGTALK_DIR_NAME: &str = ".agtalk";

/// 从 CWD 向上查找 .agtalk/ 目录，返回项目根。
/// 优先级：AGTALK_ROOT 环境变量 > CWD 向上遍历。
pub fn find_agtalk_root() -> anyhow::Result<PathBuf> {
    if let Ok(root) = std::env::var("AGTALK_ROOT") {
        let root = root.trim();
        if !root.is_empty() {
            return Ok(PathBuf::from(root));
        }
    }
    let mut cur =
        std::env::current_dir().map_err(|e| anyhow::anyhow!("无法获取当前目录: {}", e))?;
    loop {
        if cur.join(AGTALK_DIR_NAME).is_dir() {
            return Ok(cur);
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => anyhow::bail!(
                "不在 agtalk workspace 内（未找到 .agtalk/，可设置 AGTALK_ROOT 环境变量）"
            ),
        }
    }
}

/// <root>/.agtalk/
pub fn agtalk_dir() -> anyhow::Result<PathBuf> {
    Ok(find_agtalk_root()?.join(AGTALK_DIR_NAME))
}

/// <root>/.agtalk/workspace.json
pub fn workspace_json_path() -> anyhow::Result<PathBuf> {
    Ok(agtalk_dir()?.join("workspace.json"))
}

/// <root>/.agtalk/sessions/
pub fn sessions_dir() -> anyhow::Result<PathBuf> {
    Ok(agtalk_dir()?.join("sessions"))
}

/// <root>/.agtalk/sessions/<name>.json（校验 name 无路径分隔符，防注入）
pub fn session_json_path(name: &str) -> anyhow::Result<PathBuf> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == ".." || name == "." {
        anyhow::bail!("非法 session 名: {:?}", name);
    }
    Ok(sessions_dir()?.join(format!("{}.json", name)))
}

/// daemon 启动生成的引导 token（join 等管理类 RPC 握手用）
#[allow(dead_code)]
pub fn bootstrap_token_path() -> PathBuf {
    config_dir().join("bootstrap.token")
}
