//! HTTP API 集成测试（mem 组；就近拆分，控制单文件行数）。

use crate::proto::ServerMsg;
use crate::server::http::routes;
use crate::server::http_tests_common::test_state;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

mod tests {
    use super::*;

    #[tokio::test]
    async fn v1_mem_plan_update_and_show() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let update = serde_json::to_string(&serde_json::json!({
            "plan": "do X",
            "context": "ctx",
            "status": "working",
            "summary": "50%",
        }))
        .unwrap();
        let update_req = Request::builder()
            .method("PATCH")
            .uri("/api/v1/mem/plan")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(update))
            .unwrap();
        let update_resp = app.clone().oneshot(update_req).await.unwrap();
        assert_eq!(update_resp.status(), StatusCode::OK);

        let show_req = Request::builder()
            .method("GET")
            .uri("/api/v1/mem/plan")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::empty())
            .unwrap();
        let show_resp = app.clone().oneshot(show_req).await.unwrap();
        assert_eq!(show_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(show_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let show: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match show {
            ServerMsg::MemPlanShow {
                address,
                plan,
                context,
                status,
                summary,
                ..
            } => {
                assert_eq!(address, nora);
                assert_eq!(plan, "do X");
                assert_eq!(context, "ctx");
                assert_eq!(status, "working");
                assert_eq!(summary, "50%");
            }
            other => panic!("expected MemPlanShow, got {:?}", other),
        }

        // mem_index 应被刷新
        let row = crate::mem::index::lookup_by_address(&state.storage, &nora).unwrap();
        assert_eq!(row.status_summary, "working: 50%");
        assert!(row.plan_updated_at > 0.0);
    }

    #[tokio::test]
    async fn v1_mem_plan_update_rejects_invalid_status() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let update = serde_json::to_string(&serde_json::json!({ "status": "running" })).unwrap();
        let req = Request::builder()
            .method("PATCH")
            .uri("/api/v1/mem/plan")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(update))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let msg: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match msg {
            ServerMsg::Error { code, message } => {
                assert_eq!(code, "invalid_status");
                assert!(message.contains("idle"), "{}", message);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_mem_plan_status_with_target_reads_remote_plan() {
        let (state, nora, quinn, _tmp) = test_state();
        let app = routes(state.clone());

        // 以 quinn 身份更新自己的 plan
        let update = serde_json::to_string(&serde_json::json!({
            "status": "waiting",
            "summary": "waiting for nora",
        }))
        .unwrap();
        let update_req = Request::builder()
            .method("PATCH")
            .uri("/api/v1/mem/plan")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", quinn.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::from(update))
            .unwrap();
        let update_resp = app.clone().oneshot(update_req).await.unwrap();
        assert_eq!(update_resp.status(), StatusCode::OK);

        // nora 通过 target=address 读取 quinn 的公开摘要
        let status_req = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/mem/plan/status?target={}", quinn))
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::empty())
            .unwrap();
        let status_resp = app.clone().oneshot(status_req).await.unwrap();
        assert_eq!(status_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(status_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        match serde_json::from_slice::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::MemPlanStatus {
                address,
                status,
                summary,
                ..
            } => {
                assert_eq!(address, quinn);
                assert_eq!(status, "waiting");
                assert_eq!(summary, "waiting for nora");
            }
            other => panic!("expected MemPlanStatus, got {:?}", other),
        }

        // nora 通过 target=name 读取 quinn
        let show_req = Request::builder()
            .method("GET")
            .uri("/api/v1/mem/plan?target=quinn")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::empty())
            .unwrap();
        let show_resp = app.clone().oneshot(show_req).await.unwrap();
        assert_eq!(show_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(show_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        match serde_json::from_slice::<ServerMsg>(&bytes).unwrap() {
            ServerMsg::MemPlanShow { address, name, .. } => {
                assert_eq!(address, quinn);
                assert_eq!(name, "quinn");
            }
            other => panic!("expected MemPlanShow, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_mem_pack_empty_topic_does_not_return_guide() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/mem/pack")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pack: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match pack {
            ServerMsg::MemPack { topic, markdown } => {
                assert_eq!(topic, "all");
                assert!(markdown.is_empty());
            }
            other => panic!("expected MemPack, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_mem_pack_agent_guide_no_longer_returns_built_in_guide() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/mem/pack?topic=agtalk/agent-guide")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pack: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match pack {
            ServerMsg::MemPack { topic, markdown } => {
                assert_eq!(topic, "agtalk/agent-guide");
                // guide 已迁到 --agent-guide，mem pack 不再特殊返回
                assert!(markdown.is_empty());
            }
            other => panic!("expected MemPack, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_mem_pack_old_handbook_alias_does_not_return_guide() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/mem/pack?topic=agent-learning-handbook")
            .header("X-AgTalk-Address", nora.clone())
            .header(
                "X-AgTalk-Workspace-Root",
                state.dot_agtalk.to_string_lossy().as_ref(),
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pack: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match pack {
            ServerMsg::MemPack { topic, markdown } => {
                assert_eq!(topic, "agent-learning-handbook");
                assert!(markdown.is_empty());
            }
            other => panic!("expected MemPack, got {:?}", other),
        }
    }
}
