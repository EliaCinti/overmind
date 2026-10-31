//! M3 acceptance tests: parallel agents, session timeouts with safe task
//! release, restart recovery of orphaned sessions, and heartbeat wakeups
//! that enforce agent autonomy.

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
    let script_path = root.join("stub-agent.sh");
    build_env_in(root, script_path, stub, db_url, tweak).await
}

/// [`build_env`] with the repository and the data dir under `root`, and the
/// stub at `script_path` -- which has to stay somewhere the cage can read.
async fn build_env_in(
    root: PathBuf,
    script_path: PathBuf,
    stub: &str,
    db_url: Option<String>,
    tweak: impl FnOnce(&mut overmind_server::Config),
) -> Env {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo '# Demo' > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );
    if let Some(dir) = script_path.parent() {
        std::fs::create_dir_all(dir).expect("create stub dir");
    }
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
    let app = common::claimed(overmind_server::app(state.clone()), &root.join("data")).await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Sched Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, project) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/projects"),
        Some(json!({ "title": "Demo" })),
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
    assert_eq!(status, StatusCode::CREATED, "workspace: {ws}");
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/goals"),
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
    hire_with_traits(env, name, archetype, json!({})).await
}

async fn make_todo_task(env: &Env, title: &str) -> String {
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({ "title": title, "goal_id": env.goal_id })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    task_id
}

/// A `knowledge` task: needs no repo, and is what a researcher is
/// characterized for (M14 — the capability gate refuses the rest).
async fn make_todo_knowledge_task(env: &Env, title: &str) -> String {
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({ "title": title, "execution_kind": "knowledge" })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
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
        &format!("/api/companies/{}/tasks", env.company_id),
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
        let (_, session) = send(app, "GET", &format!("/api/sessions/{session_id}"), None).await;
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
        agents.push(hire(&env, &format!("Agent {i}"), "builder").await);
        tasks.push(make_todo_task(&env, &format!("Task {i}")).await);
    }

    let start = |i: usize| {
        let app = env.app.clone();
        let uri = format!("/api/tasks/{}/start", tasks[i]);
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
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn timeout_kills_session_and_releases_task() {
    let env = build_env("#!/bin/sh\nsleep 30\n", None, |c| {
        c.session_timeout_secs = 1
    })
    .await;
    let agent_id = hire(&env, "Slowpoke", "builder").await;
    let task_id = make_todo_task(&env, "Never finishes").await;

    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
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
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
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
    let agent_id = hire(&env, "Phoenix", "builder").await;
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
    // The same owner, entering a server that has already been claimed.
    let app2 = common::claimed(overmind_server::app(state2.clone()), &root.join("data")).await;
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
    let task_id = make_todo_knowledge_task(&env, "Autonomous pickup").await;
    let (status, wakeup) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent_id}/wakeup"),
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

    // act_with_approval (builder): the wakeup must NOT start work.
    let env2 = build_env(HAPPY_STUB, None, |_| {}).await;
    let agent2 = hire(&env2, "Waitsforhumans", "builder").await;
    let task2 = make_todo_task(&env2, "Needs a human").await;
    let (_, wakeup2) = send(
        &env2.app,
        "POST",
        &format!("/api/agents/{agent2}/wakeup"),
        None,
    )
    .await;
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

async fn hire_with_traits(env: &Env, name: &str, archetype: &str, traits: Value) -> String {
    let (status, agent) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(json!({ "name": name, "archetype": archetype, "traits": traits })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "hire: {agent}");
    agent["id"].as_str().expect("agent id").to_string()
}

/// A sketch on a task: the multimodal gate reads the mime, not the bytes.
async fn attach_sketch(env: &Env, task_id: &str) {
    let (status, body) = common::upload(
        &env.app,
        &format!("/api/tasks/{task_id}/attachments"),
        "sketch.jpeg",
        "image/jpeg",
        b"not really a jpeg",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload: {body}");
}

async fn wakeup_row(env: &Env, wakeup_id: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, outcome FROM agent_wakeup_requests WHERE id = ?")
        .bind(wakeup_id)
        .fetch_one(&env.state.pool)
        .await
        .expect("wakeup row")
}

/// A refusal the runner can name a remedy for (the multimodal gate, ADR-0021)
/// fell through `wakeup_outcome` as an error: the beat aborted before the
/// wakeup was marked done, the row stayed queued, and the next heartbeat
/// re-refused the same task — seventeen times, thirty seconds apart, in
/// TravelAgency's log — while every wakeup behind it, and the digests, waited.
/// A refusal is this agent's problem, not the heartbeat's.
#[tokio::test]
async fn a_remediable_refusal_is_the_wakeups_outcome_not_the_heartbeats_failure() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    // The oldest todo task carries a sketch; neither researcher was ever
    // characterized to look at one.
    let task_id = make_todo_knowledge_task(&env, "Look at the sketch").await;
    attach_sketch(&env, &task_id).await;
    let first = hire_with_traits(
        &env,
        "Nearsighted",
        "researcher",
        json!({ "multimodal": false }),
    )
    .await;
    let behind =
        hire_with_traits(&env, "Behind", "researcher", json!({ "multimodal": false })).await;
    let (s, w1) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{first}/wakeup"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "wakeup: {w1}");
    let (s, w2) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{behind}/wakeup"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "wakeup: {w2}");

    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("a refusal is the wakeup's outcome, not the heartbeat's failure");

    // The refusal is recorded as the outcome, and the row is closed...
    let (status, outcome) = wakeup_row(&env, w1["id"].as_str().expect("wakeup id")).await;
    assert_eq!(status, "done", "the first wakeup must not stay queued");
    assert!(
        outcome
            .as_deref()
            .unwrap_or("")
            .contains("not characterized for visual work"),
        "outcome names the refusal: {outcome:?}"
    );
    // ...the task is left exactly where it was...
    assert_eq!(task_status(&env, &task_id).await, "todo");
    // ...and the wakeup behind it was processed in the same beat instead of
    // waiting behind an error.
    let (status, outcome) = wakeup_row(&env, w2["id"].as_str().expect("wakeup id")).await;
    assert_eq!(status, "done", "the second wakeup was starved: {outcome:?}");
    // The remedy the refusal named travels as data, never as a sentence to parse.
    let (payload,): (String,) = sqlx::query_as(
        "SELECT payload FROM audit_events WHERE kind = 'agent.wakeup_processed'
         AND json_extract(payload, '$.request_id') = ?",
    )
    .bind(w1["id"].as_str().expect("wakeup id"))
    .fetch_one(&env.state.pool)
    .await
    .expect("audit row");
    let payload: Value = serde_json::from_str(&payload).expect("payload json");
    assert_eq!(
        payload["remedies"][0]["kind"],
        json!("grant_multimodal"),
        "{payload}"
    );
    assert_eq!(
        payload["remedies"][0]["agent_id"],
        json!(first),
        "{payload}"
    );
}

/// Recording the refusal is not enough: the oldest todo task is the one the
/// picker always reaches first, so an agent that cannot take it would come
/// back to it on every wakeup and never get to the work behind it -- and,
/// with the beat no longer failing, nothing would say so. A wakeup takes the
/// work the agent can take.
#[tokio::test]
async fn a_task_an_agent_cannot_take_does_not_stand_in_front_of_one_it_can() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let sketch = make_todo_knowledge_task(&env, "Look at the sketch").await;
    attach_sketch(&env, &sketch).await;
    let plain = make_todo_knowledge_task(&env, "Write the brief").await;
    let agent = hire_with_traits(
        &env,
        "Nearsighted",
        "researcher",
        json!({ "multimodal": false }),
    )
    .await;
    let (s, w) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/wakeup"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "wakeup: {w}");

    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");

    // The sketch was refused and left alone; the brief, behind it, was taken.
    wait_for_task_status(&env, &plain, "in_review").await;
    assert_eq!(task_status(&env, &sketch).await, "todo");
    let (status, outcome) = wakeup_row(&env, w["id"].as_str().expect("wakeup id")).await;
    assert_eq!(status, "done");
    let outcome = outcome.unwrap_or_default();
    assert!(
        outcome.contains("not characterized for visual work")
            && outcome.contains(&format!("started task {plain}")),
        "the outcome says what was declined before what was started: {outcome}"
    );
}

