//! 图工程路径工具：write_paths 相交判断、共享契约文件检测、路径规范化。
//! （docs/design_graph.md §9 待定项"路径语法"的 M0 实现：相对路径 + 简单 glob + 拒绝 `..`。）

/// 共享契约文件名单：这些文件被多个写节点修改时默认互斥（docs/design_graph.md §十二）。
const SHARED_CONTRACT_FILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "go.sum",
    "poetry.lock",
];

/// 迁移目录：数据库迁移默认串行（docs/design_graph.md §十二）。
const MIGRATION_DIRS: &[&str] = &["migrations/", "db/migrate/", "db/migrations/"];

/// 规范化路径：去 `./`，去空段与尾部 `/`。调用方须保证无 `..` 段（compiler 前置拒绝）。
pub fn normalize_path(p: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        segs.push(seg);
    }
    segs.join("/")
}

/// write_paths 相交判断：精确相等、前缀包含或 glob（`*` 段通配）匹配。
pub fn paths_overlap(a: &str, b: &str) -> bool {
    let na = normalize_path(a);
    let nb = normalize_path(b);
    if na == nb {
        return true;
    }
    if na.contains('*') || nb.contains('*') {
        return glob_overlap(&na, &nb);
    }
    // 目录前缀：a/b 与 a 重叠（a 是目录）
    na.starts_with(&format!("{nb}/")) || nb.starts_with(&format!("{na}/"))
}

/// 逐段 glob 匹配：`*` 通配任意单段；短路径是长路径的目录前缀时视为重叠。
fn glob_overlap(a: &str, b: &str) -> bool {
    let a_segs: Vec<&str> = a.split('/').collect();
    let b_segs: Vec<&str> = b.split('/').collect();
    let mut i = 0;
    while i < a_segs.len() && i < b_segs.len() {
        if a_segs[i] == "*" || b_segs[i] == "*" || a_segs[i] == b_segs[i] {
            i += 1;
            continue;
        }
        return false;
    }
    true
}

/// 是否触碰共享契约文件（锁文件 / 迁移目录）。
pub fn touches_shared_contract(p: &str) -> bool {
    let np = normalize_path(p);
    if SHARED_CONTRACT_FILES.contains(&np.as_str()) {
        return true;
    }
    MIGRATION_DIRS
        .iter()
        .any(|d| np.starts_with(d) || np == d.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_drops_dot_and_trailing_slash() {
        assert_eq!(normalize_path("./src/backend/"), "src/backend");
        assert_eq!(normalize_path("src//backend"), "src/backend");
        assert_eq!(normalize_path("Cargo.lock"), "Cargo.lock");
    }

    #[test]
    fn overlap_exact_and_prefix() {
        assert!(paths_overlap("src/backend", "src/backend"));
        assert!(
            paths_overlap("src/backend", "src/backend/api.rs"),
            "目录前缀应重叠"
        );
        assert!(
            paths_overlap("src/backend/api.rs", "src/backend"),
            "反向也重叠"
        );
        assert!(!paths_overlap("src/backend", "src/frontend"));
        assert!(
            !paths_overlap("src/backend", "src/backendish"),
            "段边界不应误判"
        );
    }

    #[test]
    fn overlap_glob() {
        assert!(paths_overlap("src/*", "src/backend"));
        assert!(paths_overlap("src/backend/*", "src/backend/api.rs"));
        assert!(paths_overlap("src/*", "src/*"), "glob 间应重叠");
        assert!(!paths_overlap("src/*", "docs/api"));
    }

    #[test]
    fn shared_contract_detection() {
        assert!(touches_shared_contract("Cargo.lock"));
        assert!(touches_shared_contract("./Cargo.lock"));
        assert!(touches_shared_contract("db/migrations/2026_01_init.sql"));
        assert!(touches_shared_contract("migrations/"));
        assert!(!touches_shared_contract("src/backend"));
    }
}
