use super::*;
use crate::config::AgConfig;
use crate::server::handlers::id;
use crate::storage::Storage;
use tempfile::TempDir;

fn test_state() -> (AppState, TempDir) {
    let tmp = TempDir::new().unwrap();
    let dot = tmp.path().join(".agtalk");
    let storage = Storage::open_in_memory().unwrap();
    let state = AppState::new(storage, AgConfig::default(), dot);
    (state, tmp)
}

fn join(state: &AppState, name: &str) -> String {
    let pid = std::process::id();
    let start_time = current_pid_start_time();
    match id::handle_join(
        state,
        &state.dot_agtalk,
        Some(name.into()),
        Some("intro".into()),
        "none".into(),
        None,
        pid,
        start_time,
    ) {
        ServerMsg::Identity { address, .. } => address,
        other => panic!("expected Identity, got {:?}", other),
    }
}

fn current_pid_start_time() -> u64 {
    use sysinfo::{Pid, System};
    let pid = std::process::id();
    let mut sys = System::new_all();
    sys.refresh_processes();
    sys.process(Pid::from(pid as usize))
        .map(|p| p.start_time())
        .unwrap_or(1)
}

fn auth_headers_for(state: &AppState, name: &str) -> HeaderMap {
    let session = crate::identity::session_file::read(&state.dot_agtalk, name).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("X-AgTalk-Address", session.address.parse().unwrap());
    headers.insert(
        "X-AgTalk-Workspace-Root",
        state.dot_agtalk.to_string_lossy().as_ref().parse().unwrap(),
    );
    headers
}

#[tokio::test]
async fn send_default_returns_ok() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    let msg = handle_send(
        &state,
        &headers,
        recv_addr,
        "hi".into(),
        None,
        vec![],
        None,
        None,
        false,
    );
    match msg {
        ServerMsg::Ok { id } => assert!(!id.is_empty()),
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn send_with_notify_false_returns_ok() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    let msg = handle_send(
        &state,
        &headers,
        recv_addr,
        "hi".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    match msg {
        ServerMsg::Ok { id } => assert!(!id.is_empty()),
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn send_persists_trimmed_subject() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    let resp = handle_send(
        &state,
        &headers,
        recv_addr,
        "body".into(),
        Some("  Review plan  ".into()),
        vec![],
        Some(false),
        None,
        false,
    );
    let id = match resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };
    let msg = lookup::detail(&state.storage, &id).unwrap().unwrap();
    assert_eq!(msg.subject.as_deref(), Some("Review plan"));
}

#[test]
fn send_blank_subject_becomes_none() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    let resp = handle_send(
        &state,
        &headers,
        recv_addr,
        "body".into(),
        Some("   ".into()),
        vec![],
        Some(false),
        None,
        false,
    );
    let id = match resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };
    let msg = lookup::detail(&state.storage, &id).unwrap().unwrap();
    assert!(msg.subject.is_none());
}

#[test]
fn reply_inherits_subject() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "please review".into(),
        Some("Task X".into()),
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let recv_headers = auth_headers_for(&state, "recv");
    let reply_resp = handle_reply(
        &state,
        &recv_headers,
        sent_id[..8].to_string(),
        "done".into(),
        vec![],
        Some(false),
        None,
    );
    let reply_id = match reply_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };
    let reply_msg = lookup::detail(&state.storage, &reply_id).unwrap().unwrap();
    assert_eq!(reply_msg.subject.as_deref(), Some("Task X"));
}

#[test]
fn send_writes_subject_to_history() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &headers,
        recv_addr,
        "body".into(),
        Some("History S".into()),
        vec![],
        Some(false),
        None,
        false,
    );

    let sender_history = read_history_lines(&state.dot_agtalk, "sender");
    let receiver_history = read_history_lines(&state.dot_agtalk, "recv");
    assert_eq!(sender_history[0]["subject"], "History S");
    assert_eq!(receiver_history[0]["subject"], "History S");
}

