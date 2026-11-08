//! Who pays, and who decides (ADR-0037, carried over ADR-0051's accounts).
//!
//! The economy is detected, never configured (ADR-0030) — but when a key is
//! overriding a claude.ai login, the person can *choose* to let the plan pay,
//! and Overmind does it for them: the plan's account is made active, which
//! keeps the key out of the agent's environment, and the probe is asked again.
//! The choice is the database's, so it survives a restart, and it is refused
//! — with the previous payer put back — when it would not change who is billed.
//!
//! No test here spawns the real CLI: the economy is declared with
//! `economy_override` and the adapter is `/usr/bin/true`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use overmind_server::accounts::{self, EnvCredentials, Kind, Source};
use overmind_server::economy::{Economy, UnknownKind};
use serde_json::{Value, json};
use tower::ServiceExt;

const ENV_KEY: &str = "sk-ant-api03-WHOPAYSWHOPAYSWHOPAYSWHOPAYSWHOPAYS-0000";
const TOKEN: &str = "sk-ant-oat01-WHOPAYSTOKENWHOPAYSTOKENWHOPAYSTOKEN0001";

/// An instance with a key in the server's environment: the ordinary setup on a
/// machine where `ANTHROPIC_API_KEY` is exported, and the one where the switch
/// means anything.
async fn setup(economy: Economy) -> (axum::Router, overmind_server::AppState) {
    let data_dir =
        std::env::temp_dir().join(format!("overmind-pays-{}", uuid::Uuid::now_v7().simple()));
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            economy_override: Some(economy.clone()),
            env_credentials: EnvCredentials::default().with("ANTHROPIC_API_KEY", ENV_KEY),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    state.set_economy(economy);
    (
        common::claimed(overmind_server::app(state.clone()), &data_dir).await,
        state,
    )
}

/// Who is paying for Anthropic right now, as the kind and source of the
/// active account -- `None` for the host's own login.
async fn payer(state: &overmind_server::AppState) -> Option<(Kind, Source)> {
    accounts::active(&state.pool, "anthropic")
        .await
        .expect("active")
        .map(|a| (a.kind, a.source))
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
    let (app, state) = setup(Economy::Key {
        overrides_login: true,
    })
    .await;
    let (s, health) = send(&app, "GET", "/api/health", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(health["economy"]["kind"], json!("key"));
    assert_eq!(health["pay_with"], json!("detected"));
    assert_eq!(payer(&state).await, Some((Kind::Key, Source::Environment)));
}

/// When the key is the only thing there is, "let the plan pay" would change
/// nothing about who is billed — so it is refused, and the key pays again.
#[tokio::test]
async fn letting_the_plan_pay_is_refused_when_the_key_would_still_pay() {
    let (app, state) = setup(Economy::Key {
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
    assert_eq!(
        payer(&state).await,
        Some((Kind::Key, Source::Environment)),
        "a refused choice puts the previous payer back"
    );
    assert!(
        body["remedy"].is_null(),
        "a key Overmind cannot reach is not something it can repair: {body}"
    );
    let (_, health) = send(&app, "GET", "/api/health", None).await;
    assert_eq!(health["pay_with"], json!("detected"));
}

/// Hiding the key must leave somebody able to pay.
///
/// A key with no claude.ai login behind it is the ordinary setup on any machine
/// where `ANTHROPIC_API_KEY` is exported. Choose the plan with nothing behind
/// it and the CLI is signed in as nobody: every run fails afterwards, having
/// been told the plan would pay. `is_metered` does not catch this, because what
/// is left is not a key — it is no one.
#[tokio::test]
async fn letting_the_plan_pay_is_refused_when_nobody_is_signed_in() {
    let (app, state) = setup(Economy::Unknown {
        kind: UnknownKind::NotSignedIn,
        reason: "the agent CLI is not signed in".into(),
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
        body["error"].as_str().unwrap_or("").contains("signed in"),
        "the refusal says what is missing, not merely that it refused: {body}"
    );
    // And says it to the interface as well as the person: this refusal has a
    // repair Overmind can offer on the spot, and matching on English prose to
    // discover that is not a contract (ADR-0038 addendum).
    assert_eq!(
        body["remedy"]["kind"],
        json!("connect_plan"),
        "the refusal carries the repair: {body}"
    );
    assert_eq!(
        payer(&state).await,
        Some((Kind::Key, Source::Environment)),
        "a refused choice puts the previous payer back"
    );
    let (_, health) = send(&app, "GET", "/api/health", None).await;
    assert_eq!(health["pay_with"], json!("detected"));
}

/// The happy path on the host's own plan: nothing is linked but the key, so
/// letting the plan pay means nobody is active and the CLI falls back on the
/// host's login -- what ADR-0037 meant until the accounts panel replaces it.
/// The choice is in the database, so a restart keeps it, and health says both
/// who pays and that it was chosen.
#[tokio::test]
async fn letting_the_plan_pay_is_remembered_and_shown() {
    let (app, state) = setup(Economy::Subscription {
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
    assert_eq!(payer(&state).await, None, "the key no longer pays");

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
    assert_eq!(payer(&state).await, Some((Kind::Key, Source::Environment)));
}

/// With a subscription linked, the plan is that account: choosing it makes it
/// the payer, and the key goes back to being one of two.
#[tokio::test]
async fn with_a_subscription_linked_the_plan_is_that_account() {
    let (app, state) = setup(Economy::Subscription { plan: None }).await;
    let plan = accounts::link(&state, "anthropic", Kind::Subscription, TOKEN, None)
        .await
        .expect("link");
    assert!(!plan.active, "the key was here first");

    let (s, body) = send(
        &app,
        "POST",
        "/api/economy/pay-with",
        Some(json!({ "with": "plan" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["pay_with"], json!("plan"));
    let active = accounts::active(&state.pool, "anthropic")
        .await
        .expect("active")
        .expect("someone pays");
    assert_eq!(active.id, plan.id);
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
