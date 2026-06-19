#[cfg(test)]
mod tests {
    use crate::config::AgConfig;
    use crate::ipc::ClientMsg;
    use crate::notify::{NotifyContext, NotifyPlugin, NotifyPluginRegistry};
    use crate::server::handle_msg;
    use crate::storage::Storage;
    use crate::transport::TransportRegistry;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    fn temp_attachment_dir() -> String {
        std::env::temp_dir()
            .join(format!("agtalk-test-attachments-{}", Uuid::new_v4()))
            .to_string_lossy()
            .to_string()
    }

    fn test_config() -> AgConfig {
        let mut cfg = AgConfig::default();
        cfg.storage.attachment_dir = temp_attachment_dir();
        cfg
    }

    fn storage() -> Storage {
        storage_with_config(test_config())
    }

    fn storage_with_config(mut cfg: AgConfig) -> Storage {
        cfg.storage.attachment_dir = temp_attachment_dir();
        Storage::open_memory_with_config(Arc::new(cfg)).unwrap()
    }

    #[test]
    fn test_register_and_lookup() {
        let s = storage();
        let p = s
            .register_participant(
                None,
                "kimi_coder_Alex",
                "agent",
                "Alex",
                "terminal",
                "{}",
                "coding agent",
                "agent",
            )
            .unwrap();
        assert_eq!(p.name, "kimi_coder_Alex");
        assert_eq!(p.intro, "coding agent");
        assert!(s
            .get_participant_by_name("kimi_coder_Alex")
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_register_reserved_names() {
        let s = storage();
        for name in ["me", "human", "ME", "Human"] {
            let result = s.register_participant(None, name, "agent", name, "terminal", "{}", "", "agent");
            assert!(result.is_err(), "保留名 {} 应注册失败", name);
        }
    }

    #[test]
    fn test_list_participants() {
        let s = storage();
        s.register_participant(None, "a", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "b", "human", "B", "popup", "{}", "", "agent")
            .unwrap();
        assert_eq!(s.list_participants(None).unwrap().len(), 2);
        assert_eq!(s.list_participants(Some("agent")).unwrap().len(), 1);
    }

    #[test]
    fn test_send_and_list() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "Alice", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "Bob", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "Hi",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(msg.sender_name, "alice");
        let convs = s.list_conversations(Some("bob")).unwrap();
        assert_eq!(convs.len(), 1);
        let msgs = s.get_messages(&msg.conversation_id, 50, None).unwrap();
        assert_eq!(msgs[0].body, "Hi");
    }

    #[test]
    fn test_mark_done() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "task",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        s.mark_delivered(&msg.id, "bob").unwrap();
        s.mark_read(&msg.id, "bob", Some("sess_1")).unwrap();
        s.mark_done(&msg.id, "bob", Some("sess_1"), &[]).unwrap();
        let msgs = s.get_messages(&msg.conversation_id, 50, None).unwrap();
        assert_eq!(msgs[0].recipients[0].status, "done");
        assert!(msgs[0].recipients[0].read_by_session_id.is_some());
        assert!(msgs[0].recipients[0].done_by_session_id.is_some());
    }

    #[test]
    fn test_unread_count() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        s.send_message(
            "alice",
            &["bob".into()],
            "1",
            "text",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        s.send_message(
            "alice",
            &["bob".into()],
            "2",
            "text",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            s.list_conversations(Some("bob")).unwrap()[0].counts.unread,
            2
        );
    }

    #[test]
    fn test_reply_chain() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let m1 = s
            .send_message(
                "alice",
                &["bob".into()],
                "Q",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let m2 = s
            .send_message(
                "bob",
                &["alice".into()],
                "A",
                "text",
                Some(&m1.id),
                Some(&m1.conversation_id),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(m2.reply_to_id, Some(m1.id));
        assert_eq!(
            s.get_messages(&m1.conversation_id, 50, None).unwrap().len(),
            2
        );
    }

    #[test]
    fn test_unregister() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.unregister_participant("alice").unwrap();
        assert!(s.get_participant_by_name("alice").unwrap().is_none());
    }

