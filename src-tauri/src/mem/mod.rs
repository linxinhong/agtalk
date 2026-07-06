//! agent 本地记忆：plan / context / status / entries。
//!
//! 文件布局（每个 agent 独立）：
//! `.agtalk/<name>/memory/{plan.md, context.md, status.json, entries.jsonl}`

use crate::paths::{set_permissions_0600, set_permissions_0700, PathsError};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

pub mod guide;
pub mod index;

#[derive(Debug, Error)]
pub enum MemError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("路径错误: {0}")]
    Paths(#[from] PathsError),
    #[error("agent 不存在: {0}")]
    AgentNotFound(String),
    #[error("记忆条目不存在: {0}")]
    EntryNotFound(String),
    #[error("目标不明确: 找到 {0} 个候选")]
    Ambiguous(usize),
}

/// agent 当前计划与上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub plan: String,
    pub context: String,
    pub status: StatusSummary,
}

/// 状态摘要，持久化在 status.json。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusSummary {
    pub status: String,
    pub summary: String,
    pub updated_at: String,
}

/// 单条记忆条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub topic: String,
    pub ty: String,
    pub title: String,
    pub text: String,
    pub tags: Vec<String>,
    pub created_at: String,
}

fn memory_dir(dot_agtalk: &Path, name: &str) -> PathBuf {
    dot_agtalk.join(name).join("memory")
}

fn ensure_memory_dir(dot_agtalk: &Path, name: &str) -> Result<PathBuf, MemError> {
    let dir = memory_dir(dot_agtalk, name);
    std::fs::create_dir_all(&dir)?;
    set_permissions_0700(&dir)?;
    Ok(dir)
}

fn plan_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    memory_dir(dot_agtalk, name).join("plan.md")
}

fn context_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    memory_dir(dot_agtalk, name).join("context.md")
}

fn status_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    memory_dir(dot_agtalk, name).join("status.json")
}

fn entries_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    memory_dir(dot_agtalk, name).join("entries.jsonl")
}

fn read_text(path: &Path) -> Result<String, MemError> {
    if path.exists() {
        Ok(std::fs::read_to_string(path)?)
    } else {
        Ok(String::new())
    }
}

fn read_status(dot_agtalk: &Path, name: &str) -> Result<StatusSummary, MemError> {
    let path = status_path(dot_agtalk, name);
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(StatusSummary::default())
    }
}

/// 原子写文件：先写临时文件再 rename，避免半写。
fn write_atomic(path: &Path, content: &str) -> Result<(), MemError> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
    }
    std::fs::rename(&tmp, path)?;
    set_permissions_0600(path)?;
    Ok(())
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 读取当前 agent 的完整计划状态。
pub fn plan_show(dot_agtalk: &Path, name: &str) -> Result<AgentState, MemError> {
    let plan = read_text(&plan_path(dot_agtalk, name))?;
    let context = read_text(&context_path(dot_agtalk, name))?;
    let status = read_status(dot_agtalk, name)?;
    Ok(AgentState {
        plan,
        context,
        status,
    })
}

/// 更新 plan / context / status。
pub fn plan_update(
    dot_agtalk: &Path,
    name: &str,
    plan: Option<String>,
    context: Option<String>,
    status: Option<String>,
    summary: Option<String>,
) -> Result<StatusSummary, MemError> {
    ensure_memory_dir(dot_agtalk, name)?;

    if let Some(p) = plan {
        write_atomic(&plan_path(dot_agtalk, name), &p)?;
    }
    if let Some(c) = context {
        write_atomic(&context_path(dot_agtalk, name), &c)?;
    }

    let mut st = read_status(dot_agtalk, name)?;
    let mut changed = false;
    if let Some(s) = status {
        st.status = s;
        changed = true;
    }
    if let Some(s) = summary {
        st.summary = s;
        changed = true;
    }
    if changed {
        st.updated_at = iso_now();
        let content = serde_json::to_string_pretty(&st)?;
        write_atomic(&status_path(dot_agtalk, name), &content)?;
    }

    Ok(st)
}