#[test]
fn send_to_human_records_pending_deliveries() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let human_addr =
        crate::identity::mailbox::ensure_human(&state.storage, &state.config.human).unwrap();

    let headers = auth_headers_for(&state, "sender");
    let resp = handle_send(
        &state,
        &headers,
        human_addr,
        "hi human".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let id = match resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let deliveries = crate::human::delivery::list_for_message(&state.storage, &id).unwrap();
    assert_eq!(deliveries.len(), state.config.human.surfaces.len());
    assert!(deliveries.iter().all(|d| d.status == "pending"));
}

#[test]
fn send_to_agent_records_no_human_delivery() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    let resp = handle_send(
        &state,
        &headers,
        recv_addr,
        "hi agent".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let id = match resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };
    assert!(
        crate::human::delivery::list_for_message(&state.storage, &id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ask_records_pending_deliveries_for_human() {
    let (state, _tmp) = test_state();
    join(&state, "asker");

    let headers = auth_headers_for(&state, "asker");
    let resp = handle_ask(
        &state,
        &headers,
        "approve deploy?".into(),
        vec![],
        AskOptions {
            questions: vec![],
            options: vec!["yes".into(), "no".into()],
            recommended: Some("yes".into()),
            single: true,
            select_only: false,
        },
        false,
        None,
        false,
    );
    let id = match resp {
        ServerMsg::AskResult { message_id } => message_id,
        other => panic!("expected AskResult, got {:?}", other),
    };
    let deliveries = crate::human::delivery::list_for_message(&state.storage, &id).unwrap();
    assert_eq!(deliveries.len(), state.config.human.surfaces.len());
}

#[test]
fn read_with_short_id() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello short id".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };
    let short_id = &sent_id[..8];

    let recv_headers = auth_headers_for(&state, "recv");
    let read_resp = handle_read(&state, &recv_headers, Some(short_id.to_string()));
    match read_resp {
        ServerMsg::MsgDetail(msg) => assert_eq!(msg.id, sent_id),
        other => panic!("expected MsgDetail, got {:?}", other),
    }
}

#[test]
fn reply_with_short_id() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    let inbox_resp = handle_inbox(&state, &recv_headers, Default::default());
    let first_id = match inbox_resp {
        ServerMsg::InboxResult { messages } => messages[0].id.clone(),
        other => panic!("expected InboxResult, got {:?}", other),
    };
    let short_id = &first_id[..8];

    let reply_resp = handle_reply(
        &state,
        &recv_headers,
        short_id.to_string(),
        "ok".into(),
        vec![],
        Some(false),
        None,
    );
    match reply_resp {
        ServerMsg::Ok { id } => assert!(!id.is_empty()),
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn done_with_short_id() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    let inbox_resp = handle_inbox(&state, &recv_headers, Default::default());
    let first_id = match inbox_resp {
        ServerMsg::InboxResult { messages } => messages[0].id.clone(),
        other => panic!("expected InboxResult, got {:?}", other),
    };
    let short_id = &first_id[..8];

    let done_resp = handle_done(
        &state,
        &recv_headers,
        Some(short_id.to_string()),
        None,
        vec![],
    );
    match done_resp {
        ServerMsg::Ok { id } => assert_eq!(id, first_id),
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn send_writes_history_for_sender_and_receiver() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &headers,
        recv_addr,
        "hi".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let sender_history = read_history_lines(&state.dot_agtalk, "sender");
    let receiver_history = read_history_lines(&state.dot_agtalk, "recv");
    assert_eq!(sender_history.len(), 1);
    assert_eq!(receiver_history.len(), 1);
    assert_eq!(sender_history[0]["type"], "message");
    assert_eq!(sender_history[0]["dir"], "out");
    assert_eq!(sender_history[0]["source"], "msg.send");
    assert_eq!(receiver_history[0]["dir"], "in");
    assert_eq!(receiver_history[0]["source"], "msg.send");
}

#[test]
fn reply_writes_history_and_status() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let recv_headers = auth_headers_for(&state, "recv");
    handle_read(&state, &recv_headers, Some(sent_id[..8].to_string()));

    let reply_resp = handle_reply(
        &state,
        &recv_headers,
        sent_id[..8].to_string(),
        "ok".into(),
        vec![],
        Some(false),
        None,
    );
    assert!(matches!(reply_resp, ServerMsg::Ok { .. }));

    let sender_history = read_history_lines(&state.dot_agtalk, "sender");
    let recv_history = read_history_lines(&state.dot_agtalk, "recv");

    // sender: original out message, reply in message
    assert!(sender_history
        .iter()
        .any(|h| h["dir"] == "out" && h["source"] == "msg.send"));
    assert!(sender_history
        .iter()
        .any(|h| h["dir"] == "in" && h["source"] == "msg.reply"));

    // receiver: original in message (from send), out reply message, status for original pending->read (from handle_read)
    // 已 read 消息被 reply 不再产生 read->read 的冗余 status
    assert!(recv_history
        .iter()
        .any(|h| h["dir"] == "in" && h["source"] == "msg.send"));
    assert!(recv_history
        .iter()
        .any(|h| h["dir"] == "out" && h["source"] == "msg.reply"));
    assert!(recv_history
        .iter()
        .any(|h| h["type"] == "status" && h["reason"] == "msg.read"));
    assert!(!recv_history
        .iter()
        .any(|h| h["type"] == "status" && h["reason"] == "msg.reply"));
}

#[test]
fn read_writes_status_history() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr,
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    handle_read(&state, &recv_headers, None);

    let recv_history = read_history_lines(&state.dot_agtalk, "recv");
    assert!(recv_history
        .iter()
        .any(|h| h["type"] == "status" && h["to"] == "read" && h["reason"] == "msg.read"));
}

