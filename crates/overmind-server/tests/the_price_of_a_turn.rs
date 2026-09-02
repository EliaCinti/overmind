//! M26 (ADR-0035): the reservation at checkout is the agent's own recent
//! cost, read from the ledger -- not a flat guess that is wrong in both
//! directions.

mod common;

use std::path::PathBuf;

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
    std::env::temp_dir().join(format!(
        "overmind-price-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ))
}

/// A stub adapter: writes a file, answers a five-cent envelope.
const STUB: &str = r#"#!/bin/sh
echo "work" > out.txt
echo '{"model":"stub","session_id":"s","total_cost_usd":0.05,"usage":{"input_tokens":10,"output_tokens":5}}'
"#;

struct Env {
    app: axum::Router,
    state: overmind_server::AppState,
    company: String,
    goal: String,
}

async fn setup() -> Env {
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
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000,
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = common::claimed(overmind_server::app(state.clone()), &root.join("data")).await;
    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Price Co" })),
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
    Env {
        app,
        state,
        company,
        goal,
    }
}

async fn hire(env: &Env, name: &str, budget: i64) -> String {
    let (s, a) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({
            "name": name,
            "archetype": "builder",
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

/// Write history into the ledger the way the runner would have: one finished
/// session per past run, one cost event naming it. `costs` is in cents,
/// oldest first.
async fn past_task_runs(env: &Env, agent: &str, costs: &[i64]) {
    for (i, cents) in costs.iter().enumerate() {
        let task = make_todo(env, &format!("past run {i}")).await;
        let session = format!("past-{agent}-{i}");
        let at = format!("2026-07-{:02}T10:00:00+00:00", i + 1);
        sqlx::query(
            "INSERT INTO agent_task_sessions
                 (id, task_id, agent_id, status, branch, workspace_path, created_at, finished_at)
             VALUES (?, ?, ?, 'completed', 'work', '/tmp/nowhere', ?, ?)",
        )
        .bind(&session)
        .bind(&task)
        .bind(agent)
        .bind(&at)
        .bind(&at)
        .execute(&env.state.pool)
        .await
        .expect("insert session");
        sqlx::query(
            "INSERT INTO cost_events
                 (id, company_id, agent_id, task_id, session_id, provider, model,
                  cost_cents, occurred_at, created_at)
             VALUES (?, ?, ?, ?, ?, 'stub', 'stub', ?, ?, ?)",
        )
        .bind(format!("cost-{session}"))
        .bind(&env.company)
        .bind(agent)
        .bind(&task)
        .bind(&session)
        .bind(cents)
        .bind(&at)
        .bind(&at)
        .execute(&env.state.pool)
        .await
        .expect("insert cost");
    }
}

/// The whole point: a three-cent agent at a ten-cent cap used to be refused,
/// because the flat fifty did not fit. Priced by its own ledger, it starts --
/// and the summary says what was reserved and on how many runs it rests.
#[tokio::test]
async fn a_cheap_agent_is_priced_by_its_own_ledger_not_a_flat_guess() {
    let env = setup().await;
    let agent = hire(&env, "Frugal", 10).await;
    // Four past runs, last month -- outside the window, so they do not count
    // as spend, but they are exactly the history the estimate reads.
    past_task_runs(&env, &agent, &[2, 3, 3, 4]).await;

    let (_, v) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/budget", env.company),
        None,
    )
    .await;
    let frugal = v["budgets"]
        .as_array()
        .expect("budgets")
        .iter()
        .find(|b| b["agent_id"] == json!(agent))
        .expect("the agent's line")
        .clone();
    // p75 of [2,3,3,4] leans dear: 4. And it says on how much it rests.
    assert_eq!(frugal["estimates"]["task"]["cents"], json!(4), "{frugal}");
    assert_eq!(frugal["estimates"]["task"]["samples"], json!(4), "{frugal}");
    // Turns have no history yet: the flat number, and the count says so.
    assert_eq!(frugal["estimates"]["turn"]["samples"], json!(0), "{frugal}");
    assert_eq!(frugal["estimates"]["turn"]["cents"], json!(50), "{frugal}");

    let task = make_todo(&env, "Cheap work").await;
    let (s, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::ACCEPTED,
        "a cheap agent must start under a cap its own history fits: {body}"
    );
}

/// The other direction: a dear agent is held at a cap the flat guess would
/// have let it cross.
#[tokio::test]
async fn a_dear_agent_is_held_where_the_flat_guess_would_have_let_it_through() {
    let env = setup().await;
    // Cap 60: the flat fifty fits; the agent's own runs (~80) do not.
    let agent = hire(&env, "Lavish", 60).await;
    past_task_runs(&env, &agent, &[70, 80, 90]).await;

    let task = make_todo(&env, "Dear work").await;
    let (s, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED, "{body}");
}

/// Fewer than three samples: the flat number stands, and says so.
#[tokio::test]
async fn one_data_point_is_not_a_price() {
    let env = setup().await;
    let agent = hire(&env, "New", 10).await;
    past_task_runs(&env, &agent, &[2]).await;
    let (_, v) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/budget", env.company),
        None,
    )
    .await;
    let line = v["budgets"]
        .as_array()
        .expect("budgets")
        .iter()
        .find(|b| b["agent_id"] == json!(agent))
        .expect("line")
        .clone();
    assert_eq!(line["estimates"]["task"]["cents"], json!(50), "{line}");
    assert_eq!(line["estimates"]["task"]["samples"], json!(1), "{line}");
}
