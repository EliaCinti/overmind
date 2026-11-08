//! Connecting an account is the provider's to say (ADR-0051 decision 5), and
//! the sign-in Claude Code needed is now one provider's implementation of it.
//!
//! The real CLI never runs here. A subscription sign-in drives a stub that
//! behaves like `claude setup-token` on a pty -- prints an authorization URL,
//! reads a pasted code, prints a token -- through `Config::sign_in_stub`.
//!
//! The flow is one process-wide slot, as it is in production (one instance
//! per process), so every scenario that starts one lives in a single test.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use overmind_server::accounts::{self, Kind};
use overmind_server::economy::Economy;
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKEN: &str = "sk-ant-oat01-SIGNEDINSIGNEDINSIGNEDINSIGNEDINSIGNEDIN-0001";

fn unique_root(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "overmind-sign-in-{tag}-{}",
        uuid::Uuid::now_v7().simple()
    ))
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
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
    let raw = String::from_utf8_lossy(&bytes).to_string();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        raw,
    )
}

/// A `setup-token` look-alike: the URL, a prompt, the code read back, the
/// token printed the way the CLI prints it, a clean exit.
fn write_stub(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root).expect("mkdir");
    let stub = root.join("setup-token-stub.sh");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             echo 'Browser did not open? Use the url below to sign in:'\n\
             echo 'https://claude.ai/oauth/authorize?code=true&state=STUB'\n\
             printf 'Paste code here if prompted > '\n\
             read code\n\
             echo ''\n\
             echo 'Your OAuth token (valid for 1 year):'\n\
             echo '{TOKEN}'\n\
             echo 'Store this token securely.'\n"
        ),
    )
    .expect("write stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    stub
}

async fn instance(root: &Path, stub: Option<PathBuf>) -> (overmind_server::AppState, axum::Router) {
    let data_dir = root.join("data");
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            sign_in_stub: stub,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = common::claimed(overmind_server::app(state.clone()), &data_dir).await;
    (state, app)
}

async fn poll_until(app: &axum::Router, want: &str, seen: &mut Vec<String>) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let (s, v, raw) = send(app, "GET", "/api/accounts/sign-in", None).await;
        assert_eq!(s, StatusCode::OK, "{raw}");
        seen.push(raw);
        if v["state"] == json!(want) {
            return v;
        }
        assert!(
            v["state"] != json!("failed"),
            "the sign-in failed while waiting for {want}: {v}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "never reached {want}: {v}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_subscription_signed_in_becomes_an_account_and_its_token_never_shows() {
    let root = unique_root("subscription");
    let stub = write_stub(&root);
    let (state, app) = instance(&root, Some(stub)).await;

    let (s, v, raw) = send(
        &app,
        "POST",
        "/api/accounts/sign-in",
        Some(json!({ "provider": "anthropic", "kind": "subscription" })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{raw}");
    assert_eq!(v["shape"], json!("paste_back"), "{v}");

    let mut seen = Vec::new();
    let ready = poll_until(&app, "url_ready", &mut seen).await;
    assert_eq!(
        ready["url"],
        json!("https://claude.ai/oauth/authorize?code=true&state=STUB")
    );
    assert_eq!(ready["provider"], json!("anthropic"));

    let (s, _, raw) = send(
        &app,
        "POST",
        "/api/accounts/sign-in/code",
        Some(json!({ "code": "the-code" })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{raw}");
    let done = poll_until(&app, "done", &mut seen).await;

    // The token is an account now -- the provider's first, so the payer --
    // and no file outside `credentials/` holds it.
    let all = accounts::list(&state.pool).await.expect("list");
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].kind, Kind::Subscription);
    assert_eq!(all[0].provider, "anthropic");
    assert!(all[0].active);
    assert_eq!(done["account"]["id"], json!(all[0].id), "{done}");
    assert!(!root.join("data").join("claude-oauth-token").exists());

    for raw in &seen {
        assert!(
            !raw.contains("SIGNEDINSIGNEDIN"),
            "a status answer carried the token: {raw}"
        );
    }

    // Polling a finished sign-in reads what was learned once; it does not run
    // the probe again and overwrite the economy everybody else reads (NEXT,
    // "the sign-in and the cage": every poll used to re-detect).
    let marked = Economy::Unknown {
        kind: overmind_server::economy::UnknownKind::Unreadable,
        reason: "set by the test after the sign-in finished".into(),
    };
    state.set_economy(marked.clone());
    for _ in 0..3 {
        let (s, v, _) = send(&app, "GET", "/api/accounts/sign-in", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["state"], json!("done"));
    }
    assert_eq!(
        state.economy(),
        marked,
        "a poll re-ran the economy probe after the sign-in was done"
    );

    // The old route still answers, for one release, onto the same flow.
    let (s, v, _) = send(&app, "GET", "/api/claude-auth", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], json!("done"));
}

/// A key is pasted, so nothing is spawned -- and a custom adapter, which
/// rightly refuses a CLI sign-in, has no reason to refuse it. Before, the one
/// guard sat in front of every sign-in, so a provider whose accounts are keys
/// could never be connected on an instance configured that way.
#[tokio::test]
async fn a_key_needs_no_cli_so_a_custom_adapter_does_not_refuse_it() {
    let root = unique_root("key");
    let (_state, app) = instance(&root, None).await;

    let (s, v, raw) = send(
        &app,
        "POST",
        "/api/accounts/sign-in",
        Some(json!({ "provider": "anthropic", "kind": "key" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{raw}");
    assert_eq!(v["shape"], json!("key"), "{v}");

    // The subscription is the CLI's to drive, and a custom adapter is not
    // the CLI: refused, and nothing spawned.
    let (s, _, raw) = send(
        &app,
        "POST",
        "/api/accounts/sign-in",
        Some(json!({ "provider": "anthropic", "kind": "subscription" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{raw}");
    assert!(raw.contains("OVERMIND_AGENT_CMD"), "{raw}");

    for body in [
        json!({ "provider": "nobody", "kind": "key" }),
        json!({ "provider": "anthropic", "kind": "barter" }),
    ] {
        let (s, _, raw) = send(&app, "POST", "/api/accounts/sign-in", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} → {raw}");
    }
}

/// What the panel needs to draw a provider: which kinds it takes, and in
/// what shape each is connected.
#[tokio::test]
async fn the_accounts_list_says_how_each_kind_is_connected() {
    let root = unique_root("shapes");
    let (_state, app) = instance(&root, None).await;
    let (s, v, raw) = send(&app, "GET", "/api/accounts", None).await;
    assert_eq!(s, StatusCode::OK, "{raw}");
    let anthropic = v["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|p| p["provider"] == json!("anthropic"))
        .expect("anthropic is listed")
        .clone();
    assert_eq!(
        anthropic["kinds"]["subscription"]["shape"],
        json!("paste_back")
    );
    assert_eq!(anthropic["kinds"]["key"]["shape"], json!("key"));
}
