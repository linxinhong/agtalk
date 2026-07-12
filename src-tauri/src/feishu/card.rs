//! 飞书消息卡片 JSON 2.0：审批卡片（choices→按钮）/ 文本卡片（带回复入口）/
//! compose 草稿卡（agent 选择 + 发送）/ 回复表单卡 / 终态卡片。
//! 红线：按钮 value 只存动作类型 + agtalk message UUID（+ choice index），不存业务结论。

use crate::routing::Message;
use serde_json::{json, Value};

pub const ACTION_MSG_KEY: &str = "agtalk_msg";
pub const ACTION_CHOICE_KEY: &str = "choice_index";

/// 对话类动作标记：旧版一键审批按钮不带 action 字段（向后兼容），其余按钮必带。
pub const ACTION_KEY: &str = "action";
pub const ACTION_COMPOSE_SUBMIT: &str = "compose_submit";
pub const ACTION_REPLY_OPEN: &str = "reply_open";
pub const ACTION_REPLY_SUBMIT: &str = "reply_submit";
pub const ACTION_APPROVAL_SUBMIT: &str = "approval_submit";
pub const ACTION_DONE: &str = "done";
pub const ACTION_CANCEL: &str = "cancel";
/// form 组件 name：正文输入框 / 目标 agent 下拉 / 审批选项勾选框前缀。
pub const FORM_BODY: &str = "body";
pub const FORM_TARGET: &str = "target";
pub const FORM_OPT_PREFIX: &str = "opt_";

/// 卡片回调动作：审批选择（旧版一键按钮）/ 审批表单提交 / 完成 / 取消 /
/// compose 发送 / 打开回复表单 / 提交回复。
pub enum CardAction {
    Approval { msg_id: String, choice_index: usize },
    ApprovalSubmit { msg_id: String },
    Done { msg_id: String },
    Cancel { msg_id: String },
    ComposeSubmit,
    ReplyOpen { msg_id: String },
    ReplySubmit { msg_id: String },
}

/// 解码按钮 value 为动作；形状不符返回 None（调用方按忽略处理）。
/// 兼容飞书把 value 作为 JSON 字符串回传；无 action 字段时回落旧版审批形状。
pub fn decode_action(value: &Value) -> Option<CardAction> {
    let obj = match value {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        v => v.clone(),
    };
    let msg_id = |key: &str| obj.get(key).and_then(|v| v.as_str()).map(|m| m.to_string());
    match obj.get(ACTION_KEY).and_then(|v| v.as_str()) {
        Some(ACTION_COMPOSE_SUBMIT) => Some(CardAction::ComposeSubmit),
        Some(ACTION_REPLY_OPEN) => {
            msg_id(ACTION_MSG_KEY).map(|msg_id| CardAction::ReplyOpen { msg_id })
        }
        Some(ACTION_REPLY_SUBMIT) => {
            msg_id(ACTION_MSG_KEY).map(|msg_id| CardAction::ReplySubmit { msg_id })
        }
        Some(ACTION_APPROVAL_SUBMIT) => {
            msg_id(ACTION_MSG_KEY).map(|msg_id| CardAction::ApprovalSubmit { msg_id })
        }
        Some(ACTION_DONE) => msg_id(ACTION_MSG_KEY).map(|msg_id| CardAction::Done { msg_id }),
        Some(ACTION_CANCEL) => msg_id(ACTION_MSG_KEY).map(|msg_id| CardAction::Cancel { msg_id }),
        Some(_) => None,
        None => decode_action_value(&obj).map(|(msg_id, choice_index)| CardAction::Approval {
            msg_id,
            choice_index,
        }),
    }
}

/// 编码按钮 value：精确路由键（UUID + choice index）。
pub fn encode_action_value(message_id: &str, choice_index: usize) -> Value {
    json!({ ACTION_MSG_KEY: message_id, ACTION_CHOICE_KEY: choice_index })
}

/// 解码按钮 value；形状不符返回 None（调用方按忽略处理）。
/// 兼容飞书把 value 作为 JSON 字符串回传的情况。
pub fn decode_action_value(value: &Value) -> Option<(String, usize)> {
    let obj = match value {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        v => v.clone(),
    };
    let msg = obj.get(ACTION_MSG_KEY)?.as_str()?.to_string();
    let idx = obj.get(ACTION_CHOICE_KEY)?.as_u64()? as usize;
    Some((msg, idx))
}

fn skeleton(elements: Vec<Value>) -> Value {
    json!({
        "schema": "2.0",
        "config": { "update_multi": true },
        "body": { "elements": elements },
    })
}

/// 样式化头部行：蓝色小字 + maybe_filled 图标 + hr（对齐 AskHuman 生产样式，
/// 不用原生大 banner——标题字体固定且渲染成整条色带）。
fn styled_header(title: &str) -> Vec<Value> {
    vec![
        json!({
            "tag": "div",
            "text": {
                "tag": "plain_text",
                "content": title,
                "text_size": "notation",
                "text_align": "left",
                "text_color": "blue",
            },
            "icon": { "tag": "standard_icon", "token": "maybe_filled", "color": "blue" },
            "margin": "0px 0px 0px 0px",
        }),
        json!({ "tag": "hr", "margin": "0px 0px 0px 0px" }),
    ]
}

