//! HTTP API 集成测试（msg 组；就近拆分，控制单文件行数）。

use crate::proto::ServerMsg;
use crate::server::http::routes;
use crate::server::http_tests_common::test_state;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

mod tests {
    use super::*;

    #[tokio::test]
    async fn v1_msg_send_ok() {
        let (state, nora, quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let body = serde_json::to_string(&serde_json::json!({
            "to": quinn,
            "body": "hi from http",
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/send")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let msg_id = match resp {
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
        assert_eq!(msg.to_address, quinn);
        assert_eq!(msg.to_name, "quinn");
        assert_eq!(msg.from_address, nora);
        assert_eq!(msg.from_name, "nora");
        assert_eq!(msg.body, "hi from http");
    }

    #[tokio::test]
    async fn v1_msg_send_missing_auth() {
        let (state, _nora, quinn, _tmp) = test_state();
        let app = routes(state);

        let body = serde_json::to_string(&serde_json::json!({
            "to": quinn,
            "body": "hi",
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/send")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn v1_msg_read_empty_inbox_returns_inbox_empty() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/read")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(r#"{"message_id":null}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let msg: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match msg {
            ServerMsg::Error { code, .. } => assert_eq!(code, "inbox_empty"),
            other => panic!("expected Error, got {:?}", other),
        }
    }
    #[tokio::test]
    async fn v1_msg_ask_notify_omitted_null_and_false_all_ok() {
        let (state, nora, _quinn, _tmp) = test_state();

        for (label, extra) in [
            ("omitted", serde_json::json!({})),
            ("explicit null", serde_json::json!({ "notify": null })),
            ("false", serde_json::json!({ "notify": false })),
            ("true", serde_json::json!({ "notify": true })),
        ] {
            let mut body = serde_json::json!({ "message": format!("deploy? {label}") });
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());

            let app = routes(state.clone());
            let request = Request::builder()
                .method("POST")
                .uri("/api/v1/msg/ask")
                .header("Content-Type", "application/json")
                .header("X-AgTalk-Address", nora.clone())
                .header(
                    "X-AgTalk-Workspace-Root",
                    state.dot_agtalk.to_string_lossy().as_ref(),
                )
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "case: {label}");

            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let resp: ServerMsg = serde_json::from_slice(&bytes).unwrap();
            match resp {
                ServerMsg::AskResult { message_id } => assert!(!message_id.is_empty()),
                other => panic!("case {label}: expected AskResult, got {:?}", other),
            }
        }
    }
}
