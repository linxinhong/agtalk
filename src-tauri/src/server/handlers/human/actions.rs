//! human 审批动作：reply（approval 仲裁）与 done（标记完成）。

use super::{after_human_reply, authenticate_human, err, human_error_msg, resolve_human_id};
use crate::human::{self, approval};
use crate::proto::ServerMsg;
use crate::routing::inbox;
use crate::server::state::AppState;
use axum::http::HeaderMap;

pub fn handle_reply(
    state: &AppState,
    headers: &HeaderMap,
    message_id: String,
    body: String,
    choice: Option<String>,
    surface: String,
    external_event_id: Option<String>,
) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let resolved = match resolve_human_id(state, &session.address, &message_id) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match approval::reply(
        &state.storage,
        approval::HumanReplyRequest {
            message_id: &resolved,
            body: &body,
            choice: choice.as_deref(),
            surface: &surface,
            external_event_id: external_event_id.as_deref(),
        },
    ) {
        Ok(out) => {
            // 重复事件回放：消息未重复创建，也不再触发 SSE/notify 与抢答收尾
            if !out.deduplicated {
                after_human_reply(state, &out.reply);
                // 抢答收尾：他端 surface 的展示同步收敛
                // - 本端非 popup：关闭该消息的桌面弹窗（feishu/GUI/API 胜出）
                if surface != crate::human::popup::POPUP_SURFACE {
                    state.popup.settle(&resolved);
                }
                // - 本端非 feishu：回写飞书卡片为终态（approval 回显选项，文本回显回复）
                if surface != crate::feishu::router::SURFACE {
                    state
                        .feishu
                        .settle(&state.storage, &resolved, &surface, Some(&out.reply));
                }
            }
            ServerMsg::Ok { id: out.reply.id }
        }
        Err(e) => human_error_msg(&e),
    }
}

pub fn handle_done(
    state: &AppState,
    headers: &HeaderMap,
    message_id: String,
    surface: Option<String>,
    external_event_id: Option<String>,
) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let resolved = match resolve_human_id(state, &session.address, &message_id) {
        Ok(r) => r,
        Err(e) => return e,
    };
    // 提供 surface+external_event_id 时走同事务幂等路径；否则普通 mark_done
    match (&surface, &external_event_id) {
        (Some(surface), Some(eid)) => {
            match human::done_with_receipt(
                &state.storage,
                &resolved,
                &session.address,
                surface,
                eid,
            ) {
                Ok(_) => ServerMsg::Ok { id: resolved },
                Err(e) => human_error_msg(&e),
            }
        }
        _ => match inbox::mark_done(&state.storage, &resolved, &session.address) {
            Ok(_) => ServerMsg::Ok { id: resolved },
            Err(e) => err("done_failed", e),
        },
    }
}

/// delivery 回执：surface 确认已展示该消息。delivery 行不存在返回 delivery_not_found。
pub fn handle_delivery_ack(
    state: &AppState,
    headers: &HeaderMap,
    message_id: String,
    surface: String,
) -> ServerMsg {
    if let Err(e) = authenticate_human(state, headers) {
        return e;
    }
    if surface.trim().is_empty() {
        return ServerMsg::Error {
            code: "invalid_surface".into(),
            message: "surface 不能为空".into(),
        };
    }
    match human::delivery::ack(&state.storage, &message_id, &surface) {
        Ok(true) => ServerMsg::Ok { id: message_id },
        Ok(false) => ServerMsg::Error {
            code: "delivery_not_found".into(),
            message: format!("delivery 不存在: {} / {}", message_id, surface),
        },
        Err(e) => human_error_msg(&e),
    }
}

#[cfg(test)]
mod tests {
    use crate::identity::mailbox::create;
    use crate::proto::ServerMsg;
    use crate::routing::{inbox, lookup};
    use crate::server::handlers::human::testkit::*;

