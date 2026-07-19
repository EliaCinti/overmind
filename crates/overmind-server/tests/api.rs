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
    let (status, company) = send(&app, "POST", "/companies", Some(json!({ "name": "Acme" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let company_id = company["id"].as_str().expect("company id").to_string();

    // Archetype catalog is seeded
    let (status, catalog) = send(&app, "GET", "/archetypes", None).await;
    assert_eq!(status, StatusCode::OK);
    let slugs: Vec<&str> = catalog["archetypes"]
        .as_array()
        .expect("archetypes array")
        .iter()
        .map(|a| a["slug"].as_str().expect("slug"))
        .collect();
    assert!(slugs.contains(&"security-engineer"), "catalog: {slugs:?}");

    // Hire an agent: archetype defaults + structured override (ADR-0005)
    let (status, agent) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/agents"),
        Some(json!({
            "name": "Sentinel",
            "archetype": "security-engineer",
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
        &format!("/companies/{company_id}/agents"),
        Some(json!({ "name": "X", "archetype": "does-not-exist" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Project -> goal -> task cascade
    let (status, project) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/projects"),
        Some(json!({ "title": "Ship M1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let project_id = project["id"].as_str().expect("project id").to_string();

    let (status, goal) = send(
        &app,
        "POST",
        &format!("/projects/{project_id}/goals"),
        Some(json!({ "title": "Audit log shipped" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let goal_id = goal["id"].as_str().expect("goal id").to_string();

    let (status, task) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/tasks"),
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
        &format!("/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, t) = send(
        &app,
        "POST",
        &format!("/tasks/{task_id}/transition"),
        Some(json!({ "to": "in_progress", "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "transition failed: {t}");
    assert_eq!(t["assignee_agent_id"], json!(agent_id));

    // Invalid transition is rejected with 400 (done requires review first)
    let (status, err) = send(
        &app,
        "POST",
        &format!("/tasks/{task_id}/transition"),
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
            &format!("/tasks/{task_id}/transition"),
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "transition to {to} failed");
    }

    // Terminal status: no way out
    let (status, _) = send(
        &app,
        "POST",
        &format!("/tasks/{task_id}/transition"),
        Some(json!({ "to": "in_progress" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The audit log replays the full history, in order
    let (status, events) = send(
        &app,
        "GET",
        &format!("/audit/events?company_id={company_id}"),
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
    let (status, report) = send(&app, "GET", "/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["valid"], json!(true), "report: {report}");
    assert_eq!(report["events_checked"], json!(9));
}

#[tokio::test]
async fn blocked_path_roundtrip() {
    let (app, _state) = setup().await;

    let (_, company) = send(
        &app,
        "POST",
        "/companies",
        Some(json!({ "name": "Blockers" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, task) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/tasks"),
        Some(json!({ "title": "Waits on upstream" })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();

    // backlog -> todo -> blocked -> in_progress
    for to in ["todo", "blocked", "in_progress"] {
        let (status, body) = send(
            &app,
            "POST",
            &format!("/tasks/{task_id}/transition"),
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "to {to}: {body}");
    }

    // blocked is not reachable from review-less terminal attempts
    let (status, _) = send(
        &app,
        "POST",
        &format!("/tasks/{task_id}/transition"),
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
        "/companies",
        Some(json!({ "name": "Tamperproof Inc" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();
    let (_, _) = send(
        &app,
        "POST",
        &format!("/companies/{company_id}/projects"),
        Some(json!({ "title": "Real project" })),
    )
    .await;

    // Sane before tampering
    let (_, report) = send(&app, "GET", "/audit/verify", None).await;
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
    let (status, report) = send(&app, "GET", "/audit/verify", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["valid"], json!(false));
    assert_eq!(report["first_invalid_seq"], json!(1));
}