#[test]
fn done_writes_status_history() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr,
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    handle_done(&state, &recv_headers, None, None, vec![]);

    let recv_history = read_history_lines(&state.dot_agtalk, "recv");
    assert!(recv_history
        .iter()
        .any(|h| h["type"] == "status" && h["to"] == "done" && h["reason"] == "msg.done"));
}

#[test]
fn read_twice_does_not_duplicate_status() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr,
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    let first = handle_read(&state, &recv_headers, None);
    assert!(matches!(first, ServerMsg::InboxResult { .. }));

    let second = handle_read(&state, &recv_headers, None);
    assert!(
        matches!(second, ServerMsg::Error { code, .. } if code == "inbox_empty"),
        "第二次 read 应返回 inbox_empty"
    );

    let recv_history = read_history_lines(&state.dot_agtalk, "recv");
    let status_events: Vec<_> = recv_history
        .iter()
        .filter(|h| h["type"] == "status")
        .collect();
    assert_eq!(status_events.len(), 1);
    assert_eq!(status_events[0]["from"], "pending");
    assert_eq!(status_events[0]["to"], "read");
    assert_eq!(status_events[0]["reason"], "msg.read");
}

#[test]
fn read_done_message_does_not_write_status() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr,
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    handle_done(&state, &recv_headers, None, None, vec![]);

    let done_history = read_history_lines(&state.dot_agtalk, "recv");
    let before_count = done_history
        .iter()
        .filter(|h| h["type"] == "status")
        .count();

    // 对已 done 消息执行 read 不应产生 done->read
    let read_resp = handle_read(&state, &recv_headers, None);
    assert!(
        matches!(read_resp, ServerMsg::Error { code, .. } if code == "inbox_empty"),
        "已 done 消息不应被 read 再次列出"
    );

    let recv_history = read_history_lines(&state.dot_agtalk, "recv");
    let after_count = recv_history
        .iter()
        .filter(|h| h["type"] == "status")
        .count();
    assert_eq!(before_count, after_count);
}

