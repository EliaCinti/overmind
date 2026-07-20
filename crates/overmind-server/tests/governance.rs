//! M6 acceptance tests: budgets stop over-spend server-side, an approval gate
//! blocks a start until a human decides, agents can be paused/terminated, and
//! config revisions can be rolled back — all reflected in the audit chain.

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

fn sh(dir: &std::path::Path, cmd: &str) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .output()
        .expect("run shell");
    assert!(out.status.success(), "cmd failed: {cmd}");
}

fn unique_root() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("overmind-gov-{nanos}-{}-{n}", std::process::id()))
}

const STUB: &str = r#"#!/bin/sh
echo "work" > out.txt
echo '{"model":"stub","session_id":"s","total_cost_usd":0.05,"usage":{"input_tokens":10,"output_tokens":5}}'
"#;

struct Env {
    app: axum::Router,
    company: String,
    goal: String,
}

async fn setup(tweak: impl FnOnce(&mut overmind_server::Config)) -> Env {
    let root = unique_root();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo x > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );
    let script = root.join("stub.sh");
    std::fs::write(&script, STUB).expect("write stub");

    let mut config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000, // effectively off for these tests
        ..overmind_server::Config::default()
    };
    tweak(&mut config);
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = overmind_server::app(state);

    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Gov Co" })),
    )
    .await;
    let company = co["id"].as_str().expect("id").to_string();
    let (_, pr) = send(
        &app,
        "POST",
        &format!("/api/companies/{company}/projects"),
        Some(json!({ "title": "P" })),
    )
    .await;
    let project = pr["id"].as_str().expect("id").to_string();
    send(
        &app,
        "POST",
        &format!("/api/projects/{project}/workspaces"),
        Some(json!({ "name": "main", "cwd": repo.to_string_lossy() })),
    )
    .await;
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/api/projects/{project}/goals"),
        Some(json!({ "title": "G" })),
    )
    .await;
    let goal = goal["id"].as_str().expect("id").to_string();
    Env { app, company, goal }
}

async fn hire(env: &Env, name: &str, budget: i64) -> String {
    let (s, a) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({
            "name": name,
            "archetype": "backend-developer",
            "traits": { "monthly_budget_cents": budget }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "hire: {a}");
    a["id"].as_str().expect("id").to_string()
}

async fn make_todo(env: &Env, title: &str) -> String {
    let (_, t) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company),
        Some(json!({ "title": title, "goal_id": env.goal })),
    )
    .await;
    let id = t["id"].as_str().expect("id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    id
}

#[tokio::test]
async fn start_is_stopped_when_over_budget() {
    // Budget below the start reservation (default 50c) → any start is refused.
    let env = setup(|_| {}).await;
    let agent = hire(&env, "Frugal", 30).await;
    let task = make_todo(&env, "Costs money").await;

    let (s, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED, "body: {body}");
    assert!(body["error"].as_str().unwrap_or("").contains("budget"));

    // Task was never checked out.
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "todo");

    // The budget summary shows the cap and zero spend, and the audit verifies.
    let (_, budgets) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/budget", env.company),
        None,
    )
    .await;
    assert_eq!(budgets["budgets"][0]["budget_cents"], 30);
    assert_eq!(budgets["budgets"][0]["spent_cents"], 0);
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

async fn wait_session_done(app: &axum::Router, session_id: &str) {
    for _ in 0..100 {
        let (_, s) = send(app, "GET", &format!("/api/sessions/{session_id}"), None).await;
        let st = s["status"].as_str().unwrap_or("");
        if st == "completed" || st == "failed" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session never finished");
}

#[tokio::test]
async fn approval_gate_blocks_until_approved() {
    let env = setup(|_| {}).await;
    let agent = hire(&env, "Gated", 100000).await;
    send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/approval-gate"),
        Some(json!({ "requires_approval": true })),
    )
    .await;
    let task = make_todo(&env, "Needs sign-off").await;

    // Starting files an approval and launches nothing; task stays todo.
    let (s, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "approval_required");
    let approval = body["approval_id"]
        .as_str()
        .expect("approval id")
        .to_string();

    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "todo");
    let (_, inbox) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/approvals", env.company),
        None,
    )
    .await;
    assert_eq!(inbox["approvals"][0]["status"], "pending");

    // Approving carries out the start.
    let (s, decided) = send(
        &env.app,
        "POST",
        &format!("/api/approvals/{approval}/decision"),
        Some(json!({ "decision": "approve" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "decided: {decided}");
    let session = decided["session_id"].as_str().expect("session id");
    wait_session_done(&env.app, session).await;
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "in_review");

    // Deciding an already-decided approval is a conflict.
    let (s, _) = send(
        &env.app,
        "POST",
        &format!("/api/approvals/{approval}/decision"),
        Some(json!({ "decision": "reject" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn paused_and_terminated_agents_cannot_start() {
    let env = setup(|_| {}).await;
    let agent = hire(&env, "Sleepy", 100000).await;
    let task = make_todo(&env, "Work").await;

    send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/pause"),
        None,
    )
    .await;
    let (s, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "body: {body}");
    assert!(body["error"].as_str().unwrap_or("").contains("paused"));

    // Resume, then terminate — terminated is permanent.
    send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/resume"),
        None,
    )
    .await;
    send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/terminate"),
        None,
    )
    .await;
    let (s, _) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/resume"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn config_revisions_roll_back() {
    let env = setup(|_| {}).await;
    let agent = hire(&env, "Original", 5000).await;

    // Change the title (a config revision), then roll back to the hire state.
    send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/reassign"),
        Some(json!({ "title": "Renamed" })),
    )
    .await;

    let (_, revs) = send(
        &env.app,
        "GET",
        &format!("/api/agents/{agent}/revisions"),
        None,
    )
    .await;
    let list = revs["revisions"].as_array().expect("revisions");
    assert_eq!(list.len(), 2); // hire + patch
    // Newest first; the hire revision is the last one.
    let hire_rev = list.last().expect("hire rev");
    assert_eq!(hire_rev["source"], "hire");
    assert_eq!(hire_rev["config"]["title"], Value::Null);
    let hire_rev_id = hire_rev["id"].as_str().expect("rev id").to_string();

    // Confirm the title changed, then roll back.
    let (_, agents) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/agents", env.company),
        None,
    )
    .await;
    assert_eq!(agents["agents"][0]["title"], "Renamed");

    let (s, _) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/rollback"),
        Some(json!({ "revision_id": hire_rev_id })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, agents) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/agents", env.company),
        None,
    )
    .await;
    assert_eq!(agents["agents"][0]["title"], Value::Null);

    // Rollback appended a third revision; history is forward-only.
    let (_, revs) = send(
        &env.app,
        "GET",
        &format!("/api/agents/{agent}/revisions"),
        None,
    )
    .await;
    assert_eq!(revs["revisions"].as_array().expect("revisions").len(), 3);
    assert_eq!(revs["revisions"][0]["source"], "rollback");

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}
