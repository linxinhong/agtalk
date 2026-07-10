//! agents.json：PID anchor map，pid -> { name, start_time }。

use super::IdentityError;
use crate::paths::set_permissions_0600;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub name: String,
    pub start_time: u64,
}

/// PID anchor map。
///
/// v2 将顶层字段从 `agents` 改为 `anchors`，但底层结构仍是 pid -> {name, start_time}。
/// 读取时兼容旧格式 `agents`，写入统一输出 `anchors`。
#[derive(Debug, Clone, Default)]
pub struct AgentsMap {
    pub anchors: HashMap<String, AgentEntry>,
}

impl Serialize for AgentsMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Wrapper<'a> {
            anchors: &'a HashMap<String, AgentEntry>,
        }
        Wrapper {
            anchors: &self.anchors,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AgentsMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            anchors: HashMap<String, AgentEntry>,
            #[serde(default)]
            agents: HashMap<String, AgentEntry>,
        }
        let wrapper = Wrapper::deserialize(deserializer)?;
        let anchors = if !wrapper.anchors.is_empty() || wrapper.agents.is_empty() {
            wrapper.anchors
        } else {
            wrapper.agents
        };
        Ok(AgentsMap { anchors })
    }
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
    map.anchors.insert(
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
    map.anchors.remove(&pid.to_string());
    write(dot_agtalk, &map)
}

/// 删除 agents.json 中所有指向指定 name 的 pid anchor。
pub fn remove_by_name(dot_agtalk: &Path, name: &str) -> Result<(), IdentityError> {
    let mut map = read(dot_agtalk)?;
    map.anchors.retain(|_, entry| entry.name != name);
    write(dot_agtalk, &map)
}

pub fn get_by_pid(dot_agtalk: &Path, pid: u32) -> Result<Option<AgentEntry>, IdentityError> {
    let map = read(dot_agtalk)?;
    Ok(map.anchors.get(&pid.to_string()).cloned())
}

/// 删除 agents.json 中所有 session 已经不存在的 pid anchor。
/// `valid_names` 是 `.agtalk/` 下仍有 session.json 的 agent name 集合。
/// 返回被删除的 (pid, name) 列表。
pub fn cleanup_stale_anchors(
    dot_agtalk: &Path,
    valid_names: &[String],
) -> Result<Vec<(u32, String)>, IdentityError> {
    let mut map = read(dot_agtalk)?;
    let valid: std::collections::HashSet<_> = valid_names.iter().cloned().collect();
    let mut removed = Vec::new();
    map.anchors.retain(|pid_str, entry| {
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

/// 清理已死亡（进程不存在或 start_time 不匹配）的 PID anchor。
pub fn cleanup_dead_anchors(dot_agtalk: &Path) -> Result<Vec<(u32, String)>, IdentityError> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    cleanup_dead_anchors_with(dot_agtalk, &sys)
}

/// 清理指定 name 的已死亡 PID anchor，用于 join 前轻量清理。
pub fn cleanup_dead_anchors_for_name(
    dot_agtalk: &Path,
    name: &str,
) -> Result<Vec<(u32, String)>, IdentityError> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    let mut map = read(dot_agtalk)?;
    let mut removed = Vec::new();
    map.anchors.retain(|pid_str, entry| {
        if entry.name != name {
            return true;
        }
        let keep = is_live_anchor(pid_str, entry, &sys);
        if !keep {
            if let Ok(pid) = pid_str.parse::<u32>() {
                removed.push((pid, entry.name.clone()));
            }
        }
        keep
    });
    write(dot_agtalk, &map)?;
    Ok(removed)
}

fn cleanup_dead_anchors_with(
    dot_agtalk: &Path,
    sys: &sysinfo::System,
) -> Result<Vec<(u32, String)>, IdentityError> {
    let mut map = read(dot_agtalk)?;
    let mut removed = Vec::new();
    map.anchors.retain(|pid_str, entry| {
        let keep = is_live_anchor(pid_str, entry, sys);
        if !keep {
            if let Ok(pid) = pid_str.parse::<u32>() {
                removed.push((pid, entry.name.clone()));
            }
        }
        keep
    });
    write(dot_agtalk, &map)?;
    Ok(removed)
}

fn is_live_anchor(pid_str: &str, entry: &AgentEntry, sys: &sysinfo::System) -> bool {
    let Ok(pid) = pid_str.parse::<usize>() else {
        return false;
    };
    let Some(process) = sys.process(sysinfo::Pid::from(pid)) else {
        return false;
    };
    process.start_time() == entry.start_time
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

    #[test]
    fn write_outputs_anchors_field() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        register_pid(&dot, 12345, "nora", 1_700_000_000).unwrap();
        let content = std::fs::read_to_string(agents_path(&dot)).unwrap();
        assert!(content.contains("\"anchors\""));
        assert!(!content.contains("\"agents\""));
    }

    #[test]
    fn read_compatible_with_legacy_agents_field() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot).unwrap();
        let legacy = r#"{"agents":{"12345":{"name":"nora","start_time":1700000000}}}"#;
        std::fs::write(agents_path(&dot), legacy).unwrap();
        let map = read(&dot).unwrap();
        let entry = map.anchors.get("12345").unwrap();
        assert_eq!(entry.name, "nora");
        assert_eq!(entry.start_time, 1_700_000_000);
    }

    #[test]
    fn cleanup_dead_anchors_removes_nonexistent_pid() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        // 使用一个几乎不可能存在的 PID
        register_pid(&dot, 99_999_999, "nora", 1_700_000_000).unwrap();
        let removed = cleanup_dead_anchors(&dot).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].1, "nora");
        assert!(get_by_pid(&dot, 99_999_999).unwrap().is_none());
    }

    #[test]
    fn cleanup_dead_anchors_for_name_only_affects_target_name() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        register_pid(&dot, 99_999_998, "nora", 1_700_000_000).unwrap();
        register_pid(&dot, 99_999_997, "quinn", 1_700_000_000).unwrap();
        let removed = cleanup_dead_anchors_for_name(&dot, "nora").unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].1, "nora");
        assert!(get_by_pid(&dot, 99_999_998).unwrap().is_none());
        assert!(get_by_pid(&dot, 99_999_997).unwrap().is_some());
    }
}
