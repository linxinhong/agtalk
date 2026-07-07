//! agents.json：pid -> { name, start_time } 映射。

use super::IdentityError;
use crate::paths::set_permissions_0600;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentsMap {
    pub agents: HashMap<String, AgentEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub name: String,
    pub start_time: u64,
}

fn agents_path(dot_agtalk: &Path) -> PathBuf {
    dot_agtalk.join("agents.json")
}

pub fn read(dot_agtalk: &Path) -> Result<AgentsMap, IdentityError> {
    let path = agents_path(dot_agtalk);
    if !path.exists() {
        return Ok(AgentsMap::default());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&content)?)
}

pub fn write(dot_agtalk: &Path, map: &AgentsMap) -> Result<(), IdentityError> {
    std::fs::create_dir_all(dot_agtalk)?;
    let path = agents_path(dot_agtalk);
    let content = serde_json::to_string_pretty(map)?;
    std::fs::write(&path, content)?;
    set_permissions_0600(&path)?;
    Ok(())
}

pub fn register_pid(
    dot_agtalk: &Path,
    pid: u32,
    name: &str,
    start_time: u64,
) -> Result<(), IdentityError> {
    let mut map = read(dot_agtalk)?;
    map.agents.insert(
        pid.to_string(),
        AgentEntry {
            name: name.to_string(),
            start_time,
        },
    );
    write(dot_agtalk, &map)
}

pub fn remove_pid(dot_agtalk: &Path, pid: u32) -> Result<(), IdentityError> {
    let mut map = read(dot_agtalk)?;
    map.agents.remove(&pid.to_string());
    write(dot_agtalk, &map)
}

/// 删除 agents.json 中所有指向指定 name 的 pid entry。
pub fn remove_by_name(dot_agtalk: &Path, name: &str) -> Result<(), IdentityError> {
    let mut map = read(dot_agtalk)?;
    map.agents.retain(|_, entry| entry.name != name);
    write(dot_agtalk, &map)
}

pub fn get_by_pid(dot_agtalk: &Path, pid: u32) -> Result<Option<AgentEntry>, IdentityError> {
    let map = read(dot_agtalk)?;
    Ok(map.agents.get(&pid.to_string()).cloned())
}

/// 删除 agents.json 中所有 session 已经不存在的 pid entry。
/// `valid_names` 是 `.agtalk/` 下仍有 session.json 的 agent name 集合。
/// 返回被删除的 (pid, name) 列表。
pub fn cleanup_stale_pids(
    dot_agtalk: &Path,
    valid_names: &[String],
) -> Result<Vec<(u32, String)>, IdentityError> {
    let mut map = read(dot_agtalk)?;
    let valid: std::collections::HashSet<_> = valid_names.iter().cloned().collect();
    let mut removed = Vec::new();
    map.agents.retain(|pid_str, entry| {
        if valid.contains(&entry.name) {
            true
        } else {
            if let Ok(pid) = pid_str.parse::<u32>() {
                removed.push((pid, entry.name.clone()));
            }
            false
        }
    });
    write(dot_agtalk, &map)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn register_and_lookup_pid() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        register_pid(&dot, 12345, "nora", 1_700_000_000).unwrap();
        let entry = get_by_pid(&dot, 12345).unwrap().unwrap();
        assert_eq!(entry.name, "nora");
        assert_eq!(entry.start_time, 1_700_000_000);
    }

    #[test]
    fn remove_pid_entry() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        register_pid(&dot, 12345, "nora", 1_700_000_000).unwrap();
        super::remove_pid(&dot, 12345).unwrap();
        assert!(get_by_pid(&dot, 12345).unwrap().is_none());
    }

    #[test]
    fn remove_by_name_removes_all_entries() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        register_pid(&dot, 12345, "nora", 1_700_000_000).unwrap();
        register_pid(&dot, 12346, "nora", 1_700_000_000).unwrap();
        register_pid(&dot, 12347, "quinn", 1_700_000_000).unwrap();
        super::remove_by_name(&dot, "nora").unwrap();
        assert!(get_by_pid(&dot, 12345).unwrap().is_none());
        assert!(get_by_pid(&dot, 12346).unwrap().is_none());
        assert_eq!(get_by_pid(&dot, 12347).unwrap().unwrap().name, "quinn");
    }
}
