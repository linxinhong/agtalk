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
    /// 审批选中项（空切片 = 自由文本；select_only 审批必填且每项须 ∈ choices；
    /// single 审批至多一项）。多选存 `join("、")` 展示字符串。
    pub choices: &'a [&'a str],
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
///
/// 跨端事件幂等：预生成 reply_id 直接写入 receipt（receipt 存在 ⟺ 回复消息存在，
/// 同事务提交），重复事件回放首次结果；不存在 NULL 占位窗口。
pub fn reply(
    storage: &Storage,
    req: HumanReplyRequest<'_>,
) -> Result<HumanReplyOutcome, HumanError> {
    let mut conn = storage.conn();
    let tx = conn.transaction()?;

    // 1. 跨端事件去重：预生成回复 id 写入 receipt；重复事件回放首次执行的结果
    let reply_id = Uuid::new_v4().to_string();
    if let Some(eid) = req.external_event_id {
        if !delivery::record_receipt(&tx, req.surface, eid, Some(&reply_id), "reply")? {
            let original_reply_id: Option<String> = tx
                .query_row(
                    "SELECT message_id FROM human_action_receipts \
                     WHERE surface = ?1 AND external_event_id = ?2",
                    params![req.surface, eid],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            // receipt 必带结果 id（同事务写入）；NULL 属历史占位数据，无法确定
            // 原动作是否落库，明确报错而不是盲目重放/重试
            let original_reply_id =
                original_reply_id.ok_or_else(|| HumanError::ReceiptInconclusive {
                    surface: req.surface.to_string(),
                    event: eid.to_string(),
                })?;
            let reply_msg = query_message(&tx, &original_reply_id)?
                .ok_or_else(|| HumanError::MessageNotFound(original_reply_id.clone()))?;
            let resolved = tx
                .query_row(
                    "SELECT 1 FROM approval_resolutions WHERE response_message_id = ?1",
                    [&original_reply_id],
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
        let single = meta
            .get("single")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if select_only && req.choices.is_empty() {
            return Err(HumanError::SelectOnlyRequiresChoice);
        }
        if single && req.choices.len() > 1 {
            return Err(HumanError::SingleChoiceOnly(req.choices.len()));
        }
        let known: Vec<&str> = meta
            .get("choices")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        for choice in req.choices {
            if !known.contains(choice) {
                return Err(HumanError::InvalidChoice(choice.to_string()));
            }
        }
    }

    // 4. 插入回复消息（to = 原消息发送方，event_id 按接收方地址分配）
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
    // 多选存 join("、") 展示字符串（agent 阅读用，不做结构化多值存储）
    let selected = req.choices.join("、");
    let metadata = if selected.is_empty() {
        "{}".to_string()
    } else {
        serde_json::json!({ "choice": selected }).to_string()
    };
    // 审批回复正文为空时补「选择：X」——popup/GUI 只传补充文本，
    // 空正文会让 agent 在 msg read 里看不到选择了什么
    let body = if is_approval && !selected.is_empty() && req.body.trim().is_empty() {
        format!("选择：{}", selected)
    } else {
        req.body.to_string()
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
            body,
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
        let selected_choice = if selected.is_empty() {
            None
        } else {
            Some(selected.as_str())
        };
        let changed = tx.execute(
            "INSERT OR IGNORE INTO approval_resolutions \
             (request_message_id, resolved_by, resolution, selected_choice, response_message_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                original.id,
                req.surface,
                content_type,
                selected_choice,
                reply_id
            ],
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

    // 7. receipt 已在第 1 步携带 reply_id 写入，同事务提交，无需事后补写
    tx.commit()?;

    let reply_msg = Message {
        id: reply_id,
        to_address: original.from_address,
        to_name: original.from_name,
        from_address: human_addr,
        from_name: "human".to_string(),
        body,
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

/// human 取消（跨端事件幂等）：给原发送方回一条「（已取消）」并终结原消息。
///
/// 与 reply 的区别：取消是一等结果——agent 收到带 `metadata.cancelled=true` 的回复，
/// `msg wait` 立即返回，不必等超时。approval 消息同时写 resolution（resolution="cancelled"），
/// 后续旧卡点击被仲裁拒绝。单事务：receipt + 回复 + resolution + 原消息置 done。
/// 重复事件回放首次结果（不重复创建回复、不重复置状态）。
pub fn cancel_with_receipt(
    storage: &Storage,
    message_id: &str,
    surface: &str,
    external_event_id: Option<&str>,
) -> Result<(Message, bool), HumanError> {
    let mut conn = storage.conn();
    let tx = conn.transaction()?;

    // 1. 跨端事件去重（与 reply 同构：预生成回复 id 写入 receipt）
    let reply_id = Uuid::new_v4().to_string();
    if let Some(eid) = external_event_id {
        if !delivery::record_receipt(&tx, surface, eid, Some(&reply_id), "cancel")? {
            let original_reply_id: Option<String> = tx
                .query_row(
                    "SELECT message_id FROM human_action_receipts \
                     WHERE surface = ?1 AND external_event_id = ?2",
                    params![surface, eid],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let original_reply_id =
                original_reply_id.ok_or_else(|| HumanError::ReceiptInconclusive {
                    surface: surface.to_string(),
                    event: eid.to_string(),
                })?;
            let reply_msg = query_message(&tx, &original_reply_id)?
                .ok_or_else(|| HumanError::MessageNotFound(original_reply_id.clone()))?;
            return Ok((reply_msg, true));
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
    let original = query_message(&tx, message_id)?
        .filter(|m| m.to_address == human_addr)
        .ok_or_else(|| HumanError::MessageNotFound(message_id.to_string()))?;

    // 3. 插入「（已取消）」回复（to = 原消息发送方，event_id 按接收方地址分配）
    let event_id: i64 = tx
        .query_row(
            "UPDATE event_sequences SET last_event_id = last_event_id + 1 \
             WHERE address = ?1 RETURNING last_event_id",
            [&original.from_address],
            |row| row.get(0),
        )
        .map_err(|_| HumanError::EventIdAllocation)?;
    let now = unix_timestamp();
    let metadata = serde_json::json!({ "cancelled": true }).to_string();
    tx.execute(
        "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, \
         content_type, reply_to_id, subject, metadata, event_id, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'text', ?7, ?8, ?9, ?10, 'pending', ?11)",
        params![
            reply_id,
            original.from_address,
            original.from_name,
            human_addr,
            "human",
            "（已取消）",
            original.id,
            original.subject,
            metadata,
            event_id,
            now
        ],
    )?;

    // 4. approval 消息写取消仲裁：首个动作胜出，后续旧卡点击被 AlreadyResolved 拒绝
    if original.content_type == "approval_request" {
        let changed = tx.execute(
            "INSERT OR IGNORE INTO approval_resolutions \
             (request_message_id, resolved_by, resolution, selected_choice, response_message_id) \
             VALUES (?1, ?2, 'cancelled', NULL, ?3)",
            params![original.id, surface, reply_id],
        )?;
        if changed == 0 {
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

    // 5. 原消息置 done（取消即终结，不再期待处理）
    tx.execute(
        "UPDATE messages SET status = 'done' WHERE id = ?1 AND status != 'done'",
        [&original.id],
    )?;

    tx.commit()?;

    Ok((
        Message {
            id: reply_id,
            to_address: original.from_address,
            to_name: original.from_name,
            from_address: human_addr,
            from_name: "human".to_string(),
            body: "（已取消）".to_string(),
            content_type: "text".to_string(),
            reply_to_id: Some(original.id),
            subject: original.subject,
            metadata,
            event_id,
            status: "pending".to_string(),
            created_at: now,
        },
        false,
    ))
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
#[path = "approval_tests.rs"]
mod tests;
