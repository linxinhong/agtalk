#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn test_register_and_lookup() {
        let s = Storage::open_memory().unwrap();
        let p = s.register_participant("kimi_coder_Alex", "agent", "Alex", "terminal", "{}").unwrap();
        assert_eq!(p.name, "kimi_coder_Alex");
        assert!(s.get_participant_by_name("kimi_coder_Alex").unwrap().is_some());
    }

    #[test]
    fn test_list_participants() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("a", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("b", "human", "B", "popup", "{}").unwrap();
        assert_eq!(s.list_participants(None).unwrap().len(), 2);
        assert_eq!(s.list_participants(Some("agent")).unwrap().len(), 1);
    }

    #[test]
    fn test_send_and_list() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "Alice", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "Bob", "terminal", "{}").unwrap();
        let msg = s.send_message("alice", &["bob".into()], "Hi", "text", None, None, None, None, None).unwrap();
        assert_eq!(msg.sender_name, "alice");
        let convs = s.list_conversations(Some("bob")).unwrap();
        assert_eq!(convs.len(), 1);
        let msgs = s.get_messages(&msg.conversation_id, 50, None).unwrap();
        assert_eq!(msgs[0].body, "Hi");
    }

    #[test]
    fn test_mark_done() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "B", "terminal", "{}").unwrap();
        let msg = s.send_message("alice", &["bob".into()], "task", "text", None, None, None, None, None).unwrap();
        s.mark_delivered(&msg.id, "bob").unwrap();
        s.mark_read(&msg.id, "bob").unwrap();
        s.mark_done(&msg.id, "bob").unwrap();
        let msgs = s.get_messages(&msg.conversation_id, 50, None).unwrap();
        assert_eq!(msgs[0].recipients[0].status, "done");
    }

    #[test]
    fn test_unread_count() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "B", "terminal", "{}").unwrap();
        s.send_message("alice", &["bob".into()], "1", "text", None, None, None, None, None).unwrap();
        s.send_message("alice", &["bob".into()], "2", "text", None, None, None, None, None).unwrap();
        assert_eq!(s.list_conversations(Some("bob")).unwrap()[0].unread_count, 2);
    }

    #[test]
    fn test_reply_chain() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "B", "terminal", "{}").unwrap();
        let m1 = s.send_message("alice", &["bob".into()], "Q", "text", None, None, None, None, None).unwrap();
        let m2 = s.send_message("bob", &["alice".into()], "A", "text", Some(&m1.id), Some(&m1.conversation_id), None, None, None).unwrap();
        assert_eq!(m2.reply_to_id, Some(m1.id));
        assert_eq!(s.get_messages(&m1.conversation_id, 50, None).unwrap().len(), 2);
    }

    #[test]
    fn test_unregister() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.unregister_participant("alice").unwrap();
        assert!(s.get_participant_by_name("alice").unwrap().is_none());
    }

    #[test]
    fn test_full_message_bus() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("@codex", "agent", "C", "cli", "{}").unwrap();
        s.register_participant("@me", "human", "M", "gui", "{}").unwrap();
        s.register_participant("@rv", "agent", "R", "terminal", "{}").unwrap();

        // codex → me
        let m1 = s.send_message("@codex", &["@me".into()], "deploy?", "approval_request", None, None, None, None, None).unwrap();
        assert_eq!(s.list_conversations(Some("@me")).unwrap()[0].unread_count, 1);
        s.mark_read(&m1.id, "@me").unwrap();

        // me → codex reply
        s.send_message("@me", &["@codex".into()], "ok", "approval_response", Some(&m1.id), Some(&m1.conversation_id), None, None, None).unwrap();
        let msgs = s.get_messages(&m1.conversation_id, 50, None).unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn test_ask_reply_flow() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("@agent", "agent", "Ag", "terminal", "{}").unwrap();
        s.register_participant("@human", "human", "Hu", "gui", "{}").unwrap();

        // Agent asks
        let ask = s.send_message("@agent", &["@human".into()], "删除 target?", "approval_request", None, None, None, None, None).unwrap();
        assert_eq!(ask.content_type, "approval_request");

        // Human replies
        let reply = s.send_message("@human", &["@agent".into()], "同意", "approval_response", Some(&ask.id), Some(&ask.conversation_id), None, None, None).unwrap();
        assert_eq!(reply.reply_to_id, Some(ask.id.clone()));

        let msgs = s.get_messages(&ask.conversation_id, 50, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content_type, "approval_request");
        assert_eq!(msgs[1].content_type, "approval_response");
    }

    #[test]
    fn test_two_participant_conversation() {
        // ── 场景：@codex 和 @emma 的完整对话 ──
        let storage = Storage::open_memory().unwrap();

        // 1. 注册两个参与者
        storage.register_participant("@codex", "agent", "Codex", "terminal", "{}").unwrap();
        storage.register_participant("@emma", "human", "Emma", "gui", "{}").unwrap();

        // 2. @codex 向 @emma 发审批请求
        let ask = storage.send_message(
            "@codex", &["@emma".into()],
            "是否允许部署到生产环境？",
            "approval_request", None, None, None, None, None,
        ).unwrap();
        assert_eq!(ask.sender_name, "@codex");
        assert_eq!(ask.content_type, "approval_request");

        // 3. @emma 查看收件箱
        let emma_inbox = storage.list_conversations(Some("@emma")).unwrap();
        assert_eq!(emma_inbox.len(), 1);
        assert_eq!(emma_inbox[0].unread_count, 1);
        assert!(emma_inbox[0].last_message.is_some());
        eprintln!("  @emma 收到: {}", emma_inbox[0].last_message.as_ref().unwrap().body);

        // 4. @emma 读取消息
        storage.mark_read(&ask.id, "@emma").unwrap();
        let msgs = storage.get_messages(&ask.conversation_id, 50, None).unwrap();
        assert_eq!(msgs.len(), 1);
        eprintln!("  @emma 读取: [{}] {}", msgs[0].sender_name, msgs[0].body);

        // 5. @emma 回复同意
        let reply = storage.send_message(
            "@emma", &["@codex".into()],
            "同意部署，我检查过所有测试通过了",
            "approval_response",
            Some(&ask.id), Some(&ask.conversation_id), None, None, None,
        ).unwrap();
        assert_eq!(reply.sender_name, "@emma");
        assert_eq!(reply.reply_to_id.as_ref(), Some(&ask.id));
        eprintln!("  @emma 回复: {}", reply.body);

        // 6. @codex 查看 inbox（应有 1 条未读回复）
        let codex_inbox = storage.list_conversations(Some("@codex")).unwrap();
        assert_eq!(codex_inbox.len(), 1);
        assert_eq!(codex_inbox[0].unread_count, 1);
        eprintln!("  @codex 收到回复，未读数: {}", codex_inbox[0].unread_count);

        // 7. @codex 读取完整对话
        let full = storage.get_messages(&ask.conversation_id, 50, None).unwrap();
        assert_eq!(full.len(), 2);
        eprintln!("  ── 完整对话 ──");
        for m in &full {
            let status = m.recipients.first().map(|r| r.status.as_str()).unwrap_or("?");
            eprintln!("  [{}] {}: {}  ({})", m.content_type, m.sender_name, m.body, status);
        }

        // 8. @codex 标记完成
        storage.mark_done(&reply.id, "@codex").unwrap();
        let final_inbox = storage.list_conversations(Some("@codex")).unwrap();
        assert_eq!(final_inbox[0].unread_count, 0);

        // 9. 验证对话类型和回复链
        assert_eq!(full[0].content_type, "approval_request");
        assert_eq!(full[1].content_type, "approval_response");
        assert_eq!(full[1].reply_to_id, Some(full[0].id.clone()));
    }

    #[test]
    fn test_created_at_nonzero() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "B", "terminal", "{}").unwrap();
        let msg = s.send_message("alice", &["bob".into()], "Hi", "text", None, None, None, None, None).unwrap();
        assert!(msg.created_at > 0.0, "created_at should be real timestamp, got {}", msg.created_at);
    }

    #[test]
    fn test_metadata_stored() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "B", "terminal", "{}").unwrap();
        let meta = r#"{"choices":["approve","reject"],"timeout":60}"#;
        let msg = s.send_message("alice", &["bob".into()], "deploy?", "approval_request", None, None, None, None, Some(meta)).unwrap();
        assert_eq!(msg.metadata, meta);
        // Verify round-trip through get_messages
        let msgs = s.get_messages(&msg.conversation_id, 50, None).unwrap();
        assert_eq!(msgs[0].metadata, meta);
    }

    #[test]
    fn test_metadata_default_empty() {
        let s = Storage::open_memory().unwrap();
        s.register_participant("alice", "agent", "A", "terminal", "{}").unwrap();
        s.register_participant("bob", "agent", "B", "terminal", "{}").unwrap();
        let msg = s.send_message("alice", &["bob".into()], "Hi", "text", None, None, None, None, None).unwrap();
        assert_eq!(msg.metadata, "{}");
    }
}
