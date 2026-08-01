//! Verification Gate（docs/design_graph.md §七；P0/M2 不 spawn）。
//!
//! 三项抽查（全部在 daemon 进程内，不执行外部命令）：
//! 1. 路径验证：changed_files ⊆ write_paths 且 ∩ forbidden_paths = ∅（拒绝 `..` 段）
//! 2. Artifact 验证：本地 uri 存在 + sha256 checksum 匹配
//! 3. Schema 验证：result 非空（schema 语义注册表留待后续；先保证结构化结果存在）
//!
//! 局限记录：worktree（workspace.rs）就位后，路径验证需 canonicalize 解析 symlink 防逃逸；
//! M2 先做字符串级校验（symlink 逃逸防护随 worktree 落地）。

use crate::graph::dto::GraphArtifactRef;
use crate::graph::paths::paths_overlap;
use std::path::Path;

/// 单条验证结果（写入 verifications 表）。
#[derive(Debug, Clone)]
pub struct VerifyCheck {
    pub verifier_type: String, // path / artifact / schema / claim
    pub rule: String,
    pub status: String, // passed / failed
    pub detail: String,
}

/// 节点结果整体验证结论。
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    pub passed: bool,
    pub checks: Vec<VerifyCheck>,
    /// 失败分类（供 failed 与重试判定）：path_violation / contract_violation / artifact_violation。
    pub failure_type: Option<String>,
}

/// 校验节点候选结果（docs/design_graph.md §七成功条件）。
pub fn verify_node_result(
    changed_files: &[String],
    write_paths: &[String],
    forbidden_paths: &[String],
    output_artifacts: &[GraphArtifactRef],
    result: &str,
) -> VerifyOutcome {
    let mut checks = Vec::new();
    let mut failure: Option<String> = None;

    // 1. 路径验证
    match verify_paths(changed_files, write_paths, forbidden_paths) {
        Ok(()) => checks.push(VerifyCheck {
            verifier_type: "path".into(),
            rule: "changed ⊆ write_paths ∧ changed ∩ forbidden = ∅".into(),
            status: "passed".into(),
            detail: format!("{} 个 changed_files 均在合法范围内", changed_files.len()),
        }),
        Err(e) => {
            failure.get_or_insert_with(|| "path_violation".into());
            checks.push(VerifyCheck {
                verifier_type: "path".into(),
                rule: "changed ⊆ write_paths ∧ changed ∩ forbidden = ∅".into(),
                status: "failed".into(),
                detail: e,
            });
        }
    }

    // 2. Artifact 验证（本地 uri 存在 + checksum）
    for artifact in output_artifacts {
        match verify_artifact(artifact) {
            Ok(()) => checks.push(VerifyCheck {
                verifier_type: "artifact".into(),
                rule: format!("{} 存在且 checksum 匹配", artifact.uri),
                status: "passed".into(),
                detail: artifact.checksum.clone(),
            }),
            Err(e) => {
                failure.get_or_insert_with(|| "artifact_violation".into());
                checks.push(VerifyCheck {
                    verifier_type: "artifact".into(),
                    rule: format!("{} 存在且 checksum 匹配", artifact.uri),
                    status: "failed".into(),
                    detail: e,
                });
            }
        }
    }

    // 3. Schema 验证：result 非空（M2 语义级）
    if result.trim().is_empty() {
        failure.get_or_insert_with(|| "contract_violation".into());
        checks.push(VerifyCheck {
            verifier_type: "schema".into(),
            rule: "result 非空".into(),
            status: "failed".into(),
            detail: "候选结果缺少 result 描述".into(),
        });
    } else {
        checks.push(VerifyCheck {
            verifier_type: "schema".into(),
            rule: "result 非空".into(),
            status: "passed".into(),
            detail: String::new(),
        });
    }

    VerifyOutcome {
        passed: failure.is_none(),
        checks,
        failure_type: failure,
    }
}

/// 路径验证：changed ⊆ write_paths 且 ∩ forbidden_paths = ∅。
pub fn verify_paths(
    changed_files: &[String],
    write_paths: &[String],
    forbidden_paths: &[String],
) -> Result<(), String> {
    for f in changed_files {
        if f.split('/').any(|seg| seg == "..") {
            return Err(format!("changed_files 含非法 '..' 段: {f}"));
        }
        let within = write_paths.iter().any(|w| paths_overlap(w, f));
        if !within {
            return Err(format!("越界写入: {f} 不在 write_paths 内"));
        }
        if forbidden_paths.iter().any(|fb| paths_overlap(fb, f)) {
            return Err(format!("触碰 forbidden_paths: {f}"));
        }
    }
    Ok(())
}

