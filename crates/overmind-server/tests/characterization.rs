//! M14 acceptance tests (ADR-0021): characterization on two axes.
//!
//! The milestone's criterion is one sentence — *a "Media & A/V quality" agent,
//! hired **without free text**, uses a declared web-research capability and
//! returns a structured result*. Until slice 3 that agent could only be made
//! by writing prose: a `researcher` with a job title typed over it, focus areas
//! typed in by hand, and a custom brief explaining what it was for. Everything
//! that made it a media agent was free text, which UX.md calls a catalog bug.
//!
//! Here it is two clicks: the function `reviewer`, the field `media-av`.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// The stub hands back what the agent was actually given: the prompt it was
/// told, the model it was launched on, and a structured deliverable. All three
/// were unverifiable before ADR-0021 — the model in particular reached nothing.
const REPORTING_STUB: &str = r#"#!/bin/sh
printf '%s' "$OVERMIND_TASK_PROMPT" > PROMPT.md
printf '%s' "$OVERMIND_AGENT_MODEL" > MODEL.txt
printf '{"verdict":"pass","measured_nits":312}' > findings.json
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

async fn upload(
    app: &axum::Router,
    uri: &str,
    filename: &str,
    bytes: &[u8],
) -> (StatusCode, Value) {
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
}

async fn setup() -> TestEnv {
    let root = std::env::temp_dir().join(format!(
        "overmind-charact-{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    let script_path = root.join("stub-agent.sh");
    std::fs::write(&script_path, REPORTING_STUB).expect("write stub script");

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
        Some(json!({ "name": "Sala Grande" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    TestEnv { app, company_id }
}

/// Create a knowledge task, optionally attach a file, hand it to `agent_id`,
/// and return `(start status, body, task id)`. Does not wait — a refused
/// checkout never produces a session to wait on.
async fn start_knowledge_task(
    env: &TestEnv,
    agent_id: &str,
    title: &str,
    attachment: Option<(&str, &[u8])>,
) -> (StatusCode, Value, String) {
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({
            "title": title,
            "description": "Judge it.",
            "execution_kind": "knowledge",
        })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    if let Some((filename, bytes)) = attachment {
        let (status, up) = upload(
            &env.app,
            &format!("/api/tasks/{task_id}/attachments"),
            filename,
            bytes,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "upload: {up}");
    }
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
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    (status, started, task_id)
}

async fn await_session(app: &axum::Router, started: &Value) {
    let session_id = started["session_id"].as_str().expect("session id");
    for _ in 0..100 {
        let (_, s) = send(app, "GET", &format!("/api/sessions/{session_id}"), None).await;
        match s["status"].as_str().unwrap_or("") {
            "completed" | "failed" => return,
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("session {session_id} did not finish in time");
}

async fn artifact(app: &axum::Router, task_id: &str, title: &str) -> String {
    let (_, artifacts) = send(app, "GET", &format!("/api/tasks/{task_id}/artifacts"), None).await;
    artifacts["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .find(|a| a["title"] == json!(title))
        .and_then(|a| a["content"].as_str())
        .unwrap_or_else(|| panic!("no `{title}` artifact: {artifacts}"))
        .to_string()
}

/// The M14 acceptance criterion, end to end.
#[tokio::test]
async fn a_media_and_av_agent_is_hired_without_writing_a_word() {
    let env = setup().await;

    // Two clicks. No custom_brief, no traits patch, no typed focus areas — the
    // only prose is the job title a human wants to read on the org chart.
    let (status, nova) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(json!({
            "name": "Nova",
            "archetype": "reviewer",
            "domain": "media-av",
            "title": "Media & A/V quality",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "hire failed: {nova}");
    let nova_id = nova["id"].as_str().expect("agent id").to_string();
    assert!(
        nova["custom_brief"].is_null(),
        "the criterion is 'without free text': {nova}"
    );

    let traits = &nova["traits"];
    let has = |perm: &str| {
        traits["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .any(|p| p == perm)
    };
    // The declared web-research capability the criterion names. It comes from
    // the *field*, not from the function: a reviewer of code has no business
    // browsing, a reviewer of projectors does.
    assert!(has("web:read"), "the field grants web research: {traits}");
    // …and the field cannot widen what the server enforces (ADR-0021): the
    // execution kinds still come from the function alone.
    assert!(has("task:knowledge"), "{traits}");
    assert!(
        has("task:code"),
        "the reviewer function reads code: {traits}"
    );

    let focus: Vec<&str> = traits["focus_areas"]
        .as_array()
        .expect("focus areas")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        focus.contains(&"correctness"),
        "the function's own focus survives: {focus:?}"
    );
    assert!(
        focus.contains(&"picture-quality") && focus.contains(&"room-acoustics"),
        "the field adds its own, unwritten by hand: {focus:?}"
    );

    // Judging pictures and sound means looking at things.
    assert_eq!(traits["multimodal"], json!(true), "{traits}");
    // And it runs on a model we actually ship — this was a decorative field.
    assert!(
        overmind_server::model::is_known(traits["model"].as_str().unwrap_or_default()),
        "model must be a real identifier: {traits}"
    );

    // It does the work and hands back something structured (ADR-0017).
    let (status, started, task_id) =
        start_knowledge_task(&env, &nova_id, "Grade the projector", None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");
    await_session(&env.app, &started).await;

    let findings = artifact(&env.app, &task_id, "findings.json").await;
    let findings: Value = serde_json::from_str(&findings).expect("a structured result");
    assert_eq!(findings["verdict"], json!("pass"), "{findings}");

    // The agent was told which field it stands in — without anyone typing it.
    let prompt = artifact(&env.app, &task_id, "PROMPT.md").await;
    assert!(
        prompt.contains("Media & A/V quality"),
        "it works in role: {prompt}"
    );
    assert!(
        prompt.contains("picture and sound"),
        "the field speaks for itself in the prompt: {prompt}"
    );
    // …and which company it works at (M21): with an empty brain and no name
    // in the prompt, "the company" is left to world knowledge, which is how
    // M19's acceptance run wrote about somebody else's product.
    assert!(
        prompt.contains("at «Sala Grande», an AI company"),
        "the prompt never names the company: {prompt}"
    );

    // And the model reached the adapter, which is what makes any of the above
    // more than a value in a database.
    let model = artifact(&env.app, &task_id, "MODEL.txt").await;
    assert_eq!(
        model,
        traits["model"].as_str().unwrap_or_default(),
        "{model}"
    );
}

/// The multimodal gate. Same shape as the capability gate, and the same
/// honesty about what it is: a refusal to hand an agent work it was never
/// characterized for, not a claim about what the spawned CLI can open.
#[tokio::test]
async fn an_agent_is_refused_material_it_was_not_characterized_to_look_at() {
    let env = setup().await;
    // A PNG header is enough: the content type comes from the extension (M17).
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";

    let hire = |name: &'static str, archetype: &'static str, domain: &'static str| {
        let app = env.app.clone();
        let company_id = env.company_id.clone();
        async move {
            let (status, agent) = send(
                &app,
                "POST",
                &format!("/api/companies/{company_id}/agents"),
                Some(json!({ "name": name, "archetype": archetype, "domain": domain })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "hire {name}: {agent}");
            agent
        }
    };

    let bruno = hire("Bruno", "researcher", "finance").await;
    assert_eq!(
        bruno["traits"]["multimodal"],
        json!(false),
        "a finance researcher is not hired to look at things: {bruno}"
    );
    let nova = hire("Nova", "reviewer", "media-av").await;
    assert_eq!(nova["traits"]["multimodal"], json!(true), "{nova}");

    // Bruno is refused at checkout, before anything is spawned.
    let (status, body, task_id) = start_knowledge_task(
        &env,
        bruno["id"].as_str().expect("id"),
        "Is this projector any good",
        Some(("screenshot.png", PNG)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "should be refused: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("screenshot.png"),
        "the refusal names what it cannot look at: {body}"
    );

    // A refused checkout consumes nothing: the task is still waiting.
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let still = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == json!(task_id))
        .expect("the task");
    assert_eq!(still["status"], json!("todo"), "{still}");

    // Nova takes the very same task.
    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": nova["id"].as_str().expect("id") })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    await_session(&env.app, &started).await;

    // A text attachment is not material to look at, so it gates nothing —
    // otherwise every non-multimodal agent would lose the M17 inputs feature.
    let (status, started, _) = start_knowledge_task(
        &env,
        bruno["id"].as_str().expect("id"),
        "Check these numbers",
        Some(("quarter.csv", b"target\n1200\n")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    await_session(&env.app, &started).await;
}

/// A characterization the server cannot honour is refused where it enters, not
/// stored and handed to a prompt later (ADR-0021) — the rule M16 already
/// applies to language codes.
#[tokio::test]
async fn a_model_we_do_not_ship_is_refused_at_the_boundary() {
    let env = setup().await;

    let hire_with = |traits: Value| {
        let app = env.app.clone();
        let company_id = env.company_id.clone();
        async move {
            send(
                &app,
                "POST",
                &format!("/api/companies/{company_id}/agents"),
                Some(json!({ "name": "Ghost", "archetype": "builder", "traits": traits })),
            )
            .await
        }
    };

    // The three strings the hire dialog used to offer were not model ids at all.
    let (status, body) = hire_with(json!({ "model": "claude-sonnet" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("claude-sonnet"),
        "the refusal names what it did not recognise: {body}"
    );

    // A real id is taken.
    let real = overmind_server::model::default_model().id;
    let (status, body) = hire_with(json!({ "model": real })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["traits"]["model"], json!(real));

    // An unknown domain is refused for the same reason an unknown archetype is.
    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(json!({ "name": "Nobody", "archetype": "reviewer", "domain": "astrology" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
