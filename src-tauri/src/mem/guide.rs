//! 内置 agent 使用指南。
//!
//! 源文件：`docs/agent-usage.md`，编译时通过 `include_str!` 嵌入二进制。
//! 该 guide 不属于任何 agent 的本地 memory，也不进入 agtalk.db。
//!
//! 读取入口为 CLI 全局参数 `agtalk --agent-guide`，不再通过 `mem pack`。

/// 返回内置 agent 使用指南的 Markdown 全文。
pub fn agent_guide_markdown() -> String {
    include_str!("../../../docs/agent-usage.md").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_is_non_empty() {
        let guide = agent_guide_markdown();
        assert!(!guide.is_empty());
        assert!(guide.contains("agtalk --agent-guide"));
        assert!(guide.contains("inbox_empty"));
    }
}
