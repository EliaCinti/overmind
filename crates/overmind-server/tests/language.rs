//! M16: the company's language is chosen when the company is founded.
//!
//! The setting shipped with M16 and worked — but only through
//! `POST /companies/{id}/language`, and `POST /companies` took a name and
//! nothing else. A request that said `"language": "it"` was accepted, stored as
//! English, and told nothing: serde drops what it does not know. That mattered
//! more than a missing field usually does, because a company is founded *with a
//! CEO* (M15) and the next thing that happens is that CEO speaking. Found live
//! on 2026-08-13, when a company created "in Italian" answered in English.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Hands back the prompt it was given, so a test can read what the agent read.
const ECHOING_STUB: &str = r#"#!/bin/sh
printf '%s' "$OVERMIND_TASK_PROMPT" > PROMPT.txt
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
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

async fn app() -> axum::Router {
    let root =
        std::env::temp_dir().join(format!("overmind-lang-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("create test root");
    let script = root.join("stub.sh");
    std::fs::write(&script, ECHOING_STUB).expect("write stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init in-memory db");
    overmind_server::app(state)
}

#[tokio::test]
async fn a_company_is_founded_in_the_language_it_asked_for() {
    let app = app().await;

    let (status, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Ricerca SpA", "language": "it" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{company}");
    assert_eq!(
        company["language"],
        json!("it"),
        "the answer must report the language it stored, not the default it \
         would have used: {company}"
    );

    // And it is the stored row, not only the response body.
    let (_, listed) = send(&app, "GET", "/api/companies", None).await;
    let mine = listed["companies"]
        .as_array()
        .expect("companies")
        .iter()
        .find(|c| c["id"] == company["id"])
        .cloned()
        .expect("the company we just made");
    assert_eq!(mine["language"], json!("it"), "{mine}");
}

#[tokio::test]
async fn a_language_we_do_not_speak_is_refused_where_it_enters() {
    let app = app().await;

    let (status, body) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Nowhere Ltd", "language": "tlh" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("tlh"),
        "the refusal names what it did not recognise: {body}"
    );

    // Saying nothing is still allowed, and still means English.
    let (status, body) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Quiet Ltd" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["language"], json!("en"), "{body}");
}

/// The setting is only worth having if it reaches the prompt. This is the half
/// that was never in doubt and never checked end to end: the stub hands the
/// prompt back as an artifact, so the test reads exactly what the agent read.
#[tokio::test]
async fn the_founding_language_reaches_the_agent() {
    let app = app().await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Ricerca SpA", "language": "it" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company").to_string();
    let agent_id = company["ceo"]["id"].as_str().expect("ceo").to_string();

    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({
            "title": "Dimmi qualcosa",
            "description": "Qualsiasi cosa.",
            "execution_kind": "knowledge",
        })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (status, started) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");

    let session_id = started["session_id"].as_str().expect("session id");
    for _ in 0..100 {
        let (_, s) = send(&app, "GET", &format!("/api/sessions/{session_id}"), None).await;
        if matches!(s["status"].as_str().unwrap_or(""), "completed" | "failed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let (_, artifacts) = send(
        &app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    let prompt = artifacts["artifacts"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["title"] == json!("PROMPT.txt")))
        .and_then(|a| a["content"].as_str())
        .unwrap_or("")
        .to_string();

    assert!(
        prompt.contains("in Italian"),
        "the agent must be told which language to write in: {prompt}"
    );
}
