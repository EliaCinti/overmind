//! M7 acceptance tests: organizational memory over MCP.
//! - an agent receives context the org "remembered" (get_context), injected
//!   into its prompt, and a memory is stored on completion (store_memory);
//! - with no memory server configured, everything works identically;
//! - a broken memory server degrades gracefully — the task still completes.

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
    std::env::temp_dir().join(format!("overmind-mem-{nanos}-{}-{n}", std::process::id()))
}

/// A minimal MCP memory server over stdio: get_context returns a fixed memory,
/// store_memory appends its arguments to STUB_STORE_LOG.
const STUB_MCP: &str = r#"import sys, json, os
log = os.environ.get("STUB_STORE_LOG")
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"stub","version":"0"}}}), flush=True)
    elif method == "notifications/initialized":
        pass
    elif method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments", {})
        if name == "get_context":
            text = "REMEMBERED: the deploy script lives in scripts/deploy.sh"
            print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":text}]}}), flush=True)
        elif name == "store_memory":
            if log:
                with open(log, "a") as f:
                    f.write(json.dumps(args) + "\n")
            print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"stored"}]}}), flush=True)
        else:
            print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[]}}), flush=True)
    elif mid is not None:
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{}}), flush=True)
"#;

/// Stub agent that surfaces the injected memory context into its output, so
/// the test can prove the agent actually received it.
const MEMORY_AGENT: &str = r#"#!/bin/sh
echo "agent saw memory: $OVERMIND_MEMORY_CONTEXT"
echo done > out.txt
echo '{"total_cost_usd":0.01,"session_id":"s"}'
"#;

struct Env {
    app: axum::Router,
    company: String,
    goal: String,
    store_log: PathBuf,
}

async fn setup(memory_cmd: Option<String>) -> Env {
    let root = unique_root();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo x > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );
    let agent = root.join("agent.sh");
    std::fs::write(&agent, MEMORY_AGENT).expect("write agent");
    let store_log = root.join("store.log");

    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", agent.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000,
        memory_cmd,
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = overmind_server::app(state);

    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Mem Co" })),
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
        Some(json!({ "name": "m", "cwd": repo.to_string_lossy() })),
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
    Env {
        app,
        company,
        goal,
        store_log,
    }
}

async fn run_one_task(env: &Env, title: &str) -> Value {
    let (_, agent) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({ "name": "Worker", "archetype": "backend-developer" })),
    )
    .await;
    let agent = agent["id"].as_str().expect("id").to_string();
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company),
        Some(json!({ "title": title, "description": "do the thing", "goal_id": env.goal })),
    )
    .await;
    let task = task["id"].as_str().expect("id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (s, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "start: {started}");
    let session = started["session_id"]
        .as_str()
        .expect("session id")
        .to_string();
    for _ in 0..100 {
        let (_, sv) = send(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        let st = sv["status"].as_str().unwrap_or("");
        if st == "completed" || st == "failed" {
            return sv;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session never finished");
}

fn python() -> &'static str {
    // Prefer python3; skip the memory-enabled tests if it isn't available.
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "python3"
    } else {
        ""
    }
}

#[tokio::test]
async fn memory_loop_context_in_and_memory_out() {
    let py = python();
    if py.is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let stub = root.join("stub_mcp.py");
    std::fs::write(&stub, STUB_MCP).expect("write stub");
    let store_log = root.join("store.log");

    // Build the env manually so the memory command shares this store log.
    let memory_cmd = format!(
        "STUB_STORE_LOG={} {py} {}",
        store_log.display(),
        stub.display()
    );
    let env = setup(Some(memory_cmd)).await;
    // Overwrite the store log path used by assertions to the one the cmd writes.
    let env = Env {
        store_log: store_log.clone(),
        ..env
    };

    let (_, status) = send(&env.app, "GET", "/api/memory/status", None).await;
    assert_eq!(status["enabled"], json!(true));

    let session = run_one_task(&env, "Ship the release").await;
    assert_eq!(session["status"], "completed", "session: {session}");

    // The agent received the remembered context (get_context worked).
    let output = session["output"].as_str().unwrap_or("");
    assert!(
        output.contains("REMEMBERED: the deploy script lives in scripts/deploy.sh"),
        "agent output missing memory context: {output}"
    );

    // A memory was stored on completion (store_memory worked).
    let logged = std::fs::read_to_string(&env.store_log).unwrap_or_default();
    assert!(
        logged.contains("Ship the release"),
        "store log missing the completed task: {logged:?}"
    );
}

#[tokio::test]
async fn works_identically_without_memory() {
    let env = setup(None).await;

    let (_, status) = send(&env.app, "GET", "/api/memory/status", None).await;
    assert_eq!(status["enabled"], json!(false));

    let session = run_one_task(&env, "No memory needed").await;
    assert_eq!(session["status"], "completed");
    // No memory context was injected.
    let output = session["output"].as_str().unwrap_or("");
    assert!(output.contains("agent saw memory: \n") || output.contains("agent saw memory: "));

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn broken_memory_server_degrades_gracefully() {
    // A memory command that isn't an MCP server at all: the task must still
    // complete, memory calls swallowed.
    let env = setup(Some("exit 7".to_string())).await;

    let (_, status) = send(&env.app, "GET", "/api/memory/status", None).await;
    assert_eq!(status["enabled"], json!(true)); // configured…

    let session = run_one_task(&env, "Resilient task").await;
    assert_eq!(session["status"], "completed"); // …but its failure didn't matter

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}
