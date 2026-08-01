//! HTTP API 集成测试（id 组；就近拆分，控制单文件行数）。

use crate::proto::ServerMsg;
use crate::server::http::routes;
use crate::server::http_tests_common::{browser_test_state, test_state};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

mod tests {
    use super::*;

    use crate::identity::browser_session;
    use crate::identity::{mailbox, mailbox as mailbox_db};

    #[tokio::test]
    async fn v1_browser_join_lookup_leave() {
        let (state, _tmp, _browser_tmp, _guard) = browser_test_state();
        let app = routes(state.clone());

        let body = serde_json::to_string(&serde_json::json!({
            "name": "browser-agent",
            "intro": "bridge",
            "workspace": "web"
        }))
        .unwrap();
        let join_req = Request::builder()
            .method("POST")
            .uri("/api/v1/browser/join")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let join_resp = app.clone().oneshot(join_req).await.unwrap();
        assert_eq!(join_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(join_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let join: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let (addr, token) = match join {
            ServerMsg::BrowserJoinResult { address, token, .. } => (address, token),
            other => panic!("expected BrowserJoinResult, got {:?}", other),
        };

        let lookup_req = Request::builder()
            .method("GET")
            .uri("/api/v1/id/lookup?name=browser-agent")
            .body(Body::empty())
            .unwrap();
        let lookup_resp = app.clone().oneshot(lookup_req).await.unwrap();
        assert_eq!(lookup_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(lookup_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let lookup: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let found = match lookup {
            ServerMsg::LookupResult { mailboxes } => mailboxes
                .iter()
                .any(|m| m.address == addr && m.name == "browser-agent"),
            other => panic!("expected LookupResult, got {:?}", other),
        };
        assert!(found);

        let leave_req = Request::builder()
            .method("POST")
            .uri("/api/v1/id/leave")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", addr.clone())
            .header("X-AgTalk-Browser-Token", token)
            .body(Body::from(r#"{}"#))
            .unwrap();
        let leave_resp = app.clone().oneshot(leave_req).await.unwrap();
        assert_eq!(leave_resp.status(), StatusCode::OK);

        let lookup_req2 = Request::builder()
            .method("GET")
            .uri("/api/v1/id/lookup?name=browser-agent")
            .body(Body::empty())
            .unwrap();
        let lookup_resp2 = app.clone().oneshot(lookup_req2).await.unwrap();
        let bytes = axum::body::to_bytes(lookup_resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let lookup2: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match lookup2 {
            ServerMsg::LookupResult { mailboxes } => assert!(mailboxes.is_empty()),
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_browser_send_via_token() {
        let (state, _tmp, _browser_tmp, _guard) = browser_test_state();
        let app = routes(state.clone());

        let (addr, _name, token) = browser_session::create(
            &state.storage,
            Some("browser-agent".to_string()),
            Some("bridge".to_string()),
            Some("web".to_string()),
        )
        .unwrap();

        let recipient = mailbox::create(&state.storage, "quinn", "后端", "projB").unwrap();

        let body = serde_json::to_string(&serde_json::json!({
            "to": recipient,
            "body": "hello from browser",
        }))
        .unwrap();
        let send_req = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/send")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", addr.clone())
            .header("X-AgTalk-Browser-Token", token)
            .body(Body::from(body))
            .unwrap();
        let send_resp = app.oneshot(send_req).await.unwrap();
        assert_eq!(send_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(send_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let send: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let msg_id = match send {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        let conn = state.storage.conn();
        let msg: crate::routing::Message = conn
            .query_row(
                "SELECT * FROM messages WHERE id = ?1",
                [&msg_id],
                crate::routing::Message::from_row,
            )
            .unwrap();
        assert_eq!(msg.to_address, recipient);
        assert_eq!(msg.from_address, addr);
        assert_eq!(msg.from_name, "browser-agent");
        assert_eq!(msg.body, "hello from browser");
    }

    #[tokio::test]
    async fn v1_id_join_idempotent_reuses_address() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let cur_pid = std::process::id();
        let mut sys = sysinfo::System::new_all();
        sys.refresh_processes();
        let cur_start = sys
            .process(sysinfo::Pid::from(cur_pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(1);

        let body1 = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "intro": "前端",
            "notify": "none",
            "pid": cur_pid,
            "start_time": cur_start,
        }))
        .unwrap();
        let join1 = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(body1))
            .unwrap();
        let resp1 = app.clone().oneshot(join1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let (addr1, intro1) = match serde_json::from_slice::<ServerMsg>(
            &axum::body::to_bytes(resp1.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap()
        {
            ServerMsg::Identity { address, intro, .. } => (address, intro),
            other => panic!("expected Identity, got {:?}", other),
        };

        let body2 = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "intro": "后端",
            "notify": "none",
            "pid": cur_pid,
            "start_time": cur_start,
        }))
        .unwrap();
        let join2 = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(body2))
            .unwrap();
        let resp2 = app.clone().oneshot(join2).await.unwrap();
        let (addr2, intro2) = match serde_json::from_slice::<ServerMsg>(
            &axum::body::to_bytes(resp2.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap()
        {
            ServerMsg::Identity { address, intro, .. } => (address, intro),
            other => panic!("expected Identity, got {:?}", other),
        };

        assert_eq!(addr1, addr2);
        assert_eq!(addr1, nora);
        assert_eq!(intro1, "前端");
        assert_eq!(intro2, "后端");

        let mb = mailbox_db::get_by_address(&state.storage, &addr1)
            .unwrap()
            .unwrap();
        assert_eq!(mb.intro, "后端");
        assert_eq!(mb.workspace, "");
    }

    #[tokio::test]
    async fn v1_id_join_requires_workspace_header() {
        let (state, _nora, _quinn, _tmp) = test_state();
        let app = routes(state);
        let body = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "notify": "none",
            "pid": std::process::id(),
            "start_time": 1,
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let msg: ServerMsg = serde_json::from_slice(&body).unwrap();
        assert!(
            matches!(msg, ServerMsg::Error { ref code, .. } if code == "workspace_root_required")
        );
    }

    #[tokio::test]
    async fn v1_id_join_revives_left_mailbox() {
        let (state, nora, _quinn, _tmp) = test_state();
        mailbox_db::mark_left(&state.storage, &nora).unwrap();
        assert!(mailbox_db::get_by_address(&state.storage, &nora)
            .unwrap()
            .is_none());

        let cur_pid = std::process::id();
        let mut sys = sysinfo::System::new_all();
        sys.refresh_processes();
        let cur_start = sys
            .process(sysinfo::Pid::from(cur_pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(1);

        let app = routes(state.clone());
        let body = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "notify": "none",
            "pid": cur_pid,
            "start_time": cur_start,
        }))
        .unwrap();
        let join = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(join).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mb = mailbox_db::get_by_address(&state.storage, &nora)
            .unwrap()
            .unwrap();
        assert_eq!(mb.address, nora);
        assert!(mb.left_at.is_none());
    }
}
