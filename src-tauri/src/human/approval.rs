//! approval 仲裁：human 回复的原子裁决。
//!
//! - 普通文本消息允许多次 reply；首次 reply 把原消息 pending/delivered → read。
//! - approval_request 首个有效回复原子胜出：单事务内写回复消息 +
//!   approval_resolutions（PK 冲突即已被处理）+ 原消息置 done，任一失败整体回滚。
//! - select_only 的审批拒绝自由文本；choice 必须属于原消息 metadata.choices。
//! - 跨端事件经 receipts 去重（同一 surface + external_event_id 只处理一次）。

use super::{delivery, HumanError};
use crate::routing::{Message, StatusChange};
use crate::storage::Storage;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

pub struct HumanReplyRequest<'a> {
    /// human inbox 中的原消息 id（完整 UUID；短 ID 解析在调用方完成）。
    pub message_id: &'a str,
    /// 回复正文（选择 choice 时的说明文本，可为空）。
    pub body: &'a str,
    /// 审批选项；select_only 审批必填。
    pub choice: Option<&'a str>,
    /// 发起回复的 surface（如 popup / gui / feishu）。
    pub surface: &'a str,
    /// 跨端事件 id（飞书 event、Android command id）；提供时参与去重。
    pub external_event_id: Option<&'a str>,
}

pub struct HumanReplyOutcome {
    pub reply: Message,
    pub original_status_change: Option<StatusChange>,
    /// 是否产生了 approval_resolutions 记录（即原消息是 approval_request）。
    pub resolved: bool,
    /// 是否为重复外部事件：true 时 reply 是首次执行的结果回放，未重复创建消息。
    pub deduplicated: bool,
}

