//! Who pays, and who decides (ADR-0037).
//!
//! The economy is detected, never configured (ADR-0030) — but when a key is
//! overriding a claude.ai login, the person can *choose* to let the plan pay,
//! and Overmind does it for them: the key is kept out of the agent's
//! environment and the probe is asked again. The choice is remembered across
//! restarts, and refused when it would not change who is billed.
//!
//! No test here spawns the real CLI: the economy is declared with
//! `economy_override` and the adapter is `/usr/bin/true`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use overmind_server::economy::Economy;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn setup(economy: Economy) -> (axum::Router, std::path::PathBuf) {
    let data_dir =
        std::env::temp_dir().join(format!("overmind-pays-{}", uuid::Uuid::now_v7().simple()));
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            economy_override: Some(economy.clone()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    state.set_economy(economy);
    (
        common::claimed(overmind_server::app(state), &data_dir).await,
        data_dir,
    )
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// Nothing chosen yet: the health says so, and it says it on every page.
#[tokio::test]
async fn by_default_the_payer_is_whoever_the_probe_found() {
    let (app, _) = setup(Economy::Key {
        overrides_login: true,
    })
    .await;
    let (s, health) = send(&app, "GET", "/api/health", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(health["economy"]["kind"], json!("key"));
    assert_eq!(health["pay_with"], json!("detected"));
}

/// When the key is the only thing there is, "let the plan pay" would change
/// nothing about who is billed — so it is refused, and the choice is not kept.
#[tokio::test]
async fn letting_the_plan_pay_is_refused_when_the_key_would_still_pay() {
    let (app, data_dir) = setup(Economy::Key {
        overrides_login: true,
    })
    .await;
    let (s, body) = send(
        &app,
        "POST",
        "/api/economy/pay-with",
        Some(json!({ "with": "plan" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("key"),
        "the refusal names the key: {body}"
    );
    assert!(
        !data_dir.join("pay-with-plan").exists(),
        "a refused choice is not remembered"
    );
    let (_, health) = send(&app, "GET", "/api/health", None).await;
    assert_eq!(health["pay_with"], json!("detected"));
}

/// The happy path: the plan answers the probe once the key is out of the way,
/// the choice is recorded on disk (so a restart keeps it), and health says
/// both who pays and that it was chosen.
#[tokio::test]
async fn letting_the_plan_pay_is_remembered_and_shown() {
    let (app, data_dir) = setup(Economy::Subscription {
        plan: Some("max".into()),
    })
    .await;
    let (s, body) = send(
        &app,
        "POST",
        "/api/economy/pay-with",
        Some(json!({ "with": "plan" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["economy"]["kind"], json!("subscription"));
    assert_eq!(body["pay_with"], json!("plan"));
    assert!(
        data_dir.join("pay-with-plan").exists(),
        "the choice is on disk"
    );

    let (_, health) = send(&app, "GET", "/api/health", None).await;
    assert_eq!(health["pay_with"], json!("plan"));

    // And it can be undone.
    let (s, body) = send(
        &app,
        "POST",
        "/api/economy/pay-with",
        Some(json!({ "with": "detected" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["pay_with"], json!("detected"));
    assert!(!data_dir.join("pay-with-plan").exists());
}

/// Anything but the two words is a bad request, not a silent default.
#[tokio::test]
async fn an_unknown_payer_is_refused() {
    let (app, _) = setup(Economy::Subscription { plan: None }).await;
    let (s, _) = send(
        &app,
        "POST",
        "/api/economy/pay-with",
        Some(json!({ "with": "mastercard" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
