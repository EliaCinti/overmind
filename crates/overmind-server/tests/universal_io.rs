//! M17 acceptance tests: an agent takes anything you hand it and hands
//! anything back.
//!
//! The two ends are tested separately because they fail separately: a file can
//! reach the agent and its output still be lost, and output can be collected
//! from a run that never saw the input.

mod common;

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
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A multipart upload of one file, the shape a browser sends.
async fn upload(
    app: &axum::Router,
    uri: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> (StatusCode, Value) {
    const BOUNDARY: &str = "----overmindtestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
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
        .expect("body")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// Fetch raw bytes with their content type — what a download actually is.
async fn fetch_raw(app: &axum::Router, uri: &str) -> (StatusCode, String, String, Vec<u8>) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let disposition = response
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, mime, disposition, bytes)
}

async fn setup(stub: &str) -> (axum::Router, String, String) {
    let root = std::env::temp_dir().join(format!("overmind-io-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("root");
    let script = root.join("stub.sh");
    std::fs::write(&script, stub).expect("stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = common::claimed(overmind_server::app(state), &root.join("data")).await;
    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Deliverables Inc" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company").to_string();
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents"),
        Some(json!({ "name": "Rosa", "archetype": "researcher" })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent").to_string();
    (app, company_id, agent_id)
}

/// Poll a task's artifacts until at least `n` exist.
async fn wait_for_artifacts(app: &axum::Router, task_id: &str, n: usize) -> Vec<Value> {
    for _ in 0..120 {
        let (_, list) = send(app, "GET", &format!("/api/tasks/{task_id}/artifacts"), None).await;
        let items = list["artifacts"].as_array().cloned().unwrap_or_default();
        if items.len() >= n {
            return items;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (_, list) = send(app, "GET", &format!("/api/tasks/{task_id}/artifacts"), None).await;
    panic!("never produced {n} artifacts: {list}");
}

/// Reads the file it was handed, writes back a nested tree of four formats —
/// prose, data, a code snippet and a binary — plus the prompt it received, so
/// the test can assert on what the agent was actually told.
const PRODUCES_A_TREE_STUB: &str = r#"#!/bin/sh
mkdir -p research
printf '%s' "$OVERMIND_TASK_PROMPT" > research/PROMPT.txt
cat inputs/brief.csv > research/echoed-input.csv 2>/dev/null || echo "NO INPUT" > research/echoed-input.csv
echo '# Findings' > REPORT.md
printf 'name,value\nalpha,1\n' > research/sources.csv
echo 'def solve(): return 42' > research/solver.py
printf '\211PNG\r\n\032\n binary bytes here' > chart.png
echo '{"total_cost_usd":0.02}'
"#;

#[tokio::test]
async fn a_task_carries_files_in_and_a_whole_tree_out() {
    let (app, company_id, agent_id) = setup(PRODUCES_A_TREE_STUB).await;

    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({
            "title": "Summarise the brief",
            "description": "Read what is attached.",
            "execution_kind": "knowledge",
        })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task").to_string();

    // --- IN: a file is attached to the task, not to a chat.
    let (status, uploaded) = upload(
        &app,
        &format!("/api/tasks/{task_id}/attachments"),
        "brief.csv",
        "application/octet-stream",
        b"quarter,target\nQ3,1200\n",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload: {uploaded}");
    // The extension is trusted over the browser's "I don't know".
    assert_eq!(uploaded["mime"], json!("text/csv"), "{uploaded}");
    assert_eq!(uploaded["size_bytes"], json!(23), "{uploaded}");

    let (_, listed) = send(
        &app,
        "GET",
        &format!("/api/tasks/{task_id}/attachments"),
        None,
    )
    .await;
    assert_eq!(
        listed["attachments"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );

    send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let artifacts = wait_for_artifacts(&app, &task_id, 5).await;
    let by_title = |t: &str| -> Value {
        artifacts
            .iter()
            .find(|a| a["title"] == json!(t))
            .unwrap_or_else(|| panic!("no artifact {t}: {artifacts:#?}"))
            .clone()
    };

    // --- IN, continued: the agent really received the file and the prompt
    // really named it, with its type and size.
    let echoed = by_title("research/echoed-input.csv");
    assert_eq!(
        echoed["content"],
        json!("quarter,target\nQ3,1200\n"),
        "the attachment reached the agent's working directory: {echoed}"
    );
    let prompt = by_title("research/PROMPT.txt");
    let prompt_text = prompt["content"].as_str().unwrap_or("");
    assert!(
        prompt_text.contains("inputs/brief.csv (text/csv, 23 B)"),
        "the prompt names the file, its type and its size: {prompt_text}"
    );

    // --- OUT: a subdirectory survives, and the path is the identity.
    let sources = by_title("research/sources.csv");
    assert_eq!(sources["mime"], json!("text/csv"), "{sources}");
    assert_eq!(sources["relative_path"], json!("research/sources.csv"));

    // --- OUT: a code snippet is typed as code, not as markdown.
    assert_eq!(
        by_title("research/solver.py")["mime"],
        json!("text/x-python")
    );

    // --- OUT: prose stays inline so the drawer can show it without a fetch.
    let report = by_title("REPORT.md");
    assert_eq!(report["mime"], json!("text/markdown"));
    assert_eq!(report["content"], json!("# Findings\n"));

    // --- OUT: a binary is not inlined, but it is downloadable.
    let chart = by_title("chart.png");
    assert_eq!(chart["mime"], json!("image/png"), "{chart}");
    assert_eq!(
        chart["content"],
        Value::Null,
        "binary is not inlined: {chart}"
    );
    assert_eq!(chart["downloadable"], json!(true), "{chart}");

    // --- OUT: and the bytes actually come back, typed, from a durable copy.
    let id = chart["id"].as_str().expect("artifact id");
    let (status, mime, disposition, bytes) =
        fetch_raw(&app, &format!("/api/artifacts/{id}/download")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mime, "image/png");
    assert!(disposition.contains("chart.png"), "{disposition}");
    assert!(
        bytes.starts_with(b"\x89PNG"),
        "the real bytes, not a path: {bytes:?}"
    );

    // --- The input is not mistaken for an output.
    assert!(
        !artifacts
            .iter()
            .any(|a| a["title"].as_str().unwrap_or("").starts_with("inputs/")),
        "what we gave the agent is not something it produced: {artifacts:#?}"
    );
}

/// Answers in chat and leaves a file behind.
const HANDS_A_FILE_BACK_STUB: &str = r#"#!/bin/sh
printf 'week,hours\n1,12\n' > plan.csv
echo '{"reply":"Here is the plan as a spreadsheet.","tasks":[]}'
"#;

#[tokio::test]
async fn an_agent_hands_a_file_back_in_chat() {
    let (app, company_id, agent_id) = setup(HANDS_A_FILE_BACK_STUB).await;

    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents/{agent_id}/conversation/messages"),
        Some(json!({ "content": "Give me the plan as a file." })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // The reply arrives asynchronously, carrying what the agent produced.
    let mut attached = Value::Null;
    for _ in 0..120 {
        let (_, convo) = send(
            &app,
            "GET",
            &format!("/api/companies/{company_id}/agents/{agent_id}/conversation"),
            None,
        )
        .await;
        let found = convo["messages"]
            .as_array()
            .and_then(|ms| ms.iter().find(|m| m["role"] == json!("ceo")))
            .and_then(|m| m["attachments"].as_array())
            .and_then(|a| a.first())
            .cloned();
        if let Some(a) = found {
            attached = a;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_ne!(attached, Value::Null, "the agent's file reached the reply");
    assert_eq!(attached["filename"], json!("plan.csv"), "{attached}");
    assert_eq!(attached["mime"], json!("text/csv"), "{attached}");

    let id = attached["id"].as_str().expect("attachment id");
    let (status, mime, _, bytes) = fetch_raw(
        &app,
        &format!("/api/companies/{company_id}/conversation/attachments/{id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mime, "text/csv");
    assert_eq!(bytes, b"week,hours\n1,12\n");
}

/// Changes a tracked file *and* writes a report next to it. The two must not
/// contaminate each other: the report is not a code change, and the code
/// change is not a document.
const CODE_PLUS_REPORT_STUB: &str = r#"#!/bin/sh
echo 'hello from the agent' > hello.txt
mkdir -p deliverables
echo '# What I changed and why' > deliverables/NOTES.md
echo '{"total_cost_usd":0.01}'
"#;

#[tokio::test]
async fn a_code_run_hands_back_a_diff_and_a_document_without_mixing_them() {
    let root = std::env::temp_dir().join(format!("overmind-io-{}", uuid::Uuid::now_v7().simple()));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    for cmd in [
        "git init -q -b main",
        "echo '# Demo' > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    ] {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&repo)
            .output()
            .expect("git");
        assert!(out.status.success(), "{cmd}: {out:?}");
    }
    let script = root.join("stub.sh");
    std::fs::write(&script, CODE_PLUS_REPORT_STUB).expect("stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = common::claimed(overmind_server::app(state), &root.join("data")).await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Both Ends Ltd" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company").to_string();
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents"),
        Some(json!({ "name": "Builder", "archetype": "builder" })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent").to_string();
    let (_, project) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/projects"),
        Some(json!({ "title": "Demo repo" })),
    )
    .await;
    let project_id = project["id"].as_str().expect("project").to_string();
    send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/workspaces"),
        Some(json!({ "name": "main", "cwd": repo.to_string_lossy() })),
    )
    .await;
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/goals"),
        Some(json!({ "title": "Working code" })),
    )
    .await;
    let goal_id = goal["id"].as_str().expect("goal").to_string();
    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({
            "title": "Add a greeting",
            "description": "And write up what you did.",
            "goal_id": goal_id,
            "execution_kind": "code",
        })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task").to_string();
    send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (_, started) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    let session_id = started["session_id"].as_str().expect("session").to_string();

    // The document comes back as an artifact…
    let artifacts = wait_for_artifacts(&app, &task_id, 1).await;
    assert_eq!(artifacts.len(), 1, "exactly the report: {artifacts:#?}");
    assert_eq!(artifacts[0]["title"], json!("NOTES.md"), "{artifacts:#?}");
    assert_eq!(artifacts[0]["mime"], json!("text/markdown"));

    // …and the code change comes back as a diff, with no trace of the report
    // or the `deliverables/` directory in it.
    // The diff endpoint answers with the patch itself, not JSON around it.
    let (status, _, _, bytes) = fetch_raw(&app, &format!("/api/sessions/{session_id}/diff")).await;
    assert_eq!(status, StatusCode::OK);
    let diff = String::from_utf8_lossy(&bytes);
    assert!(
        diff.contains("hello.txt"),
        "the code change is in the diff: {diff}"
    );
    assert!(
        !diff.contains("NOTES.md") && !diff.contains("deliverables"),
        "the document must not leak into the diff: {diff}"
    );
}

/// A crowded thread still hands files back (measured 27 Aug 2026): every
/// turn's scratch holds a copy of ALL the thread's attachments, and the
/// reply-file cap counted those copies — with 34 files already in the
/// thread, a new file the CEO wrote was never even enumerated. Rune
/// announced `rituale-sala-senza-alcol.md`; the chat never carried it.
#[tokio::test]
async fn a_crowded_thread_still_hands_files_back() {
    let (app, company_id, agent_id) = setup(HANDS_A_FILE_BACK_STUB).await;

    // Crowd the thread: 25 uploads, each a distinct staged-then-posted file.
    let mut ids = Vec::new();
    for i in 0..25 {
        let (_, a) = upload(
            &app,
            &format!("/api/companies/{company_id}/agents/{agent_id}/conversation/attachments"),
            &format!("given-{i:02}.txt"),
            "text/plain",
            b"context",
        )
        .await;
        ids.push(a["id"].as_str().expect("id").to_string());
    }
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents/{agent_id}/conversation/messages"),
        Some(json!({ "content": "Read all that, then give me the plan file.", "attachment_ids": ids })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    for _ in 0..120 {
        let (_, convo) = send(
            &app,
            "GET",
            &format!("/api/companies/{company_id}/agents/{agent_id}/conversation"),
            None,
        )
        .await;
        let produced = convo["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["role"] == json!("ceo"))
            .flat_map(|m| m["attachments"].as_array().cloned().unwrap_or_default())
            .any(|a| a["filename"] == json!("plan.csv"));
        if produced {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the produced file drowned among the thread's copies");
}
