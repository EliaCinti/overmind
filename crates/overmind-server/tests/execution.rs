//! M2 acceptance tests: a real (stub) agent completes a task in an isolated
//! worktree, the diff is visible, every step is audited, cost is recorded,
//! and concurrent checkouts of the same task can't double-run.

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

async fn send_text(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn sh(dir: &std::path::Path, cmd: &str) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .output()
        .expect("run shell command");
    assert!(
        out.status.success(),
        "command failed: {cmd}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct TestEnv {
    app: axum::Router,
    root: PathBuf,
    company_id: String,
    agent_id: String,
    task_id: String,
}

/// Fresh in-memory Overmind + a real git repo + a stub agent script, wired
/// through company -> project -> workspace -> goal -> task (status: todo).
async fn setup(stub_script: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!("overmind-test-{}", uuid_like()));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo '# Demo' > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );

    let script_path = root.join("stub-agent.sh");
    std::fs::write(&script_path, stub_script).expect("write stub script");

    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script_path.display())),
        data_dir: root.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init in-memory db");
    let app = overmind_server::app(state);

    let (_, company) = send(
        &app,
        "POST",
        "/companies",
        Some(json!({ "name": "Exec Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/agents"),
        Some(json!({ "name": "Builder", "archetype": "backend-developer" })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent id").to_string();
    let (_, project) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/projects"),
        Some(json!({ "title": "Demo repo" })),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id").to_string();
    let (status, ws) = send(
        &app,
        "POST",
        &format!("/projects/{project_id}/workspaces"),
        Some(json!({ "name": "main", "cwd": repo.to_string_lossy() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "workspace failed: {ws}");
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/projects/{project_id}/goals"),
        Some(json!({ "title": "Working code" })),
    )
    .await;
    let goal_id = goal["id"].as_str().expect("goal id").to_string();
    let (_, task) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/tasks"),
        Some(json!({ "title": "Add greeting file", "description": "Create hello.txt saying hi.", "goal_id": goal_id })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    TestEnv {
        app,
        root,
        company_id,
        agent_id,
        task_id,
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    // A process-wide counter: two parallel test threads can read the same
    // nanosecond, so the timestamp alone is not unique.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{}-{n}", std::process::id())
}

async fn wait_for_session(app: &axum::Router, session_id: &str) -> Value {
    for _ in 0..100 {
        let (_, session) = send(app, "GET", &format!("/sessions/{session_id}"), None).await;
        let status = session["status"].as_str().unwrap_or("");
        if status == "completed" || status == "failed" {
            return session;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session {session_id} did not finish in time");
}

const HAPPY_STUB: &str = r#"#!/bin/sh
echo "starting work on: $OVERMIND_TASK_TITLE"
echo "hi from the agent" > hello.txt
echo '{"model":"stub-model","total_cost_usd":0.42,"usage":{"input_tokens":1200,"cache_read_input_tokens":300,"output_tokens":400}}'
"#;

#[tokio::test]
async fn agent_completes_task_in_isolated_worktree() {
    let env = setup(HAPPY_STUB).await;

    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/tasks/{}/start", env.task_id),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");
    let session_id = started["session_id"].as_str().expect("session id");
    assert!(
        started["branch"]
            .as_str()
            .expect("branch")
            .starts_with("overmind/task-")
    );

    let session = wait_for_session(&env.app, session_id).await;
    assert_eq!(session["status"], "completed", "session: {session}");
    assert_eq!(session["exit_code"], 0);
    assert!(
        session["output"]
            .as_str()
            .expect("output")
            .contains("starting work on: Add greeting file")
    );
    // Cost captured from the final JSON line (0.42 USD -> 42 cents)
    assert_eq!(session["cost_cents"], 42);

    // The worktree is isolated: the original repo has no hello.txt
    assert!(!env.root.join("repo").join("hello.txt").exists());
    let workspace_path = session["workspace_path"].as_str().expect("workspace path");
    assert!(PathBuf::from(workspace_path).join("hello.txt").exists());

    // The diff shows the change against the base commit
    let (status, diff) = send_text(&env.app, &format!("/sessions/{session_id}/diff")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(diff.contains("hello.txt"), "diff: {diff}");
    assert!(diff.contains("+hi from the agent"), "diff: {diff}");

    // Task landed in review
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "in_review");

    // Every step is in the audit log, and the chain still verifies
    let (_, events) = send(
        &env.app,
        "GET",
        &format!("/audit/events?company_id={}", env.company_id),
        None,
    )
    .await;
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["kind"].as_str().expect("kind"))
        .collect();
    for expected in ["workspace.created", "session.started", "session.finished"] {
        assert!(kinds.contains(&expected), "missing {expected} in {kinds:?}");
    }
    let (_, report) = send(&env.app, "GET", "/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn concurrent_checkouts_exactly_one_wins() {
    let env = setup(HAPPY_STUB).await;
    let uri = format!("/tasks/{}/start", env.task_id);
    let body = json!({ "agent_id": env.agent_id });

    let (a, b) = tokio::join!(
        send(&env.app, "POST", &uri, Some(body.clone())),
        send(&env.app, "POST", &uri, Some(body.clone())),
    );
    let statuses = [a.0, b.0];
    assert!(
        statuses.contains(&StatusCode::ACCEPTED) && statuses.contains(&StatusCode::CONFLICT),
        "expected one 202 and one 409, got {statuses:?} ({} / {})",
        a.1,
        b.1
    );

    // Let the winning session finish so the worktree teardown is orderly.
    let winner = if a.0 == StatusCode::ACCEPTED {
        a.1
    } else {
        b.1
    };
    let session_id = winner["session_id"].as_str().expect("session id");
    wait_for_session(&env.app, session_id).await;
}

const FAILING_STUB: &str = r#"#!/bin/sh
echo "something went badly"
exit 3
"#;

#[tokio::test]
async fn failed_session_blocks_task_with_error() {
    let env = setup(FAILING_STUB).await;

    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/tasks/{}/start", env.task_id),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let session_id = started["session_id"].as_str().expect("session id");

    let session = wait_for_session(&env.app, session_id).await;
    assert_eq!(session["status"], "failed");
    assert_eq!(session["exit_code"], 3);
    assert_eq!(session["last_error"], "agent exited with code 3");
    assert_eq!(session["cost_cents"], 0);

    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "blocked");

    let (_, report) = send(&env.app, "GET", "/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}