#[test]
fn reply_read_message_does_not_write_read_to_read() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let recv_headers = auth_headers_for(&state, "recv");
    handle_read(&state, &recv_headers, Some(sent_id[..8].to_string()));
    let after_read = read_history_lines(&state.dot_agtalk, "recv");
    let status_count_before = after_read.iter().filter(|h| h["type"] == "status").count();

    handle_reply(
        &state,
        &recv_headers,
        sent_id[..8].to_string(),
        "ok".into(),
        vec![],
        Some(false),
        None,
    );

    let after_reply = read_history_lines(&state.dot_agtalk, "recv");
    let status_count_after = after_reply.iter().filter(|h| h["type"] == "status").count();
    assert_eq!(
        status_count_before, status_count_after,
        "已 read 消息被 reply 不应再产生 status 事件"
    );
}

#[test]
fn reply_done_message_does_not_write_done_to_read() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let recv_headers = auth_headers_for(&state, "recv");
    handle_done(
        &state,
        &recv_headers,
        Some(sent_id[..8].to_string()),
        None,
        vec![],
    );
    let after_done = read_history_lines(&state.dot_agtalk, "recv");
    let status_count_before = after_done.iter().filter(|h| h["type"] == "status").count();

    handle_reply(
        &state,
        &recv_headers,
        sent_id[..8].to_string(),
        "ok".into(),
        vec![],
        Some(false),
        None,
    );

    let after_reply = read_history_lines(&state.dot_agtalk, "recv");
    let status_count_after = after_reply.iter().filter(|h| h["type"] == "status").count();
    assert_eq!(
        status_count_before, status_count_after,
        "已 done 消息被 reply 不应再产生 status 事件"
    );
}

#[test]
fn done_twice_does_not_duplicate_status() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &sender_headers,
        recv_addr,
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let recv_headers = auth_headers_for(&state, "recv");
    let first = handle_done(&state, &recv_headers, None, None, vec![]);
    assert!(matches!(first, ServerMsg::Ok { .. }));

    let after_first = read_history_lines(&state.dot_agtalk, "recv");
    let status_count_before = after_first.iter().filter(|h| h["type"] == "status").count();

    let second = handle_done(&state, &recv_headers, None, None, vec![]);
    assert!(
        matches!(second, ServerMsg::Error { code, .. } if code == "inbox_empty"),
        "没有未完成消息时 done 应返回 inbox_empty"
    );

    let after_second = read_history_lines(&state.dot_agtalk, "recv");
    let status_count_after = after_second
        .iter()
        .filter(|h| h["type"] == "status")
        .count();
    assert_eq!(status_count_before, status_count_after);
}

#[test]
fn send_skips_receiver_history_when_no_session() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr =
        crate::identity::mailbox::create(&state.storage, "recv", "receiver", "").unwrap();

    let headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &headers,
        recv_addr,
        "hi".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let sender_history = read_history_lines(&state.dot_agtalk, "sender");
    assert_eq!(sender_history.len(), 1);
    assert!(!history_path(&state.dot_agtalk, "recv").exists());
}

fn history_path(dot: &std::path::Path, name: &str) -> std::path::PathBuf {
    dot.join(name).join("history.jsonl")
}

fn read_history_lines(dot: &std::path::Path, name: &str) -> Vec<serde_json::Value> {
    let path = history_path(dot, name);
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn read_short_id_ambiguous_returns_error() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_addr = crate::identity::session_file::read(&state.dot_agtalk, "sender")
        .unwrap()
        .address;
    // 直接插入两条前缀相同的消息（UUID v4 前 8 位不可控）。
    for (i, id) in [
        "6f0d4353-1111-46ab-afb6-8c7f6af02049",
        "6f0d4353-2222-46ab-afb6-8c7f6af02049",
    ]
    .iter()
    .enumerate()
    {
        state
            .storage
            .conn()
            .execute(
                "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, content_type, reply_to_id, metadata, event_id, status, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11)",
                rusqlite::params![id, &recv_addr, "recv", &sender_addr, "sender", "hi", "text", Option::<String>::None, "{}", (i + 1) as i64, 1.0],
            )
            .unwrap();
    }

    let recv_headers = auth_headers_for(&state, "recv");
    let read_resp = handle_read(&state, &recv_headers, Some("6f0d4353".into()));
    match read_resp {
        ServerMsg::Error { code, .. } => assert_eq!(code, "message_id_ambiguous"),
        other => panic!("expected Error, got {:?}", other),
    }
}

