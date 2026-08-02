//! 随机 agent 名字生成（2 字中文代号，如"赛文""韦多"）。
//!
//! Rust 生态没有专门的中文起名 crate（避免引入不可维护依赖），
//! 这里用内置字库 + rand 组合：代号风（不是传统姓+名），短小可读、适合做 participant 名。
//! 头像无需额外机制：GUI 按 djb2(name) 自动映射像素头像（src/lib/identity.ts）。

use rand::seq::SliceRandom;

/// 代号字库（60 字，语义中性偏"行动/智慧"），任取 2 字组合。
const GLYPHS: &[&str] = &[
    "赛", "文", "韦", "多", "洛", "奇", "峰", "岚", "澈", "珩", "曜", "臻", "逸", "宸", "骁", "翊",
    "蕴", "诺", "衡", "牧", "湛", "衍", "澈", "凛", "苍", "朔", "泽", "晏", "恒", "观", "照", "羽",
    "驰", "越", "宁", "澈", "铭", "轩", "睿", "朗", "行", "鉴", "枢", "衡", "岳", "川", "凌", "岚",
    "泊", "遥", "策", "铸", "研", "衡", "弈", "臻", "汐", "衍", "珞", "祁",
];

/// 随机生成一个 2 字中文代号名（如 "赛文"）。同名概率极低（60×60 组合）。
pub fn random_agent_name() -> String {
    let mut rng = rand::thread_rng();
    let a = GLYPHS.choose(&mut rng).unwrap_or(&"赛");
    let b = GLYPHS.choose(&mut rng).unwrap_or(&"文");
    format!("{a}{b}")
}

/// 批量生成（去重，最多 len 个；名字池 60²=3600 组合足够）。
pub fn random_agent_names(len: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut guard = 0;
    while out.len() < len && guard < len * 20 + 100 {
        let n = random_agent_name();
        if seen.insert(n.clone()) {
            out.push(n);
        }
        guard += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_two_chinese_chars() {
        for _ in 0..50 {
            let n = random_agent_name();
            assert_eq!(n.chars().count(), 2, "应为 2 个中文字: {n}");
        }
    }

    #[test]
    fn batch_is_unique() {
        let names = random_agent_names(30);
        assert_eq!(names.len(), 30, "批量生成应去重");
        let set: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(set.len(), 30);
    }

    #[test]
    fn different_calls_differ() {
        // 概率性：100 次中至少出现 2 个不同名字（3600 组合下几乎必然）
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            seen.insert(random_agent_name());
        }
        assert!(seen.len() > 1, "随机性异常");
    }
}