// --- M36a, slice 1: the attempts cap (ADR-0050 §1, §10) -------------------

const SLOW_STUB: &str = "#!/bin/sh\nsleep 30\n";
const FAILING_STUB: &str = "#!/bin/sh\necho 'it broke' >&2\nexit 1\n";

/// One attempt: start the task, wait for its session to end, return it.
async fn attempt(env: &Env, task_id: &str, agent_id: &str) -> Value {
    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start: {started}");
    let session_id = started["session_id"].as_str().expect("session id");
    wait_for_session(&env.app, session_id).await
}

async fn stalled_payloads(env: &Env, task_id: &str) -> Vec<Value> {
    sqlx::query_as::<_, (String,)>(
        "SELECT payload FROM audit_events WHERE kind = 'task.stalled' AND task_id = ? ORDER BY seq",
    )
    .bind(task_id)
    .fetch_all(&env.state.pool)
    .await
    .expect("audit")
    .into_iter()
    .map(|(p,)| serde_json::from_str(&p).expect("payload"))
    .collect()
}

#[tokio::test]
async fn a_task_that_times_out_three_times_stalls_instead_of_looping() {
    let env = build_env(SLOW_STUB, None, |c| c.session_timeout_secs = 1).await;
    let agent_id = hire(&env, "Slowpoke", "builder").await;
    let task_id = make_todo_task(&env, "Never finishes").await;

    // The first two time-outs release the task, as they always have: a
    // time-out can be bad luck.
    for _ in 0..2 {
        attempt(&env, &task_id, &agent_id).await;
        assert_eq!(task_status(&env, &task_id).await, "todo");
    }
    // The third is no longer luck: the task stops, and says why, instead of
    // going back to the front of the queue for the next wakeup.
    attempt(&env, &task_id, &agent_id).await;
    assert_eq!(task_status(&env, &task_id).await, "stalled");
    let stalled = stalled_payloads(&env, &task_id).await;
    assert_eq!(stalled.len(), 1, "{stalled:?}");
    assert_eq!(stalled[0]["cause"], "timed_out");
    assert_eq!(stalled[0]["attempts"], 3);
    assert_eq!(stalled[0]["sessions"].as_array().map(Vec::len), Some(3));
    assert!(
        stalled[0]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("timed out")),
        "{stalled:?}"
    );
    // The board reads the same packet: the reason travels with the task.
    let (_, tasks) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/tasks", env.company_id),
        None,
    )
    .await;
    let task = tasks["tasks"]
        .as_array()
        .and_then(|ts| ts.iter().find(|t| t["id"] == task_id.as_str()))
        .expect("task");
    assert_eq!(task["stall"]["cause"], "timed_out", "{task}");
    assert_eq!(task["stall"]["attempts"], 3, "{task}");
    // Stalling is not releasing: the agent that failed stays on the task, so
    // nothing reports it as unowned (ADR-0046).
    assert_eq!(task["assignee_agent_id"], agent_id.as_str(), "{task}");
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn a_different_cause_does_not_add_to_the_count() {
    let env = build_env(SLOW_STUB, None, |c| c.session_timeout_secs = 1).await;
    let agent_id = hire(&env, "Mixed", "builder").await;
    let task_id = make_todo_task(&env, "Fails two ways").await;

    for _ in 0..2 {
        attempt(&env, &task_id, &agent_id).await;
    }
    // A third failure, but of another kind: it goes where that kind always
    // went, and the two time-outs before it do not make it the third strike.
    std::fs::write(env.root.join("stub-agent.sh"), FAILING_STUB).expect("swap stub");
    let session = attempt(&env, &task_id, &agent_id).await;
    assert_eq!(session["status"], "failed");
    assert_eq!(task_status(&env, &task_id).await, "blocked");
    assert!(stalled_payloads(&env, &task_id).await.is_empty());
}

