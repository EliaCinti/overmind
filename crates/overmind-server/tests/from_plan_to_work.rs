//! From the CEO's plan to a running task (ADR-0038).
//!
//! Measured on the owner's first real brief (23 Aug 2026): the CEO planned a
//! task, and nothing happened. Three reasons, all ours:
//! - the CEO did not know its teammate held Blender, so it called the work
//!   impossible and wrote a script for the human to paste;
//! - it planned the task as `code`, and a `code` task needs a repository the
//!   company did not have — it could never have started;
//! - a planned task assigned to an agent that *acts with approval* asked
//!   nobody for approval; it sat in `todo` until a human found the button.
//!
//! No test here spawns the real CLI: the adapter is a stub shell script.

use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
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

struct Env {
    app: axum::Router,
    company: String,
    ceo: String,
    root: PathBuf,
}

/// A CEO stub: logs the prompt it was given and answers with `plan`.
fn ceo_stub(log: &std::path::Path, plan: &str) -> String {
    format!(
        "#!/bin/sh\nprintf '%s' \"$OVERMIND_TASK_PROMPT\" >> \"{}\"\necho '{}'\n",
        log.display(),
        plan.replace('\'', "'\"'\"'")
    )
}

async fn setup(plan: &str) -> Env {
    let root = std::env::temp_dir().join(format!(
        "overmind-plan-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&root).expect("mkdir");
    let script = root.join("stub.sh");
    std::fs::write(&script, ceo_stub(&root.join("prompt.log"), plan)).expect("stub");
    let registry = root.join("tools.json");
    std::fs::write(
        &registry,
        json!({
            "mcpServers": { "probe": { "command": "true", "args": [] } },
            "descriptions": { "probe": "a probe that answers nothing" }
        })
        .to_string(),
    )
    .expect("registry");
    let mut config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000,
        ..overmind_server::Config::default()
    };
    config.agent_tools = overmind_server::Config::load_agent_tools(&registry);
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = overmind_server::app(state);
    let (s, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Casa Co" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{co}");
    Env {
        company: co["id"].as_str().expect("id").to_string(),
        ceo: co["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
        root,
    }
}

async fn hire(env: &Env, name: &str, autonomy: &str, tools: Value) -> String {
    let (s, a) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({
            "name": name,
            "archetype": "writer",
            "traits": { "tools": tools, "autonomy": autonomy }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    a["id"].as_str().expect("agent id").to_string()
}

async fn tell_the_ceo(env: &Env, text: &str) {
    let (s, v) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company, env.ceo
        ),
        Some(json!({ "content": text })),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
}

async fn tasks(env: &Env) -> Vec<Value> {
    let (_, v) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company),
        None,
    )
    .await;
    v.as_array()
        .cloned()
        .or_else(|| v["tasks"].as_array().cloned())
        .unwrap_or_default()
}

/// Poll until the CEO's plan produced a task (or give up).
async fn wait_for_a_task(env: &Env) -> Value {
    for _ in 0..150 {
        let ts = tasks(env).await;
        if let Some(t) = ts.into_iter().next() {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the CEO never opened a task");
}

async fn approvals(env: &Env) -> Vec<Value> {
    let (_, v) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/approvals", env.company),
        None,
    )
    .await;
    v["approvals"].as_array().cloned().unwrap_or_default()
}

const PLAN_FOR_TOBIA_AS_CODE: &str = r#"{"reply":"Task opened for Tobia.","tasks":[{"title":"House volumes from the sketch","description":"Build the rooms as boxes.","execution_kind":"code","assignee":"Tobia"}]}"#;
const PLAN_FOR_TOBIA: &str = r#"{"reply":"Task opened for Tobia.","tasks":[{"title":"House volumes from the sketch","description":"Build the rooms as boxes.","execution_kind":"knowledge","assignee":"Tobia"}]}"#;
const NO_PLAN: &str = r#"{"reply":"Noted.","tasks":[]}"#;

/// The CEO is told what each teammate holds, so it plans with the tools the
/// team actually has instead of declaring the work impossible.
#[tokio::test]
async fn the_ceo_knows_what_its_teammates_hold() {
    let env = setup(NO_PLAN).await;
    hire(&env, "Tobia", "act_with_approval", json!(["probe"])).await;
    tell_the_ceo(&env, "Can we build the model?").await;
    let log = env.root.join("prompt.log");
    let mut prompt = String::new();
    for _ in 0..150 {
        prompt = std::fs::read_to_string(&log).unwrap_or_default();
        if !prompt.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let team = prompt
        .split("Your teammates:")
        .nth(1)
        .unwrap_or_default()
        .to_string();
    assert!(
        team.contains("Tobia") && team.contains("probe"),
        "the team block names the tool beside the teammate: {team}"
    );
    assert!(
        team.contains("a probe that answers nothing"),
        "and says what it is: {team}"
    );
}

/// A `code` task needs a repository; a company without one cannot run it, so
/// a plan that says `code` there is opened as `knowledge` — which is what the
/// work was anyway — rather than as a task that can never start.
#[tokio::test]
async fn a_code_task_without_a_repository_is_opened_as_knowledge() {
    let env = setup(PLAN_FOR_TOBIA_AS_CODE).await;
    hire(&env, "Tobia", "propose_only", json!(["probe"])).await;
    tell_the_ceo(&env, "Build the house.").await;
    let task = wait_for_a_task(&env).await;
    assert_eq!(task["execution_kind"], json!("knowledge"), "{task}");
}

/// Acts with approval: the planned task asks you to start it, in the inbox,
/// the moment it is opened — and your approval starts it.
#[tokio::test]
async fn a_task_planned_for_an_agent_that_acts_with_approval_asks_you_to_start_it() {
    let env = setup(PLAN_FOR_TOBIA).await;
    let tobia = hire(&env, "Tobia", "act_with_approval", json!(["probe"])).await;
    tell_the_ceo(&env, "Build the house.").await;
    let task = wait_for_a_task(&env).await;
    assert_eq!(task["status"], json!("todo"), "{task}");
    assert_eq!(task["assignee_agent_id"], json!(tobia));

    let mut pending = Vec::new();
    for _ in 0..100 {
        pending = approvals(&env)
            .await
            .into_iter()
            .filter(|a| a["type"] == "task_start" && a["status"] == "pending")
            .collect();
        if !pending.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(pending.len(), 1, "one start to approve: {pending:?}");

    let id = pending[0]["id"].as_str().expect("id");
    let (s, v) = send(
        &env.app,
        "POST",
        &format!("/api/approvals/{id}/decision"),
        Some(json!({ "decision": "approve" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..100 {
        let t = &tasks(&env).await[0];
        if t["status"] != "todo" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("approved, but the task never left todo");
}

/// Acts within budget: the planned task starts on its own.
#[tokio::test]
async fn a_task_planned_for_an_agent_that_acts_within_budget_starts_at_once() {
    let env = setup(PLAN_FOR_TOBIA).await;
    hire(&env, "Tobia", "act_within_budget", json!(["probe"])).await;
    tell_the_ceo(&env, "Build the house.").await;
    wait_for_a_task(&env).await;
    for _ in 0..150 {
        let t = &tasks(&env).await[0];
        if t["status"] != "todo" {
            assert!(
                approvals(&env)
                    .await
                    .iter()
                    .all(|a| a["type"] != "task_start"),
                "nothing to approve: it just runs"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the task never started");
}

/// Proposes only: the task waits for a human, and nobody is asked.
#[tokio::test]
async fn a_task_planned_for_an_agent_that_only_proposes_waits_for_you() {
    let env = setup(PLAN_FOR_TOBIA).await;
    hire(&env, "Tobia", "propose_only", json!(["probe"])).await;
    tell_the_ceo(&env, "Build the house.").await;
    let task = wait_for_a_task(&env).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(tasks(&env).await[0]["status"], json!("todo"), "{task}");
    assert!(
        approvals(&env)
            .await
            .iter()
            .all(|a| a["type"] != "task_start"),
        "no approval was filed"
    );
}

/// A multipart upload of one file, the shape a browser sends.
async fn upload(app: &axum::Router, uri: &str, filename: &str, bytes: &[u8]) -> Value {
    const BOUNDARY: &str = "----overmindtestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .expect("build upload");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// The files a task's chat carried ride into the task (ADR-0038, measured:
/// the CEO's task told the modeler to read the sketch, and the run directory
/// was empty — the sketch lived on the conversation, the run copies only the
/// task's own attachments). A task born from a conversation lists — and its
/// run receives — that conversation's files.
#[tokio::test]
async fn a_task_born_from_a_chat_carries_the_chats_files() {
    let env = setup(PLAN_FOR_TOBIA).await;
    hire(&env, "Tobia", "propose_only", json!(["probe"])).await;
    let att = upload(
        &env.app,
        &format!(
            "/api/companies/{}/agents/{}/conversation/attachments",
            env.company, env.ceo
        ),
        "BozzaCasa.jpeg",
        b"not really a jpeg",
    )
    .await;
    let att_id = att["id"].as_str().expect("attachment id").to_string();
    let (s, v) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company, env.ceo
        ),
        Some(json!({ "content": "Here is the sketch. Build the house.", "attachment_ids": [att_id] })),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
    let task = wait_for_a_task(&env).await;
    let (_, listed) = send(
        &env.app,
        "GET",
        &format!(
            "/api/tasks/{}/attachments",
            task["id"].as_str().expect("id")
        ),
        None,
    )
    .await;
    let names: Vec<&str> = listed["attachments"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["filename"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        names.contains(&"BozzaCasa.jpeg"),
        "the sketch rides into the task: {listed}"
    );
}
