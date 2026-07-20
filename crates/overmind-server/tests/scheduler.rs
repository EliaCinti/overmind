//! M3 acceptance tests: parallel agents, session timeouts with safe task
//! release, restart recovery of orphaned sessions, and heartbeat wakeups
//! that enforce agent autonomy.

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

fn sh(dir: &std::path::Path, cmd: &str) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn unique_root() -> PathBuf {
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
    std::env::temp_dir().join(format!("overmind-sched-{nanos}-{}-{n}", std::process::id()))
}

const HAPPY_STUB: &str = r#"#!/bin/sh
echo "working on: $OVERMIND_TASK_TITLE"
echo "done by agent" > result.txt
echo '{"model":"stub","session_id":"stub-sess","total_cost_usd":0.01,"usage":{"input_tokens":10,"output_tokens":5}}'
"#;

struct Env {
    app: axum::Router,
    state: overmind_server::AppState,
    root: PathBuf,
    company_id: String,
    goal_id: String,
}

async fn build_env(
    stub: &str,
    db_url: Option<String>,
    tweak: impl FnOnce(&mut overmind_server::Config),
) -> Env {
    let root = unique_root();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo '# Demo' > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );
    let script_path = root.join("stub-agent.sh");
    std::fs::write(&script_path, stub).expect("write stub script");

    let mut config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script_path.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 100,
        ..overmind_server::Config::default()
    };
    tweak(&mut config);
    let url = db_url.unwrap_or_else(|| "sqlite::memory:".to_string());
    let state = overmind_server::init_with(&url, config)
        .await
        .expect("init db");
    let app = overmind_server::app(state.clone());

    let (_, company) = send(
        &app,
        "POST",
        "/companies",
        Some(json!({ "name": "Sched Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, project) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/projects"),
        Some(json!({ "title": "Demo" })),
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
    assert_eq!(status, StatusCode::CREATED, "workspace: {ws}");
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/projects/{project_id}/goals"),
        Some(json!({ "title": "Goal" })),
    )
    .await;
    let goal_id = goal["id"].as_str().expect("goal id").to_string();

    Env {
        app,
        state,
        root,
        company_id,
        goal_id,
    }
}

async fn hire(env: &Env, name: &str, archetype: &str) -> String {
    let (status, agent) = send(
        &env.app,
        "POST",
        &format!("/companies/{}/agents", env.company_id),
        Some(json!({ "name": name, "archetype": archetype })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "hire: {agent}");
    agent["id"].as_str().expect("agent id").to_string()
}

async fn make_todo_task(env: &Env, title: &str) -> String {
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/companies/{}/tasks", env.company_id),
        Some(json!({ "title": title, "goal_id": env.goal_id })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &env.app,
        "POST",
        &format!("/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    task_id
}

async fn task_status(env: &Env, task_id: &str) -> String {
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == task_id)
        .and_then(|t| t["status"].as_str())
        .expect("task status")
        .to_string()
}

async fn wait_for_task_status(env: &Env, task_id: &str, wanted: &str) {
    for _ in 0..100 {
        if task_status(env, task_id).await == wanted {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "task {task_id} never reached '{wanted}' (currently '{}')",
        task_status(env, task_id).await
    );
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

#[tokio::test]
async fn three_agents_work_three_tasks_in_parallel() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let mut agents = Vec::new();
    let mut tasks = Vec::new();
    for i in 0..3 {
        agents.push(hire(&env, &format!("Agent {i}"), "backend-developer").await);
        tasks.push(make_todo_task(&env, &format!("Task {i}")).await);
    }

    let start = |i: usize| {
        let app = env.app.clone();
        let uri = format!("/tasks/{}/start", tasks[i]);
        let body = json!({ "agent_id": agents[i] });
        async move { send(&app, "POST", &uri, Some(body)).await }
    };
    let (a, b, c) = tokio::join!(start(0), start(1), start(2));
    let mut worktrees = Vec::new();
    for (status, started) in [a, b, c] {
        assert_eq!(status, StatusCode::ACCEPTED, "start: {started}");
        let session_id = started["session_id"].as_str().expect("session id");
        let session = wait_for_session(&env.app, session_id).await;
        assert_eq!(session["status"], "completed", "session: {session}");
        worktrees.push(
            session["workspace_path"]
                .as_str()
                .expect("workspace")
                .to_string(),
        );
    }
    // One isolated worktree each, no interference
    worktrees.sort();
    worktrees.dedup();
    assert_eq!(worktrees.len(), 3);
    for task_id in &tasks {
        assert_eq!(task_status(&env, task_id).await, "in_review");
    }
    let (_, report) = send(&env.app, "GET", "/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn timeout_kills_session_and_releases_task() {
    let env = build_env("#!/bin/sh\nsleep 30\n", None, |c| {
        c.session_timeout_secs = 1
    })
    .await;
    let agent_id = hire(&env, "Slowpoke", "backend-developer").await;
    let task_id = make_todo_task(&env, "Never finishes").await;

    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let session_id = started["session_id"].as_str().expect("session id");

    let session = wait_for_session(&env.app, session_id).await;
    assert_eq!(session["status"], "failed");
    assert!(
        session["last_error"]
            .as_str()
            .expect("error")
            .contains("timed out"),
        "session: {session}"
    );
    // The task is released safely: back to todo, unassigned, startable again
    assert_eq!(task_status(&env, &task_id).await, "todo");
    let (_, report) = send(&env.app, "GET", "/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn restart_recovery_resumes_orphaned_session() {
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let db_url = format!("sqlite://{}", root.join("overmind.sqlite").display());

    // "First server run": full org setup, then craft an orphaned session as a
    // crash mid-run would leave it (running in the DB, worktree on disk, no
    // live process).
    let env = build_env(HAPPY_STUB, Some(db_url.clone()), |_| {}).await;
    let agent_id = hire(&env, "Phoenix", "backend-developer").await;
    let task_id = make_todo_task(&env, "Survives restarts").await;

    let repo = env.root.join("repo");
    let orphan_wt = env.root.join("data").join("worktrees").join("orphan-sess");
    std::fs::create_dir_all(orphan_wt.parent().expect("parent")).expect("mkdir worktrees");
    sh(
        &repo,
        &format!(
            "git worktree add {} -b overmind/orphan",
            orphan_wt.display()
        ),
    );
    let base_sha = sh(&orphan_wt, "git rev-parse HEAD");
    sqlx::query(
        "INSERT INTO agent_task_sessions (id, task_id, agent_id, status, branch, workspace_path, base_sha, created_at, started_at)
         VALUES ('orphan-sess', ?, ?, 'running', 'overmind/orphan', ?, ?, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
    )
    .bind(&task_id)
    .bind(&agent_id)
    .bind(orphan_wt.to_string_lossy().as_ref())
    .bind(&base_sha)
    .execute(&env.state.pool)
    .await
    .expect("insert orphan session");
    sqlx::query("UPDATE tasks SET status = 'in_progress', assignee_agent_id = ? WHERE id = ?")
        .bind(&agent_id)
        .bind(&task_id)
        .execute(&env.state.pool)
        .await
        .expect("mark task in progress");
    env.state.pool.close().await;

    // "Second server run" over the same database: its registry is empty, so
    // the heartbeat must recover and resume the orphan.
    let script_path = root.join("stub2.sh");
    std::fs::write(&script_path, HAPPY_STUB).expect("write stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script_path.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 100,
        ..overmind_server::Config::default()
    };
    let state2 = overmind_server::init_with(&db_url, config)
        .await
        .expect("reopen db");
    let app2 = overmind_server::app(state2.clone());
    let _scheduler = overmind_server::scheduler::spawn(state2.clone());

    let session = wait_for_session(&app2, "orphan-sess").await;
    assert_eq!(session["status"], "completed", "session: {session}");
    assert!(
        orphan_wt.join("result.txt").exists(),
        "agent worked in the resumed worktree"
    );

    let (resumed_count, task_state): (i64, String) = {
        let row: (i64,) = sqlx::query_as(
            "SELECT resumed_count FROM agent_task_sessions WHERE id = 'orphan-sess'",
        )
        .fetch_one(&state2.pool)
        .await
        .expect("resumed_count");
        let task: (String,) = sqlx::query_as("SELECT status FROM tasks WHERE id = ?")
            .bind(&task_id)
            .fetch_one(&state2.pool)
            .await
            .expect("task status");
        (row.0, task.0)
    };
    assert_eq!(resumed_count, 1);
    assert_eq!(task_state, "in_review");
}

#[tokio::test]
async fn wakeup_enforces_agent_autonomy() {
    // act_within_budget (researcher archetype): may pick up todo work alone.
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let agent_id = hire(&env, "Selfstarter", "researcher").await;
    let task_id = make_todo_task(&env, "Autonomous pickup").await;
    let (status, wakeup) = send(
        &env.app,
        "POST",
        &format!("/agents/{agent_id}/wakeup"),
        Some(json!({ "reason": "heartbeat test" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "wakeup: {wakeup}");
    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    wait_for_task_status(&env, &task_id, "in_review").await;
    let outcome: (String, Option<String>) =
        sqlx::query_as("SELECT status, outcome FROM agent_wakeup_requests WHERE id = ?")
            .bind(wakeup["id"].as_str().expect("wakeup id"))
            .fetch_one(&env.state.pool)
            .await
            .expect("wakeup row");
    assert_eq!(outcome.0, "done");
    assert!(
        outcome.1.as_deref().unwrap_or("").contains("started task"),
        "outcome: {outcome:?}"
    );

    // act_with_approval (backend-developer): the wakeup must NOT start work.
    let env2 = build_env(HAPPY_STUB, None, |_| {}).await;
    let agent2 = hire(&env2, "Waitsforhumans", "backend-developer").await;
    let task2 = make_todo_task(&env2, "Needs a human").await;
    let (_, wakeup2) = send(&env2.app, "POST", &format!("/agents/{agent2}/wakeup"), None).await;
    overmind_server::scheduler::beat(&env2.state)
        .await
        .expect("beat");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(task_status(&env2, &task2).await, "todo");
    let outcome2: (Option<String>,) =
        sqlx::query_as("SELECT outcome FROM agent_wakeup_requests WHERE id = ?")
            .bind(wakeup2["id"].as_str().expect("wakeup id"))
            .fetch_one(&env2.state.pool)
            .await
            .expect("wakeup row");
    assert!(
        outcome2
            .0
            .as_deref()
            .unwrap_or("")
            .contains("requires a human"),
        "outcome: {outcome2:?}"
    );
}