#[tokio::test]
async fn a_person_restarting_a_stalled_task_resets_its_count() {
    let env = build_env(SLOW_STUB, None, |c| c.session_timeout_secs = 1).await;
    let agent_id = hire(&env, "Again", "builder").await;
    let task_id = make_todo_task(&env, "Worth another look").await;
    for _ in 0..3 {
        attempt(&env, &task_id, &agent_id).await;
    }
    assert_eq!(task_status(&env, &task_id).await, "stalled");

    // The person has looked, and puts it back: it starts from zero, not from
    // one strike away from stalling again.
    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    attempt(&env, &task_id, &agent_id).await;
    assert_eq!(task_status(&env, &task_id).await, "todo");
    assert_eq!(stalled_payloads(&env, &task_id).await.len(), 1);
}

// --- M36a, slice 2: the repetition detector on refused starts (ADR-0050 §3) --

/// One wakeup of `agent`, processed by one beat.
async fn wake(env: &Env, agent_id: &str) -> String {
    let (status, w) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent_id}/wakeup"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "wakeup: {w}");
    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    w["id"].as_str().expect("wakeup id").to_string()
}

async fn wakeup_payload(env: &Env, wakeup_id: &str) -> Value {
    let (payload,): (String,) = sqlx::query_as(
        "SELECT payload FROM audit_events WHERE kind = 'agent.wakeup_processed'
         AND json_extract(payload, '$.request_id') = ?",
    )
    .bind(wakeup_id)
    .fetch_one(&env.state.pool)
    .await
    .expect("audit row");
    serde_json::from_str(&payload).expect("payload json")
}

async fn constrained_payloads(env: &Env, task_id: &str) -> Vec<Value> {
    sqlx::query_as::<_, (String,)>(
        "SELECT payload FROM audit_events WHERE kind = 'loop.constrained' AND task_id = ? ORDER BY seq",
    )
    .bind(task_id)
    .fetch_all(&env.state.pool)
    .await
    .expect("audit")
    .into_iter()
    .map(|(p,)| serde_json::from_str(&p).expect("payload"))
    .collect()
}

