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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specialties: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_for: Vec<String>,
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

/// 写入 relations.json。首次写入时升级到 version 2。
pub fn write(
    dot_agtalk: &Path,
    name: &str,
    relations: &RelationsFile,
) -> Result<PathBuf, IdentityError> {
    let dir = dot_agtalk.join(name);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("relations.json");
    let mut to_write = relations.clone();
    to_write.version = 2;
    let content = serde_json::to_string_pretty(&to_write)?;
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
    if event.peer_address == event.owner_address {
        return Ok(());
    }
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
    if event.peer_address == event.owner_address {
        return Ok(());
    }
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

/// 列出所有 peers；可按 specialty 大小写不敏感精确匹配过滤。
pub fn list(
    dot_agtalk: &Path,
    name: &str,
    specialty: Option<&str>,
) -> Result<Vec<Relation>, IdentityError> {
    let relations = read(dot_agtalk, name)?;
    let mut peers: Vec<Relation> = relations.peers.into_values().collect();
    if let Some(filter) = specialty {
        let filter = filter.to_lowercase();
        peers.retain(|r| r.specialties.iter().any(|s| s.to_lowercase() == filter));
    }
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

/// 清洗字符串数组：trim、去空、去重（保留原顺序）。
pub fn clean_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// 清洗字符串数组：trim、去空、按大小写不敏感去重，保留首次出现的原始展示文本和顺序。
///
/// 用于 `specialties` / `preferred_for`，使其与 `list --specialty` 的大小写不敏感匹配语义一致。
pub fn clean_case_insensitive_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.to_lowercase()))
        .collect()
}

/// 更新 peer 手动字段的参数包。
pub struct RelationUpdate {
    pub role: Option<String>,
    pub tags: Option<Vec<String>>,
    pub note: Option<String>,
    pub specialties: Option<Vec<String>>,
    pub preferred_for: Option<Vec<String>>,
}