/// 审批卡片：样式化头部（来源 agent）+ 正文 + 勾选表单（checker 多选 +
/// 非 select_only 时补充输入框 + 提交按钮）+ 表单外「取消」按钮（danger）。
/// 对齐 AskHuman 审批表单：勾选状态经 form_value 回传（opt_{i}: bool），
/// 不在按钮 value 里放业务结论；recommended 选项带绿色 [推荐] 标记。
/// Card JSON 2.0：回调数据放 behaviors callback value。
pub fn approval_card(msg: &Message, choices: &[String]) -> Value {
    let recommended = approval_recommended(msg);
    let select_only = approval_select_only(msg);
    let mut elements = styled_header(&format!("来自 {} 的审批", msg.from_name));
    elements.push(json!({ "tag": "markdown", "content": msg.body }));
    elements.push(json!({ "tag": "hr" }));
    let mut form_elements: Vec<Value> = choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let label = match &recommended {
                Some(r) if r == c => format!("{}  <font color='green'>[推荐]</font>", c),
                _ => c.clone(),
            };
            json!({
                "tag": "checker",
                "name": format!("{}{}", FORM_OPT_PREFIX, i),
                "checked": false,
                "text": { "tag": "lark_md", "content": label },
            })
        })
        .collect();
    if !select_only {
        form_elements.push(json!({
            "tag": "input",
            "name": FORM_BODY,
            "placeholder": { "tag": "plain_text", "content": "补充说明（可选）" },
        }));
    }
    form_elements.push(json!({
        "tag": "button",
        "name": "submit",
        "form_action_type": "submit",
        "text": { "tag": "plain_text", "content": "提交" },
        "type": "primary",
        "behaviors": [
            { "type": "callback", "value": { ACTION_KEY: ACTION_APPROVAL_SUBMIT, ACTION_MSG_KEY: msg.id } }
        ],
    }));
    elements.push(json!({
        "tag": "form",
        "name": "approval_form",
        "elements": form_elements,
    }));
    elements.push(json!({
        "tag": "button",
        "text": { "tag": "plain_text", "content": "取消" },
        "type": "danger",
        "behaviors": [
            { "type": "callback", "value": { ACTION_KEY: ACTION_CANCEL, ACTION_MSG_KEY: msg.id } }
        ],
    }));
    skeleton(elements)
}

/// 文本卡片：样式化头部（来源 agent）+ markdown 正文 + 三件套按钮
/// （回复 / 完成 / 取消，对齐 GUI popup 的一等动作；取消是 danger 样式）。
/// 点击回复后卡片原地切换为回复表单（reply_form_card），目标锁定原发送 agent。
pub fn text_card(msg: &Message) -> Value {
    let mut elements = styled_header(&format!("来自 {} 的消息", msg.from_name));
    elements.push(json!({ "tag": "markdown", "content": msg.body }));
    elements.push(json!({ "tag": "hr" }));
    let button = |label: &str, kind: &str, action: &str| {
        json!({
            "tag": "column",
            "width": "auto",
            "elements": [json!({
                "tag": "button",
                "text": { "tag": "plain_text", "content": label },
                "type": kind,
                "behaviors": [
                    { "type": "callback", "value": { ACTION_KEY: action, ACTION_MSG_KEY: msg.id } }
                ],
            })],
        })
    };
    elements.push(json!({
        "tag": "column_set",
        "horizontal_spacing": "8px",
        "columns": [
            button("回复", "default", ACTION_REPLY_OPEN),
            button("完成", "default", ACTION_DONE),
            button("取消", "danger", ACTION_CANCEL),
        ],
    }));
    skeleton(elements)
}

/// compose 卡片的 agent 选项：展示 name + intro，option value 只放 address UUID。
pub struct ComposeAgent {
    pub address: String,
    pub name: String,
    pub intro: String,
}

/// compose 草稿卡：p2p 文本预填正文 + agent 下拉（name — intro）+ 发送按钮。
/// 完整 UUID 不出现在可见文案中，只在 option value 里。
pub fn compose_card(draft: &str, agents: &[ComposeAgent]) -> Value {
    let options: Vec<Value> = agents
        .iter()
        .map(|a| {
            let label = if a.intro.is_empty() {
                a.name.clone()
            } else {
                format!("{} — {}", a.name, truncate(&a.intro, 40))
            };
            json!({
                "text": { "tag": "plain_text", "content": label },
                "value": a.address,
            })
        })
        .collect();
    let mut elements = styled_header("选择 Agent 并发送");
    elements.push(json!({
        "tag": "form",
        "name": "compose_form",
        "elements": [
            {
                "tag": "input",
                "name": FORM_BODY,
                "default_value": draft,
                "placeholder": { "tag": "plain_text", "content": "输入要发送的消息" },
            },
            {
                "tag": "select_static",
                "name": FORM_TARGET,
                "placeholder": { "tag": "plain_text", "content": "选择目标 Agent" },
                "options": options,
            },
            {
                "tag": "button",
                "name": "submit",
                "form_action_type": "submit",
                "text": { "tag": "plain_text", "content": "发送" },
                "type": "primary",
                "behaviors": [
                    { "type": "callback", "value": { ACTION_KEY: ACTION_COMPOSE_SUBMIT } }
                ],
            },
        ],
    }));
    skeleton(elements)
}