async fn notifications_of(env: &Env, kind: &str) -> Vec<Value> {
    let (status, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/notifications", env.company_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "notifications: {body}");
    body.as_array()
        .cloned()
        .or_else(|| body["notifications"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n["kind"] == kind)
        .collect()
}

/// A code task with no goal, in a company with a second repository: it has
/// no single home, so its start is refused as `invalid` whoever asks.
async fn make_homeless_code_task(env: &Env) -> String {
    let other = env.root.join("other-repo");
    std::fs::create_dir_all(&other).expect("create other repo");
    sh(
        &other,
        "git init -q -b main && git -c user.email=t@t -c user.name=T commit -q --allow-empty -m init",
    );
    let (_, project) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/projects", env.company_id),
        Some(json!({ "title": "Other" })),
    )
    .await;
    let (status, ws) = send(
        &env.app,
        "POST",
        &format!(
            "/api/projects/{}/workspaces",
            project["id"].as_str().expect("project id")
        ),
        Some(json!({ "name": "main", "cwd": other.to_string_lossy() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "workspace: {ws}");
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company_id),
        Some(json!({ "title": "Homeless" })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    let (status, _) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    task_id
}

/// TravelAgency's seventeen: after #107 a refused start no longer stalls the
/// beat, but the next wakeup refuses the same task for the same reason, and
/// the one after that, with nobody counting. When the reason is the task's
/// own — no agent could start it — the third time the task stops and says why.
#[tokio::test]
async fn a_start_the_task_makes_impossible_three_times_stalls_the_task() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let task_id = make_homeless_code_task(&env).await;
    let agent = hire_with_traits(
        &env,
        "Selfstarter",
        "builder",
        json!({ "autonomy": "act_within_budget" }),
    )
    .await;

    // The first two refusals leave the task where it was: a refusal can be
    // fixed between one wakeup and the next. Each is recorded as data.
    for _ in 0..2 {
        let w = wake(&env, &agent).await;
        assert_eq!(task_status(&env, &task_id).await, "todo");
        let payload = wakeup_payload(&env, &w).await;
        assert_eq!(
            payload["refusals"],
            json!([{ "task_id": task_id, "cause": "invalid" }]),
            "{payload}"
        );
    }
    // The third is a repetition: the task leaves the queue.
    wake(&env, &agent).await;
    assert_eq!(task_status(&env, &task_id).await, "stalled");
    let stalled = stalled_payloads(&env, &task_id).await;
    assert_eq!(stalled.len(), 1, "{stalled:?}");
    assert_eq!(stalled[0]["cause"], "refused", "{stalled:?}");
    assert_eq!(stalled[0]["refusal"], "invalid");
    assert_eq!(stalled[0]["attempts"], 3);
    assert_eq!(stalled[0]["agent_id"], agent.as_str());
    let reason = stalled[0]["reason"].as_str().unwrap_or("");
    assert!(reason.contains("refused 3 times"), "{reason}");
    assert_eq!(stalled[0]["remedies"], json!([]), "{stalled:?}");
    assert!(constrained_payloads(&env, &task_id).await.is_empty());

    // A fourth wakeup does not reach it: a stalled task is out of the queue.
    let w = wake(&env, &agent).await;
    let (_, outcome) = wakeup_row(&env, &w).await;
    assert_eq!(outcome.as_deref(), Some("no todo tasks it can take"));
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

/// A sketch, and an agent not characterized to look at it.
async fn sketch_and_nearsighted(env: &Env) -> (String, String) {
    let task_id = make_todo_knowledge_task(env, "Look at the sketch").await;
    attach_sketch(env, &task_id).await;
    let agent = hire_with_traits(
        env,
        "Nearsighted",
        "researcher",
        json!({ "multimodal": false }),
    )
    .await;
    (task_id, agent)
}

/// A refusal that belongs to the agent is the agent's to pay for, not the
/// task's (ADR-0050, addendum of 28 Oct). Stalling the task would hide a
/// sketch from the one colleague who can look at it, merely because they were
/// busy while a nearsighted one kept waking up — and the board would then
/// offer to give sight to the wrong agent. Past the tolerance, the agent stops
/// being offered the task; the task stays in the queue for everyone else.
#[tokio::test]
async fn the_refused_agent_stops_being_offered_the_task_others_still_are() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let (task_id, nearsighted) = sketch_and_nearsighted(&env).await;
    let sighted =
        hire_with_traits(&env, "Sighted", "researcher", json!({ "multimodal": true })).await;

    for _ in 0..3 {
        let w = wake(&env, &nearsighted).await;
        let payload = wakeup_payload(&env, &w).await;
        assert_eq!(
            payload["refusals"],
            json!([{ "task_id": task_id, "cause": "remediable:grant_multimodal" }]),
            "{payload}"
        );
        assert_eq!(task_status(&env, &task_id).await, "todo");
    }
    assert!(stalled_payloads(&env, &task_id).await.is_empty());
    let constrained = constrained_payloads(&env, &task_id).await;
    assert_eq!(constrained.len(), 1, "{constrained:?}");
    assert_eq!(constrained[0]["cause"], "remediable:grant_multimodal");
    assert_eq!(constrained[0]["agent_id"], nearsighted.as_str());
    assert_eq!(constrained[0]["attempts"], 3);
    assert_eq!(constrained[0]["cap"], 3);
    assert_eq!(
        constrained[0]["remedies"],
        json!([{ "kind": "grant_multimodal", "agent_id": nearsighted }]),
        "{constrained:?}"
    );
    assert_eq!(
        constrained[0]["wakeups"].as_array().map(Vec::len),
        Some(3),
        "{constrained:?}"
    );

    // The fourth wakeup does not offer it again: nothing to refuse.
    let w = wake(&env, &nearsighted).await;
    let (_, outcome) = wakeup_row(&env, &w).await;
    assert_eq!(outcome.as_deref(), Some("no todo tasks it can take"));
    assert_eq!(wakeup_payload(&env, &w).await["refusals"], json!([]));

    // The colleague who can look at it, once awake, takes it.
    let w = wake(&env, &sighted).await;
    let payload = wakeup_payload(&env, &w).await;
    assert_eq!(payload["taken"], json!(task_id), "{payload}");
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

/// The owner hears of it once, with the remedy as data — not once per
/// wakeup: after the trip the agent is not offered the task, so there is
/// nothing left to repeat.
#[tokio::test]
async fn the_owner_is_told_once_with_the_remedy() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let (task_id, agent) = sketch_and_nearsighted(&env).await;
    for _ in 0..5 {
        wake(&env, &agent).await;
    }
    let told = notifications_of(&env, "loop.constrained").await;
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0]["agent_id"], agent.as_str(), "{told:?}");
    assert_eq!(told[0]["subject_type"], "task", "{told:?}");
    assert_eq!(told[0]["subject_id"], task_id.as_str(), "{told:?}");
    let params = &told[0]["params"];
    assert_eq!(params["agent"], "Nearsighted", "{params}");
    assert_eq!(params["task"], "Look at the sketch", "{params}");
    assert_eq!(params["attempts"], 3, "{params}");
    assert_eq!(
        params["remedy"],
        json!({ "kind": "grant_multimodal", "agent_id": agent }),
        "{params}"
    );
}

/// One missing trait is one thing to fix, however many tasks it holds back:
/// five sketches refused to the same agent are one item in the inbox, naming
/// them all, not five items with the same button.
#[tokio::test]
async fn one_missing_trait_is_told_once_for_all_the_tasks_it_holds_back() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let (first, agent) = sketch_and_nearsighted(&env).await;
    let second = make_todo_knowledge_task(&env, "Look at the other sketch").await;
    attach_sketch(&env, &second).await;
    for _ in 0..3 {
        wake(&env, &agent).await;
    }
    assert_eq!(constrained_payloads(&env, &first).await.len(), 1);
    assert_eq!(constrained_payloads(&env, &second).await.len(), 1);
    let told = notifications_of(&env, "loop.constrained").await;
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(
        told[0]["params"]["tasks"],
        json!(["Look at the sketch", "Look at the other sketch"]),
        "{told:?}"
    );
}

