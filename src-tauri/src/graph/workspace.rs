//! Workspace / Worktree 隔离（docs/design_graph.md §六；P1-2）。
//!
//! 写节点在独立 git worktree 中执行，agent 只能修改自己的 worktree；
//! Runtime 负责：创建 worktree、验证后 commit（白名单路径）、图完成后 merge。
//! 全部 git 操作走参数数组（不经 shell），白名单子命令。

use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::process::Command;

/// git 白名单子命令：允许的操作集合（禁 push/rebase/checkout 分支切换等）。
const ALLOWED_GIT_SUBCOMMANDS: &[&str] = &[
    "worktree",
    "add",
    "commit",
    "merge",
    "checkout",
    "rev-parse",
    "branch",
    "status",
    "diff",
    "log",
    "ls-tree",
];

/// Workspace 数据库行。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceRow {
    pub id: String,
    pub graph_run_id: String,
    pub owner_node_run_id: Option<String>,
    pub repository: Option<String>,
    pub base_revision: Option<String>,
    pub branch: Option<String>,
    pub path: Option<String>,
    pub status: String,
    pub dirty: bool,
    pub commit_hash: Option<String>,
}

/// 执行白名单 git 命令（参数数组，不经 shell）。
/// 白名单取第一个非全局选项（`-c key value` / `-C path` / `--xxx`）参数作为子命令。
pub fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut subcmd: Option<&str> = None;
    let mut skip_next = false;
    for a in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if *a == "-c" || *a == "-C" {
            skip_next = true;
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        subcmd = Some(a);
        break;
    }
    if let Some(sc) = subcmd {
        if !ALLOWED_GIT_SUBCOMMANDS.contains(&sc) {
            return Err(format!("git 子命令不在白名单: {sc}"));
        }
    }
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git 执行失败: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 确保仓库根是 git 仓库（`git rev-parse --show-toplevel`）。
pub fn ensure_repo_root(repository: &str) -> Result<PathBuf, String> {
    let root = git(Path::new(repository), &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(root))
}

/// 为写节点创建 worktree（幂等：workspaces 已有记录则复用）。
/// 路径：`<repo_root>/.agtalk/worktrees/<run_id>-<node_key>`（.agtalk 已在 gitignore）。
pub fn ensure_worktree(
    conn: &Connection,
    graph_run_id: &str,
    node_key: &str,
    repository: &str,
    base_revision: &str,
) -> Result<WorkspaceRow, String> {
    // 幂等：已有则返回
    if let Some(row) = get_by_node(conn, graph_run_id, node_key).map_err(|e| e.to_string())? {
        return Ok(row);
    }
    let repo_root = ensure_repo_root(repository)?;
    let branch = format!("agtalk/{}-{}", short(graph_run_id), node_key);
    let wt_path = repo_root.join(".agtalk").join("worktrees").join(format!(
        "{}-{}",
        short(graph_run_id),
        node_key
    ));
    // worktree add：基于 base_revision 检出新分支
    git(
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            wt_path.to_str().unwrap(),
            base_revision,
        ],
    )
    .map_err(|e| format!("worktree add 失败: {e}"))?;

    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO workspaces (id, graph_run_id, owner_node_run_id, repository, base_revision, \
         branch, path, status, dirty) VALUES (?1,?2,NULL,?3,?4,?5,?6,'ready',0)",
        params![
            id,
            graph_run_id,
            repository,
            base_revision,
            branch,
            wt_path.to_string_lossy()
        ],
    )
    .map_err(|e| e.to_string())?;
    // 绑定到节点（get_by_node 通过 node_runs.workspace_id 关联）
    conn.execute(
        "UPDATE node_runs SET workspace_id=?1 WHERE graph_run_id=?2 AND node_key=?3",
        params![id, graph_run_id, node_key],
    )
    .map_err(|e| e.to_string())?;
    get(conn, &id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "workspace 创建后读取失败".into())
}

