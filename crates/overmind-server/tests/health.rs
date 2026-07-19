use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let state = overmind_server::init("sqlite::memory:")
        .await
        .expect("init in-memory db");
    overmind_server::app(state)
}

#[tokio::test]
async fn health_returns_ok_with_name_and_version() {
    let app = test_app().await;

    let request = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .expect("build request");
    let response = app.oneshot(request).await.expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("body is JSON");

    assert_eq!(body["status"], "ok");
    assert_eq!(body["name"], "overmind-server");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = test_app().await;

    let request = Request::builder()
        .uri("/does-not-exist")
        .body(Body::empty())
        .expect("build request");
    let response = app.oneshot(request).await.expect("router responds");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