/// The brake lifts when what it was about changes: applying the remedy is a
/// change of characterization, and the next wakeup offers the task again.
#[tokio::test]
async fn granting_the_remedy_offers_the_task_again() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let (task_id, agent) = sketch_and_nearsighted(&env).await;
    for _ in 0..3 {
        wake(&env, &agent).await;
    }
    assert_eq!(constrained_payloads(&env, &task_id).await.len(), 1);

    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/traits"),
        Some(json!({ "multimodal": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "traits: {body}");
    let w = wake(&env, &agent).await;
    let payload = wakeup_payload(&env, &w).await;
    assert_eq!(payload["taken"], json!(task_id), "{payload}");
}

/// Only a change lifts it: a patch that changes nothing still writes a
/// revision, and must not hand the task back to be refused three more times.
#[tokio::test]
async fn a_patch_that_changes_nothing_does_not_lift_the_brake() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let (task_id, agent) = sketch_and_nearsighted(&env).await;
    for _ in 0..3 {
        wake(&env, &agent).await;
    }
    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent}/traits"),
        Some(json!({ "multimodal": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "traits: {body}");
    let w = wake(&env, &agent).await;
    assert_eq!(wakeup_payload(&env, &w).await["refusals"], json!([]));
    assert_eq!(constrained_payloads(&env, &task_id).await.len(), 1);
}

/// A repetition is the same refusal, not any refusal: a task refused twice
/// because it has no repository to work in, then twice because it carries a
/// sketch, has been refused four times but repeated nothing three times. The
/// run counts from the change.
#[tokio::test]
async fn a_refusal_that_changes_reason_is_not_a_repetition() {
    let env = build_env(HAPPY_STUB, None, |_| {}).await;
    let task_id = make_homeless_code_task(&env).await;
    let agent = hire_with_traits(
        &env,
        "Selfstarter",
        "builder",
        json!({ "autonomy": "act_within_budget", "multimodal": false }),
    )
    .await;

    for _ in 0..2 {
        let w = wake(&env, &agent).await;
        assert_eq!(
            wakeup_payload(&env, &w).await["refusals"][0]["cause"],
            "invalid"
        );
    }
    // The multimodal gate stands before the repository check: the same task,
    // refused for another reason.
    attach_sketch(&env, &task_id).await;
    for _ in 0..2 {
        let w = wake(&env, &agent).await;
        assert_eq!(
            wakeup_payload(&env, &w).await["refusals"][0]["cause"],
            "remediable:grant_multimodal"
        );
    }
    assert_eq!(task_status(&env, &task_id).await, "todo");
    assert!(stalled_payloads(&env, &task_id).await.is_empty());
    assert!(constrained_payloads(&env, &task_id).await.is_empty());

    // The third of the new reason is a repetition — of a cause that belongs
    // to the agent, so it is the agent that stops being offered the task.
    wake(&env, &agent).await;
    assert_eq!(task_status(&env, &task_id).await, "todo");
    assert_eq!(constrained_payloads(&env, &task_id).await[0]["attempts"], 3);
}

// --- M36a, slice 4: a stop is a wind-down, with a handoff (ADR-0050 §4) -----

/// Says what it did, leaves a file, then outlives its deadline.
const TALKING_SLOW_STUB: &str =
    "#!/bin/sh\necho 'read the brief, drafted the outline'\necho draft > notes.md\nsleep 30\n";
/// Repeats the prompt it was given, then finishes.
const PROMPT_STUB: &str = "#!/bin/sh\nprintf '%s\\n' \"$OVERMIND_TASK_PROMPT\"\n";

async fn handoffs(env: &Env, task_id: &str) -> Vec<String> {
    sqlx::query_as::<_, (Option<String>,)>(
        "SELECT content FROM task_artifacts WHERE task_id = ? AND kind = 'handoff' ORDER BY created_at",
    )
    .bind(task_id)
    .fetch_all(&env.state.pool)
    .await
    .expect("artifacts")
    .into_iter()
    .map(|(c,)| c.unwrap_or_default())
    .collect()
}

/// A run killed at its deadline used to leave a failed session, an empty
/// output and a task back in the queue -- no account of what it had done.
/// Now Overmind writes one from what the run left behind: why it stopped,
/// what it changed, and the last thing it said.
#[tokio::test]
async fn a_run_killed_at_its_deadline_leaves_a_handoff() {
    let env = build_env(TALKING_SLOW_STUB, None, |c| c.session_timeout_secs = 1).await;
    let agent_id = hire(&env, "Slowpoke", "builder").await;
    let task_id = make_todo_task(&env, "Never finishes").await;

    let session = attempt(&env, &task_id, &agent_id).await;
    assert_eq!(session["status"], "failed");
    let handoff = handoffs(&env, &task_id).await;
    assert_eq!(handoff.len(), 1, "{handoff:?}");
    assert!(handoff[0].contains("timed out"), "{}", handoff[0]);
    assert!(handoff[0].contains("notes.md"), "{}", handoff[0]);
    assert!(
        handoff[0].contains("read the brief, drafted the outline"),
        "{}",
        handoff[0]
    );
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

/// The handoff is for the next attempt: it starts from where the last one
/// stopped, not from the brief alone.
#[tokio::test]
async fn the_next_attempt_reads_the_previous_handoff() {
    let env = build_env(TALKING_SLOW_STUB, None, |c| c.session_timeout_secs = 1).await;
    let agent_id = hire(&env, "Slowpoke", "builder").await;
    let task_id = make_todo_task(&env, "Finishes the second time").await;
    attempt(&env, &task_id, &agent_id).await;

    std::fs::write(env.root.join("stub-agent.sh"), PROMPT_STUB).expect("swap stub");
    let session = attempt(&env, &task_id, &agent_id).await;
    let prompt = session["output"].as_str().unwrap_or("");
    assert!(prompt.contains("previous attempt"), "{prompt}");
    assert!(
        prompt.contains("read the brief, drafted the outline"),
        "{prompt}"
    );
}

/// Two numbers per bound (ADR-0050 §4): the run is told the advertised one
/// and asked to hand off before it; the enforced one stays where it was and
/// fires only when the wind-down misbehaves.
#[tokio::test]
async fn the_prompt_states_the_advertised_bound_not_the_enforced_one() {
    let env = build_env(PROMPT_STUB, None, |c| c.session_timeout_secs = 100).await;
    let agent_id = hire_with_traits(
        &env,
        "Bounded",
        "builder",
        json!({ "monthly_budget_cents": 1000 }),
    )
    .await;
    let task_id = make_todo_task(&env, "Bounded work").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    let prompt = session["output"].as_str().unwrap_or("");
    assert!(prompt.contains("90 seconds"), "{prompt}");
    assert!(prompt.contains("€8.50"), "{prompt}");
    assert!(prompt.contains("HANDOFF.md"), "{prompt}");
    assert!(!prompt.contains("100 seconds"), "{prompt}");
    assert!(!prompt.contains("€10.00"), "{prompt}");
}

/// The handoff file is in a directory the agent owns, read by a server that
/// is not in the cage: a link planted in its place must not carry a host file
/// out into the drawer and the next prompt.
#[tokio::test]
async fn a_handoff_planted_as_a_symlink_is_not_followed() {
    let secret_dir = std::env::temp_dir().join(format!("om-secret-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&secret_dir).expect("secret dir");
    let secret = secret_dir.join("id_rsa");
    std::fs::write(&secret, "PRIVATE-KEY-MATERIAL").expect("secret");
    let stub = format!(
        "#!/bin/sh\nmkdir -p deliverables\nln -s '{}' deliverables/HANDOFF.md\n",
        secret.display()
    );
    let env = build_env(&stub, None, |_| {}).await;
    let agent_id = hire(&env, "Sly", "builder").await;
    let task_id = make_todo_task(&env, "Innocent work").await;

    attempt(&env, &task_id, &agent_id).await;
    for text in handoffs(&env, &task_id).await {
        assert!(!text.contains("PRIVATE-KEY-MATERIAL"), "{text}");
    }
    let (leaked,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM task_artifacts WHERE task_id = ? AND content LIKE '%PRIVATE-KEY-MATERIAL%'",
    )
    .bind(&task_id)
    .fetch_one(&env.state.pool)
    .await
    .expect("count");
    assert_eq!(leaked, 0);
}

/// Stops itself in time, as the prompt asks, and leaves its own handoff.
const HANDING_OFF_STUB: &str =
    "#!/bin/sh\nmkdir -p deliverables\necho 'pending: the second half' > deliverables/HANDOFF.md\n";

/// A run that does what the prompt asks -- stops before its bound and writes
/// its handoff -- is not finished work: the task goes back to the queue with
/// the handoff, not to review with "what is pending" filed as a deliverable.
#[tokio::test]
async fn a_run_that_hands_off_in_time_goes_back_to_the_queue_with_its_handoff() {
    let env = build_env(HANDING_OFF_STUB, None, |_| {}).await;
    let agent_id = hire(&env, "Careful", "builder").await;
    let task_id = make_todo_task(&env, "Half a day of work").await;

    let session = attempt(&env, &task_id, &agent_id).await;
    assert_eq!(session["status"], "completed");
    assert_eq!(task_status(&env, &task_id).await, "todo");
    let handoff = handoffs(&env, &task_id).await;
    assert_eq!(handoff.len(), 1, "{handoff:?}");
    assert!(
        handoff[0].contains("pending: the second half"),
        "{}",
        handoff[0]
    );
    let (documents,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM task_artifacts WHERE task_id = ? AND kind <> 'handoff'",
    )
    .bind(&task_id)
    .fetch_one(&env.state.pool)
    .await
    .expect("count");
    assert_eq!(
        documents, 0,
        "the handoff must not be filed as a deliverable"
    );
}

// --- a run owns a clone (ADR-0052) ------------------------------------------

/// Where a planted command would leave its mark. Under `target/`, outside
/// every path the cage grants -- temp is granted, so a marker there could be
/// written by a caged command and would prove nothing about the server.
fn marker_outside_the_cage() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("om-pwned-{}", uuid::Uuid::now_v7()))
}

/// An environment whose repository and data dir are outside every path the
/// cage grants, so that what the cage refuses is visible. The stub stays in
/// temp: the cage has to be able to read it.
async fn build_env_outside_the_cage(
    stub: &str,
    tweak: impl FnOnce(&mut overmind_server::Config),
) -> Env {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("om-root-{}", uuid::Uuid::now_v7()));
    let script_path = unique_root().join("stub-agent.sh");
    build_env_in(root, script_path, stub, None, tweak).await
}

/// Is anything holding agents on this machine? What these tests prove is what
/// a cage refuses; where there is none they have nothing to say.
fn no_cage_here(env: &Env) -> bool {
    let said = overmind_server::sandbox::announce(&env.state.config);
    if said.contains("read-only") {
        eprintln!("skipped: no cage on this machine ({said})");
        return true;
    }
    false
}

fn refs_of(repo: &std::path::Path) -> String {
    sh(repo, "git for-each-ref --format='%(refname) %(objectname)'")
}

/// The operator's repository is not in the cage (ADR-0052). With the
/// repositories dir set, as it always is in the image, the agent could write
/// `<repo>/.git`: a hook planted there ran as the server at the next run's
/// checkout -- and as the owner, on the host, the next time they used git.
#[tokio::test]
async fn a_hook_planted_in_the_operators_repository_is_impossible() {
    let marker = marker_outside_the_cage();
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("om-root-{}", uuid::Uuid::now_v7()));
    let repo = root.join("repo");
    let stub = format!(
        r#"#!/bin/sh
printf '#!/bin/sh\ntouch "{marker}"\n' > "{repo}/.git/hooks/post-checkout"
chmod +x "{repo}/.git/hooks/post-checkout"
printf '[alias]\n\tplanted = true\n' >> "{repo}/.git/config"
echo planted
"#,
        marker = marker.display(),
        repo = repo.display(),
    );
    let repos_dir = root.clone();
    let env = build_env_in(
        root,
        unique_root().join("stub-agent.sh"),
        &stub,
        None,
        |c| c.repos_dir = Some(repos_dir),
    )
    .await;
    if no_cage_here(&env) {
        return;
    }
    let agent_id = hire(&env, "Hostile", "builder").await;
    let first = make_todo_task(&env, "Plants a hook").await;
    attempt(&env, &first, &agent_id).await;
    // A second run on the same repository: its checkout is where a planted
    // hook would have run.
    let second = make_todo_task(&env, "Runs after it").await;
    attempt(&env, &second, &agent_id).await;

    assert!(
        !repo.join(".git/hooks/post-checkout").exists(),
        "the agent wrote a hook into the operator's repository"
    );
    let config = std::fs::read_to_string(repo.join(".git/config")).expect("config");
    assert!(
        !config.contains("planted"),
        "the agent wrote the operator's git config:\n{config}"
    );
    assert!(!marker.exists(), "a planted hook ran outside the cage");
}