/// 节点成功后 commit worktree 的允许路径。
/// `allowed_paths` = 节点 write_paths 与 changed_files 的交集（白名单）。
pub fn commit_worktree(
    conn: &Connection,
    workspace: &WorkspaceRow,
    allowed_paths: &[String],
    message: &str,
) -> Result<String, String> {
    let Some(path) = &workspace.path else {
        return Err("workspace 无 path".into());
    };
    let wt = Path::new(path);
    let dirty = git(wt, &["status", "--porcelain"])?;
    if dirty.trim().is_empty() {
        return Ok(String::new()); // 无改动，跳过 commit
    }
    // add 白名单路径（相对 worktree 根；单个失败跳过——文件可能已被 agent 删除，
    // 但全部失败则明确报错，避免静默吞掉后提交空改动）
    let mut added_any = false;
    for p in allowed_paths {
        if git(wt, &["add", "--", p]).is_ok() {
            added_any = true;
        }
    }
    if !added_any {
        return Err("无法 add 任何 changed_files（路径不存在或不可访问）".into());
    }
    // 提交（作者信息用 git 环境兜底，无则本地配置）
    git(
        wt,
        &[
            "-c",
            "user.name=agtalk-graph",
            "-c",
            "user.email=agtalk@local",
            "commit",
            "-m",
            message,
        ],
    )?;
    let hash = git(wt, &["rev-parse", "HEAD"])?;
    conn.execute(
        "UPDATE workspaces SET status='committed', dirty=0, commit_hash=?1 WHERE id=?2",
        params![hash, workspace.id],
    )
    .map_err(|e| e.to_string())?;
    Ok(hash)
}

/// 图完成后 merge 所有 workspace 到集成分支。
pub fn merge_workspaces(
    conn: &Connection,
    graph_run_id: &str,
    integration_target: &str,
) -> Result<(usize, usize), String> {
    let rows = list_by_graph(conn, graph_run_id).map_err(|e| e.to_string())?;
    let mut merged = 0;
    let mut conflicts = 0;
    for ws in rows {
        let Some(branch) = &ws.branch else { continue };
        let Some(repository) = &ws.repository else {
            continue;
        };
        let repo = Path::new(repository);
        // 在仓库根执行 merge（worktree 分支合入 integration_target）
        let _ = git(repo, &["checkout", integration_target]);
        let out = git(repo, &["merge", "--no-edit", branch]);
        match out {
            Ok(_) => {
                conn.execute(
                    "UPDATE workspaces SET status='merged' WHERE id=?1",
                    params![ws.id],
                )
                .map_err(|e| e.to_string())?;
                merged += 1;
            }
            Err(e) => {
                // 合并冲突：保留现场（不自动处理），发事件带指引，继续合并其余 workspace
                conn.execute(
                    "UPDATE workspaces SET status='failed' WHERE id=?1",
                    params![ws.id],
                )
                .map_err(|e| e.to_string())?;
                crate::graph::events::append(
                    conn,
                    graph_run_id,
                    "merge_conflict",
                    None,
                    &serde_json::json!({
                        "workspace": ws.id,
                        "branch": branch,
                        "path": ws.path,
                        "error": e,
                        "hint": "集成分支存在冲突：请手动解决（在 worktree 分支上 git merge --abort 或解决冲突后提交），现场已保留",
                    }),
                )
                .map_err(|e| e.to_string())?;
                conflicts += 1;
            }
        }
    }
    Ok((merged, conflicts))
}

