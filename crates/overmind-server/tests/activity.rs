//! The work is visible while it happens (ADR-0039).
//!
//! The owner, watching a 20-minute Blender run with nothing moving: "the user
//! must know what is happening — and above all that something is happening."
//! The adapter already narrates itself (`stream-json`, one event per line);
//! Overmind used to collect that stream and read it only at the end. Now it
//! is read as it arrives: the current activity — which tool, or a first line
//! of what the agent is saying — rides on the conversation while a turn is
//! answering, and on the session while a task runs.

use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

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

/// An adapter that narrates: announces a tool call as a stream-json line,
/// works for two seconds, then delivers the result envelope.
fn narrating_stub() -> String {
    let tool_line = json!({
        "type": "assistant",
        "message": { "content": [
            { "type": "tool_use", "name": "mcp__blender__execute_blender_code", "input": {} }
        ]}
    })
    .to_string();
    let result_line = json!({
        "type": "result",
        "result": "{\"reply\":\"done\",\"tasks\":[]}",
        "total_cost_usd": 0.001,
        "session_id": "stub"
    })
    .to_string();
    format!("#!/bin/sh\necho '{tool_line}'\nsleep 2\necho '{result_line}'\n")
}

struct Env {
    app: axum::Router,
    company: String,
    ceo: String,
    _root: PathBuf,
}

async fn setup() -> Env {
    let root = std::env::temp_dir().join(format!(
        "overmind-activity-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&root).expect("mkdir");
    let script = root.join("stub.sh");
    std::fs::write(&script, narrating_stub()).expect("stub");
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            agent_cmd: Some(format!("sh {}", script.display())),
            data_dir: root.join("data"),
            heartbeat_ms: 1_000_000,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = overmind_server::app(state);
    let (s, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Live Co" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{co}");
    Env {
        company: co["id"].as_str().expect("id").to_string(),
        ceo: co["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
        _root: root,
    }
}

/// While the CEO answers, the conversation names what it is doing right now;
/// when the reply lands, the activity is gone.
#[tokio::test]
async fn a_conversation_names_what_the_agent_is_doing() {
    let env = setup().await;
    let (s, v) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company, env.ceo
        ),
        Some(json!({ "content": "Build it." })),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");

    // The tool line arrives within the stub's first moments.
    let mut seen_tool = false;
    for _ in 0..40 {
        let (_, c) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company, env.ceo
            ),
            None,
        )
        .await;
        if c["activity"]["kind"] == json!("tool") {
            assert_eq!(c["activity"]["server"], json!("blender"), "{c}");
            assert_eq!(c["activity"]["tool"], json!("execute blender code"), "{c}");
            seen_tool = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(seen_tool, "the tool call was visible while it ran");

    // And once the reply lands, the narration is over.
    for _ in 0..100 {
        let (_, c) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company, env.ceo
            ),
            None,
        )
        .await;
        if c["answering"] == json!(false) {
            assert!(c["activity"].is_null(), "{c}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the turn never finished");
}

/// A running task session names what its agent is doing right now.
#[tokio::test]
async fn a_running_session_names_what_the_agent_is_doing() {
    let env = setup().await;
    let (_, a) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({ "name": "Tobia", "archetype": "writer" })),
    )
    .await;
    let agent = a["id"].as_str().expect("agent").to_string();
    let (_, t) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company),
        Some(json!({ "title": "Do it", "execution_kind": "knowledge" })),
    )
    .await;
    let task = t["id"].as_str().expect("task").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (s, v) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let session = v["session_id"].as_str().expect("session").to_string();

    let mut seen = false;
    for _ in 0..40 {
        let (_, sv) = send(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        if sv["activity"]["kind"] == json!("tool") {
            assert_eq!(sv["activity"]["server"], json!("blender"), "{sv}");
            seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(seen, "the session named the tool while it ran");

    for _ in 0..100 {
        let (_, sv) = send(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        if sv["status"] != "running" {
            assert!(sv["activity"].is_null(), "{sv}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the session never finished");
}

/// The reply streams (ADR-0039 addendum): text deltas accumulate into a
/// `draft` activity holding the readable reply-so-far — JSON plan syntax
/// stripped — so the chat shows the words appearing.
#[tokio::test]
async fn the_reply_streams_as_a_draft() {
    use overmind_server::runner_test_hooks as hooks;
    // Unit-level: extraction from the plan JSON as it grows.
    assert_eq!(
        hooks::draft_reply("Ecco cosa penso"),
        Some("Ecco cosa penso".into())
    );
    assert_eq!(
        hooks::draft_reply("{\"reply\": \"Ciao El"),
        Some("Ciao El".into())
    );
    assert_eq!(
        hooks::draft_reply("{\"reply\": \"Riga uno.\\nRiga due\", \"tasks\": []}"),
        Some("Riga uno.\nRiga due".into())
    );
    assert_eq!(hooks::draft_reply("{\"tas"), None);
    assert_eq!(
        hooks::text_delta_in(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Ciao"}}}"#
        ),
        Some("Ciao".into())
    );
}
