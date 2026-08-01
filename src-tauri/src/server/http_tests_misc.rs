//! HTTP API 集成测试（misc 组；就近拆分，控制单文件行数）。

use crate::config::AgConfig;
use crate::identity::mailbox;
use crate::proto::ServerMsg;
use crate::server::http::routes;
use crate::server::http_tests_common::{browser_test_state, test_state, EnvGuard};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::TempDir;
use tower::ServiceExt;

mod tests {
    use super::*;

    #[tokio::test]
    async fn v1_daemon_status_returns_running() {
        let (state, _nora, _quinn, _tmp) = test_state();
        let cfg_tmp = TempDir::new().unwrap();
        let _cfg_guard = EnvGuard::set(cfg_tmp.path());
        crate::server::daemon::write_status_file(
            std::process::id(),
            crate::server::daemon::now_unix_secs(),
            19527,
            env!("CARGO_PKG_VERSION"),
            std::path::Path::new("/tmp/config.json"),
            std::path::Path::new("/tmp/agtalk.db"),
        )
        .unwrap();

        let app = routes(state.clone());
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/daemon/status")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match resp {
            ServerMsg::DaemonStatus {
                active_mailboxes,
                pending_messages,
                ..
            } => {
                assert_eq!(active_mailboxes, 2);
                assert_eq!(pending_messages, 0);
            }
            other => panic!("expected DaemonStatus, got {:?}", other),
        }

        crate::server::daemon::remove_status_file();
    }

    #[tokio::test]
    async fn v1_events_human_token_branch() {
        let (state, _tmp, _cfg_tmp, _guard) = browser_test_state();
        let cfg = AgConfig::default();
        let human_addr = mailbox::ensure_human(&state.storage, &cfg.human).unwrap();
        let session = crate::identity::human_session::ensure(&human_addr, &cfg.human.name).unwrap();

        // 正确 token → 200（SSE 响应头立即返回）
        let app = routes(state.clone());
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/events")
            .header("X-AgTalk-Human-Token", session.token.clone())
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 错误 token → 401
        let app = routes(state.clone());
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/events")
            .header("X-AgTalk-Human-Token", "wrong-token")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn v1_human_api_requires_token_and_serves() {
        let (state, _tmp, _cfg_tmp, _guard) = browser_test_state();
        let cfg = AgConfig::default();
        let human_addr = mailbox::ensure_human(&state.storage, &cfg.human).unwrap();
        let session = crate::identity::human_session::ensure(&human_addr, &cfg.human.name).unwrap();

        // 无 token → 401
        let app = routes(state.clone());
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/human/agents")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // 正确 token → 200 LookupResult（空列表）
        let app = routes(state.clone());
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/human/agents")
            .header("X-AgTalk-Human-Token", session.token.clone())
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match resp {
            ServerMsg::LookupResult { mailboxes } => assert!(mailboxes.is_empty()),
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }
}