/// What a run committed reaches the operator's repository as its branch, and
/// nothing else of the run does.
#[tokio::test]
async fn the_runs_commits_come_back_as_its_branch() {
    let stub = r#"#!/bin/sh
echo work > work.txt
git add work.txt
git commit -qm "the agent's work"
echo '{"model":"stub","total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1}}'
"#;
    let env = build_env(stub, None, |_| {}).await;
    let agent_id = hire(&env, "Committer", "builder").await;
    let task_id = make_todo_task(&env, "Commits its work").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    assert_eq!(session["status"], "completed", "{session}");
    let branch = session["branch"].as_str().expect("branch");
    let run_dir = PathBuf::from(session["workspace_path"].as_str().expect("run dir"));

    let repo = env.root.join("repo");
    let tip = sh(&run_dir, &format!("git rev-parse refs/heads/{branch}"));
    let back = sh(&repo, &format!("git rev-parse refs/heads/{branch}"));
    assert_eq!(back, tip, "the branch is not at the run's commit");
    assert_eq!(sh(&repo, &format!("git show {branch}:work.txt")), "work");
}

/// The bundle is the agent's, so it can name any ref it likes. Only the run's
/// own branch, under `overmind/`, is taken from it.
#[tokio::test]
async fn a_bundle_cannot_move_a_ref_outside_overmind() {
    let stub = r#"#!/bin/sh
echo evil > evil.txt
git add evil.txt
git commit -qm evil
git update-ref refs/heads/main HEAD
git tag planted HEAD
git branch -f overmind/elsewhere HEAD
"#;
    let env = build_env(stub, None, |_| {}).await;
    let repo = env.root.join("repo");
    let main_before = sh(&repo, "git rev-parse refs/heads/main");
    let agent_id = hire(&env, "Reacher", "builder").await;
    let task_id = make_todo_task(&env, "Reaches for main").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    let branch = session["branch"].as_str().expect("branch");

    assert_eq!(sh(&repo, "git rev-parse refs/heads/main"), main_before);
    let refs = refs_of(&repo);
    let names: Vec<&str> = refs
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(
        names,
        vec![
            "refs/heads/main".to_string(),
            format!("refs/heads/{branch}")
        ],
        "{refs}"
    );
}

