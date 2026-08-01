//! 身份接管提示词渲染（Tim 设计稿：agtalk id prompt + GUI 复制按钮）。
//!
//! 单一事实来源：CLI（`agtalk id prompt`）与 GUI（`gui_node_prompt`）复用本模块，
//! 协议演进不漂移。输出 3 行极简接管文本（不内嵌协议全文，指向 agtalk-bridge skill）。

use crate::identity::session_file::SessionFile;

/// 结构化接管提示（--json 输出用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct OnboardingPrompt {
    pub name: String,
    pub address: String,
    pub intro: String,
    pub prompt_text: String,
}

/// 渲染 3 行极简接管文本（纯文本，stdout 直接可复制）。
pub fn render_onboarding_prompt(session: &SessionFile) -> String {
    let name = session.name.trim();
    let intro = session.intro.trim();
    format!(
        "你是 {name}，{intro}（agtalk 本地总线身份）。\n\
         协作协议见 agtalk-bridge skill（身份在文件系统，compact 后用 agtalk --as {name} id show 恢复）。\n\
         现在开始：执行 agtalk --as {name} msg read 接收任务，并按协议闭环回复。"
    )
}

/// 结构化版本（--json / GUI 复用）。
pub fn onboarding_prompt(session: &SessionFile) -> OnboardingPrompt {
    OnboardingPrompt {
        name: session.name.clone(),
        address: session.address.clone(),
        intro: session.intro.clone(),
        prompt_text: render_onboarding_prompt(session),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::session_file::{SessionFile, SessionNotify};

    fn fake_session() -> SessionFile {
        SessionFile {
            version: 2,
            address: "93b837c6-68fa-4ab8-b359-141fada2494a".into(),
            name: "alan".into(),
            intro: "负责开发维护 agtalk 项目".into(),
            created_at: "2026-08-01T00:00:00Z".into(),
            registered_by: None,
            notify: SessionNotify::none(),
        }
    }

    #[test]
    fn prompt_contains_name_and_intro() {
        let text = render_onboarding_prompt(&fake_session());
        assert!(text.contains("你是 alan"), "应含 name: {text}");
        assert!(text.contains("负责开发维护 agtalk 项目"), "应含 intro");
        assert!(text.contains("agtalk --as alan msg read"), "应含取信命令");
        assert!(
            text.contains("agtalk-bridge skill"),
            "应指向 skill 而非内嵌协议"
        );
    }

    #[test]
    fn structured_prompt_shape() {
        let p = onboarding_prompt(&fake_session());
        assert_eq!(p.name, "alan");
        assert_eq!(p.address, "93b837c6-68fa-4ab8-b359-141fada2494a");
        assert!(p.prompt_text.starts_with("你是 alan"));
        // --json 可序列化
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["name"], "alan");
        assert!(v["prompt_text"].as_str().unwrap().contains("agtalk-bridge"));
    }
}
