//! relations.json：agent 私有协作关系索引。
//!
//! 定位：history.jsonl 是事件流水；relations.json 是按 peer 聚合后的协作画像。
//! 不参与认证、路由、SSE、notify。

use super::IdentityError;
use crate::paths::set_permissions_0600;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RelationOwner {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Relation {
    pub name: String,
    pub address: String,
    pub intro: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub sent_count: u64,
    #[serde(default)]
    pub received_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RelationsFile {
    pub version: u32,
    pub owner: RelationOwner,
    #[serde(default)]
    pub peers: HashMap<String, Relation>,
}

fn relations_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    dot_agtalk.join(name).join("relations.json")
}

/// 读取 relations.json；不存在则返回空结构。
pub fn read(dot_agtalk: &Path, name: &str) -> Result<RelationsFile, IdentityError> {
    let path = relations_path(dot_agtalk, name);
    if !path.exists() {
        return Ok(RelationsFile {
            version: 1,
            owner: RelationOwner {
                name: name.to_string(),
                address: String::new(),
            },
            peers: HashMap::new(),
        });
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&content)?)
}

/// 写入 relations.json。
pub fn write(
    dot_agtalk: &Path,
    name: &str,
    relations: &RelationsFile,
) -> Result<PathBuf, IdentityError> {
    let dir = dot_agtalk.join(name);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("relations.json");
    let content = serde_json::to_string_pretty(relations)?;
    std::fs::write(&path, content)?;
    set_permissions_0600(&path)?;
    Ok(path)
}

/// 确保 owner.address 与当前 session 一致。
fn ensure_owner(relations: &mut RelationsFile, owner_name: &str, owner_address: &str) {
    if relations.owner.name != owner_name || relations.owner.address != owner_address {
        relations.owner = RelationOwner {
            name: owner_name.to_string(),
            address: owner_address.to_string(),
        };
    }
}

/// 一次 relation 更新所需的事件信息。
pub struct RelationEvent<'a> {
    pub owner_name: &'a str,
    pub owner_address: &'a str,
    pub peer_name: &'a str,
    pub peer_address: &'a str,
    pub peer_intro: &'a str,
    pub message_id: &'a str,
    pub timestamp: f64,
}

/// 更新发送方的 relation：peer 是接收方。
pub fn record_send(dot_agtalk: &Path, event: RelationEvent<'_>) -> Result<(), IdentityError> {
    let mut relations = read(dot_agtalk, event.owner_name)?;
    ensure_owner(&mut relations, event.owner_name, event.owner_address);

    let relation = relations
        .peers
        .entry(event.peer_address.to_string())
        .or_insert(Relation {
            name: event.peer_name.to_string(),
            address: event.peer_address.to_string(),
            intro: event.peer_intro.to_string(),
            first_seen_at: Some(event.timestamp),
            ..Default::default()
        });

    relation.name = event.peer_name.to_string();
    relation.address = event.peer_address.to_string();
    relation.intro = event.peer_intro.to_string();
    if relation.first_seen_at.is_none() {
        relation.first_seen_at = Some(event.timestamp);
    }
    relation.last_seen_at = Some(event.timestamp);
    relation.last_message_id = Some(event.message_id.to_string());
    relation.sent_count += 1;

    write(dot_agtalk, event.owner_name, &relations)?;
    Ok(())
}

/// 更新接收方的 relation：peer 是发送方。
pub fn record_receive(dot_agtalk: &Path, event: RelationEvent<'_>) -> Result<(), IdentityError> {
    let mut relations = read(dot_agtalk, event.owner_name)?;
    ensure_owner(&mut relations, event.owner_name, event.owner_address);

    let relation = relations
        .peers
        .entry(event.peer_address.to_string())
        .or_insert(Relation {
            name: event.peer_name.to_string(),
            address: event.peer_address.to_string(),
            intro: event.peer_intro.to_string(),
            first_seen_at: Some(event.timestamp),
            ..Default::default()
        });

    relation.name = event.peer_name.to_string();
    relation.address = event.peer_address.to_string();
    relation.intro = event.peer_intro.to_string();
    if relation.first_seen_at.is_none() {
        relation.first_seen_at = Some(event.timestamp);
    }
    relation.last_seen_at = Some(event.timestamp);
    relation.last_message_id = Some(event.message_id.to_string());
    relation.received_count += 1;

    write(dot_agtalk, event.owner_name, &relations)?;
    Ok(())
}

/// 列出所有 peers。
pub fn list(dot_agtalk: &Path, name: &str) -> Result<Vec<Relation>, IdentityError> {
    let relations = read(dot_agtalk, name)?;
    let mut peers: Vec<Relation> = relations.peers.into_values().collect();
    peers.sort_by(|a, b| b.last_seen_at.partial_cmp(&a.last_seen_at).unwrap());
    Ok(peers)
}