/// An agent that repoints its clone's `.git` at a git dir of its own, whose
/// config names `key = value`, then makes one real change. `marker` is where
/// the planted command would write if the server ran it.
fn planting_stub(marker: &std::path::Path, key: &str, extra: &str) -> String {
    format!(
        r#"#!/bin/sh
echo change > changed.txt
printf '#!/bin/sh\ntouch "{marker}"\ncat "$@"\n' > planted.sh
chmod +x planted.sh
git init -q --bare evil.git
git --git-dir=evil.git fetch -q "$PWD" '+refs/heads/*:refs/heads/*'
git --git-dir=evil.git config core.bare false
git --git-dir=evil.git config {key} "$PWD/planted.sh"
{extra}
rm -rf .git
printf 'gitdir: %s/evil.git\n' "$PWD" > .git
"#,
        marker = marker.display()
    )
}

/// Run `stub` as a code task, then ask for its diff as a person opening it
/// would.
async fn diff_after(stub: &str) -> (Value, String) {
    let env = build_env(stub, None, |_| {}).await;
    let agent_id = hire(&env, "Hostile", "builder").await;
    let task_id = make_todo_task(&env, "Plants a git command").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    let session_id = session["id"].as_str().expect("session id");
    let diff = overmind_server::runner::session_diff(&env.state, session_id)
        .await
        .unwrap_or_else(|e| format!("diff refused: {e}"));
    (session, diff)
}

/// The clone's `.git` is in the run dir, which the agent owns: pointed at a
/// git dir it made, its config would make the server's own git run a command.
#[tokio::test]
async fn a_planted_gitdir_does_not_run_as_the_server() {
    let marker = marker_outside_the_cage();
    let (_, diff) = diff_after(&planting_stub(&marker, "core.fsmonitor", "")).await;
    assert!(!marker.exists(), "the planted fsmonitor ran as the server");
    assert!(diff.contains("changed.txt"), "{diff}");
}

/// The same door through `git diff`: a textconv driver named in the planted
/// config and switched on by a `.gitattributes` the agent wrote.
#[tokio::test]
async fn a_textconv_driver_in_the_clone_does_not_run() {
    let marker = marker_outside_the_cage();
    let (_, diff) = diff_after(&planting_stub(
        &marker,
        "diff.planted.textconv",
        "echo '* diff=planted' > .gitattributes",
    ))
    .await;
    assert!(!marker.exists(), "the planted textconv ran as the server");
    assert!(diff.contains("changed.txt"), "{diff}");
}

/// No pointer needed: the clone's own config is the agent's, and a clean
/// filter there runs whenever git reads the working tree. No list of keys
/// closes this. It runs -- in the cage, with the agent's reach and no more.
#[tokio::test]
async fn a_clean_filter_in_the_clones_config_runs_only_in_the_cage() {
    let marker = marker_outside_the_cage();
    let stub = format!(
        r#"#!/bin/sh
echo change > changed.txt
printf '#!/bin/sh\ntouch "{marker}"\ntouch "%s/ran-in-the-cage"\ncat\n' "$(git rev-parse --absolute-git-dir)" > planted.sh
chmod +x planted.sh
git config filter.planted.clean "$PWD/planted.sh"
echo '* filter=planted' > .gitattributes
"#,
        marker = marker.display()
    );
    let (session, diff) = diff_after(&stub).await;
    assert!(
        !marker.exists(),
        "the planted clean filter ran as the server"
    );
    assert!(diff.contains("changed.txt"), "{diff}");
    let run_dir = PathBuf::from(session["workspace_path"].as_str().expect("run dir"));
    assert!(
        run_dir.join(".git/ran-in-the-cage").exists(),
        "the filter never ran at all, so this proved nothing"
    );
}