/// 按 name 或 address 更新 peer 的手动字段。
pub fn update(
    dot_agtalk: &Path,
    name: &str,
    query: &str,
    update: RelationUpdate,
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
        if let Some(role) = update.role {
            relation.role = Some(role);
        }
        if let Some(tags) = update.tags {
            relation.tags = clean_strings(tags);
        }
        if let Some(note) = update.note {
            relation.note = Some(note);
        }
        if let Some(specialties) = update.specialties {
            relation.specialties = clean_case_insensitive_strings(specialties);
        }
        if let Some(preferred_for) = update.preferred_for {
            relation.preferred_for = clean_case_insensitive_strings(preferred_for);
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
            RelationUpdate {
                role: Some("implementation".to_string()),
                tags: Some(vec!["rust".to_string()]),
                note: Some("good partner".to_string()),
                specialties: None,
                preferred_for: None,
            },
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

        let list = list(&dot, "tom", None).unwrap();
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

    #[test]
    fn v1_file_readable_and_write_upgrades_to_v2() {
        let (dot, _tmp) = tmp_dot();
        std::fs::create_dir_all(dot.join("tom")).unwrap();
        let v1 = r#"{
            "version": 1,
            "owner": { "name": "tom", "address": "addr-tom" },
            "peers": {
                "addr-jerry": {
                    "name": "jerry",
                    "address": "addr-jerry",
                    "intro": "designer",
                    "sent_count": 3,
                    "received_count": 2,
                    "role": "reviewer",
                    "tags": ["rust"]
                }
            }
        }"#;
        std::fs::write(dot.join("tom").join("relations.json"), v1).unwrap();

        // v1 可读，新增字段默认为空
        let relations = read(&dot, "tom").unwrap();
        let r = relations.peers.get("addr-jerry").unwrap();
        assert_eq!(r.role, Some("reviewer".to_string()));
        assert!(r.specialties.is_empty());
        assert!(r.preferred_for.is_empty());

        // 任意写操作升级到 version 2，不丢失旧字段
        update(
            &dot,
            "tom",
            "addr-jerry",
            RelationUpdate {
                role: None,
                tags: None,
                note: None,
                specialties: Some(vec!["Rust 实现".to_string()]),
                preferred_for: None,
            },
        )
        .unwrap();

        let content = std::fs::read_to_string(dot.join("tom").join("relations.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["version"], 2);
        assert_eq!(json["peers"]["addr-jerry"]["role"], "reviewer");
        assert_eq!(json["peers"]["addr-jerry"]["specialties"][0], "Rust 实现");
    }

    #[test]
    fn update_cleans_specialties_and_preferred_for() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "addr-jerry",
                peer_intro: "",
                message_id: "m1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        update(
            &dot,
            "tom",
            "addr-jerry",
            RelationUpdate {
                role: None,
                tags: None,
                note: None,
                specialties: Some(vec![
                    "  Rust 实现 ".to_string(),
                    "Rust 实现".to_string(),
                    "".to_string(),
                    "测试隔离".to_string(),
                ]),
                preferred_for: Some(vec!["功能开发".to_string(), "  ".to_string()]),
            },
        )
        .unwrap();

        let r = read(&dot, "tom").unwrap().peers["addr-jerry"].clone();
        assert_eq!(
            r.specialties,
            vec!["Rust 实现".to_string(), "测试隔离".to_string()]
        );
        assert_eq!(r.preferred_for, vec!["功能开发".to_string()]);
    }

    #[test]
    fn list_filters_by_specialty_case_insensitive() {
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
        update(
            &dot,
            "tom",
            "addr-a",
            RelationUpdate {
                role: None,
                tags: None,
                note: None,
                specialties: Some(vec!["Rust 实现".to_string()]),
                preferred_for: None,
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
                timestamp: 2.0,
            },
        )
        .unwrap();
        update(
            &dot,
            "tom",
            "addr-b",
            RelationUpdate {
                role: None,
                tags: None,
                note: None,
                specialties: Some(vec!["前端".to_string()]),
                preferred_for: None,
            },
        )
        .unwrap();

        let filtered = list(&dot, "tom", Some("rust 实现")).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "a");

        let all = list(&dot, "tom", None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "b");
    }

    #[test]
    fn update_normalizes_specialties_and_preferred_for_case_insensitive() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "addr-jerry",
                peer_intro: "",
                message_id: "m1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        update(
            &dot,
            "tom",
            "addr-jerry",
            RelationUpdate {
                role: None,
                tags: None,
                note: None,
                specialties: Some(vec![
                    "Rust 实现".to_string(),
                    "rust 实现".to_string(),
                    "  Rust 实现 ".to_string(),
                    "测试隔离".to_string(),
                    "".to_string(),
                ]),
                preferred_for: Some(vec![
                    "功能开发".to_string(),
                    "功能开发".to_string(),
                    "  修复 Rust 测试 ".to_string(),
                ]),
            },
        )
        .unwrap();

        let r = read(&dot, "tom").unwrap().peers["addr-jerry"].clone();
        assert_eq!(
            r.specialties,
            vec!["Rust 实现".to_string(), "测试隔离".to_string()]
        );
        assert_eq!(
            r.preferred_for,
            vec!["功能开发".to_string(), "修复 Rust 测试".to_string()]
        );
    }

    #[test]
    fn list_filter_matches_case_insensitive_normalized_specialty() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "jerry",
                peer_address: "addr-jerry",
                peer_intro: "",
                message_id: "m1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        // 通过大小写混合的输入写入，最终保留首次出现的展示文本 "Rust 实现"
        update(
            &dot,
            "tom",
            "addr-jerry",
            RelationUpdate {
                role: None,
                tags: None,
                note: None,
                specialties: Some(vec!["rust 实现".to_string(), "Rust 实现".to_string()]),
                preferred_for: None,
            },
        )
        .unwrap();

        // 用另一种大小写过滤，仍能命中
        let filtered = list(&dot, "tom", Some("RUST 实现")).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].specialties, vec!["rust 实现".to_string()]);
    }

    #[test]
    fn record_send_does_not_add_owner_as_peer() {
        let (dot, _tmp) = tmp_dot();
        record_send(
            &dot,
            RelationEvent {
                owner_name: "tom",
                owner_address: "addr-tom",
                peer_name: "tom",
                peer_address: "addr-tom",
                peer_intro: "self",
                message_id: "m1",
                timestamp: 1.0,
            },
        )
        .unwrap();

        let relations = read(&dot, "tom").unwrap();
        assert!(relations.peers.is_empty());
    }
}
