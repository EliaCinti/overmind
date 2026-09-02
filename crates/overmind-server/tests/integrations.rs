//! M9 acceptance tests (ADR-0028): Overmind as an MCP server for callers
//! outside it.
//!
//! The criterion is one sentence — *a Claude Code session outside Overmind
//! creates a task via MCP* — and most of what these hold is the other half of
//! it: what such a caller deliberately **cannot** do. Filing work is a request;
//! starting it is authority, and since M6 that authority has been a human's.
//! A tool list is only a boundary if something checks it.

mod common;

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

/// One JSON-RPC call to `/mcp`, the way an MCP client makes it.
async fn mcp(app: &axum::Router, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).expect("build"))
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The text a tool answered with, and whether it was an error.
fn answer(v: &Value) -> (String, bool) {
    (
        v.pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        v.pointer("/result/isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

struct Env {
    app: axum::Router,
    company_id: String,
    token: String,
    token_id: String,
}

async fn setup() -> Env {
    let root = std::env::temp_dir().join(format!("overmind-m9-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("create test root");
    let config = overmind_server::Config {
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
        Some(json!({ "name": "Outside Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company").to_string();

    let (status, issued) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tokens"),
        Some(json!({ "label": "my editor" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "issue a token: {issued}");

    Env {
        token: issued["token"].as_str().expect("the secret").to_string(),
        token_id: issued["id"].as_str().expect("token id").to_string(),
        company_id,
        app,
    }
}

/// The M9 criterion.
#[tokio::test]
async fn a_session_outside_overmind_files_a_task() {
    let env = setup().await;

    let (status, listed) = mcp(
        &env.app,
        Some(&env.token),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"create_task"), "{names:?}");
    assert!(names.contains(&"list_tasks"), "{names:?}");

    let (status, called) = mcp(
        &env.app,
        Some(&env.token),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": {
                    "title": "Write the release notes",
                    "description": "For 0.2, from the changelog.",
                    "execution_kind": "knowledge"
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{called}");
    let (text, is_error) = answer(&called);
    assert!(!is_error, "{text}");
    assert!(text.contains("backlog"), "{text}");

    // It is a real task on the real board, not a message that says so.
    let (_, board) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let task = board["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["title"] == json!("Write the release notes"))
        .cloned()
        .expect("the task we filed");
    assert_eq!(task["status"], json!("backlog"), "{task}");
    assert_eq!(task["execution_kind"], json!("knowledge"), "{task}");
    assert!(
        task["assignee_agent_id"].is_null(),
        "filed, not handed to anyone: {task}"
    );

    // And the audit log has it, so a task that arrived while nobody was
    // watching is still accounted for.
    let (_, events) = send(&env.app, "GET", "/api/audit/events", None).await;
    assert!(
        events["events"]
            .as_array()
            .expect("events")
            .iter()
            .any(|e| e["kind"] == json!("task.created")),
        "the filing is in the log: {events}"
    );
}

/// The other half of the criterion, and the reason the tool list is a list.
#[tokio::test]
async fn an_integration_cannot_start_work_or_read_memory() {
    let env = setup().await;

    for forbidden in ["start_task", "hire_agent", "recall", "store_memory"] {
        let (status, called) = mcp(
            &env.app,
            Some(&env.token),
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": forbidden, "arguments": {} }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{called}");
        let (text, is_error) = answer(&called);
        assert!(is_error, "`{forbidden}` must be refused: {text}");
        assert!(
            text.contains(forbidden),
            "the refusal names what was asked for: {text}"
        );
        assert!(
            text.contains("started by a person"),
            "and says why, so a model stops asking: {text}"
        );
    }

    // The memory tools are not merely refused when called — they are not
    // offered, so a model never learns they might exist.
    let (_, listed) = mcp(
        &env.app,
        Some(&env.token),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(!names.contains(&"recall"), "{names:?}");
}

/// A credential you cannot withdraw is worse than none.
#[tokio::test]
async fn a_revoked_token_stops_working_immediately() {
    let env = setup().await;
    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });

    let (status, _) = mcp(&env.app, Some(&env.token), call.clone()).await;
    assert_eq!(status, StatusCode::OK, "it works before revocation");

    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/tokens/{}/revoke", env.token_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _) = mcp(&env.app, Some(&env.token), call.clone()).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a revoked token is as unknown as one that never existed"
    );

    // The row stays, so the audit events that name it still point at something.
    let (_, listed) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tokens", env.company_id),
        None,
    )
    .await;
    let row = listed["tokens"]
        .as_array()
        .expect("tokens")
        .iter()
        .find(|t| t["id"] == json!(env.token_id))
        .cloned()
        .expect("the revoked token is still listed");
    assert!(!row["revoked_at"].is_null(), "{row}");
    assert!(
        row.get("token").is_none(),
        "a listing must never carry the secret: {row}"
    );
}

/// The token is the company. There is no argument that could reach another one.
#[tokio::test]
async fn a_token_reaches_only_its_own_company() {
    let env = setup().await;
    let (_, other) = send(
        &env.app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Somebody Else Ltd" })),
    )
    .await;
    let other_id = other["id"].as_str().expect("company").to_string();

    // A task in the other company, filed through the ordinary API.
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{other_id}/tasks"),
        Some(json!({ "title": "Not yours", "execution_kind": "knowledge" })),
    )
    .await;
    let other_task_id = task["id"].as_str().expect("task").to_string();

    // The board this token can see does not contain it...
    let (_, called) = mcp(
        &env.app,
        Some(&env.token),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "list_tasks", "arguments": {} }
        }),
    )
    .await;
    let (text, _) = answer(&called);
    assert!(!text.contains("Not yours"), "{text}");

    // ...and naming its id directly does not reach it either, which is the
    // difference between a filter and a boundary.
    let (_, called) = mcp(
        &env.app,
        Some(&env.token),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "get_task", "arguments": { "task_id": other_task_id } }
        }),
    )
    .await;
    let (text, is_error) = answer(&called);
    assert!(is_error, "{text}");
    assert!(text.contains("no task"), "{text}");
}

/// Reading the log is part of the criterion, and it is the answer to "what has
/// this company actually done".
#[tokio::test]
async fn an_integration_reads_the_board_and_the_log() {
    let env = setup().await;

    let (_, called) = mcp(
        &env.app,
        Some(&env.token),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "verify_audit", "arguments": {} }
        }),
    )
    .await;
    let (text, is_error) = answer(&called);
    assert!(!is_error, "{text}");
    assert!(text.contains("verifies"), "{text}");

    let (_, called) = mcp(
        &env.app,
        Some(&env.token),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "list_events", "arguments": { "limit": 5 } }
        }),
    )
    .await;
    let (text, is_error) = answer(&called);
    assert!(!is_error, "{text}");
    assert!(
        text.contains("company.created") || text.contains("agent.hired"),
        "founding a company is the first thing in its log: {text}"
    );

    // An empty board says so rather than answering with nothing.
    let (_, called) = mcp(
        &env.app,
        Some(&env.token),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "list_tasks", "arguments": {} }
        }),
    )
    .await;
    let (text, is_error) = answer(&called);
    assert!(!is_error, "{text}");
    assert!(text.contains("empty"), "{text}");
}