/// A session from before the clone is a worktree of the operator's repository,
/// whose git dir lives there -- outside the cage now. Its diff still opens.
#[tokio::test]
async fn a_session_from_before_the_clone_still_diffs() {
    let env = build_env_outside_the_cage(HAPPY_STUB, |_| {}).await;
    if no_cage_here(&env) {
        return;
    }
    let agent_id = hire(&env, "Old", "builder").await;
    let task_id = make_todo_task(&env, "Recorded before").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    let session_id = session["id"].as_str().expect("session id");
    let run_dir = PathBuf::from(session["workspace_path"].as_str().expect("run dir"));
    let (base,): (Option<String>,) =
        sqlx::query_as("SELECT base_sha FROM agent_task_sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&env.state.pool)
            .await
            .expect("base");
    let base = base.expect("started");

    // Make it what a session from before was: a worktree of the operator's
    // repository, registered there under the session's own id.
    let repo = env.root.join("repo");
    std::fs::remove_dir_all(&run_dir).expect("drop the run dir");
    sh(
        &repo,
        &format!(
            "git worktree prune && git worktree add -q --detach '{}' {base}",
            run_dir.display()
        ),
    );
    std::fs::write(run_dir.join("result.txt"), "done by agent\n").expect("a change");

    let diff = overmind_server::runner::session_diff(&env.state, session_id)
        .await
        .expect("a session from before still diffs");
    assert!(diff.contains("result.txt"), "{diff}");
}

/// A run stopped at its deadline gets a handoff, and the handoff's `git
/// status` reads the clone the agent configured: it runs in the cage too.
#[tokio::test]
async fn the_handoff_status_runs_in_the_cage() {
    let marker = marker_outside_the_cage();
    let stub = format!(
        r#"#!/bin/sh
printf '#!/bin/sh\ntouch "{marker}"\n' > planted.sh
chmod +x planted.sh
git config core.fsmonitor "$PWD/planted.sh"
echo draft > notes.md
sleep 30
"#,
        marker = marker.display()
    );
    let env = build_env(&stub, None, |c| c.session_timeout_secs = 1).await;
    let agent_id = hire(&env, "Slowpoke", "builder").await;
    let task_id = make_todo_task(&env, "Never finishes").await;
    attempt(&env, &task_id, &agent_id).await;

    assert!(!marker.exists(), "the planted fsmonitor ran as the server");
    let handoff = handoffs(&env, &task_id).await;
    assert_eq!(handoff.len(), 1, "{handoff:?}");
    assert!(handoff[0].contains("notes.md"), "{}", handoff[0]);
}

/// Where the run itself had no cage, the agent already worked with the
/// server's reach, and the server's git over its files reaches no further: the
/// diff opens and the branch comes back.
#[tokio::test]
async fn a_run_without_a_cage_still_diffs_and_brings_its_branch_back() {
    let stub = r#"#!/bin/sh
echo work > work.txt
git add work.txt
git commit -qm "the agent's work"
echo more > more.txt
"#;
    let env = build_env(stub, None, |c| c.sandbox = false).await;
    let agent_id = hire(&env, "Uncaged", "builder").await;
    let task_id = make_todo_task(&env, "Works without a cage").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    let session_id = session["id"].as_str().expect("session id");
    let branch = session["branch"].as_str().expect("branch");

    let diff = overmind_server::runner::session_diff(&env.state, session_id)
        .await
        .expect("the diff of an uncaged run opens");
    assert!(diff.contains("more.txt"), "{diff}");
    assert_eq!(
        sh(
            &env.root.join("repo"),
            &format!("git show {branch}:work.txt")
        ),
        "work"
    );
}

/// A run that was caged is not read uncaged because the cage was turned off
/// afterwards: its clone was the agent's, and whatever it planted would run
/// as the server.
#[tokio::test]
async fn a_caged_runs_files_are_not_read_once_the_cage_is_off() {
    let db = unique_root();
    std::fs::create_dir_all(&db).expect("db dir");
    let url = format!("sqlite://{}?mode=rwc", db.join("overmind.sqlite").display());
    let env = build_env(HAPPY_STUB, Some(url.clone()), |_| {}).await;
    if no_cage_here(&env) {
        return;
    }
    let agent_id = hire(&env, "Caged", "builder").await;
    let task_id = make_todo_task(&env, "Caged work").await;
    let session = attempt(&env, &task_id, &agent_id).await;
    let session_id = session["id"].as_str().expect("session id");

    let uncaged = overmind_server::init_with(
        &url,
        overmind_server::Config {
            sandbox: false,
            data_dir: env.root.join("data"),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("the same database, the cage off");
    let refused = overmind_server::runner::session_diff(&uncaged, session_id)
        .await
        .expect_err("a caged run's diff is refused without a cage");
    assert!(refused.to_string().contains("no cage"), "{refused}");
}

/// The branch goes back to the repository the clone was made from, not to
/// whichever one the task points at when the run ends.
#[tokio::test]
async fn the_branch_goes_back_where_the_clone_came_from() {
    let stub = r#"#!/bin/sh
touch started
sleep 1
echo work > work.txt
git add work.txt
git commit -qm "the agent's work"
"#;
    let env = build_env(stub, None, |_| {}).await;
    let agent_id = hire(&env, "Mover", "builder").await;
    let task_id = make_todo_task(&env, "Outlives a move").await;
    let (status, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    let run_dir = PathBuf::from(started["workspace_path"].as_str().expect("run dir"));
    for _ in 0..100 {
        if run_dir.join("started").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // While it runs, the workspace is pointed at another repository.
    let elsewhere = env.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("dir");
    sh(
        &elsewhere,
        "git init -q -b main && git -c user.email=t@t -c user.name=T commit -q --allow-empty -m other",
    );
    sqlx::query("UPDATE project_workspaces SET cwd = ?")
        .bind(elsewhere.to_string_lossy().into_owned())
        .execute(&env.state.pool)
        .await
        .expect("repoint the workspace");

    let session_id = started["session_id"].as_str().expect("session id");
    let session = wait_for_session(&env.app, session_id).await;
    let branch = session["branch"].as_str().expect("branch");
    assert_eq!(
        sh(
            &env.root.join("repo"),
            &format!("git show {branch}:work.txt")
        ),
        "work",
        "{session}"
    );
}