    #[tokio::test]
    async fn human_reply_routes_through_approval_arbitration() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "asker", "", "").unwrap();
        let meta = r#"{"choices":["yes","no"],"select_only":true}"#;
        let msg_id = send_to_human(&fx, &agent_addr, "approval_request", meta);

        // select_only 无 choice → 拒绝
        match super::handle_reply(
            &fx.state,
            &headers_with(&fx.token),
            msg_id.clone(),
            "free text".into(),
            None,
            "popup".into(),
            None,
        ) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "select_only_requires_choice"),
            other => panic!("expected select_only_requires_choice, got {:?}", other),
        }

        // 有效 choice → 胜出
        let first = super::handle_reply(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            "ok".into(),
            Some("yes".into()),
            "popup".into(),
            None,
        );
        let reply_id = match first {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        // 第二次审批 → already_resolved
        match super::handle_reply(
            &fx.state,
            &headers_with(&fx.token),
            msg_id,
            "no".into(),
            Some("no".into()),
            "gui".into(),
            None,
        ) {
            ServerMsg::Error { code, message } => {
                assert_eq!(code, "already_resolved");
                assert!(message.contains(&reply_id));
            }
            other => panic!("expected already_resolved, got {:?}", other),
        }

        // 回复落到 asker 的 inbox
        let agent_inbox = inbox::inbox(&fx.state.storage, &agent_addr, false).unwrap();
        assert!(agent_inbox.iter().any(|m| m.id == reply_id));
    }

    #[test]
    fn human_delivery_ack_marks_delivered() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        let msg_id = send_to_human(&fx, &agent_addr, "text", "{}");
        crate::human::fanout(&fx.state.storage, &fx.state.config.human, &{
            crate::routing::lookup::detail(&fx.state.storage, &msg_id)
                .unwrap()
                .unwrap()
        })
        .unwrap();

        match super::handle_delivery_ack(
            &fx.state,
            &headers_with(&fx.token),
            msg_id.clone(),
            "popup".into(),
        ) {
            ServerMsg::Ok { id } => assert_eq!(id, msg_id),
            other => panic!("expected Ok, got {:?}", other),
        }
        let ds = crate::human::delivery::list_for_message(&fx.state.storage, &msg_id).unwrap();
        assert_eq!(ds[0].status, "delivered");

        // 不存在的 delivery → delivery_not_found
        match super::handle_delivery_ack(
            &fx.state,
            &headers_with(&fx.token),
            msg_id,
            "wechat".into(),
        ) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "delivery_not_found"),
            other => panic!("expected delivery_not_found, got {:?}", other),
        }
    }

    #[test]
    fn human_done_marks_message_done() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        let msg_id = send_to_human(&fx, &agent_addr, "text", "{}");

        match super::handle_done(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            None,
            None,
        ) {
            ServerMsg::Ok { id } => assert_eq!(id, msg_id),
            other => panic!("expected Ok, got {:?}", other),
        }
        let msg = lookup::detail(&fx.state.storage, &msg_id).unwrap().unwrap();
        assert_eq!(msg.status, "done");
    }

    #[test]
    fn human_done_duplicate_event_replays_original_result() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        let msg_id = send_to_human(&fx, &agent_addr, "text", "{}");

        let first = super::handle_done(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            Some("android".into()),
            Some("cmd-done-1".into()),
        );
        match first {
            ServerMsg::Ok { id } => assert_eq!(id, msg_id),
            other => panic!("expected Ok, got {:?}", other),
        }

        // 重复事件：回放原结果
        let second = super::handle_done(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            Some("android".into()),
            Some("cmd-done-1".into()),
        );
        match second {
            ServerMsg::Ok { id } => assert_eq!(id, msg_id),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn human_done_inconclusive_receipt_errors() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        let msg_id = send_to_human(&fx, &agent_addr, "text", "{}");

        // 历史占位数据：receipt 无结果 id，无法确定原动作是否已落库
        {
            let conn = fx.state.storage.conn();
            crate::human::delivery::record_receipt(&conn, "android", "cmd-crash", None, "done")
                .unwrap();
        }

        // 同一事件：明确报错，不盲目重放/重试
        match super::handle_done(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            Some("android".into()),
            Some("cmd-crash".into()),
        ) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "receipt_inconclusive"),
            other => panic!("expected receipt_inconclusive, got {:?}", other),
        }
    }

    #[test]
    fn human_done_failed_event_rolls_back_and_allows_retry() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        let msg_id = send_to_human(&fx, &agent_addr, "text", "{}");

        // 第一次事件用在不存在的消息上：事务回滚，receipt 无残留
        match super::handle_done(
            &fx.state,
            &headers_with(&fx.token),
            "zzzzzzzz-dead-beef".into(),
            Some("android".into()),
            Some("cmd-retry".into()),
        ) {
            ServerMsg::Error { .. } => {}
            other => panic!("expected error, got {:?}", other),
        }
        let count: i64 = fx
            .state
            .storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM human_action_receipts WHERE external_event_id = 'cmd-retry'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        // 同一事件修正后重试成功
        match super::handle_done(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            Some("android".into()),
            Some("cmd-retry".into()),
        ) {
            ServerMsg::Ok { id } => assert_eq!(id, msg_id),
            other => panic!("expected Ok after retry, got {:?}", other),
        }
        let msg = lookup::detail(&fx.state.storage, &msg_id).unwrap().unwrap();
        assert_eq!(msg.status, "done");
    }
}