/// human 回复入口：所有校验与写入在单个事务内完成。
pub fn reply(
    storage: &Storage,
    req: HumanReplyRequest<'_>,
) -> Result<HumanReplyOutcome, HumanError> {
    let mut conn = storage.conn();
    let tx = conn.transaction()?;

    // 1. 跨端事件去重：重复事件回放首次执行的结果，不重复创建回复
    if let Some(eid) = req.external_event_id {
        if !delivery::record_receipt(&tx, req.surface, eid, None, "reply")? {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT message_id FROM human_action_receipts \
                     WHERE surface = ?1 AND external_event_id = ?2",
                    params![req.surface, eid],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            match existing {
                Some(reply_id) => {
                    let reply_msg = query_message(&tx, &reply_id)?
                        .ok_or_else(|| HumanError::MessageNotFound(reply_id.clone()))?;
                    let resolved = tx
                        .query_row(
                            "SELECT 1 FROM approval_resolutions WHERE response_message_id = ?1",
                            [&reply_id],
                            |_| Ok(true),
                        )
                        .optional()?
                        .unwrap_or(false);
                    return Ok(HumanReplyOutcome {
                        reply: reply_msg,
                        original_status_change: None,
                        resolved,
                        deduplicated: true,
                    });
                }
                // 上次动作未完成（占位 NULL）：删除占位，本次正常执行
                None => {
                    tx.execute(
                        "DELETE FROM human_action_receipts \
                         WHERE surface = ?1 AND external_event_id = ?2",
                        params![req.surface, eid],
                    )?;
                    delivery::record_receipt(&tx, req.surface, eid, None, "reply")?;
                }
            }
        }
    }

    // 2. 原消息必须存在且属于 human mailbox
    let human_addr: String = tx
        .query_row(
            "SELECT address FROM system_mailboxes WHERE role = 'human'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(HumanError::HumanMailboxMissing)?;
    let original = query_message(&tx, req.message_id)?
        .filter(|m| m.to_address == human_addr)
        .ok_or_else(|| HumanError::MessageNotFound(req.message_id.to_string()))?;

    // 3. approval 校验
    let is_approval = original.content_type == "approval_request";
    if is_approval {
        let meta: serde_json::Value = serde_json::from_str(&original.metadata)?;
        let select_only = meta
            .get("select_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        match req.choice {
            None if select_only => return Err(HumanError::SelectOnlyRequiresChoice),
            Some(choice) => {
                let choices: Vec<&str> = meta
                    .get("choices")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                if !choices.contains(&choice) {
                    return Err(HumanError::InvalidChoice(choice.to_string()));
                }
            }
            None => {}
        }
    }

    // 4. 插入回复消息（to = 原消息发送方，event_id 按接收方地址分配）
    let reply_id = Uuid::new_v4().to_string();
    let event_id: i64 = tx
        .query_row(
            "UPDATE event_sequences SET last_event_id = last_event_id + 1 \
             WHERE address = ?1 RETURNING last_event_id",
            [&original.from_address],
            |row| row.get(0),
        )
        .map_err(|_| HumanError::EventIdAllocation)?;
    let content_type = if is_approval {
        "approval_response"
    } else {
        "text"
    };
    let metadata = match req.choice {
        Some(c) => serde_json::json!({ "choice": c }).to_string(),
        None => "{}".to_string(),
    };
    let now = unix_timestamp();
    tx.execute(
        "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, \
         content_type, reply_to_id, subject, metadata, event_id, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12)",
        params![
            reply_id,
            original.from_address,
            original.from_name,
            human_addr,
            "human",
            req.body,
            content_type,
            original.id,
            original.subject,
            metadata,
            event_id,
            now
        ],
    )?;

    // 5. approval 仲裁：INSERT OR IGNORE + 主键冲突判定首个胜出
    if is_approval {
        let changed = tx.execute(
            "INSERT OR IGNORE INTO approval_resolutions \
             (request_message_id, resolved_by, resolution, selected_choice, response_message_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![original.id, req.surface, content_type, req.choice, reply_id],
        )?;
        if changed == 0 {
            // 回滚事务（撤销回复消息与 receipt），返回已处理信息
            let (resolved_by, response_id): (String, String) = tx.query_row(
                "SELECT resolved_by, response_message_id FROM approval_resolutions \
                 WHERE request_message_id = ?1",
                [&original.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            return Err(HumanError::AlreadyResolved {
                resolved_by,
                response_id,
            });
        }
    }

    // 6. 原消息状态推进
    let status_change = if is_approval {
        update_status(&tx, &original.id, &original.status, "done")?
    } else if original.status == "pending" || original.status == "delivered" {
        update_status(&tx, &original.id, &original.status, "read")?
    } else {
        None
    };

    // 7. 写回 receipt 结果 id：重复事件可回放本次回复
    if let Some(eid) = req.external_event_id {
        tx.execute(
            "INSERT INTO human_action_receipts (surface, external_event_id, message_id, action) \
             VALUES (?1, ?2, ?3, 'reply') \
             ON CONFLICT(surface, external_event_id) DO UPDATE SET message_id = ?3",
            params![req.surface, eid, reply_id],
        )?;
    }

    tx.commit()?;

    let reply_msg = Message {
        id: reply_id,
        to_address: original.from_address,
        to_name: original.from_name,
        from_address: human_addr,
        from_name: "human".to_string(),
        body: req.body.to_string(),
        content_type: content_type.to_string(),
        reply_to_id: Some(original.id),
        subject: original.subject,
        metadata,
        event_id,
        status: "pending".to_string(),
        created_at: now,
    };
    Ok(HumanReplyOutcome {
        reply: reply_msg,
        original_status_change: status_change,
        resolved: is_approval,
        deduplicated: false,
    })
}

fn query_message(conn: &rusqlite::Connection, id: &str) -> Result<Option<Message>, HumanError> {
    let msg = conn
        .query_row(
            "SELECT id, to_address, to_name, from_address, from_name, body, content_type, \
             reply_to_id, subject, metadata, event_id, status, created_at \
             FROM messages WHERE id = ?1",
            [id],
            Message::from_row,
        )
        .optional()?;
    Ok(msg)
}

fn update_status(
    conn: &rusqlite::Connection,
    id: &str,
    old_status: &str,
    new_status: &str,
) -> Result<Option<StatusChange>, HumanError> {
    if old_status == new_status {
        return Ok(None);
    }
    let changed = conn.execute(
        "UPDATE messages SET status = ?2 WHERE id = ?1 AND status = ?3",
        params![id, new_status, old_status],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    conn.execute(
        "INSERT INTO message_status_log (message_id, old_status, new_status) VALUES (?1, ?2, ?3)",
        params![id, old_status, new_status],
    )?;
    Ok(Some(StatusChange {
        message_id: id.to_string(),
        old_status: old_status.to_string(),
        new_status: new_status.to_string(),
    }))
}

fn unix_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HumanConfig;
    use crate::identity::mailbox::{create, ensure_human};
    use crate::routing::lookup;
    use crate::routing::{send::send, SendRequest};

    struct Fixture {
        storage: Storage,
        human_addr: String,
        agent_addr: String,
    }

    fn setup() -> Fixture {
        let storage = Storage::open_in_memory().unwrap();
        let cfg = HumanConfig::default();
        let human_addr = ensure_human(&storage, &cfg).unwrap();
        let agent_addr = create(&storage, "agent", "", "").unwrap();
        Fixture {
            storage,
            human_addr,
            agent_addr,
        }
    }

    fn send_msg(fx: &Fixture, content_type: &str, metadata: &str) -> Message {
        send(
            &fx.storage,
            SendRequest {
                to: &fx.human_addr,
                to_name: "human",
                from: &fx.agent_addr,
                from_name: "agent",
                body: "question",
                content_type,
                reply_to_id: None,
                subject: None,
                metadata,
                more_coming: false,
            },
        )
        .unwrap()
    }

    fn req<'a>(msg: &'a Message, body: &'a str, choice: Option<&'a str>) -> HumanReplyRequest<'a> {
        HumanReplyRequest {
            message_id: &msg.id,
            body,
            choice,
            surface: "popup",
            external_event_id: None,
        }
    }

    #[test]
    fn text_message_allows_multiple_replies() {
        let fx = setup();
        let msg = send_msg(&fx, "text", "{}");

        let first = reply(&fx.storage, req(&msg, "第一条", None)).unwrap();
        assert_eq!(first.reply.to_address, fx.agent_addr);
        assert_eq!(first.reply.reply_to_id.as_deref(), Some(msg.id.as_str()));
        assert_eq!(first.reply.content_type, "text");
        assert!(!first.resolved);
        // 原消息 pending → read
        let change = first.original_status_change.unwrap();
        assert_eq!(change.old_status, "pending");
        assert_eq!(change.new_status, "read");

        // 再次回复允许，且不再产生状态变化
        let second = reply(&fx.storage, req(&msg, "第二条", None)).unwrap();
        assert!(second.original_status_change.is_none());
        assert_ne!(first.reply.id, second.reply.id);
    }

    #[test]
    fn approval_first_valid_reply_wins_and_marks_done() {
        let fx = setup();
        let meta = r#"{"choices":["yes","no"],"select_only":false}"#;
        let msg = send_msg(&fx, "approval_request", meta);

        let out = reply(&fx.storage, req(&msg, "同意", Some("yes"))).unwrap();
        assert!(out.resolved);
        assert_eq!(out.reply.content_type, "approval_response");
        assert_eq!(out.reply.metadata, r#"{"choice":"yes"}"#);
        let change = out.original_status_change.unwrap();
        assert_eq!(change.new_status, "done");

        // resolution 落库
        let conn = fx.storage.conn();
        let (resolved_by, choice): (String, String) = conn
            .query_row(
                "SELECT resolved_by, selected_choice FROM approval_resolutions \
                 WHERE request_message_id = ?1",
                [&msg.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(resolved_by, "popup");
        assert_eq!(choice, "yes");
    }

    #[test]
    fn approval_second_reply_returns_already_resolved() {
        let fx = setup();
        let meta = r#"{"choices":["yes","no"],"select_only":false}"#;
        let msg = send_msg(&fx, "approval_request", meta);

        let first = reply(&fx.storage, req(&msg, "同意", Some("yes"))).unwrap();
        match reply(&fx.storage, req(&msg, "反对", Some("no"))) {
            Err(HumanError::AlreadyResolved {
                resolved_by,
                response_id,
            }) => {
                assert_eq!(resolved_by, "popup");
                assert_eq!(response_id, first.reply.id);
            }
            other => panic!("expected AlreadyResolved, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn select_only_rejects_free_text() {
        let fx = setup();
        let meta = r#"{"choices":["a","b"],"select_only":true}"#;
        let msg = send_msg(&fx, "approval_request", meta);

        match reply(&fx.storage, req(&msg, "随便说说", None)) {
            Err(HumanError::SelectOnlyRequiresChoice) => {}
            other => panic!("expected SelectOnlyRequiresChoice, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn invalid_choice_rejected() {
        let fx = setup();
        let meta = r#"{"choices":["a","b"],"select_only":false}"#;
        let msg = send_msg(&fx, "approval_request", meta);

        match reply(&fx.storage, req(&msg, "", Some("c"))) {
            Err(HumanError::InvalidChoice(c)) => assert_eq!(c, "c"),
            other => panic!("expected InvalidChoice, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn reply_to_non_human_message_rejected() {
        let fx = setup();
        // agent → agent 的消息，human 无权回复
        let other_agent = create(&fx.storage, "agent2", "", "").unwrap();
        let msg = send(
            &fx.storage,
            SendRequest {
                to: &other_agent,
                to_name: "agent2",
                from: &fx.agent_addr,
                from_name: "agent",
                body: "not for human",
                content_type: "text",
                reply_to_id: None,
                subject: None,
                metadata: "{}",
                more_coming: false,
            },
        )
        .unwrap();

        match reply(&fx.storage, req(&msg, "hack", None)) {
            Err(HumanError::MessageNotFound(_)) => {}
            other => panic!("expected MessageNotFound, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn duplicate_external_event_replays_original_reply() {
        let fx = setup();
        let msg = send_msg(&fx, "text", "{}");

        let r1 = HumanReplyRequest {
            external_event_id: Some("evt-1"),
            ..req(&msg, "第一条", None)
        };
        let first = reply(&fx.storage, r1).unwrap();
        assert!(!first.deduplicated);

        // 重复事件：回放首次结果，不重复创建回复
        let r2 = HumanReplyRequest {
            external_event_id: Some("evt-1"),
            ..req(&msg, "重复", None)
        };
        let second = reply(&fx.storage, r2).unwrap();
        assert!(second.deduplicated);
        assert_eq!(second.reply.id, first.reply.id);

        // agent inbox 里只有一条回复
        let inbox = crate::routing::inbox::inbox(&fx.storage, &fx.agent_addr, false).unwrap();
        let replies: Vec<_> = inbox
            .iter()
            .filter(|m| m.reply_to_id.as_deref() == Some(msg.id.as_str()))
            .collect();
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn aborted_receipt_allows_retry() {
        let fx = setup();
        let msg = send_msg(&fx, "text", "{}");

        // 模拟上次动作中途失败：占位 NULL 的 receipt
        {
            let conn = fx.storage.conn();
            delivery::record_receipt(&conn, "popup", "evt-crash", None, "reply").unwrap();
        }

        // 重复事件但无结果：允许重新执行
        let r = HumanReplyRequest {
            external_event_id: Some("evt-crash"),
            ..req(&msg, "重试", None)
        };
        let out = reply(&fx.storage, r).unwrap();
        assert!(!out.deduplicated);
    }

    #[test]
    fn reply_allocates_event_id_on_receiver() {
        let fx = setup();
        let msg = send_msg(&fx, "text", "{}");
        let out = reply(&fx.storage, req(&msg, "hi", None)).unwrap();
        // 回复消息落在 agent 的 event 序列上，可用 agent inbox 查到
        let inbox = crate::routing::inbox::inbox(&fx.storage, &fx.agent_addr, false).unwrap();
        assert!(inbox.iter().any(|m| m.id == out.reply.id));
        // 短 ID 可解析
        let resolved = lookup::resolve_id(&fx.storage, &fx.agent_addr, &out.reply.id[..8]).unwrap();
        assert_eq!(resolved, out.reply.id);
    }
}
