//! Running "only on localhost" is not a security boundary: the browser is the
//! attack surface, not the network. Any page you have open can talk to
//! 127.0.0.1. Overmind has no authentication and its API starts tasks that run
//! a CLI on this machine, so the only thing standing between a hostile tab and
//! your shell is that cross-origin requests are refused. These tests hold that
//! line — they fail the day someone "fixes a CORS error" by widening it.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn app() -> axum::Router {
    let root =
        std::env::temp_dir().join(format!("overmind-cors-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("root");
    // A built frontend present = production shape: we serve the SPA ourselves,
    // so the UI is same-origin and no CORS policy should be published at all.
    let web = root.join("web");
    std::fs::create_dir_all(&web).expect("web dir");
    std::fs::write(web.join("index.html"), "<!doctype html>").expect("index");
    let config = overmind_server::Config {
        data_dir: root.join("data"),
        web_dir: web,
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    overmind_server::app(state)
}

#[tokio::test]
async fn a_hostile_page_cannot_reach_the_api() {
    let app = app().await;

    // The preflight a browser sends before a cross-origin JSON POST. Nothing
    // may come back that grants the origin: no allow-origin header, no consent.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/companies")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert!(
        res.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "the API told a hostile origin it was welcome: {:?}",
        res.headers()
    );

    // And the request itself, if attempted, is never blessed for reading.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/companies")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Pwned Inc"}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert!(
        res.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "a cross-origin response must never be readable by the caller"
    );
}

#[tokio::test]
async fn the_live_socket_refuses_a_foreign_origin() {
    let app = app().await;

    // WebSockets bypass CORS entirely, so the origin is checked by hand.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ws")
                .header(header::HOST, "127.0.0.1:7070")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::CONNECTION, "Upgrade")
                .header(header::UPGRADE, "websocket")
                .header(header::SEC_WEBSOCKET_VERSION, "13")
                .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "a hostile page subscribed to the company's live feed"
    );

    // Our own page, same origin as the host it dialled, is let through.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/ws")
                .header(header::HOST, "127.0.0.1:7070")
                .header(header::ORIGIN, "http://127.0.0.1:7070")
                .header(header::CONNECTION, "Upgrade")
                .header(header::UPGRADE, "websocket")
                .header(header::SEC_WEBSOCKET_VERSION, "13")
                .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_ne!(
        res.status(),
        StatusCode::FORBIDDEN,
        "our own UI was locked out"
    );
}

#[tokio::test]
async fn a_non_browser_client_still_works() {
    // curl, tests and MCP clients send no Origin and must keep working: the
    // check defends against pages, not against you.
    let app = app().await;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/companies")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Local Co"}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(v["name"], "Local Co");
}
