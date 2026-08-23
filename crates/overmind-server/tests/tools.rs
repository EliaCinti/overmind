//! ADR-0036: tools in the agent's hand. Declared by the operator
//! (`OVERMIND_AGENT_TOOLS`), granted per agent as a structured trait, written
//! into the run's own MCP config -- and nothing else is.

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

fn unique_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "overmind-tools-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ))
}

/// An agent that waits at a gate, so the run -- and its MCP config -- is
/// alive while the test looks at it. A path in temp, which the cage grants.
fn gated_agent(gate: &std::path::Path) -> String {
    format!(
        r#"#!/bin/sh
i=0
while [ ! -f "{}" ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done
echo done > out.txt
echo '{{"total_cost_usd":0.01,"session_id":"s"}}'
"#,
        gate.display()
    )
}

struct Env {
    app: axum::Router,
    company: String,
}

/// A server whose operator declared one tool, `probe`, and a stub agent.
async fn setup(registry: Option<Value>, agent_body: &str) -> Env {
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let agent = root.join("agent.sh");
    std::fs::write(&agent, agent_body).expect("write agent");
    let mut config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", agent.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000,
        ..overmind_server::Config::default()
    };
    if let Some(reg) = registry {
        let path = root.join("tools.json");
        std::fs::write(&path, reg.to_string()).expect("write registry");
        config.agent_tools = overmind_server::Config::load_agent_tools(&path);
    }
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = overmind_server::app(state);
    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Tool Co" })),
    )
    .await;
    let company = co["id"].as_str().expect("id").to_string();
    Env { app, company }
}

fn registry() -> Value {
    json!({
        "mcpServers": {
            "probe": { "command": "true", "args": [] }
        },
        "descriptions": { "probe": "a probe that answers nothing" }
    })
}

async fn hire(env: &Env, name: &str, tools: Value) -> (StatusCode, Value) {
    send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({
            "name": name,
            "archetype": "writer",
            "traits": { "tools": tools }
        })),
    )
    .await
}

/// The registry is what the operator declared: listed by name, with the
/// command and the description, so the interface can offer exactly that.
#[tokio::test]
async fn the_registry_is_listed_by_name() {
    let env = setup(Some(registry()), "#!/bin/sh\necho done\n").await;
    let (s, v) = send(&env.app, "GET", "/api/tools", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let tools = v["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 1, "{v}");
    assert_eq!(tools[0]["name"], json!("probe"));
    assert_eq!(tools[0]["command"], json!("true"));
    assert_eq!(
        tools[0]["description"],
        json!("a probe that answers nothing")
    );

    // No registry, no tools -- and no error.
    let bare = setup(None, "#!/bin/sh\necho done\n").await;
    let (s, v) = send(&bare.app, "GET", "/api/tools", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["tools"].as_array().map(Vec::len), Some(0), "{v}");
}

/// A grant names a tool the operator declared, or it is refused at the
/// boundary -- never stored and handed to a run later.
#[tokio::test]
async fn granting_an_unknown_tool_is_refused() {
    let env = setup(Some(registry()), "#!/bin/sh\necho done\n").await;
    let (s, v) = hire(&env, "Wrong", json!(["blender"])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"].as_str().unwrap_or("").contains("blender"),
        "the refusal names the tool: {v}"
    );
    let (s, v) = hire(&env, "Right", json!(["probe"])).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["traits"]["tools"], json!(["probe"]));
}

/// The whole point: a granted tool rides in the run's own MCP config -- the
/// file the CLI is pointed at with --strict-mcp-config -- and an agent
/// without the grant gets no such file at all (memory is off here, so there
/// is nothing else to write).
#[tokio::test]
async fn a_granted_tool_rides_in_the_runs_mcp_config() {
    let gate = std::env::temp_dir().join(format!(
        "overmind-tools-gate-{}",
        uuid::Uuid::now_v7().simple()
    ));
    let env = setup(Some(registry()), &gated_agent(&gate)).await;
    let (s, a) = hire(&env, "Modeler", json!(["probe"])).await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    let agent = a["id"].as_str().expect("agent id").to_string();

    let (_, t) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company),
        Some(json!({ "title": "Model the sala", "execution_kind": "knowledge" })),
    )
    .await;
    let task = t["id"].as_str().expect("task id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (s, v) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let session = v["session_id"].as_str().expect("session").to_string();

    // The run is held at the gate: its MCP config exists right now.
    let path = std::env::temp_dir().join(format!("overmind-mcp-{session}.json"));
    let mut cfg = None;
    for _ in 0..60 {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(v) = serde_json::from_str::<Value>(&text)
        {
            cfg = Some(v);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    std::fs::write(&gate, b"go").expect("release the agent");
    let cfg = cfg.expect("the run's MCP config was written");
    assert_eq!(
        cfg["mcpServers"]["probe"]["command"],
        json!("true"),
        "{cfg}"
    );
    assert!(
        cfg.get("descriptions").is_none(),
        "descriptions are ours, not the CLI's: {cfg}"
    );
    // Memory is off in this suite: no overmind endpoint rides along.
    assert!(cfg["mcpServers"].get("overmind").is_none(), "{cfg}");
}

/// The agent is told what it holds, in its own prompt.
#[tokio::test]
async fn the_prompt_names_the_granted_tools() {
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let log = root.join("prompt.txt");
    let agent_body = format!(
        "#!/bin/sh\nprintf '%s' \"$OVERMIND_TASK_PROMPT\" > {}\necho done > out.txt\necho '{{\"total_cost_usd\":0.01,\"session_id\":\"s\"}}'\n",
        log.display()
    );
    let env = setup(Some(registry()), &agent_body).await;
    let (_, a) = hire(&env, "Modeler", json!(["probe"])).await;
    let agent = a["id"].as_str().expect("agent id").to_string();
    let (_, t) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company),
        Some(json!({ "title": "Say hello", "execution_kind": "knowledge" })),
    )
    .await;
    let task = t["id"].as_str().expect("task id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (s, v) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let session = v["session_id"].as_str().expect("session").to_string();
    for _ in 0..150 {
        let (_, sv) = send(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        if sv["status"] == "completed" || sv["status"] == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let prompt = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        prompt.contains("probe") && prompt.contains("a probe that answers nothing"),
        "the prompt names the tool and says what it is: {prompt}"
    );
}