/// 读取当前 agent 的状态摘要。
pub fn plan_status(dot_agtalk: &Path, name: &str) -> Result<StatusSummary, MemError> {
    read_status(dot_agtalk, name)
}

/// 添加记忆条目，返回条目 id。
pub fn add(
    dot_agtalk: &Path,
    name: &str,
    text: String,
    topic: String,
    ty: String,
    title: Option<String>,
    tags: Vec<String>,
) -> Result<String, MemError> {
    ensure_memory_dir(dot_agtalk, name)?;
    let entry = MemoryEntry {
        id: Uuid::new_v4().to_string(),
        topic,
        ty,
        title: title.unwrap_or_default(),
        text,
        tags,
        created_at: iso_now(),
    };
    let line = serde_json::to_string(&entry)?;
    let path = entries_path(dot_agtalk, name);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{}", line)?;
    f.flush()?;
    set_permissions_0600(&path)?;
    Ok(entry.id)
}

fn read_entries(dot_agtalk: &Path, name: &str) -> Result<Vec<MemoryEntry>, MemError> {
    let path = entries_path(dot_agtalk, name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let mut entries = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        entries.push(serde_json::from_str(line)?);
    }
    Ok(entries)
}

fn matches_query(entry: &MemoryEntry, query: &str) -> bool {
    let q = query.to_lowercase();
    entry.text.to_lowercase().contains(&q)
        || entry.title.to_lowercase().contains(&q)
        || entry.tags.iter().any(|t| t.to_lowercase().contains(&q))
        || entry.topic.to_lowercase().contains(&q)
}

/// 搜索记忆。query 为空时返回最新条目。
pub fn search(
    dot_agtalk: &Path,
    name: &str,
    query: &str,
    topic: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<MemoryEntry>, MemError> {
    let mut entries = read_entries(dot_agtalk, name)?;
    entries.reverse();
    let limit = limit.unwrap_or(10);

    let filtered: Vec<MemoryEntry> = entries
        .into_iter()
        .filter(|e| {
            let topic_ok = topic.map(|t| e.topic == t).unwrap_or(true);
            let query_ok = query.is_empty() || matches_query(e, query);
            topic_ok && query_ok
        })
        .take(limit)
        .collect();
    Ok(filtered)
}

/// 按 id 读取单条记忆。
pub fn show_entry(dot_agtalk: &Path, name: &str, id: &str) -> Result<MemoryEntry, MemError> {
    let entries = read_entries(dot_agtalk, name)?;
    entries
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| MemError::EntryNotFound(id.to_string()))
}

/// 列出记忆，可选按 topic 过滤，最新在前。
pub fn list(
    dot_agtalk: &Path,
    name: &str,
    topic: Option<&str>,
) -> Result<Vec<MemoryEntry>, MemError> {
    let mut entries = read_entries(dot_agtalk, name)?;
    entries.reverse();
    if let Some(t) = topic {
        entries.retain(|e| e.topic == t);
    }
    Ok(entries)
}

/// 打包记忆为 Markdown，适合注入 prompt。
pub fn pack(
    dot_agtalk: &Path,
    name: &str,
    topic: &str,
    limit: Option<usize>,
) -> Result<String, MemError> {
    let entries = read_entries(dot_agtalk, name)?;
    let mut entries: Vec<MemoryEntry> = entries.into_iter().rev().collect();

    let all_topics = topic.is_empty();
    if !all_topics {
        entries.retain(|e| e.topic == topic);
    }
    if let Some(l) = limit {
        entries.truncate(l);
    }

    if entries.is_empty() {
        return Ok(String::new());
    }

    // 按 topic 分组，保持原有顺序（已是时间倒序）。
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, Vec<&MemoryEntry>> = BTreeMap::new();
    for e in &entries {
        groups.entry(e.topic.clone()).or_default().push(e);
    }

    let mut out = String::new();
    for (t, items) in groups {
        out.push_str(&format!("## {}\n\n", t));
        for e in items {
            if !e.title.is_empty() {
                out.push_str(&format!("### {}\n", e.title));
            }
            if !e.ty.is_empty() {
                out.push_str(&format!("_type: {}_  ", e.ty));
            }
            if !e.tags.is_empty() {
                out.push_str(&format!("_tags: {}_\n", e.tags.join(", ")));
            } else if !e.ty.is_empty() {
                out.push('\n');
            }
            out.push_str(&e.text);
            out.push_str("\n\n");
        }
    }
    Ok(out.trim().to_string())
}

