//! 内置 agent 使用指南。
//!
//! 源文件：`docs/agent-usage.md`，编译时通过 `include_str!` 嵌入二进制。
//! 该 guide 不属于任何 agent 的本地 memory，也不进入 agtalk.db。

pub const AGENT_GUIDE_TOPIC: &str = "agtalk/agent-guide";

/// 返回内置 agent 使用指南的 Markdown 全文。
pub fn agent_guide_markdown() -> String {
    include_str!("../../../docs/agent-usage.md").to_string()
}

/// 判断 topic 是否为内置 guide。
pub fn is_agent_guide_topic(topic: &str) -> bool {
    topic == AGENT_GUIDE_TOPIC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_is_non_empty() {
        let guide = agent_guide_markdown();
        assert!(!guide.is_empty());
        assert!(guide.contains("agtalk mem pack agtalk/agent-guide"));
        assert!(guide.contains("inbox_empty"));
    }

    #[test]
    fn detects_guide_topic() {
        assert!(is_agent_guide_topic("agtalk/agent-guide"));
        assert!(!is_agent_guide_topic("agent-learning-handbook"));
        assert!(!is_agent_guide_topic(""));
    }
}