#[test]
fn send_updates_relations_for_both_sides() {
    let (state, _tmp) = test_state();
    let sender_addr = join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &headers,
        recv_addr.clone(),
        "hi".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let sender_relations = crate::identity::relations::read(&state.dot_agtalk, "sender")
        .unwrap()
        .peers;
    let recv_relations = crate::identity::relations::read(&state.dot_agtalk, "recv")
        .unwrap()
        .peers;

    assert_eq!(sender_relations.len(), 1);
    let r = sender_relations
        .get(&recv_addr)
        .expect("sender should have recv peer");
    assert_eq!(r.name, "recv");
    assert_eq!(r.sent_count, 1);
    assert_eq!(r.received_count, 0);

    assert_eq!(recv_relations.len(), 1);
    let r = recv_relations
        .get(&sender_addr)
        .expect("recv should have sender peer");
    assert_eq!(r.name, "sender");
    assert_eq!(r.received_count, 1);
    assert_eq!(r.sent_count, 0);
}

#[test]
fn send_skips_receiver_relation_when_no_session() {
    let (state, _tmp) = test_state();
    join(&state, "sender");
    let recv_addr =
        crate::identity::mailbox::create(&state.storage, "recv", "receiver", "").unwrap();

    let headers = auth_headers_for(&state, "sender");
    handle_send(
        &state,
        &headers,
        recv_addr,
        "hi".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );

    let sender_relations = crate::identity::relations::read(&state.dot_agtalk, "sender")
        .unwrap()
        .peers;
    assert_eq!(sender_relations.len(), 1);

    assert!(!state
        .dot_agtalk
        .join("recv")
        .join("relations.json")
        .exists());
}

