//! End-to-end API tests for the M1 acceptance criteria:
//! - tasks move through their lifecycle via the API
//! - the audit log replays the full history
//! - tampering with an event breaks chain verification

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn setup() -> (axum::Router, overmind_server::AppState) {
    let state = overmind_server::init("sqlite::memory:")
        .await
        .expect("init in-memory db");
    (overmind_server::app(state.clone()), state)
}

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
        serde_json::from_slice(&bytes).expect("body is JSON")
    };
    (status, value)
}

#[tokio::test]
async fn full_lifecycle_with_audit_trail() {
    let (app, _state) = setup().await;

    // Company
    let (status, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Acme" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let company_id = company["id"].as_str().expect("company id").to_string();

    // Both catalogs are seeded. Since ADR-0021 they are two axes: the
    // archetype is the *function*, the domain is the *field* it works in — so
    // a security reviewer is `reviewer × security`, not a row of its own.
    let (status, catalog) = send(&app, "GET", "/api/archetypes", None).await;
    assert_eq!(status, StatusCode::OK);
    let slugs: Vec<&str> = catalog["archetypes"]
        .as_array()
        .expect("archetypes array")
        .iter()
        .map(|a| a["slug"].as_str().expect("slug"))
        .collect();
    assert!(slugs.contains(&"reviewer"), "catalog: {slugs:?}");

    let (status, catalog) = send(&app, "GET", "/api/domains", None).await;
    assert_eq!(status, StatusCode::OK);
    let domains: Vec<&str> = catalog["domains"]
        .as_array()
        .expect("domains array")
        .iter()
        .map(|d| d["slug"].as_str().expect("slug"))
        .collect();
    assert!(domains.contains(&"security"), "domains: {domains:?}");
    assert!(domains.contains(&"media-av"), "domains: {domains:?}");

    // Hire an agent: archetype defaults + structured override (ADR-0005)
    let (status, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents"),
        Some(json!({
            "name": "Sentinel",
            "archetype": "reviewer",
            "domain": "security",
            "traits": { "focus_areas": ["auth", "secrets-handling"] },
            "custom_brief": "Pay special attention to the audit log code."
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "hire failed: {agent}");
    let agent_id = agent["id"].as_str().expect("agent id").to_string();
    // Overridden field takes the patch, untouched fields keep archetype defaults
    assert_eq!(
        agent["traits"]["focus_areas"],
        json!(["auth", "secrets-handling"])
    );
    assert_eq!(agent["traits"]["autonomy"], "propose_only");
    assert_eq!(agent["traits"]["review_strictness"], "strict");

    // Unknown archetype is a 404, not a silent default
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents"),
        Some(json!({ "name": "X", "archetype": "does-not-exist" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Project -> goal -> task cascade
    let (status, project) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/projects"),
        Some(json!({ "title": "Ship M1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let project_id = project["id"].as_str().expect("project id").to_string();

    let (status, goal) = send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/goals"),
        Some(json!({ "title": "Audit log shipped" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let goal_id = goal["id"].as_str().expect("goal id").to_string();

    let (status, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({ "title": "Implement hash chain", "goal_id": goal_id, "priority": "high" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(task["status"], "backlog");
    assert_eq!(task["priority"], "high");
    let task_id = task["id"].as_str().expect("task id").to_string();

    // Lifecycle: backlog -> todo -> in_progress (assign) -> in_review -> done
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, t) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "in_progress", "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "transition failed: {t}");
    assert_eq!(t["assignee_agent_id"], json!(agent_id));

    // Invalid transition is rejected with 400 (done requires review first)
    let (status, err) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "done" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"]
            .as_str()
            .expect("error msg")
            .contains("invalid transition"),
        "unexpected error: {err}"
    );

    for to in ["in_review", "done"] {
        let (status, _) = send(
            &app,
            "POST",
            &format!("/api/tasks/{task_id}/transition"),
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "transition to {to} failed");
    }

    // Terminal status: no way out
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "in_progress" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The audit log replays the full history, in order
    let (status, events) = send(
        &app,
        "GET",
        &format!("/api/audit/events?company_id={company_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|e| e["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(
        kinds,
        vec![
            "company.created",
            // The founding CEO (M15): a company is never empty.
            "agent.hired",
            "agent.hired",
            "project.created",
            "goal.created",
            "task.created",
            "task.transitioned",
            "task.transitioned",
            "task.transitioned",
            "task.transitioned",
        ]
    );

    // Chain verifies end to end
    let (status, report) = send(&app, "GET", "/api/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["valid"], json!(true), "report: {report}");
    assert_eq!(report["events_checked"], json!(10));
}

#[tokio::test]
async fn blocked_path_roundtrip() {
    let (app, _state) = setup().await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Blockers" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({ "title": "Waits on upstream" })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();

    // backlog -> todo -> blocked -> in_progress
    for to in ["todo", "blocked", "in_progress"] {
        let (status, body) = send(
            &app,
            "POST",
            &format!("/api/tasks/{task_id}/transition"),
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "to {to}: {body}");
    }

    // blocked is not reachable from review-less terminal attempts
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "done" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tampering_with_an_event_breaks_the_chain() {
    let (app, state) = setup().await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Tamperproof Inc" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, _) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/projects"),
        Some(json!({ "title": "Real project" })),
    )
    .await;

    // Sane before tampering
    let (_, report) = send(&app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));

    // The append-only triggers block mutation through the SQL surface...
    let blocked = sqlx::query("UPDATE audit_events SET payload = '{}' WHERE seq = 1")
        .execute(&state.pool)
        .await;
    assert!(blocked.is_err(), "append-only trigger should block UPDATE");

    // ...so simulate an attacker with raw file access: drop the trigger, rewrite history.
    sqlx::query("DROP TRIGGER audit_events_no_update")
        .execute(&state.pool)
        .await
        .expect("drop trigger");
    sqlx::query("UPDATE audit_events SET payload = '{\"name\":\"Innocent Co\"}' WHERE seq = 1")
        .execute(&state.pool)
        .await
        .expect("tamper with event");

    // The hash chain catches it, pointing at the exact event
    let (status, report) = send(&app, "GET", "/api/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["valid"], json!(false));
    assert_eq!(report["first_invalid_seq"], json!(1));
}

/// Deleting a company takes every row it owns -- tasks, agents, projects,
/// the lot -- and returns 404 for whoever asks again. The audit chain is
/// deliberately NOT thinned: history that a company existed, worked and was
/// deleted is exactly what an audit log is for, and the deletion itself is
/// the chain's newest event.
#[tokio::test]
async fn deleting_a_company_takes_its_rows_and_leaves_the_audit_chain_whole() {
    let (app, state) = setup().await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Ephemeral" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();

    // Give it a life worth deleting: project -> goal -> task.
    let (_, project) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/projects"),
        Some(json!({ "title": "Short lived" })),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id").to_string();
    let (_, goal) = send(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/goals"),
        Some(json!({ "title": "Be deleted cleanly" })),
    )
    .await;
    let goal_id = goal["id"].as_str().expect("goal id").to_string();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({ "title": "Leave no orphans", "goal_id": goal_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(
        &app,
        "DELETE",
        &format!("/api/companies/{company_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete failed: {body}");

    // The list no longer names it, and the rows are truly gone.
    let (_, v) = send(&app, "GET", "/api/companies", None).await;
    assert_eq!(v["companies"].as_array().map(Vec::len), Some(0), "{v}");
    for table in ["companies", "tasks", "projects", "agents", "conversations"] {
        let (n,): (i64,) = sqlx::query_as(&format!(
            "SELECT COUNT(*) FROM {table} WHERE {} = ?",
            if table == "companies" {
                "id"
            } else {
                "company_id"
            }
        ))
        .bind(&company_id)
        .fetch_one(&state.pool)
        .await
        .expect("count");
        assert_eq!(n, 0, "{table} still holds rows for the deleted company");
    }

    // The audit chain still verifies, and its newest company event says why.
    let (status, report) = send(&app, "GET", "/api/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["valid"], json!(true), "report: {report}");
    let (_, v) = send(
        &app,
        "GET",
        &format!("/api/audit/events?company_id={company_id}"),
        None,
    )
    .await;
    let kinds: Vec<&str> = v["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["kind"].as_str().expect("kind"))
        .collect();
    assert!(
        kinds.contains(&"company.deleted"),
        "the deletion is an audit event: {kinds:?}"
    );

    // Asking again finds nothing: the verb is not idempotent theater.
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/companies/{company_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A session still queued or running is an agent mid-thought: deleting the
/// company under it would leave the runner writing into a void. The door
/// holds with 409 until the work settles.
#[tokio::test]
async fn a_live_session_holds_the_door_against_deletion() {
    let (app, state) = setup().await;

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Busy" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents"),
        Some(json!({ "name": "Worker", "archetype": "reviewer" })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent id").to_string();
    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({ "title": "In flight" })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();

    sqlx::query(
        "INSERT INTO agent_task_sessions
             (id, task_id, agent_id, status, branch, workspace_path, created_at)
         VALUES ('sess-live', ?, ?, 'running', 'work', '/tmp/nowhere', '2026-08-22T00:00:00Z')",
    )
    .bind(&task_id)
    .bind(&agent_id)
    .execute(&state.pool)
    .await
    .expect("insert running session");

    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/companies/{company_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Once the session settles, the same request goes through.
    sqlx::query("UPDATE agent_task_sessions SET status = 'completed' WHERE id = 'sess-live'")
        .execute(&state.pool)
        .await
        .expect("settle session");
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/companies/{company_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// The company's corner of the data dir goes with it: the ROADMAP names the
/// wound -- cleaning up used to mean surgery on the volume.
#[tokio::test]
async fn deleting_a_company_sweeps_its_directory() {
    let data_dir =
        std::env::temp_dir().join(format!("overmind-del-{}", uuid::Uuid::now_v7().simple()));
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = overmind_server::app(state);

    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Dusty" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();

    // Whether or not a brain was provisioned, the corner may hold files.
    let corner = data_dir.join("companies").join(&company_id);
    std::fs::create_dir_all(corner.join("brain")).expect("lay a corner");

    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/companies/{company_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !corner.exists(),
        "the company's directory should be swept: {corner:?}"
    );
}