/// 释放 worktree（移除）。
pub fn release_worktree(conn: &Connection, workspace_id: &str) -> Result<(), String> {
    let Some(row) = get(conn, workspace_id).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    if let Some(path) = &row.path {
        let _ = git(Path::new(path), &["worktree", "remove", "--force", path]);
    }
    conn.execute(
        "UPDATE workspaces SET status='released' WHERE id=?1",
        params![workspace_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn short(s: &str) -> String {
    if s.len() > 8 {
        s[..8].to_string()
    } else {
        s.to_string()
    }
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<WorkspaceRow>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, graph_run_id, owner_node_run_id, repository, base_revision, branch, path, \
         status, dirty, commit_hash FROM workspaces WHERE id=?1",
        params![id],
        row_from,
    )
    .optional()
}

pub fn get_by_node(
    conn: &Connection,
    graph_run_id: &str,
    node_key: &str,
) -> Result<Option<WorkspaceRow>, rusqlite::Error> {
    conn.query_row(
        "SELECT w.id, w.graph_run_id, w.owner_node_run_id, w.repository, w.base_revision, \
         w.branch, w.path, w.status, w.dirty, w.commit_hash \
         FROM workspaces w JOIN node_runs n ON n.workspace_id = w.id \
         WHERE w.graph_run_id=?1 AND n.node_key=?2",
        params![graph_run_id, node_key],
        row_from,
    )
    .optional()
}

pub(crate) fn list_by_graph(
    conn: &Connection,
    graph_run_id: &str,
) -> Result<Vec<WorkspaceRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, graph_run_id, owner_node_run_id, repository, base_revision, branch, path, \
         status, dirty, commit_hash FROM workspaces WHERE graph_run_id=?1",
    )?;
    let rows = stmt
        .query_map(params![graph_run_id], row_from)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: r.get(0)?,
        graph_run_id: r.get(1)?,
        owner_node_run_id: r.get(2)?,
        repository: r.get(3)?,
        base_revision: r.get(4)?,
        branch: r.get(5)?,
        path: r.get(6)?,
        status: r.get(7)?,
        dirty: r.get::<_, i64>(8)? != 0,
        commit_hash: r.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::fs;

    /// 建临时 git 仓库（含初始 commit；测试基础设施，不经白名单）。
    fn init_repo() -> tempfile::TempDir {
        use std::process::Command;
        fn run(root: &std::path::Path, args: &[&str]) {
            let out = Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} 失败: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-b", "main"]);
        run(root, &["config", "user.name", "t"]);
        run(root, &["config", "user.email", "t@t"]);
        run(root, &["commit", "--allow-empty", "-m", "init"]);
        dir
    }

    #[test]
    fn create_and_commit_worktree() {
        let repo = init_repo();
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g-run-1234', 'g', '{}', '{}', 'ready')",
            [],
        )
        .unwrap();

        let ws = ensure_worktree(
            &conn,
            "g-run-1234",
            "impl",
            repo.path().to_str().unwrap(),
            "main",
        )
        .unwrap();
        assert_eq!(ws.status, "ready");
        assert!(ws.path.is_some());

        // worktree 内写文件
        let wt = Path::new(ws.path.as_ref().unwrap());
        fs::create_dir_all(wt.join("src")).unwrap();
        fs::write(wt.join("src/a.rs"), "fn a() {}").unwrap();

        // commit 白名单路径
        let hash = commit_worktree(
            &conn,
            &ws,
            &["src/a.rs".to_string()],
            "graph g-run-1234 impl",
        )
        .unwrap();
        assert!(!hash.is_empty(), "应产生 commit");
        let row = get(&conn, &ws.id).unwrap().unwrap();
        assert_eq!(row.status, "committed");
        assert_eq!(row.commit_hash.as_deref(), Some(hash.as_str()));

        // merge 到 main
        let merged = merge_workspaces(&conn, "g-run-1234", "main").unwrap();
        assert_eq!(merged, (1, 0));
        // main 分支应含 src/a.rs
        let files = git(repo.path(), &["ls-tree", "-r", "--name-only", "main"]).unwrap();
        assert!(
            files.contains("src/a.rs"),
            "merge 后 main 应包含 worktree 改动: {files}"
        );
    }

    #[test]
    fn worktree_idempotent() {
        let repo = init_repo();
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g-run-5678', 'g', '{}', '{}', 'ready')",
            [],
        )
        .unwrap();
        crate::graph::state::create_node_run(
            &conn,
            "g-run-5678",
            "node-x",
            crate::graph::spec::NodeType::Executor,
            1,
            Some("p1"),
            Some("w1"),
        )
        .unwrap();
        let ws1 = ensure_worktree(
            &conn,
            "g-run-5678",
            "node-x",
            repo.path().to_str().unwrap(),
            "main",
        )
        .unwrap();
        let ws2 = ensure_worktree(
            &conn,
            "g-run-5678",
            "node-x",
            repo.path().to_str().unwrap(),
            "main",
        )
        .unwrap();
        assert_eq!(ws1.id, ws2.id, "同节点 worktree 应幂等复用");
    }

    #[test]
    fn commit_fails_when_no_staged_files() {
        // Tim 评审风险②：allowed_paths 全部不可 add（文件不存在）→ 明确报错而非静默空提交
        let repo = init_repo();
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g-cf', 'g', '{}', '{}', 'ready')",
            [],
        )
        .unwrap();
        crate::graph::state::create_node_run(
            &conn,
            "g-cf",
            "a",
            crate::graph::spec::NodeType::Executor,
            1,
            Some("p1"),
            Some("w1"),
        )
        .unwrap();
        let ws =
            ensure_worktree(&conn, "g-cf", "a", repo.path().to_str().unwrap(), "main").unwrap();
        // worktree 内写文件（产生 dirty）但 allowed_paths 指向不存在的文件
        let wt = std::path::Path::new(ws.path.as_ref().unwrap());
        std::fs::write(wt.join("src_a.rs"), "x").unwrap();
        let err = commit_worktree(&conn, &ws, &["ghost/file.rs".to_string()], "msg").unwrap_err();
        assert!(err.contains("无法 add"), "应明确报 add 失败: {err}");
    }
}
