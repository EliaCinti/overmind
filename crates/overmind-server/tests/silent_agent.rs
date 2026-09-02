//! A conversational turn that fails must say so.
//!
//! The first live run after M8 (2026-08-13) produced an **empty `[ceo]` bubble**
//! and nothing else: no error, no log line, no row anywhere naming a cause. The
//! turn had collected the adapter's stderr and thrown it away, and returned its
//! empty stdout as a success, so ten minutes were spent looking for a failure
//! the system had already seen and discarded.
//!
//! The task runner has kept stderr since M2. These tests hold the conversational
//! path — the one a human sits and watches — to the same standard.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Dies the way a real adapter dies: a reason on stderr, a non-zero exit, and
/// not one byte on stdout.
const DYING_STUB: &str = r#"#!/bin/sh
echo 'Credit balance is too low' >&2
exit 1
"#;

/// Worse than dying, because it looks like success: exit 0, nothing said. This
/// is the shape the sandbox regression took.
const MUTE_STUB: &str = r#"#!/bin/sh
exit 0
"#;

/// Waits for input before answering. With stdin closed it reads EOF at once;
/// inheriting an open one, it waits forever.
const READS_STDIN_STUB: &str = r#"#!/bin/sh
cat > /dev/null
echo '{"reply":"I heard nothing, which is the point.","tasks":[]}'
"#;

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

struct TestEnv {
    app: axum::Router,
    company_id: String,
    ceo_id: String,
}

async fn setup(stub: &str) -> TestEnv {
    let root =
        std::env::temp_dir().join(format!("overmind-silent-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("create test root");
    let script = root.join("stub.sh");
    std::fs::write(&script, stub).expect("write stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init in-memory db");
    let app = common::claimed(overmind_server::app(state), &root.join("data")).await;
    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Quiet Co" })),
    )
    .await;
    TestEnv {
        company_id: company["id"].as_str().expect("company").to_string(),
        ceo_id: company["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
    }
}

async fn say(env: &TestEnv, text: &str) {
    let (status, body) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.ceo_id
        ),
        Some(json!({ "content": text })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "post message: {body}");
}

/// Wait for the turn — it runs detached — and return the thread.
async fn settle(env: &TestEnv, expected: usize) -> Vec<Value> {
    for _ in 0..100 {
        let (_, convo) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company_id, env.ceo_id
            ),
            None,
        )
        .await;
        let m = convo["messages"].as_array().cloned().unwrap_or_default();
        if m.len() >= expected {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("thread never reached {expected} messages");
}

/// The reply to a question, whichever role answered it.
fn answer(messages: &[Value]) -> (String, String) {
    let last = messages.last().expect("a reply");
    (
        last["role"].as_str().unwrap_or_default().to_string(),
        last["content"].as_str().unwrap_or_default().to_string(),
    )
}

#[tokio::test]
async fn a_dying_adapter_is_quoted_not_swallowed() {
    let env = setup(DYING_STUB).await;
    say(&env, "What should we do first?").await;
    let (role, content) = answer(&settle(&env, 2).await);

    assert_eq!(role, "system", "Overmind's own voice, not the agent's");
    assert!(
        content.contains("Credit balance is too low"),
        "the reason the adapter gave must reach the person who asked: {content}"
    );
    assert!(
        content.contains("code 1"),
        "and how it ended, so a silent exit and a loud one read differently: {content}"
    );
}

#[tokio::test]
async fn a_mute_adapter_is_not_an_empty_bubble() {
    let env = setup(MUTE_STUB).await;
    say(&env, "What should we do first?").await;
    let (role, content) = answer(&settle(&env, 2).await);

    assert_eq!(role, "system", "an empty [ceo] message is the bug itself");
    assert!(
        !content.trim().is_empty(),
        "a turn that produced nothing must still say something"
    );
    assert!(
        content.contains("no output"),
        "and it must name what happened: {content}"
    );
}

/// The adapter's stdin is closed, so a CLI that waits for piped input does not
/// wait. The Claude CLI announces this itself — *"no stdin data received in 3s,
/// proceeding without it"* — and a server running as a daemon holds a stdin that
/// never reaches EOF for the child to inherit.
///
/// Honest about its reach: where the test process already has a closed or
/// redirected stdin (CI), this passes with or without the fix. Where it does not
/// (a developer's terminal, and the daemon this was found on), it hangs without
/// it. Only one of those two environments could ever have shown the defect.
#[tokio::test]
async fn an_adapter_that_waits_for_stdin_is_not_left_waiting() {
    let env = setup(READS_STDIN_STUB).await;
    say(&env, "Are you there?").await;
    let (role, content) = answer(&settle(&env, 2).await);

    assert_eq!(role, "ceo", "the turn completed normally");
    assert!(content.contains("I heard nothing"), "got: {content}");
}
