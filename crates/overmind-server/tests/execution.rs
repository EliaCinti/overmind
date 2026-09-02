//! M2 acceptance tests: a real (stub) agent completes a task in an isolated
//! worktree, the diff is visible, every step is audited, cost is recorded,
//! and concurrent checkouts of the same task can't double-run.

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
    /// The CEO the company is founded with (M15) — the org leader.
    ceo_id: String,
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
    let app = common::claimed(overmind_server::app(state), &root.join("data")).await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Exec Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let ceo_id = company["ceo"]["id"]
        .as_str()
        .expect("every company is founded with a CEO")
        .to_string();
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents"),
        Some(json!({ "name": "Builder", "archetype": "builder" })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent id").to_string();
    let (_, project) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/projects"),
        Some(json!({ "title": "Demo repo" })),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id").to_string();
    let (status, ws) = send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/workspaces"),
        Some(json!({ "name": "main", "cwd": repo.to_string_lossy() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "workspace failed: {ws}");
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/goals"),
        Some(json!({ "title": "Working code" })),
    )
    .await;
    let goal_id = goal["id"].as_str().expect("goal id").to_string();
    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({ "title": "Add greeting file", "description": "Create hello.txt saying hi.", "goal_id": goal_id })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    TestEnv {
        app,
        root,
        company_id,
        ceo_id,
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
        let (_, session) = send(app, "GET", &format!("/api/sessions/{session_id}"), None).await;
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
        &format!("/api/tasks/{}/start", env.task_id),
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
    let (status, diff) = send_text(&env.app, &format!("/api/sessions/{session_id}/diff")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(diff.contains("hello.txt"), "diff: {diff}");
    assert!(diff.contains("+hi from the agent"), "diff: {diff}");

    // Task landed in review
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "in_review");

    // Every step is in the audit log, and the chain still verifies
    let (_, events) = send(
        &env.app,
        "GET",
        &format!("/api/audit/events?company_id={}", env.company_id),
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
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn concurrent_checkouts_exactly_one_wins() {
    let env = setup(HAPPY_STUB).await;
    let uri = format!("/api/tasks/{}/start", env.task_id);
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
        &format!("/api/tasks/{}/start", env.task_id),
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
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    assert_eq!(tasks["tasks"][0]["status"], "blocked");

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

// M11 / ADR-0017: a knowledge run produces a document artifact, not a diff.
const KNOWLEDGE_STUB: &str = r#"#!/bin/sh
echo "researching: $OVERMIND_TASK_TITLE"
printf '# Avengers - best editions\n\n4K UHD, Dolby Atmos.\n' > ARTIFACT.md
echo '{"model":"stub-model","total_cost_usd":0.10,"usage":{"input_tokens":100,"output_tokens":50}}'
"#;

#[tokio::test]
async fn knowledge_task_produces_document_artifact() {
    let env = setup(KNOWLEDGE_STUB).await;

    // A knowledge task needs neither a goal nor a git workspace (ADR-0017).
    let (status, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({
            "title": "Research Avengers editions",
            "description": "Find the best 4K release.",
            "execution_kind": "knowledge"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {task}");
    assert_eq!(task["execution_kind"], "knowledge");
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");
    let session_id = started["session_id"].as_str().expect("session id");
    // Knowledge runs have no git branch.
    assert_eq!(started["branch"], "");

    let session = wait_for_session(&env.app, session_id).await;
    assert_eq!(session["status"], "completed", "session: {session}");

    // The deliverable is an artifact, not a diff.
    let (status, artifacts) = send(
        &env.app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let doc = artifacts["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .find(|a| a["title"] == "ARTIFACT.md")
        .expect("ARTIFACT.md artifact");
    assert!(
        doc["content"].as_str().expect("content").contains("4K UHD"),
        "artifact: {doc}"
    );

    // Task landed in review; the artifact is audited and the chain still verifies.
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let ours = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == task_id.as_str())
        .expect("our task");
    assert_eq!(ours["status"], "in_review");
    assert_eq!(ours["execution_kind"], "knowledge");

    let (_, events) = send(
        &env.app,
        "GET",
        &format!("/api/audit/events?company_id={}", env.company_id),
        None,
    )
    .await;
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["kind"].as_str().expect("kind"))
        .collect();
    assert!(
        kinds.contains(&"artifact.created"),
        "missing artifact.created in {kinds:?}"
    );
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

/// Exits cleanly, costs money, and writes nothing — the exact shape a real
/// agent takes when it is not permitted to write. Measured in the container on
/// 2026-08-15, where the cage does not reach and the CLI therefore denies every
/// Write in headless mode.
const MUTE_KNOWLEDGE_STUB: &str = r#"#!/bin/sh
echo "thinking about it"
echo '{"model":"stub-model","total_cost_usd":0.10,"usage":{"input_tokens":100,"output_tokens":50}}'
"#;

/// A knowledge run that wrote no file delivered nothing, and must not say
/// otherwise.
///
/// This shipped as a success: session `completed`, task `in_review`, and the
/// `Run output` fallback — the adapter's own transcript — offered to a person
/// as the thing to review. `ttft_ms` and `permission_denials` where a document
/// belongs. The fallback is still written, because an empty panel says less
/// than a transcript does; what it may no longer do is stand in for a
/// deliverable.
#[tokio::test]
async fn a_knowledge_run_that_wrote_nothing_is_not_ready_for_review() {
    let env = setup(MUTE_KNOWLEDGE_STUB).await;

    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({
            "title": "Write something down",
            "description": "Anything, in a file.",
            "execution_kind": "knowledge"
        })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");

    let session = wait_for_session(
        &env.app,
        started["session_id"].as_str().expect("session id"),
    )
    .await;
    assert_eq!(
        session["status"], "failed",
        "a run that delivered nothing did not succeed: {session}"
    );
    assert!(
        session["last_error"]
            .as_str()
            .unwrap_or_default()
            .contains("no file"),
        "and it must say what went wrong: {session}"
    );
    // The exit code stays truthful: the adapter did exit 0. What failed is the
    // run, not the process.
    assert_eq!(session["exit_code"], json!(0), "{session}");

    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let ours = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == task_id.as_str())
        .expect("our task");
    assert_eq!(
        ours["status"], "blocked",
        "nothing was produced, so there is nothing in review: {ours}"
    );

    // The transcript is still in the drawer — it is the only account of what
    // happened, and a person looking for the reason should find it there.
    let (_, artifacts) = send(
        &env.app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    let items = artifacts["artifacts"].as_array().expect("artifacts");
    assert!(
        items.iter().any(|a| a["title"] == "Run output"),
        "the fallback survives: {artifacts}"
    );
    assert!(
        items.iter().all(|a| a["relative_path"].is_null()),
        "and it is not dressed up as a file the agent wrote: {artifacts}"
    );

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

// M12 / ADR-0018: talking to the CEO opens a task. The stub CEO returns a plan.
const CEO_STUB: &str = r#"#!/bin/sh
echo "thinking..."
echo '{"reply":"On it - I will research that.","tasks":[{"title":"Research Avengers 4K editions","description":"Best release per film.","execution_kind":"knowledge"}]}'
"#;

#[tokio::test]
async fn ceo_replies_and_opens_a_task() {
    let env = setup(CEO_STUB).await;

    let (status, posted) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.agent_id
        ),
        Some(json!({
            "content": "Find the best 4K Avengers editions."
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "post failed: {posted}");

    // The CEO's turn runs in the background; poll until it has replied.
    let mut convo = json!({});
    for _ in 0..100 {
        let (_, c) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company_id, env.agent_id
            ),
            None,
        )
        .await;
        let replied = c["messages"]
            .as_array()
            .map(|m| m.iter().any(|x| x["role"] == "ceo"))
            .unwrap_or(false);
        if replied {
            convo = c;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let messages = convo["messages"].as_array().expect("messages");
    assert!(
        messages.iter().any(|m| m["role"] == "user"),
        "no user message"
    );
    let ceo = messages
        .iter()
        .find(|m| m["role"] == "ceo")
        .expect("ceo reply");
    assert!(
        ceo["content"].as_str().unwrap_or("").contains("research"),
        "ceo: {ceo}"
    );

    // The CEO opened a knowledge task, visible on the board and dispatchable.
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let opened = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["title"] == "Research Avengers 4K editions")
        .expect("the CEO's task");
    assert_eq!(opened["status"], "todo");
    assert_eq!(opened["execution_kind"], "knowledge");

    // Every step audited; the chain still verifies.
    let (_, events) = send(
        &env.app,
        "GET",
        &format!("/api/audit/events?company_id={}", env.company_id),
        None,
    )
    .await;
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["kind"].as_str().expect("kind"))
        .collect();
    for expected in ["conversation.created", "message.posted", "task.created"] {
        assert!(kinds.contains(&expected), "missing {expected} in {kinds:?}");
    }
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

// M12 / ADR-0018: an uploaded file reaches the CEO agent's working directory.
// This stub reports the files it can see, so the test can prove the file arrived.
const CEO_ATTACH_STUB: &str = r#"#!/bin/sh
printf '{"reply":"files present: %s","tasks":[]}\n' "$(ls)"
"#;

/// POST a multipart upload (agent_id field + a file part) to the attachments endpoint.
async fn upload_file(
    app: &axum::Router,
    company_id: &str,
    agent_id: &str,
    filename: &str,
    mime: &str,
    content: &[u8],
) -> (StatusCode, Value) {
    let boundary = "OMTESTBOUNDARY";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/companies/{company_id}/agents/{agent_id}/conversation/attachments"
        ))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
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

#[tokio::test]
async fn ceo_sees_an_attachment() {
    let env = setup(CEO_ATTACH_STUB).await;

    // Upload a file to the CEO thread.
    let (status, att) = upload_file(
        &env.app,
        &env.company_id,
        &env.agent_id,
        "room.txt",
        "text/plain",
        b"a cozy living room",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "upload failed: {att}");
    let attachment_id = att["id"].as_str().expect("attachment id").to_string();
    assert_eq!(att["filename"], "room.txt");

    // Post a message that references the attachment.
    let (status, _) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.agent_id
        ),
        Some(json!({
            "content": "What do you think of this room?",
            "attachment_ids": [attachment_id],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // Poll for the CEO's reply; the stub lists the files in its working dir.
    let mut ceo_reply = String::new();
    let mut convo = json!({});
    for _ in 0..100 {
        let (_, c) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company_id, env.agent_id
            ),
            None,
        )
        .await;
        if let Some(m) = c["messages"]
            .as_array()
            .and_then(|m| m.iter().find(|x| x["role"] == "ceo"))
        {
            ceo_reply = m["content"].as_str().unwrap_or("").to_string();
            convo = c;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The file reached the agent's working directory.
    assert!(
        ceo_reply.contains("room.txt"),
        "the attachment didn't reach the agent — reply: {ceo_reply:?}"
    );

    // The attachment is shown on the user's message.
    let user = convo["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|m| m["role"] == "user")
        .expect("user message");
    assert_eq!(user["attachments"][0]["filename"], "room.txt");

    // And it's downloadable, with the exact bytes we uploaded.
    let (status, body) = send_text(
        &env.app,
        &format!(
            "/api/companies/{}/conversation/attachments/{}",
            env.company_id, attachment_id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "a cozy living room");

    // Audited (attachment.added), and the chain still verifies.
    let (_, events) = send(
        &env.app,
        "GET",
        &format!("/api/audit/events?company_id={}", env.company_id),
        None,
    )
    .await;
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or(""))
        .collect();
    assert!(
        kinds.contains(&"attachment.added"),
        "missing attachment.added in {kinds:?}"
    );
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

// ADR-0019: talking to a single agent ripples onto the team — a specialist
// assigns a task to a teammate and escalates to the CEO. The stub returns that plan.
const SPECIALIST_STUB: &str = r#"#!/bin/sh
echo '{"reply":"On it - I will bring in Guard.","tasks":[{"title":"Harden the login flow","description":"Review auth.","execution_kind":"code","assignee":"Guard"}],"escalate":"This affects the whole app; the CEO should know."}'
"#;

#[tokio::test]
async fn agent_conversation_ripples_to_teammates() {
    // env.agent_id ("Builder") reports to nobody, so it is the org leader (the CEO).
    let env = setup(SPECIALIST_STUB).await;

    // A teammate to assign to, and the specialist we'll talk to — both report to the leader.
    let (_, guard) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(json!({ "name": "Guard", "archetype": "builder", "reports_to": env.agent_id })),
    )
    .await;
    let guard_id = guard["id"].as_str().expect("guard id").to_string();
    let (_, iris) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(json!({ "name": "Iris", "archetype": "builder", "reports_to": env.agent_id })),
    )
    .await;
    let iris_id = iris["id"].as_str().expect("iris id").to_string();

    // Talk to Iris directly (a specialist, not the leader).
    let (status, _) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, iris_id
        ),
        Some(json!({ "content": "Can you secure the login?" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // Wait for Iris to reply.
    for _ in 0..100 {
        let (_, c) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company_id, iris_id
            ),
            None,
        )
        .await;
        if c["messages"]
            .as_array()
            .map(|m| m.iter().any(|x| x["role"] == "ceo"))
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The ripple: a task was created and ASSIGNED to Guard (a different agent).
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let task = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["title"] == "Harden the login flow")
        .expect("the assigned task");
    assert_eq!(
        task["assignee_agent_id"],
        json!(guard_id),
        "task not assigned to the teammate: {task}"
    );

    // The escalation reached the org leader's thread. Since M15 that leader is
    // the founding CEO, not Builder — Builder itself reports to it.
    let mut escalated = false;
    for _ in 0..60 {
        let (_, c) = send(
            &env.app,
            "GET",
            &format!(
                "/api/companies/{}/agents/{}/conversation",
                env.company_id, env.ceo_id
            ),
            None,
        )
        .await;
        if c["messages"]
            .as_array()
            .map(|m| {
                m.iter().any(|x| {
                    // Since M10 slice 4 this arrives as `escalation`, not
                    // `system`: an agent's words must not wear Overmind's own
                    // voice, in the thread or in the leader's next prompt.
                    x["role"] == "escalation"
                        && x["content"].as_str().unwrap_or("").contains("From Iris")
                })
            })
            .unwrap_or(false)
        {
            escalated = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(escalated, "no escalation reached the leader's thread");

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

// A plan with a `code` task and nothing else — the shape the smoke run produced
// when the CEO was asked to fix a bug in a connected repo.
const OPENS_CODE_WORK_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"You are working on the task"*)
    echo "fixing it" > fix.txt
    echo '{"model":"stub-model","total_cost_usd":0.10,"usage":{"input_tokens":10,"output_tokens":10}}' ;;
  *)
    echo '{"reply":"I opened it.","tasks":[{"title":"Fix add() in calc.py","description":"It subtracts.","execution_kind":"code"}]}' ;;
esac
"#;

/// The seam the live smoke run fell through, and the reason it is worth having
/// a smoke run at all.
///
/// Two families of test already existed and both passed: one watched an agent
/// open a task and stopped at "the row is there", the other ran a `code` task
/// that the *test* had created by hand, with a goal. Nothing crossed them — so
/// `ceo.rs` bound `goal_id` to NULL, every `code` task an agent opened was born
/// unrunnable, and the first person to find out was a human clicking Start.
#[tokio::test]
async fn a_code_task_an_agent_opens_can_actually_be_started() {
    let env = setup(OPENS_CODE_WORK_STUB).await;

    let (status, _) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.agent_id
        ),
        Some(json!({ "content": "add() is wrong, please fix it" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let mut opened = Value::Null;
    for _ in 0..100 {
        let (_, tasks) = send(
            &env.app,
            "GET",
            &format!("/api/companies/{}/tasks", env.company_id),
            None,
        )
        .await;
        if let Some(t) = tasks["tasks"]
            .as_array()
            .and_then(|ts| ts.iter().find(|t| t["title"] == "Fix add() in calc.py"))
        {
            opened = t.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!opened.is_null(), "the agent never opened the task");
    let opened_id = opened["id"].as_str().expect("task id").to_string();

    // Filed where your own task is filed. `setup` created one by hand with a
    // goal, and the two should be indistinguishable afterwards — an agent's
    // task is not a second-class row.
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let by_hand = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == json!(env.task_id))
        .expect("the task setup created");
    assert_eq!(
        opened["goal_id"], by_hand["goal_id"],
        "the agent's task was not filed where yours is: {opened}"
    );

    // And then the behavioural half: it starts.
    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{opened_id}/start"),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the agent's own task would not start: {started}"
    );
    let session =
        wait_for_session(&env.app, started["session_id"].as_str().expect("session")).await;
    assert_eq!(session["status"], "completed", "session: {session}");
}

/// The other half: tasks that are already orphaned. `POST /tasks` has always
/// accepted a `code` task with no `goal_id`, so these exist in the wild and no
/// creation-time fix reaches them. When the company has exactly one repository
/// there is nothing to decide.
#[tokio::test]
async fn an_orphaned_code_task_runs_when_there_is_only_one_repository() {
    let env = setup(HAPPY_STUB).await;

    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({ "title": "Orphan work", "description": "No goal at all." })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    assert_eq!(
        task["goal_id"],
        Value::Null,
        "meant to be an orphan: {task}"
    );
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;

    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");
    let session =
        wait_for_session(&env.app, started["session_id"].as_str().expect("session")).await;
    assert_eq!(session["status"], "completed", "session: {session}");
}

/// …and when there is more than one, guessing would run an agent against the
/// wrong codebase. That is a decision, so it goes back to the human — at both
/// ends: the task is not quietly attached when it is opened, and it is refused
/// rather than coin-flipped when it is started.
#[tokio::test]
async fn overmind_will_not_guess_which_repository_an_agent_works_in() {
    let env = setup(OPENS_CODE_WORK_STUB).await;

    let second = env.root.join("other-repo");
    std::fs::create_dir_all(&second).expect("create second repo");
    sh(&second, "git init -q -b main");
    sh(
        &second,
        "echo '# Other' > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );
    let (_, project) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/projects", env.company_id),
        Some(json!({ "title": "Other repo" })),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id").to_string();
    let (status, ws) = send(
        &env.app,
        "POST",
        &format!("/api/projects/{project_id}/workspaces"),
        Some(json!({ "name": "main", "cwd": second.to_string_lossy() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "workspace failed: {ws}");

    // Now let the agent open a `code` task, with two repositories in play.
    let (status, _) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.agent_id
        ),
        Some(json!({ "content": "add() is wrong, please fix it" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let mut opened = Value::Null;
    for _ in 0..100 {
        let (_, tasks) = send(
            &env.app,
            "GET",
            &format!("/api/companies/{}/tasks", env.company_id),
            None,
        )
        .await;
        if let Some(t) = tasks["tasks"]
            .as_array()
            .and_then(|ts| ts.iter().find(|t| t["title"] == "Fix add() in calc.py"))
        {
            opened = t.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!opened.is_null(), "the agent never opened the task");
    assert_eq!(
        opened["goal_id"],
        Value::Null,
        "attaching this would have picked a codebase nobody chose: {opened}"
    );

    let task_id = opened["id"].as_str().expect("task id").to_string();
    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": env.agent_id })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "two repositories is not a coin flip: {body}"
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("more than one repository"),
        "and the refusal has to say what to do about it: {body}"
    );
}