    #[test]
    fn test_full_message_bus() {
        let s = storage();
        s.register_participant(None, "@codex", "agent", "C", "cli", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "@me", "human", "M", "gui", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "@rv", "agent", "R", "terminal", "{}", "", "agent")
            .unwrap();

        // codex → me
        let m1 = s
            .send_message(
                "@codex",
                &["@me".into()],
                "deploy?",
                "approval_request",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            s.list_conversations(Some("@me")).unwrap()[0].counts.unread,
            1
        );
        s.mark_read(&m1.id, "@me", None).unwrap();

        // me → codex reply
        s.send_message(
            "@me",
            &["@codex".into()],
            "ok",
            "approval_response",
            Some(&m1.id),
            Some(&m1.conversation_id),
            None,
            None,
            None,
        )
        .unwrap();
        let msgs = s.get_messages(&m1.conversation_id, 50, None).unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn test_ask_reply_flow() {
        let s = storage();
        s.register_participant(None, "@agent", "agent", "Ag", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "@human", "human", "Hu", "gui", "{}", "", "agent")
            .unwrap();

        // Agent asks
        let ask = s
            .send_message(
                "@agent",
                &["@human".into()],
                "删除 target?",
                "approval_request",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(ask.content_type, "approval_request");

        // Human replies
        let reply = s
            .send_message(
                "@human",
                &["@agent".into()],
                "同意",
                "approval_response",
                Some(&ask.id),
                Some(&ask.conversation_id),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(reply.reply_to_id, Some(ask.id.clone()));

        let msgs = s.get_messages(&ask.conversation_id, 50, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content_type, "approval_request");
        assert_eq!(msgs[1].content_type, "approval_response");
    }

    #[test]
    fn test_two_participant_conversation() {
        // ── 场景：@codex 和 @emma 的完整对话 ──
        let storage = storage();

        // 1. 注册两个参与者
        storage
            .register_participant(None, "@codex", "agent", "Codex", "terminal", "{}", "coding", "agent")
            .unwrap();
        storage
            .register_participant(
                None,
                "@emma",
                "human",
                "Emma",
                "gui",
                "{}",
                "human reviewer",
                "human",
            )
            .unwrap();

        // 2. @codex 向 @emma 发审批请求
        let ask = storage
            .send_message(
                "@codex",
                &["@emma".into()],
                "是否允许部署到生产环境？",
                "approval_request",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(ask.sender_name, "@codex");
        assert_eq!(ask.content_type, "approval_request");

        // 3. @emma 查看收件箱
        let emma_inbox = storage.list_conversations(Some("@emma")).unwrap();
        assert_eq!(emma_inbox.len(), 1);
        assert_eq!(emma_inbox[0].counts.unread, 1);
        assert!(emma_inbox[0].last_message.is_some());
        eprintln!(
            "  @emma 收到: {}",
            emma_inbox[0].last_message.as_ref().unwrap().body
        );

        // 4. @emma 读取消息
        storage.mark_read(&ask.id, "@emma", None).unwrap();
        let msgs = storage
            .get_messages(&ask.conversation_id, 50, None)
            .unwrap();
        assert_eq!(msgs.len(), 1);
        eprintln!("  @emma 读取: [{}] {}", msgs[0].sender_name, msgs[0].body);

        // 5. @emma 回复同意
        let reply = storage
            .send_message(
                "@emma",
                &["@codex".into()],
                "同意部署，我检查过所有测试通过了",
                "approval_response",
                Some(&ask.id),
                Some(&ask.conversation_id),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(reply.sender_name, "@emma");
        assert_eq!(reply.reply_to_id.as_ref(), Some(&ask.id));
        eprintln!("  @emma 回复: {}", reply.body);

        // 6. @codex 查看 inbox（应有 1 条未读回复）
        let codex_inbox = storage.list_conversations(Some("@codex")).unwrap();
        assert_eq!(codex_inbox.len(), 1);
        assert_eq!(codex_inbox[0].counts.unread, 1);
        eprintln!(
            "  @codex 收到回复，未读数: {}",
            codex_inbox[0].counts.unread
        );

        // 7. @codex 读取完整对话
        let full = storage
            .get_messages(&ask.conversation_id, 50, None)
            .unwrap();
        assert_eq!(full.len(), 2);
        eprintln!("  ── 完整对话 ──");
        for m in &full {
            let status = m
                .recipients
                .first()
                .map(|r| r.status.as_str())
                .unwrap_or("?");
            eprintln!(
                "  [{}] {}: {}  ({})",
                m.content_type, m.sender_name, m.body, status
            );
        }

        // 8. @codex 标记完成
        storage.mark_done(&reply.id, "@codex", None, &[]).unwrap();
        let final_inbox = storage.list_conversations(Some("@codex")).unwrap();
        assert_eq!(final_inbox[0].counts.unread, 0);

        // 9. 验证对话类型和回复链
        assert_eq!(full[0].content_type, "approval_request");
        assert_eq!(full[1].content_type, "approval_response");
        assert_eq!(full[1].reply_to_id, Some(full[0].id.clone()));
    }

    #[test]
    fn test_created_at_nonzero() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "Hi",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(
            msg.created_at > 0.0,
            "created_at should be real timestamp, got {}",
            msg.created_at
        );
    }

    #[test]
    fn test_metadata_stored() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let meta = r#"{"choices":["approve","reject"],"timeout":60}"#;
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "deploy?",
                "approval_request",
                None,
                None,
                None,
                None,
                Some(meta),
            )
            .unwrap();
        assert_eq!(msg.metadata, meta);
        // Verify round-trip through get_messages
        let msgs = s.get_messages(&msg.conversation_id, 50, None).unwrap();
        assert_eq!(msgs[0].metadata, meta);
    }

    #[test]
    fn test_metadata_default_empty() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "Hi",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(msg.metadata, "{}");
    }

    #[test]
    fn test_inbox_structure() {
        let storage = storage();
        storage.ensure_default_human().unwrap();
        storage
            .register_participant(
                Some("agent-a"),
                "agent-a",
                "agent",
                "Agent A",
                "terminal",
                "{}",
                "coding assistant",
                "agent",
            )
            .unwrap();

        // Send a text message from agent-a to me
        let msg = storage
            .send_message(
                "agent-a",
                &["me".to_string()],
                "Hello world",
                "text",
                None,
                None,
                None,
                None,
                Some(r#"{"subject":"测试标题","notify":true}"#),
            )
            .unwrap();

        // Serialize the message to JSON
        let json = serde_json::to_value(&msg).unwrap();

        // Verify chat_id is present (not conversation_id)
        assert!(
            json.get("chat_id").is_some(),
            "Message JSON should have chat_id key"
        );
        assert!(
            json.get("conversation_id").is_none(),
            "Message JSON should NOT have conversation_id key"
        );

        // Verify subject is at top level
        assert_eq!(
            json.get("subject").and_then(|v| v.as_str()),
            Some("测试标题")
        );

        // Verify created_at is ISO8601 string
        let created_at = json.get("created_at").and_then(|v| v.as_str()).unwrap();
        assert!(
            created_at.contains("T"),
            "created_at should be ISO8601: {}",
            created_at
        );
        assert!(
            created_at.ends_with("Z") || created_at.contains("+"),
            "created_at should have timezone: {}",
            created_at
        );

        // Now test inbox
        let items = storage.list_inbox("me", None, 50).unwrap();
        assert!(!items.is_empty(), "Inbox should have items");

        let item_json = serde_json::to_value(&items[0]).unwrap();
        println!(
            "InboxItem JSON: {}",
            serde_json::to_string_pretty(&item_json).unwrap()
        );

        // Verify new flat InboxItem structure
        assert!(item_json.get("id").is_some());
        assert!(item_json.get("kind").is_some());
        assert!(item_json.get("priority").is_some());
        assert!(item_json.get("action_required").is_some());
        assert!(item_json.get("actions").is_some());

        let from = item_json.get("from").unwrap();
        assert!(from.get("id").is_some());
        assert!(from.get("name").is_some());
        assert!(from.get("type").is_some());
        assert_eq!(
            from.get("intro").and_then(|v| v.as_str()),
            Some("coding assistant")
        );

        let content = item_json.get("content").unwrap();
        assert!(content.get("mode").is_some());
        assert!(content.get("body").is_some());
        assert!(content.get("truncated").is_some());
        assert!(content.get("size").is_some());

        let delivery = item_json.get("delivery").unwrap();
        assert!(delivery.get("status").is_some());

        assert_eq!(item_json.get("action_required").unwrap(), false);
        assert_eq!(
            item_json.get("priority").and_then(|v| v.as_str()).unwrap(),
            "normal"
        );
        assert_eq!(
            item_json.get("kind").and_then(|v| v.as_str()).unwrap(),
            "message"
        );
        assert_eq!(
            item_json.get("subject").and_then(|v| v.as_str()),
            Some("测试标题")
        );
    }

    #[test]
    fn test_inbox_action_required_for_approval_requests() {
        let storage = storage();
        storage.ensure_default_human().unwrap();
        storage
            .register_participant(
                Some("agent-a"),
                "agent-a",
                "agent",
                "Agent A",
                "terminal",
                "{}",
                "",
                "agent",
            )
            .unwrap();

        // Send an approval_request
        storage
            .send_message(
                "agent-a",
                &["me".to_string()],
                "Delete target dir?",
                "approval_request",
                None,
                None,
                None,
                None,
                Some(r#"{"choices":["yes","no"]}"#),
            )
            .unwrap();

        let items = storage.list_inbox("me", None, 50).unwrap();
        assert_eq!(items.len(), 1);

        let item_json = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(item_json.get("action_required").unwrap(), true);
        assert_eq!(
            item_json.get("priority").and_then(|v| v.as_str()).unwrap(),
            "high"
        );
        assert_eq!(
            item_json.get("kind").and_then(|v| v.as_str()).unwrap(),
            "approval"
        );
        let actions = item_json.get("actions").unwrap().as_array().unwrap();
        assert!(actions.iter().any(|a| a.as_str() == Some("approve")));
        assert!(actions.iter().any(|a| a.as_str() == Some("reject")));
    }

    #[test]
    fn test_inbox_filters() {
        let storage = storage();
        storage.ensure_default_human().unwrap();
        storage
            .register_participant(
                Some("agent-a"),
                "agent-a",
                "agent",
                "Agent A",
                "terminal",
                "{}",
                "",
                "agent",
            )
            .unwrap();

        // Send a message and a question
        let m1 = storage
            .send_message(
                "agent-a",
                &["me".to_string()],
                "Hello",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let m2 = storage
            .send_message(
                "agent-a",
                &["me".to_string()],
                "Should we proceed?",
                "question",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // Default (unread): both should appear
        let items = storage.list_inbox("me", None, 50).unwrap();
        assert_eq!(items.len(), 2);

        // action_required filter: only question
        let items = storage
            .list_inbox("me", Some("action_required"), 50)
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "question");

        // Mark text message as done
        storage.mark_done(&m1.id, "me", None, &[]).unwrap();

        // After marking done, default filter should exclude done item
        let items = storage.list_inbox("me", None, 50).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, m2.id);

        // all filter should also exclude done items
        let items = storage.list_inbox("me", Some("all"), 50).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_mark_done_sets_done_at() {
        let storage = storage();
        storage.ensure_default_human().unwrap();
        storage
            .register_participant(
                Some("agent-a"),
                "agent-a",
                "agent",
                "Agent A",
                "terminal",
                "{}",
                "",
                "agent",
            )
            .unwrap();

        let msg = storage
            .send_message(
                "agent-a",
                &["me".to_string()],
                "Hello",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        storage.mark_done(&msg.id, "me", Some("sess_1"), &[]).unwrap();

        let items = storage.list_inbox("me", None, 50).unwrap();
        // After mark_done, the item should be excluded from inbox (status=done)
        assert!(items.is_empty(), "Done items should not appear in inbox");

        // Verify done_at is set by checking the DB directly
        let conn = storage.conn();
        let done_at: Option<f64> = conn.query_row(
            "SELECT done_at FROM message_recipients WHERE message_id = ?1 AND recipient_id = 'me'",
            rusqlite::params![msg.id],
            |row| row.get(0),
        ).unwrap();
        assert!(done_at.is_some(), "done_at should be set after mark_done");
        assert!(
            done_at.unwrap() > 0.0,
            "done_at should be a positive timestamp"
        );

        let done_by: Option<String> = conn.query_row(
            "SELECT done_by_session_id FROM message_recipients WHERE message_id = ?1 AND recipient_id = 'me'",
            rusqlite::params![msg.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(done_by.as_deref(), Some("sess_1"));
    }

    #[test]
    fn test_delivery_times_iso() {
        let storage = storage();
        storage.ensure_default_human().unwrap();
        storage
            .register_participant(
                Some("agent-a"),
                "agent-a",
                "agent",
                "Agent A",
                "terminal",
                "{}",
                "",
                "agent",
            )
            .unwrap();

        let msg = storage
            .send_message(
                "agent-a",
                &["me".to_string()],
                "Hello",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        storage.mark_delivered(&msg.id, "me").unwrap();
        storage.mark_read(&msg.id, "me", Some("sess_1")).unwrap();

        let items = storage.list_inbox("me", Some("all"), 50).unwrap();
        assert_eq!(items.len(), 1);

        let item_json = serde_json::to_value(&items[0]).unwrap();
        let delivery = item_json.get("delivery").unwrap();

        let delivered_at = delivery.get("delivered_at").and_then(|v| v.as_str());
        let read_at = delivery.get("read_at").and_then(|v| v.as_str());
        let read_by = delivery.get("read_by_session_id").and_then(|v| v.as_str());

        assert!(delivered_at.is_some(), "delivered_at should be set");
        assert!(read_at.is_some(), "read_at should be set");
        assert_eq!(read_by, Some("sess_1"));
        assert!(
            delivered_at.unwrap().contains("T"),
            "delivered_at should be ISO8601"
        );
        assert!(read_at.unwrap().contains("T"), "read_at should be ISO8601");
    }

    #[test]
    fn test_inbox_default_excludes_done() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let m1 = s
            .send_message(
                "alice",
                &["bob".into()],
                "task1",
                "task",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let m2 = s
            .send_message(
                "alice",
                &["bob".into()],
                "msg2",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // default inbox (None filter) should include both non-done items
        let inbox = s.list_inbox("bob", None, 50).unwrap();
        assert_eq!(inbox.len(), 2);

        // mark one done
        s.mark_done(&m1.id, "bob", None, &[]).unwrap();
        let inbox = s.list_inbox("bob", None, 50).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].id, m2.id);

        // --unread filter excludes read/done
        s.mark_read(&m2.id, "bob", None).unwrap();
        let unread = s.list_inbox("bob", Some("unread"), 50).unwrap();
        assert_eq!(unread.len(), 0);

        // --all filter is same as default: non-done
        let all = s.list_inbox("bob", Some("all"), 50).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, m2.id);
    }

    #[test]
    fn test_inbox_action_required() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let approval = s
            .send_message(
                "alice",
                &["bob".into()],
                "approve?",
                "approval_request",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        s.send_message(
            "alice",
            &["bob".into()],
            "chat",
            "text",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let action = s.list_inbox("bob", Some("action_required"), 50).unwrap();
        assert_eq!(action.len(), 1);
        assert_eq!(action[0].id, approval.id);
        assert!(action[0].action_required);
        assert_eq!(action[0].kind, "approval");
        assert_eq!(action[0].priority, "high");
    }

    #[test]
    fn test_conversation_counts_include_pending() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "task",
                "task",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let conv = &s.list_conversations(Some("bob")).unwrap()[0];
        assert_eq!(conv.counts.unread, 1);
        assert_eq!(conv.counts.pending, 1);

        s.mark_delivered(&msg.id, "bob").unwrap();
        let conv = &s.list_conversations(Some("bob")).unwrap()[0];
        assert_eq!(conv.counts.unread, 1);
        assert_eq!(conv.counts.pending, 0);

        s.mark_read(&msg.id, "bob", None).unwrap();
        let conv = &s.list_conversations(Some("bob")).unwrap()[0];
        assert_eq!(conv.counts.unread, 0);
        assert_eq!(conv.counts.pending, 0);
    }

    // ─── 自动已读新测试 ──────────────────────────────────

    #[test]
    fn test_inbox_auto_mark_read() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "task",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // 通过 mark_messages_read 模拟 daemon 的 inbox 消费行为
        s.mark_messages_read(&[msg.id.clone()], "bob", Some("sess_bob"))
            .unwrap();

        let recipients = s.get_recipients_for_msg_by_id(&msg.id).unwrap();
        let bob_status = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob_status.status, "read");
        assert!(bob_status.read_at.is_some());
        assert_eq!(bob_status.read_by_session_id.as_deref(), Some("sess_bob"));
    }

    #[test]
    fn test_detail_auto_mark_read() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "task",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let detail = s
            .get_message_by_id(&msg.id, Some("bob"), Some("sess_bob"))
            .unwrap();
        assert!(detail.is_some());

        let recipients = s.get_recipients_for_msg_by_id(&msg.id).unwrap();
        let bob_status = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob_status.status, "read");
        assert!(bob_status.read_at.is_some());
    }

    #[test]
    fn test_detail_accepts_short_message_id() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "task",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let short_id: String = msg.id.chars().take(8).collect();

        let detail = s
            .get_message_by_id(&short_id, Some("bob"), Some("sess_bob"))
            .unwrap()
            .unwrap();
        assert_eq!(detail.id, msg.id);

        let recipients = s.get_recipients_for_msg_by_id(&msg.id).unwrap();
        let bob_status = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob_status.status, "read");
    }