/// 回复表单卡：展示原文 + 输入框 + 发送按钮（目标锁定原发送 agent）。
pub fn reply_form_card(original: &Message) -> Value {
    let mut elements = styled_header(&format!("回复 {}", original.from_name));
    elements.push(json!({ "tag": "markdown", "content": original.body }));
    elements.push(json!({ "tag": "hr" }));
    elements.push(json!({
        "tag": "form",
        "name": "reply_form",
        "elements": [
            {
                "tag": "input",
                "name": FORM_BODY,
                "placeholder": { "tag": "plain_text", "content": "输入回复内容" },
            },
            {
                "tag": "button",
                "name": "submit",
                "form_action_type": "submit",
                "text": { "tag": "plain_text", "content": "发送" },
                "type": "primary",
                "behaviors": [
                    { "type": "callback", "value": { ACTION_KEY: ACTION_REPLY_SUBMIT, ACTION_MSG_KEY: original.id } }
                ],
            },
        ],
    }));
    skeleton(elements)
}

/// 对话动作终态卡：compose/reply 成功或校验失败时原地替换表单卡。
pub fn status_card(title: &str, body: &str, status_line: &str) -> Value {
    let mut elements = styled_header(title);
    if !body.is_empty() {
        elements.push(json!({ "tag": "markdown", "content": body }));
        elements.push(json!({ "tag": "hr" }));
    }
    elements.push(json!({
        "tag": "markdown",
        "content": format!("**{}**", status_line),
    }));
    skeleton(elements)
}

/// 回复终态卡：保留原消息上下文 + 回复正文 + 状态行。
/// 用于回复提交后原地替换表单卡，以及他端抢答后回写飞书卡片——
/// 让人类在终态卡片上仍能看到「回复的是哪条消息」。
pub fn reply_terminal_card(original: &Message, reply_body: &str, status_line: &str) -> Value {
    let mut elements = styled_header(&format!("回复 {}", original.from_name));
    elements.push(json!({ "tag": "markdown", "content": original.body }));
    elements.push(json!({ "tag": "hr" }));
    elements.push(json!({
        "tag": "div",
        "text": {
            "tag": "plain_text",
            "content": "你的回复",
            "text_size": "notation",
            "text_color": "grey",
        },
    }));
    elements.push(json!({ "tag": "markdown", "content": reply_body }));
    elements.push(json!({
        "tag": "markdown",
        "content": format!("**{}**", status_line),
    }));
    skeleton(elements)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// 终态卡片：样式化头部 + 正文 + 选项回显（选中项 ✅，支持多选）+ 状态行。
pub fn terminal_card(msg: &Message, status_line: &str, selected: &[&str]) -> Value {
    let mut elements = styled_header(&format!("来自 {} 的审批", msg.from_name));
    elements.push(json!({ "tag": "markdown", "content": msg.body }));
    elements.push(json!({ "tag": "hr" }));
    let choices = approval_choices(msg);
    if !choices.is_empty() {
        let lines: Vec<String> = choices
            .iter()
            .map(|c| {
                if selected.contains(&c.as_str()) {
                    format!("✅ {}", c)
                } else {
                    format!("⬜ {}", c)
                }
            })
            .collect();
        elements.push(json!({ "tag": "markdown", "content": lines.join("\n") }));
    }
    elements.push(json!({
        "tag": "markdown",
        "content": format!("**{}**", status_line),
    }));
    skeleton(elements)
}

/// 从审批消息 metadata 取 recommended 选项原文。
pub fn approval_recommended(msg: &Message) -> Option<String> {
    serde_json::from_str::<Value>(&msg.metadata)
        .ok()
        .and_then(|m| {
            m.get("recommended")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

/// 从审批消息 metadata 取 select_only 标记（缺省 false）。
pub fn approval_select_only(msg: &Message) -> bool {
    serde_json::from_str::<Value>(&msg.metadata)
        .ok()
        .and_then(|m| m.get("select_only").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

/// 卡片回调的同步「更新卡片」回包体：`{card:{type:"raw",data:<新卡片>}}`。
/// 长连接回包必须带这层包装，飞书才会把按钮卡片原地换成终态卡片；
/// 裸卡片 JSON 会被忽略——按钮无反应且可重复点击。
pub fn callback_update_card(card: Value) -> Value {
    json!({ "card": { "type": "raw", "data": card } })
}

/// 从审批消息 metadata 取 choices 列表。
pub fn approval_choices(msg: &Message) -> Vec<String> {
    serde_json::from_str::<Value>(&msg.metadata)
        .ok()
        .and_then(|m| m.get("choices").and_then(|c| c.as_array()).cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