/// 单条 artifact 验证：本地路径（file:// 或绝对路径）校验存在 + sha256。
/// http(s)/相对路径无法在 daemon 侧校验 → 不阻塞（记录为 passed 并注明）。
pub fn verify_artifact(artifact: &GraphArtifactRef) -> Result<(), String> {
    let Some(path) = local_path(&artifact.uri) else {
        return Ok(()); // 非本地 uri（http 等）暂不校验，M2 接受此局限
    };
    if !path.exists() {
        return Err(format!("artifact 不存在: {}", artifact.uri));
    }
    if let Some(expected_hex) = checksum_hex(&artifact.checksum) {
        let actual = sha256_file(path)?;
        if actual != expected_hex {
            return Err(format!(
                "artifact checksum 不匹配: {}（期望 {} 实际 {}）",
                artifact.uri, expected_hex, actual
            ));
        }
    }
    Ok(())
}

/// 解析本地路径：file:// 前缀去掉；绝对路径原样；其它返回 None。
fn local_path(uri: &str) -> Option<&Path> {
    if let Some(rest) = uri.strip_prefix("file://") {
        Some(Path::new(rest))
    } else if Path::new(uri).is_absolute() {
        Some(Path::new(uri))
    } else {
        None
    }
}

/// checksum 字段："sha256:<hex>" → hex；空/未知格式 → None。
fn checksum_hex(checksum: &str) -> Option<String> {
    checksum.strip_prefix("sha256:").map(|h| h.to_string())
}

/// 文件 sha256 hex（小文件直读；大文件留待流式）。
fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("无法打开 {}: {}", path.display(), e))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let digest = sha256(&buf);
    Ok(digest)
}

fn sha256(data: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn path_within_write_paths_passes() {
        assert!(
            verify_paths(&["src/backend/api.rs".into()], &["src/backend".into()], &[],).is_ok()
        );
    }

    #[test]
    fn path_outside_write_paths_fails() {
        let e =
            verify_paths(&["src/frontend/x.ts".into()], &["src/backend".into()], &[]).unwrap_err();
        assert!(e.contains("越界写入"));
    }

    #[test]
    fn forbidden_path_hit_fails() {
        let e = verify_paths(
            &["Cargo.lock".into()],
            &["src".into(), "Cargo.lock".into()],
            &["Cargo.lock".into()],
        )
        .unwrap_err();
        assert!(e.contains("forbidden_paths"));
    }

    #[test]
    fn traversal_rejected() {
        assert!(verify_paths(&["../secret".into()], &["src".into()], &[]).is_err());
    }

    #[test]
    fn glob_write_path_matches() {
        assert!(verify_paths(&["src/backend/api.rs".into()], &["src/*".into()], &[],).is_ok());
    }

    #[test]
    fn artifact_checksum_validated() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("out.json");
        fs::write(&f, "hello").unwrap();
        // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let ok = verify_artifact(&GraphArtifactRef {
            artifact_type: "test-report".into(),
            schema_version: "v1".into(),
            uri: format!("file://{}", f.display()),
            checksum: "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                .into(),
        });
        assert!(ok.is_ok(), "checksum 匹配应通过: {ok:?}");

        let bad = verify_artifact(&GraphArtifactRef {
            artifact_type: "test-report".into(),
            schema_version: "v1".into(),
            uri: format!("file://{}", f.display()),
            checksum: "sha256:deadbeef".into(),
        });
        assert!(bad.is_err(), "checksum 不匹配应失败");
    }

    #[test]
    fn missing_artifact_fails() {
        let e = verify_artifact(&GraphArtifactRef {
            artifact_type: "source-diff".into(),
            schema_version: "v1".into(),
            uri: "file:///tmp/definitely-not-exist-agtalk-test".into(),
            checksum: String::new(),
        })
        .unwrap_err();
        assert!(e.contains("不存在"));
    }

    #[test]
    fn full_outcome_flags_failure_type() {
        let out = verify_node_result(
            &["src/outside.rs".into()],
            &["src/backend".into()],
            &[],
            &[],
            "some result",
        );
        assert!(!out.passed);
        assert_eq!(out.failure_type.as_deref(), Some("path_violation"));
        assert!(out.checks.iter().any(|c| c.status == "failed"));
    }

    #[test]
    fn empty_result_is_contract_violation() {
        let out = verify_node_result(&[], &[], &[], &[], "   ");
        assert!(!out.passed);
        assert_eq!(out.failure_type.as_deref(), Some("contract_violation"));
    }
}