    #[test]
    fn test_done_implies_read() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                "task",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        s.mark_done(&msg.id, "bob", Some("sess_bob"), &[]).unwrap();

        let recipients = s.get_recipients_for_msg_by_id(&msg.id).unwrap();
        let bob_status = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob_status.status, "done");
        assert!(bob_status.read_at.is_some());
        assert!(bob_status.done_at.is_some());
        assert_eq!(bob_status.read_by_session_id.as_deref(), Some("sess_bob"));
        assert_eq!(bob_status.done_by_session_id.as_deref(), Some("sess_bob"));
    }

    #[test]
    fn test_reply_implies_read() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        let m1 = s
            .send_message(
                "alice",
                &["bob".into()],
                "Q",
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // bob 回复 alice，应把 m1 对 bob 标为 read
        // 注意：send_message 内部不会自动 mark_read；这是 daemon 在 Send handler 里做的。
        //  storage 层测试直接调用 mark_read 验证语义。
        s.mark_read(&m1.id, "bob", Some("sess_bob")).unwrap();

        let recipients = s.get_recipients_for_msg_by_id(&m1.id).unwrap();
        let bob_status = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob_status.status, "read");
    }

    #[test]
    fn test_chats_no_auto_read() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        s.send_message(
            "alice",
            &["bob".into()],
            "task",
            "text",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // list_conversations 不改变 read_at
        let _ = s.list_conversations(Some("bob")).unwrap();

        let conv = &s.list_conversations(Some("bob")).unwrap()[0];
        assert_eq!(conv.counts.unread, 1);
    }

    // ─── 附件与配置新测试 ─────────────────────────────────

    #[test]
    fn test_long_message_creates_attachment() {
        let mut cfg = AgConfig::default();
        cfg.message.attachment_threshold_bytes = 100; // 降低阈值便于测试
        cfg.message.preview_limit_chars = 20;
        let s = storage_with_config(cfg);
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let long_body = "a".repeat(200);
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                &long_body,
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        assert!(msg.attachments.iter().any(|a| a.role == "full_body"));
        assert!(msg.full_body.is_some());
        assert_eq!(msg.full_body.as_ref().unwrap().len(), 200);
        assert!(msg.body.len() < long_body.len());

        let items = s.list_inbox("bob", None, 50).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content.mode, "summary");
        assert_eq!(items[0].content.size, 200);
        assert!(items[0].content.truncated);
        assert!(items[0].attachments.iter().any(|a| a.role == "full_body"));
        assert!(items[0].actions.iter().any(|a| a == "attachment"));
    }

    #[test]
    fn test_attachment_read() {
        let mut cfg = AgConfig::default();
        cfg.message.attachment_threshold_bytes = 100;
        cfg.message.preview_limit_chars = 20;
        let s = storage_with_config(cfg);
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let long_body = "a".repeat(200);
        let msg = s
            .send_message(
                "alice",
                &["bob".into()],
                &long_body,
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let att_id = msg
            .attachments
            .iter()
            .find(|a| a.role == "full_body")
            .unwrap()
            .id
            .clone();

        let (att, data) = s
            .get_attachment(&att_id, Some("bob"), Some("sess_bob"))
            .unwrap()
            .unwrap();
        assert_eq!(att.role, "full_body");
        assert_eq!(data.len(), 200);

        let recipients = s.get_recipients_for_msg_by_id(&msg.id).unwrap();
        let bob_status = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob_status.status, "read");
    }

    #[test]
    fn test_external_attachment_no_copy() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let file_path = std::env::temp_dir().join("agtalk-test-hello.rs");
        std::fs::write(&file_path, "fn main() {}").unwrap();

        let attachments = vec![crate::ipc::SendAttachment {
            path: file_path.to_string_lossy().to_string(),
            filename: "hello.rs".to_string(),
            content_type: "text/rust".to_string(),
            size: 14,
        }];
        let msg = s
            .send_message_with_attachments(
                "alice",
                &["bob".into()],
                "请看附件",
                "text",
                None,
                None,
                None,
                None,
                None,
                &attachments,
            )
            .unwrap();

        assert_eq!(msg.attachments.len(), 1);
        let att = &msg.attachments[0];
        assert_eq!(att.role, "attachment");
        assert_eq!(att.filename, "hello.rs");
        assert_eq!(att.storage_path, file_path.to_string_lossy().to_string());

        // 验证可以按原路径读取
        let (att2, data) = s
            .get_attachment(&att.id, Some("bob"), Some("sess_bob"))
            .unwrap()
            .unwrap();
        assert_eq!(att2.filename, "hello.rs");
        assert_eq!(String::from_utf8_lossy(&data), "fn main() {}");
    }

    #[test]
    fn test_preview_mode() {
        let mut cfg = AgConfig::default();
        cfg.message.inbox_inline_limit_bytes = 10;
        cfg.message.attachment_threshold_bytes = 1000;
        cfg.message.preview_limit_chars = 20;
        let s = storage_with_config(cfg);
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let body = "a".repeat(100);
        s.send_message(
            "alice",
            &["bob".into()],
            &body,
            "text",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let items = s.list_inbox("bob", None, 50).unwrap();
        assert_eq!(items[0].content.mode, "preview");
        assert!(items[0].content.truncated);
        assert_eq!(items[0].content.size, 100);
        assert!(items[0].attachments.is_empty());
    }

    #[test]
    fn test_config_thresholds() {
        let mut cfg = AgConfig::default();
        cfg.message.attachment_threshold_bytes = 100;
        let s = storage_with_config(cfg);
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let body = "a".repeat(50);
        let msg1 = s
            .send_message(
                "alice",
                &["bob".into()],
                &body,
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(msg1.attachments.is_empty());

        let body = "a".repeat(200);
        let msg2 = s
            .send_message(
                "alice",
                &["bob".into()],
                &body,
                "text",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(msg2.attachments.iter().any(|a| a.role == "full_body"));
    }

    // ─── notify 集成测试 ─────────────────────────────────

    #[derive(Default)]
    struct FakeNotifyPlugin {
        calls: Mutex<Vec<NotifyContext>>,
    }

    #[async_trait]
    impl NotifyPlugin for FakeNotifyPlugin {
        fn name(&self) -> &str {
            "fake"
        }

        async fn notify(&self, ctx: &NotifyContext) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(ctx.clone());
            Ok(())
        }
    }

    struct FailingNotifyPlugin;

    #[async_trait]
    impl NotifyPlugin for FailingNotifyPlugin {
        fn name(&self) -> &str {
            "failing"
        }

        async fn notify(&self, _ctx: &NotifyContext) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("boom"))
        }
    }

    fn session_notify_config(plugin: &str) -> String {
        serde_json::json!({
            "plugin": plugin,
            "endpoint": { "session": "s", "pane_id": "p" },
            "send_enter": true,
            "captured_by": "join"
        })
        .to_string()
    }

    fn empty_pending_asks() -> crate::server::PendingAsks {
        Arc::new(Mutex::new(HashMap::new()))
    }

    async fn send_via_server(
        storage: &Storage,
        notify_plugins: &NotifyPluginRegistry,
        from: &str,
        to: &str,
        body: &str,
        notify: bool,
        send_enter: Option<bool>,
    ) -> crate::ipc::ServerMsg {
        let transports = TransportRegistry::new();
        let pending = empty_pending_asks();
        let mut session: Option<crate::storage::SessionInfo> = None;
        handle_msg(
            ClientMsg::Send {
                sender: Some(from.to_string()),
                to: to.to_string(),
                body: body.to_string(),
                conversation_id: None,
                reply_to: None,
                correlation_id: None,
                content_type: "text".to_string(),
                metadata: None,
                notify,
                send_enter,
                attachments: vec![],
            },
            storage,
            &transports,
            notify_plugins,
            &pending,
            &mut session,
        )
        .await
    }

    fn create_active_session_with_notify(
        storage: &Storage,
        participant_name: &str,
        notify_cfg: &str,
    ) {
        let workspace_id = storage.register_workspace("ws", "/tmp/ws").unwrap();
        let participant = storage
            .get_participant_by_name(participant_name)
            .unwrap()
            .unwrap();
        storage
            .create_session(&workspace_id, &participant.id, Some(notify_cfg), None)
            .unwrap();
    }

    #[tokio::test]
    async fn test_send_without_notify_does_not_attempt() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let mut registry = NotifyPluginRegistry::new();
        let fake = Arc::new(FakeNotifyPlugin::default());
        registry.register(fake.clone());

        let resp = send_via_server(&s, &registry, "alice", "bob", "task", false, None).await;
        let data = match resp {
            crate::ipc::ServerMsg::Ok { data } => data,
            _ => panic!("预期 Ok 响应"),
        };
        let notify = data.get("notify").unwrap();
        assert_eq!(
            notify.get("attempted").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            notify.get("delivered").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert!(notify.get("error").unwrap().is_null());
        assert_eq!(fake.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_send_notify_success_marks_delivered() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        create_active_session_with_notify(&s, "bob", &session_notify_config("fake"));

        let mut registry = NotifyPluginRegistry::new();
        let fake = Arc::new(FakeNotifyPlugin::default());
        registry.register(fake.clone());

        let resp = send_via_server(&s, &registry, "alice", "bob", "task", true, None).await;
        let data = match resp {
            crate::ipc::ServerMsg::Ok { data } => data,
            _ => panic!("预期 Ok 响应"),
        };
        let notify = data.get("notify").unwrap();
        assert_eq!(
            notify.get("attempted").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            notify.get("delivered").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(notify.get("error").unwrap().is_null());

        let msg = data.get("message").unwrap();
        let msg_id = msg.get("id").and_then(|v| v.as_str()).unwrap();
        let recipients = s.get_recipients_for_msg_by_id(msg_id).unwrap();
        let bob = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob.status, "delivered");

        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        let call = &fake.calls.lock().unwrap()[0];
        assert_eq!(call.to, "bob");
        assert_eq!(call.from, "alice");
        assert!(call.text.contains("task"));
    }

    #[tokio::test]
    async fn test_send_notify_failure_keeps_pending() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        create_active_session_with_notify(&s, "bob", &session_notify_config("failing"));

        let mut registry = NotifyPluginRegistry::new();
        registry.register(Arc::new(FailingNotifyPlugin));

        let resp = send_via_server(&s, &registry, "alice", "bob", "task", true, None).await;
        let data = match resp {
            crate::ipc::ServerMsg::Ok { data } => data,
            _ => panic!("预期 Ok 响应"),
        };
        let notify = data.get("notify").unwrap();
        assert_eq!(
            notify.get("attempted").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            notify.get("delivered").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert!(notify.get("error").and_then(|v| v.as_str()).is_some());

        let msg = data.get("message").unwrap();
        let msg_id = msg.get("id").and_then(|v| v.as_str()).unwrap();
        let recipients = s.get_recipients_for_msg_by_id(msg_id).unwrap();
        let bob = recipients
            .iter()
            .find(|r| r.recipient_name == "bob")
            .unwrap();
        assert_eq!(bob.status, "pending");
    }

    #[tokio::test]
    async fn test_send_notify_no_active_session_returns_error() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let mut registry = NotifyPluginRegistry::new();
        registry.register(Arc::new(FakeNotifyPlugin::default()));

        let resp = send_via_server(&s, &registry, "alice", "bob", "task", true, None).await;
        let data = match resp {
            crate::ipc::ServerMsg::Ok { data } => data,
            _ => panic!("预期 Ok 响应"),
        };
        let notify = data.get("notify").unwrap();
        assert_eq!(
            notify.get("attempted").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            notify.get("delivered").and_then(|v| v.as_bool()),
            Some(false)
        );
        let error = notify.get("error").and_then(|v| v.as_str()).unwrap();
        assert!(error.contains("active session"));
    }

    #[tokio::test]
    async fn test_send_no_enter_overrides_default_send_enter() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();
        create_active_session_with_notify(&s, "bob", &session_notify_config("fake"));

        let mut registry = NotifyPluginRegistry::new();
        let fake = Arc::new(FakeNotifyPlugin::default());
        registry.register(fake.clone());

        send_via_server(&s, &registry, "alice", "bob", "task", true, Some(false)).await;
        assert!(!fake.calls.lock().unwrap()[0].send_enter);
    }

    // ── Wait / approval_response 持久化测试 ─────────────────

    #[test]
    fn test_get_approval_response() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "agent", "B", "terminal", "{}", "", "agent")
            .unwrap();

        let ask = s
            .send_message(
                "alice",
                &["bob".into()],
                "删除 target?",
                "approval_request",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        assert!(s.get_approval_response(&ask.id).unwrap().is_none());

        s.send_message(
            "bob",
            &["alice".into()],
            "同意",
            "approval_response",
            Some(&ask.id),
            Some(&ask.conversation_id),
            None,
            None,
            Some(r#"{"choice":"approve"}"#),
        )
        .unwrap();

        let resp = s.get_approval_response(&ask.id).unwrap().unwrap();
        assert_eq!(resp.content_type, "approval_response");
        assert_eq!(resp.reply_to_id, Some(ask.id.clone()));
        assert_eq!(resp.body, "同意");
    }

    async fn ask_via_server(
        storage: &Storage,
        pending: &crate::server::PendingAsks,
        from: &str,
        to: &str,
        body: &str,
        choices: Vec<String>,
        timeout_secs: u64,
    ) -> crate::ipc::ServerMsg {
        let transports = TransportRegistry::new();
        let notify = NotifyPluginRegistry::new();
        let mut session: Option<crate::storage::SessionInfo> = None;
        handle_msg(
            ClientMsg::Ask {
                sender: Some(from.to_string()),
                to: to.to_string(),
                body: body.to_string(),
                choices,
                timeout_secs,
            },
            storage,
            &transports,
            &notify,
            pending,
            &mut session,
        )
        .await
    }

    async fn reply_via_server(
        storage: &Storage,
        pending: &crate::server::PendingAsks,
        from: &str,
        msg_id: &str,
        choice: &str,
        reason: &str,
    ) -> crate::ipc::ServerMsg {
        let transports = TransportRegistry::new();
        let notify = NotifyPluginRegistry::new();
        let mut session: Option<crate::storage::SessionInfo> = None;
        handle_msg(
            ClientMsg::Reply {
                sender: Some(from.to_string()),
                msg_id: msg_id.to_string(),
                choice: choice.to_string(),
                reason: reason.to_string(),
            },
            storage,
            &transports,
            &notify,
            pending,
            &mut session,
        )
        .await
    }

    async fn wait_via_server(
        storage: &Storage,
        pending: &crate::server::PendingAsks,
        from: &str,
        msg_id: &str,
        timeout_secs: u64,
    ) -> crate::ipc::ServerMsg {
        let transports = TransportRegistry::new();
        let notify = NotifyPluginRegistry::new();
        let mut session: Option<crate::storage::SessionInfo> = None;
        handle_msg(
            ClientMsg::Wait {
                sender: Some(from.to_string()),
                msg_id: msg_id.to_string(),
                timeout_secs,
            },
            storage,
            &transports,
            &notify,
            pending,
            &mut session,
        )
        .await
    }

    #[tokio::test]
    async fn test_ask_reply_server_flow() {
        let s = Arc::new(storage());
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        let pending = empty_pending_asks();
        let pending2 = pending.clone();
        let s2 = s.clone();
        let ask_handle: tokio::task::JoinHandle<crate::ipc::ServerMsg> = tokio::spawn(async move {
            ask_via_server(
                &s2,
                &pending2,
                "alice",
                "bob",
                "删除 target?",
                vec!["允许".into(), "拒绝".into()],
                5,
            )
            .await
        });

        // 给 tokio 调度一点时间，让 Ask 注册 waiter
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // bob 回复；需要先拿到 ask 的 msg_id
        let ask_id = {
            let conn = s.conn();
            let id: String = conn
                .query_row(
                    "SELECT id FROM messages WHERE content_type = 'approval_request' ORDER BY created_at DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            id
        };

        let reply_resp = reply_via_server(&s, &pending, "bob", &ask_id, "允许", "没问题").await;
        assert!(matches!(reply_resp, crate::ipc::ServerMsg::Ok { .. }));

        let ask_resp = ask_handle.await.unwrap();
        match ask_resp {
            crate::ipc::ServerMsg::AskResponse { choice, reason, .. } => {
                assert_eq!(choice, "允许");
                assert_eq!(reason, "没问题");
            }
            other => panic!("预期 AskResponse， got {:?}", other),
        }

        // 验证 approval_response 已持久化
        let resp = s.get_approval_response(&ask_id).unwrap().unwrap();
        assert_eq!(resp.sender_name, "bob");
        assert_eq!(resp.body, "没问题");
    }

    #[tokio::test]
    async fn test_wait_after_reply_uses_storage() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        // 直接通过 storage 写入 approval_request / response，模拟 daemon 重启后的状态
        let ask = s
            .send_message(
                "alice",
                &["bob".into()],
                "重启服务?",
                "approval_request",
                None,
                None,
                None,
                None,
                Some(r#"{"choices":["yes","no"]}"#),
            )
            .unwrap();
        s.send_message(
            "bob",
            &["alice".into()],
            "可以",
            "approval_response",
            Some(&ask.id),
            Some(&ask.conversation_id),
            None,
            None,
            Some(r#"{"choice":"yes"}"#),
        )
        .unwrap();

        let pending = empty_pending_asks();
        let resp = wait_via_server(&s, &pending, "alice", &ask.id, 1).await;
        match resp {
            crate::ipc::ServerMsg::WaitResult {
                status,
                choice,
                reason,
                ..
            } => {
                assert_eq!(status, "replied");
                assert_eq!(choice, "yes");
                assert_eq!(reason, "可以");
            }
            other => panic!("预期 WaitResult， got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_wait_timeout() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        let ask = s
            .send_message(
                "alice",
                &["bob".into()],
                "重启服务?",
                "approval_request",
                None,
                None,
                None,
                None,
                Some(r#"{"choices":["yes","no"]}"#),
            )
            .unwrap();

        let pending = empty_pending_asks();
        let resp = wait_via_server(&s, &pending, "alice", &ask.id, 0).await;
        match resp {
            crate::ipc::ServerMsg::WaitResult { status, timed_out, .. } => {
                assert_eq!(status, "timed_out");
                assert!(timed_out);
            }
            other => panic!("预期 WaitResult timed_out， got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multiple_waiters_for_same_ask() {
        let s = Arc::new(storage());
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        let ask = s
            .send_message(
                "alice",
                &["bob".into()],
                "重启?",
                "approval_request",
                None,
                None,
                None,
                None,
                Some(r#"{"choices":["yes","no"]}"#),
            )
            .unwrap();

        let pending = empty_pending_asks();
        let pending2 = pending.clone();
        let pending3 = pending.clone();
        let s2 = s.clone();
        let s3 = s.clone();
        let ask_id = ask.id.clone();
        let ask_id2 = ask_id.clone();

        let h1 = tokio::spawn(async move {
            wait_via_server(&s2, &pending2, "alice", &ask_id, 5).await
        });
        let h2 = tokio::spawn(async move {
            wait_via_server(&s3, &pending3, "alice", &ask_id2, 5).await
        });

        // 让两个 Wait 都完成注册
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let reply_resp = reply_via_server(&s, &pending, "bob", &ask.id, "yes", "ok").await;
        assert!(matches!(reply_resp, crate::ipc::ServerMsg::Ok { .. }));

        for h in [h1, h2] {
            match h.await.unwrap() {
                crate::ipc::ServerMsg::WaitResult {
                    status,
                    choice,
                    reason,
                    ..
                } => {
                    assert_eq!(status, "replied");
                    assert_eq!(choice, "yes");
                    assert_eq!(reason, "ok");
                }
                other => panic!("预期 WaitResult， got {:?}", other),
            }
        }
    }

    #[tokio::test]
    async fn test_inbox_peek_does_not_mark_read() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        let pending = empty_pending_asks();
        let transports = TransportRegistry::new();
        let notify = NotifyPluginRegistry::new();
        let mut session: Option<crate::storage::SessionInfo> = None;

        async fn call_inbox(
            storage: &Storage,
            pending: &crate::server::PendingAsks,
            transports: &TransportRegistry,
            notify: &NotifyPluginRegistry,
            session: &mut Option<crate::storage::SessionInfo>,
            participant: &str,
            peek: bool,
        ) {
            let resp = handle_msg(
                ClientMsg::Inbox {
                    sender: Some("gui".into()),
                    participant: participant.to_string(),
                    status: None,
                    limit: 50,
                    peek,
                },
                storage,
                transports,
                notify,
                pending,
                session,
            )
            .await;
            assert!(matches!(resp, crate::ipc::ServerMsg::Ok { .. }));
        }

        let msg_id = s
            .send_message("alice", &["bob".into()], "hi", "text", None, None, None, None, None)
            .unwrap()
            .id;

        // peek=true 不应改变未读状态
        call_inbox(&s, &pending, &transports, &notify, &mut session, "bob", true).await;
        let status = s
            .get_recipients_for_msg_by_id(&msg_id)
            .unwrap()
            .into_iter()
            .find(|r| r.recipient_name == "bob")
            .map(|r| r.status)
            .unwrap_or_default();
        assert_eq!(status, "pending");

        // peek=false 才会标已读
        call_inbox(&s, &pending, &transports, &notify, &mut session, "bob", false).await;
        let status = s
            .get_recipients_for_msg_by_id(&msg_id)
            .unwrap()
            .into_iter()
            .find(|r| r.recipient_name == "bob")
            .map(|r| r.status)
            .unwrap_or_default();
        assert_eq!(status, "read");
    }

    #[tokio::test]
    async fn test_get_messages_with_explicit_participant_marks_only_that_viewer() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        let msg = s
            .send_message("alice", &["bob".into()], "hi", "text", None, None, None, None, None)
            .unwrap();

        let pending = empty_pending_asks();
        let transports = TransportRegistry::new();
        let notify = NotifyPluginRegistry::new();
        let mut session: Option<crate::storage::SessionInfo> = None;

        // 以 bob 身份请求 messages
        let resp = handle_msg(
            ClientMsg::GetMessages {
                conversation_id: msg.conversation_id.clone(),
                limit: 50,
                before: None,
                participant: Some("bob".into()),
            },
            &s,
            &transports,
            &notify,
            &pending,
            &mut session,
        )
        .await;

        assert!(matches!(resp, crate::ipc::ServerMsg::Ok { .. }));

        let recipients = s.get_recipients_for_msg_by_id(&msg.id).unwrap();
        let bob = recipients.iter().find(|r| r.recipient_name == "bob").unwrap();
        assert_eq!(bob.status, "read");
    }

    #[test]
    fn test_approval_request_mark_done_excludes_from_inbox() {
        let s = storage();
        s.register_participant(None, "alice", "agent", "A", "terminal", "{}", "", "agent")
            .unwrap();
        s.register_participant(None, "bob", "human", "B", "terminal", "{}", "", "human")
            .unwrap();

        let ask = s
            .send_message(
                "alice",
                &["bob".into()],
                "ok?",
                "approval_request",
                None,
                None,
                None,
                None,
                Some(r#"{"choices":["yes","no"]}"#),
            )
            .unwrap();

        assert_eq!(s.list_inbox("bob", None, 50).unwrap().len(), 1);

        // 模拟人类回复后再把原请求标记完成
        s.send_message(
            "bob",
            &["alice".into()],
            "yes",
            "approval_response",
            Some(&ask.id),
            Some(&ask.conversation_id),
            None,
            None,
            Some(r#"{"choice":"yes"}"#),
        )
        .unwrap();
        s.mark_done(&ask.id, "bob", None, &[]).unwrap();

        assert!(s.list_inbox("bob", None, 50).unwrap().is_empty());
    }
}