/// 按 name 或 address 查找 peer。
pub fn find(dot_agtalk: &Path, name: &str, query: &str) -> Result<Option<Relation>, IdentityError> {
    let relations = read(dot_agtalk, name)?;
    let query_lower = query.to_lowercase();
    Ok(relations
        .peers
        .values()
        .find(|r| {
            r.address.eq_ignore_ascii_case(query)
                || r.name.to_lowercase() == query_lower
                || r.address.starts_with(query)
        })
        .cloned())
}

/// 按 name 或 address 更新 peer 的手动字段。
pub fn update(
    dot_agtalk: &Path,
    name: &str,
    query: &str,
    role: Option<String>,
    tags: Option<Vec<String>>,
    note: Option<String>,
) -> Result<Option<Relation>, IdentityError> {
    let mut relations = read(dot_agtalk, name)?;
    let query_lower = query.to_lowercase();
    let key = relations
        .peers
        .keys()
        .find(|k| {
            let r = &relations.peers[*k];
            r.address.eq_ignore_ascii_case(query)
                || r.name.to_lowercase() == query_lower
                || r.address.starts_with(query)
        })
        .cloned();

    if let Some(key) = key {
        let relation = relations.peers.get_mut(&key).unwrap();
        if let Some(role) = role {
            relation.role = Some(role);
        }
        if let Some(tags) = tags {
            relation.tags = tags;
        }
        if let Some(note) = note {
            relation.note = Some(note);
        }
        write(dot_agtalk, name, &relations)?;
        Ok(relations.peers.get(&key).cloned())
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_dot() -> (PathBuf, TempDir) {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        (dot, tmp)
    }

    #[test]
    fn record_send_creates_relation() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "addr-jerry",
                peer_intro: "designer",
                message_id: "msg-1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        let relations = read(&dot, "tom").unwrap();
        let r = relations.peers.get("addr-jerry").unwrap();
        assert_eq!(r.name, "jerry");
        assert_eq!(r.sent_count, 1);
        assert_eq!(r.received_count, 0);
        assert_eq!(r.last_message_id, Some("msg-1".to_string()));
    }

    #[test]
    fn record_receive_updates_counts() {
        let (dot, _tmp) = tmp_dot();
        record_receive(
            &dot,
            RelationEvent {
                owner_name: "jerry",
                owner_address: "addr-jerry",
                peer_name: "tom",
                peer_address: "addr-tom",
                peer_intro: "coder",
                message_id: "msg-2",
                timestamp: 2.0,
            },
        )
        .unwrap();

        let relations = read(&dot, "jerry").unwrap();
        let r = relations.peers.get("addr-tom").unwrap();
        assert_eq!(r.sent_count, 0);
        assert_eq!(r.received_count, 1);
    }

    #[test]
    fn manual_fields_not_overwritten_by_auto_update() {
        let (dot, _tmp) = tmp_dot();
        // 先建立一条关系，再更新手动字段，最后用自动更新覆盖计数/时间等字段。
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "addr-jerry",
                peer_intro: "designer",
                message_id: "msg-1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        update(
            &dot,
            "tom",
            "addr-jerry",
            Some("implementation".to_string()),
            Some(vec!["rust".to_string()]),
            Some("good partner".to_string()),
        )
        .unwrap();

        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "addr-jerry",
                peer_intro: "designer",
                message_id: "msg-2",
                timestamp: 2.0,
            },
        )
        .unwrap();

        let r = read(&dot, "tom").unwrap().peers["addr-jerry"].clone();
        assert_eq!(r.role, Some("implementation".to_string()));
        assert_eq!(r.tags, vec!["rust"]);
        assert_eq!(r.note, Some("good partner".to_string()));
        assert_eq!(r.sent_count, 2);
    }

    #[test]
    fn list_sorts_by_last_seen() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "a",
                peer_address: "addr-a",
                peer_intro: "",
                message_id: "m1",
                timestamp: 1.0,
            },
        )
        .unwrap();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "b",
                peer_address: "addr-b",
                peer_intro: "",
                message_id: "m2",
                timestamp: 3.0,
            },
        )
        .unwrap();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "a",
                peer_address: "addr-a",
                peer_intro: "",
                message_id: "m3",
                timestamp: 2.0,
            },
        )
        .unwrap();

        let list = list(&dot, "tom").unwrap();
        assert_eq!(list[0].name, "b");
        assert_eq!(list[1].name, "a");
    }

    #[test]
    fn find_by_short_address() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "550e8400-e29b-41d4-a716-446655440000",
                peer_intro: "",
                message_id: "m1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        let found = find(&dot, "tom", "550e8400").unwrap();
        assert!(found.is_some());
    }
}
