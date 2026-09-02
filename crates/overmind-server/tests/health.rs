mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    // A data directory of its own: the setup code is written there at boot,
    // and `claimed` reads it the way the person at the machine would.
    let data_dir =
        std::env::temp_dir().join(format!("overmind-health-{}", uuid::Uuid::now_v7().simple()));
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init in-memory db");
    common::claimed(overmind_server::app(state), &data_dir).await
}

#[tokio::test]
async fn health_returns_ok_with_name_and_version() {
    let app = test_app().await;

    let request = Request::builder()
        .uri("/api/health")
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

/// The SPA's HTML is never cached (measured: after a deploy, Safari kept
/// serving the old bundle from cache and a fixed bug looked unfixed — the
/// owner hard-reloaded to see it). Hashed assets may cache forever; the
/// document that names them must not.
#[tokio::test]
async fn the_spa_document_is_served_with_no_cache() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let web_dir =
        std::env::temp_dir().join(format!("overmind-web-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&web_dir).expect("web dir");
    std::fs::write(web_dir.join("index.html"), "<html>app</html>").expect("index");
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            web_dir: web_dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            data_dir: web_dir.join("data"),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = common::claimed(overmind_server::app(state), &web_dir.join("data")).await;
    for uri in ["/", "/some/spa/route"] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("responds");
        let cache = res
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(
            cache.contains("no-cache"),
            "{uri}: the document must revalidate, got {cache:?}"
        );
    }
}
