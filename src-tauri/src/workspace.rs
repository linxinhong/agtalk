//! workspace.json 读写（项目本地凭证，描述当前目录是哪个 workspace）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

pub const WORKSPACE_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    pub workspace: WorkspaceMeta,
    #[serde(default)]
    pub daemon: DaemonMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMeta {
    pub id: String,
    pub name: String,
    pub root: String,
    #[serde(default = "default_detected_by")]
    pub detected_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonMeta {
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default)]
    pub socket: Option<String>,
}

fn default_detected_by() -> String {
    "cwd-scan".into()
}
fn default_profile() -> String {
    "default".into()
}

pub fn read_workspace() -> Result<Option<WorkspaceFile>> {
    let path = paths::workspace_json_path().context("未找到 agtalk workspace")?;
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("读取 workspace.json 失败: {:?}", path))?;
    let wf: WorkspaceFile = serde_json::from_str(&data).context("解析 workspace.json 失败")?;
    Ok(Some(wf))
}

pub fn write_workspace(wf: &mut WorkspaceFile) -> Result<()> {
    let path = paths::workspace_json_path().context("未找到 agtalk workspace")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    wf.workspace.updated_at = current_rfc3339();
    let data = serde_json::to_string_pretty(wf)?;
    // 原子写：先 .tmp 再 rename
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &data)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn current_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
