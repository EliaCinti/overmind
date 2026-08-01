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
    let app = overmind_server::app(state);

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
        Some(json!({ "name": "Builder", "archetype": "backend-developer" })),
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
        Some(json!({ "name": "Guard", "archetype": "backend-developer", "reports_to": env.agent_id })),
    )
    .await;
    let guard_id = guard["id"].as_str().expect("guard id").to_string();
    let (_, iris) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(
            json!({ "name": "Iris", "archetype": "backend-developer", "reports_to": env.agent_id }),
        ),
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
                    x["role"] == "system"
                        && x["content"]
                            .as_str()
                            .unwrap_or("")
                            .contains("Escalation from Iris")
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
