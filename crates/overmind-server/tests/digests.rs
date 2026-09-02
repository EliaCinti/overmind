//! The CEO writes back on its own (ADR-0041).
//!
//! The owner: "quando necessario, dopo aver visto le risposte degli agenti,
//! Rune mi scriva da solo in autonomia per aggiornarmi." When tasks born in
//! a thread finish after the person's last word there, the thread's agent
//! writes one unprompted update — or deliberately stays silent (SKIP), and
//! is never asked twice about the same completions.

mod common;

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

/// One stub, three voices: a chat turn plans a task for Tobia; a digest turn
/// answers with `digest_reply`; a task run just works.
fn stub(digest_reply: &str) -> String {
    format!(
        r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"finished since your last word"*)
    echo '{{"type":"result","result":"{{\"reply\":\"{digest_reply}\",\"tasks\":[]}}","total_cost_usd":0.001}}'
    ;;
  *"Respond with a SINGLE JSON"*)
    echo '{{"type":"result","result":"{{\"reply\":\"Apro il task.\",\"tasks\":[{{\"title\":\"Studia il progetto\",\"description\":\"Fallo.\",\"execution_kind\":\"knowledge\",\"assignee\":\"Tobia\"}}]}}","total_cost_usd":0.001}}'
    ;;
  *)
    echo "Studio completo." > studio.md
    echo '{{"type":"result","result":"LAVORO-FATTO: ho studiato tutto.","total_cost_usd":0.001,"session_id":"s"}}'
    ;;
esac
"#
    )
}

struct Env {
    app: axum::Router,
    state: overmind_server::AppState,
    company: String,
    ceo: String,
}

async fn setup(digest_reply: &str) -> Env {
    let root: PathBuf = std::env::temp_dir().join(format!(
        "overmind-digest-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&root).expect("mkdir");
    let script = root.join("stub.sh");
    std::fs::write(&script, stub(digest_reply)).expect("stub");
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            agent_cmd: Some(format!("sh {}", script.display())),
            data_dir: root.join("data"),
            heartbeat_ms: 1_000_000,
            digest_debounce_secs: 0,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = common::claimed(overmind_server::app(state.clone()), &root.join("data")).await;
    let (s, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Digest Co" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{co}");
    let env = Env {
        company: co["id"].as_str().expect("id").to_string(),
        ceo: co["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
        state,
    };
    // Tobia acts within budget, so the planned task runs at once.
    let (s, a) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({ "name": "Tobia", "archetype": "writer",
                     "traits": { "autonomy": "act_within_budget" } })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    env
}

async fn messages(env: &Env) -> Vec<Value> {
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
    c["messages"].as_array().cloned().unwrap_or_default()
}

/// Drive: ask the CEO, wait until the planned task's run completed.
async fn brief_and_finish(env: &Env) {
    let (s, v) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company, env.ceo
        ),
        Some(json!({ "content": "Fate partire lo studio." })),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
    for _ in 0..200 {
        let done: Option<(i64,)> =
            sqlx::query_as("SELECT COUNT(*) FROM agent_task_sessions WHERE status = 'completed'")
                .fetch_optional(&env.state.pool)
                .await
                .expect("query");
        if done.map(|(n,)| n).unwrap_or(0) >= 1 && !env.state.is_answering_anywhere() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let tasks: Vec<(String, String)> = sqlx::query_as("SELECT title, status FROM tasks")
        .fetch_all(&env.state.pool)
        .await
        .expect("q");
    let sessions: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT status, last_error FROM agent_task_sessions")
            .fetch_all(&env.state.pool)
            .await
            .expect("q");
    let msgs = messages(env).await;
    panic!("the task never completed; tasks={tasks:?} sessions={sessions:?} msgs={msgs:?}");
}

/// A finished task after the person's last word → one unprompted update in
/// the thread; a second beat does not repeat it.
#[tokio::test]
async fn a_finished_task_earns_one_unprompted_update() {
    let env = setup("AGGIORNAMENTO: Tobia ha consegnato, nessuna decisione urgente.").await;
    brief_and_finish(&env).await;

    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    let mut said_update = false;
    for _ in 0..100 {
        let m = messages(&env).await;
        if m.iter().any(|x| {
            x["role"] == "ceo"
                && x["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("AGGIORNAMENTO")
        }) {
            said_update = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(said_update, "the update landed: {:?}", messages(&env).await);

    let before = messages(&env).await.len();
    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        messages(&env).await.len(),
        before,
        "the same completions are not re-announced"
    );
}

/// The agent may deliberately stay silent: SKIP posts nothing, and the same
/// completions are not asked about again.
#[tokio::test]
async fn a_skip_stays_silent_and_is_not_asked_twice() {
    let env = setup("SKIP").await;
    brief_and_finish(&env).await;
    let before = messages(&env).await.len();

    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(messages(&env).await.len(), before, "SKIP posts nothing");

    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        messages(&env).await.len(),
        before,
        "and it is not asked twice"
    );
}