/// 收集当前所有记忆条目的 topic 去重列表。
pub fn collect_topics(dot_agtalk: &Path, name: &str) -> Result<Vec<String>, MemError> {
    let entries = read_entries(dot_agtalk, name)?;
    let mut topics: Vec<String> = entries.into_iter().map(|e| e.topic).collect();
    topics.sort();
    topics.dedup();
    Ok(topics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use tempfile::TempDir;

    fn tmp_dot() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        (tmp, dot)
    }

    #[test]
    fn plan_roundtrip() {
        let (_tmp, dot) = tmp_dot();
        let status = plan_update(
            &dot,
            "nora",
            Some("do X".into()),
            Some("ctx".into()),
            Some("running".into()),
            Some("50%".into()),
        )
        .unwrap();
        assert_eq!(status.status, "running");
        assert_eq!(status.summary, "50%");
        assert!(!status.updated_at.is_empty());

        let state = plan_show(&dot, "nora").unwrap();
        assert_eq!(state.plan, "do X");
        assert_eq!(state.context, "ctx");
        assert_eq!(state.status.status, "running");

        let status2 = plan_status(&dot, "nora").unwrap();
        assert_eq!(status2.summary, "50%");
    }

    #[test]
    fn add_search_list_pack() {
        let (_tmp, dot) = tmp_dot();
        let id1 = add(
            &dot,
            "nora",
            "first note".into(),
            "todo".into(),
            "note".into(),
            Some("title1".into()),
            vec!["tagA".into()],
        )
        .unwrap();
        let id2 = add(
            &dot,
            "nora",
            "second idea".into(),
            "ideas".into(),
            "note".into(),
            None,
            vec!["tagB".into()],
        )
        .unwrap();

        let all = search(&dot, "nora", "", None, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, id2);

        let filtered = search(&dot, "nora", "first", None, None).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, id1);

        let by_topic = list(&dot, "nora", Some("todo")).unwrap();
        assert_eq!(by_topic.len(), 1);
        assert_eq!(by_topic[0].id, id1);

        let entry = show_entry(&dot, "nora", &id1).unwrap();
        assert_eq!(entry.title, "title1");

        let md = pack(&dot, "nora", "todo", None).unwrap();
        assert!(md.contains("## todo"));
        assert!(md.contains("first note"));
        assert!(!md.contains("second idea"));

        let topics = collect_topics(&dot, "nora").unwrap();
        assert_eq!(topics, vec!["ideas", "todo"]);
    }

    #[test]
    fn mem_index_register_refresh_remove_lookup() {
        let storage = Storage::open_in_memory().unwrap();
        let address = crate::identity::mailbox::create(&storage, "nora", "前端", "projA").unwrap();

        index::register(
            &storage,
            &address,
            "nora",
            "projA",
            std::path::Path::new("/tmp/nora/memory"),
        );

        let row = index::lookup_by_address(&storage, &address).unwrap();
        assert_eq!(row.name, "nora");
        assert_eq!(row.workspace, "projA");

        index::refresh(
            &storage,
            &address,
            "running: 50%",
            &["todo".into(), "ideas".into()],
        );
        let row = index::lookup_by_address(&storage, &address).unwrap();
        assert_eq!(row.status_summary, "running: 50%");
        assert_eq!(row.public_topics, "todo,ideas");
        assert!(row.plan_updated_at > 0.0);

        let by_name = index::lookup_by_name(&storage, "nora");
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].address, address);

        index::remove(&storage, &address);
        assert!(index::lookup_by_address(&storage, &address).is_none());
    }
}