#[test]
fn reply_updates_relations_for_both_sides() {
    let (state, _tmp) = test_state();
    let sender_addr = join(&state, "sender");
    let recv_addr = join(&state, "recv");

    let sender_headers = auth_headers_for(&state, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let recv_headers = auth_headers_for(&state, "recv");
    let reply_resp = handle_reply(
        &state,
        &recv_headers,
        sent_id[..8].to_string(),
        "ok".into(),
        vec![],
        Some(false),
        None,
    );
    assert!(
        matches!(reply_resp, ServerMsg::Ok { .. }),
        "reply should succeed, got {:?}",
        reply_resp
    );

    let sender_relations = crate::identity::relations::read(&state.dot_agtalk, "sender")
        .unwrap()
        .peers;
    let recv_relations = crate::identity::relations::read(&state.dot_agtalk, "recv")
        .unwrap()
        .peers;

    // sender: original out + reply in
    let r = sender_relations
        .get(&recv_addr)
        .expect("sender should have recv peer");
    assert_eq!(r.received_count, 1);
    assert_eq!(r.sent_count, 1);

    // recv: original in + reply out
    let r = recv_relations
        .get(&sender_addr)
        .expect("recv should have sender peer");
    assert_eq!(r.sent_count, 1);
    assert_eq!(r.received_count, 1);
}

// ---- 跨 workspace：history/relations 写入各 agent 自身根 ----

fn test_state_at(daemon_dot: &std::path::Path) -> AppState {
    let storage = Storage::open_in_memory().unwrap();
    AppState::new(storage, AgConfig::default(), daemon_dot.to_path_buf())
}

fn join_at(state: &AppState, root: &std::path::Path, name: &str) -> String {
    let pid = std::process::id();
    let start_time = current_pid_start_time();
    match id::handle_join(
        state,
        root,
        Some(name.into()),
        Some("intro".into()),
        "none".into(),
        None,
        pid,
        start_time,
    ) {
        ServerMsg::Identity { address, .. } => address,
        other => panic!("expected Identity, got {:?}", other),
    }
}

fn auth_headers_for_root(root: &std::path::Path, name: &str) -> HeaderMap {
    let session = crate::identity::session_file::read(root, name).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("X-AgTalk-Address", session.address.parse().unwrap());
    headers.insert(
        "X-AgTalk-Workspace-Root",
        root.to_str().unwrap().parse().unwrap(),
    );
    headers
}

#[test]
fn cross_workspace_history_written_to_each_agent_root() {
    let daemon_tmp = TempDir::new().unwrap();
    let sender_tmp = TempDir::new().unwrap();
    let recv_tmp = TempDir::new().unwrap();
    let daemon_dot = daemon_tmp.path().join(".agtalk");
    let sender_dot = sender_tmp.path().join(".agtalk");
    let recv_dot = recv_tmp.path().join(".agtalk");

    let state = test_state_at(&daemon_dot);
    join_at(&state, &sender_dot, "sender");
    let recv_addr = join_at(&state, &recv_dot, "recv");

    let headers = auth_headers_for_root(&sender_dot, "sender");
    let resp = handle_send(
        &state,
        &headers,
        recv_addr,
        "hi cross".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    assert!(matches!(resp, ServerMsg::Ok { .. }), "send ok: {:?}", resp);

    let sender_history = read_history_lines(&sender_dot, "sender");
    assert_eq!(sender_history.len(), 1);
    assert_eq!(sender_history[0]["dir"], "out");
    assert_eq!(sender_history[0]["source"], "msg.send");

    let recv_history = read_history_lines(&recv_dot, "recv");
    assert_eq!(recv_history.len(), 1);
    assert_eq!(recv_history[0]["dir"], "in");

    // daemon 根目录下不应生成任何 agent 目录。
    assert!(!daemon_dot.join("sender").exists());
    assert!(!daemon_dot.join("recv").exists());
}

#[test]
fn cross_workspace_reply_history_and_relations() {
    let daemon_tmp = TempDir::new().unwrap();
    let sender_tmp = TempDir::new().unwrap();
    let recv_tmp = TempDir::new().unwrap();
    let daemon_dot = daemon_tmp.path().join(".agtalk");
    let sender_dot = sender_tmp.path().join(".agtalk");
    let recv_dot = recv_tmp.path().join(".agtalk");

    let state = test_state_at(&daemon_dot);
    let sender_addr = join_at(&state, &sender_dot, "sender");
    let recv_addr = join_at(&state, &recv_dot, "recv");

    let sender_headers = auth_headers_for_root(&sender_dot, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr.clone(),
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    let recv_headers = auth_headers_for_root(&recv_dot, "recv");
    let reply_resp = handle_reply(
        &state,
        &recv_headers,
        sent_id[..8].to_string(),
        "ok".into(),
        vec![],
        Some(false),
        None,
    );
    assert!(
        matches!(reply_resp, ServerMsg::Ok { .. }),
        "reply ok: {:?}",
        reply_resp
    );

    // sender 根：send out + reply in
    let sender_history = read_history_lines(&sender_dot, "sender");
    assert!(
        sender_history.iter().any(|e| e["dir"] == "in"),
        "sender 根应记录 reply 的 in 事件"
    );
    // recv 根：send in + reply out
    let recv_history = read_history_lines(&recv_dot, "recv");
    assert!(
        recv_history.iter().any(|e| e["dir"] == "out"),
        "recv 根应记录 reply 的 out 事件"
    );

    // relations 也写入各自根
    let sender_relations = crate::identity::relations::read(&sender_dot, "sender")
        .unwrap()
        .peers;
    let recv_relations = crate::identity::relations::read(&recv_dot, "recv")
        .unwrap()
        .peers;
    assert_eq!(
        sender_relations
            .get(&recv_addr)
            .expect("sender has recv peer")
            .received_count,
        1
    );
    assert_eq!(
        recv_relations
            .get(&sender_addr)
            .expect("recv has sender peer")
            .sent_count,
        1
    );

    // daemon 根不下错写任何 agent 目录
    assert!(!daemon_dot.join("sender").exists());
    assert!(!daemon_dot.join("recv").exists());
}

#[test]
fn send_to_receiver_without_local_session_skips_receiver_side() {
    let daemon_tmp = TempDir::new().unwrap();
    let sender_tmp = TempDir::new().unwrap();
    let daemon_dot = daemon_tmp.path().join(".agtalk");
    let sender_dot = sender_tmp.path().join(".agtalk");

    let state = test_state_at(&daemon_dot);
    join_at(&state, &sender_dot, "sender");
    // 仅在 DB 建 mailbox（workspace_root=""），不创建任何 session 文件。
    let recv_addr =
        crate::identity::mailbox::create(&state.storage, "recv", "receiver", "").unwrap();

    let headers = auth_headers_for_root(&sender_dot, "sender");
    let resp = handle_send(
        &state,
        &headers,
        recv_addr,
        "hi".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    assert!(
        matches!(resp, ServerMsg::Ok { .. }),
        "send to non-local receiver should still succeed: {:?}",
        resp
    );

    // sender 侧写入
    let sender_history = read_history_lines(&sender_dot, "sender");
    assert_eq!(sender_history.len(), 1);
    assert_eq!(sender_history[0]["dir"], "out");

    // receiver 侧不可达：daemon 根下也不应错写 recv 目录
    assert!(!daemon_dot.join("recv").exists());
    assert!(!daemon_dot.join("sender").exists());
}

#[test]
fn done_status_written_to_actor_root_in_cross_workspace() {
    let daemon_tmp = TempDir::new().unwrap();
    let sender_tmp = TempDir::new().unwrap();
    let recv_tmp = TempDir::new().unwrap();
    let daemon_dot = daemon_tmp.path().join(".agtalk");
    let sender_dot = sender_tmp.path().join(".agtalk");
    let recv_dot = recv_tmp.path().join(".agtalk");

    let state = test_state_at(&daemon_dot);
    join_at(&state, &sender_dot, "sender");
    let recv_addr = join_at(&state, &recv_dot, "recv");

    let sender_headers = auth_headers_for_root(&sender_dot, "sender");
    let send_resp = handle_send(
        &state,
        &sender_headers,
        recv_addr,
        "hello".into(),
        None,
        vec![],
        Some(false),
        None,
        false,
    );
    let sent_id = match send_resp {
        ServerMsg::Ok { id } => id,
        other => panic!("expected Ok, got {:?}", other),
    };

    // recv 标记完成：status 事件应写入 recv（操作者）的根。
    let recv_headers = auth_headers_for_root(&recv_dot, "recv");
    let done_resp = handle_done(
        &state,
        &recv_headers,
        Some(sent_id[..8].to_string()),
        None,
        vec![],
    );
    assert!(
        matches!(done_resp, ServerMsg::Ok { .. }),
        "done ok: {:?}",
        done_resp
    );

    let recv_history = read_history_lines(&recv_dot, "recv");
    assert!(
        recv_history
            .iter()
            .any(|e| e["type"] == "status" && e["reason"] == "msg.done"),
        "recv 根应记录 msg.done 的 status 事件"
    );

    // sender 根不应出现 status 事件（只有 send 的 out）
    let sender_history = read_history_lines(&sender_dot, "sender");
    assert!(
        sender_history.iter().all(|e| e["type"] != "status"),
        "sender 根不应写入 done 的 status 事件"
    );
}
