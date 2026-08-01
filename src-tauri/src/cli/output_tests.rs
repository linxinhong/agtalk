use super::*;
use crate::cli::output_misc::format_uptime;
use crate::proto::ServerMsg;

#[test]
fn identity_left_text_format() {
    let msg = ServerMsg::IdentityLeft {
        address: "550e8400-e29b-41d4-a716-446655440000".into(),
        name: "reviewer".into(),
        removed_session: true,
        purge: true,
    };
    print_text_server_msg(&msg);
}

#[test]
fn identity_json_includes_auto_notify_diagnostics() {
    let msg = ServerMsg::Identity {
        address: "550e8400-e29b-41d4-a716-446655440000".into(),
        name: "nora".into(),
        intro: "reviewer".into(),
        notify_channel: "none".into(),
        notify_ready: false,
        workspace_root: "/tmp/project/.agtalk".into(),
        notify_diagnostics: vec![crate::proto::NotifyProbe {
            name: "plugin:zellij".into(),
            status: "not_ready".into(),
            message: "zellij action not reachable from current context".into(),
        }],
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("notify_diagnostics"), "{}", json);
    assert!(json.contains("plugin:zellij"), "{}", json);
}

#[test]
fn identity_json_omits_empty_notify_diagnostics() {
    let msg = ServerMsg::Identity {
        address: "550e8400-e29b-41d4-a716-446655440000".into(),
        name: "nora".into(),
        intro: "reviewer".into(),
        notify_channel: "none".into(),
        notify_ready: false,
        workspace_root: "/tmp/project/.agtalk".into(),
        notify_diagnostics: Vec::new(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("notify_diagnostics"), "{}", json);
}

#[test]
fn inbox_text_uses_short_id_and_summary() {
    let msg = ServerMsg::InboxResult {
        messages: vec![Message {
            id: "6f0d4353-f4b2-46ab-afb6-8c7f6af02049".into(),
            to_address: "to".into(),
            to_name: "to".into(),
            from_address: "from".into(),
            from_name: "notify-receiver-zellij".into(),
            body: "test notify via zellij plugin".into(),
            content_type: "text".into(),
            reply_to_id: None,
            subject: None,
            metadata: "{}".into(),
            event_id: 1,
            status: "read".into(),
            created_at: 0.0,
        }],
    };
    print_text_server_msg(&msg);
}

#[test]
fn msg_detail_text_uses_summary() {
    let msg = ServerMsg::MsgDetail(Message {
        id: "6f0d4353-f4b2-46ab-afb6-8c7f6af02049".into(),
        to_address: "to".into(),
        to_name: "to".into(),
        from_address: "from".into(),
        from_name: "notify-receiver-zellij".into(),
        body: "test notify via zellij plugin".into(),
        content_type: "text".into(),
        reply_to_id: None,
        subject: None,
        metadata: "{}".into(),
        event_id: 1,
        status: "read".into(),
        created_at: 0.0,
    });
    print_text_server_msg(&msg);
}

#[test]
fn message_json_includes_subject() {
    let msg = ServerMsg::MsgDetail(Message {
        id: "6f0d4353-f4b2-46ab-afb6-8c7f6af02049".into(),
        to_address: "to".into(),
        to_name: "to".into(),
        from_address: "from".into(),
        from_name: "notify-receiver-zellij".into(),
        body: "body".into(),
        content_type: "text".into(),
        reply_to_id: None,
        subject: Some("Review plan".into()),
        metadata: "{}".into(),
        event_id: 1,
        status: "read".into(),
        created_at: 0.0,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"subject\""), "{}", json);
    assert!(json.contains("Review plan"), "{}", json);
}

#[test]
fn identity_ambiguous_json_has_candidates() {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "candidates".to_string(),
        serde_json::json!(["nora", "quinn"]),
    );
    let e = CliError::new("identity_ambiguous", "多个 session").with_extra(extra);
    let json = serde_json::to_string(&JsonError {
        ty: "error",
        code: &e.code,
        message: &e.message,
        extra: e.extra.as_ref(),
    })
    .unwrap();
    assert!(json.contains("identity_ambiguous"));
    assert!(json.contains("candidates"));
    assert!(json.contains("nora"));
}

#[test]
fn format_uptime_seconds() {
    assert_eq!(format_uptime(11), "11s");
}

#[test]
fn format_uptime_minutes() {
    assert_eq!(format_uptime(611), "10m 11s");
}

#[test]
fn format_uptime_hours() {
    assert_eq!(format_uptime(43811), "12h 10m 11s");
}

#[test]
fn format_uptime_days() {
    assert_eq!(format_uptime(993011), "11days 11h 50m 11s");
}

#[test]
fn format_uptime_one_day_one_second() {
    assert_eq!(format_uptime(86401), "1days 0h 0m 1s");
}

#[test]
fn relation_list_text_includes_specialties_and_preferred_for() {
    let text = format_relation_list(&[crate::identity::relations::Relation {
        name: "Codex-Tom".to_string(),
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        intro: "reviewer".to_string(),
        first_seen_at: None,
        last_seen_at: None,
        last_message_id: None,
        sent_count: 1,
        received_count: 2,
        tags: vec!["rust".to_string()],
        role: Some("reviewer".to_string()),
        note: None,
        specialties: vec!["Rust 实现".to_string(), "测试隔离".to_string()],
        preferred_for: vec!["功能开发".to_string()],
    }]);
    assert!(
        text.contains("specialties: Rust 实现, 测试隔离"),
        "{}",
        text
    );
    assert!(text.contains("preferred_for: 功能开发"), "{}", text);
    assert!(text.contains("role: reviewer"), "{}", text);
}

#[test]
fn relation_show_text_includes_specialties_and_preferred_for() {
    let text = format_relation(&crate::identity::relations::Relation {
        name: "Codex-Tom".to_string(),
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        intro: "reviewer".to_string(),
        first_seen_at: None,
        last_seen_at: None,
        last_message_id: None,
        sent_count: 3,
        received_count: 0,
        tags: vec![],
        role: None,
        note: Some("good partner".to_string()),
        specialties: vec!["Rust 实现".to_string()],
        preferred_for: vec!["功能开发".to_string(), "修复 Rust 测试".to_string()],
    });
    assert!(text.contains("specialties   : Rust 实现"), "{}", text);
    assert!(
        text.contains("preferred_for : 功能开发, 修复 Rust 测试"),
        "{}",
        text
    );
    assert!(text.contains("note          : good partner"), "{}", text);
}

#[test]
fn relation_list_json_includes_specialties_and_preferred_for() {
    let msg = ServerMsg::MemRelationList {
        relations: vec![crate::identity::relations::Relation {
            name: "Codex-Tom".to_string(),
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            intro: "reviewer".to_string(),
            first_seen_at: None,
            last_seen_at: None,
            last_message_id: None,
            sent_count: 1,
            received_count: 0,
            tags: vec![],
            role: None,
            note: None,
            specialties: vec!["Rust 实现".to_string()],
            preferred_for: vec!["功能开发".to_string()],
        }],
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("specialties"), "{}", json);
    assert!(json.contains("preferred_for"), "{}", json);
    assert!(json.contains("Rust 实现"), "{}", json);
    assert!(json.contains("功能开发"), "{}", json);
}
