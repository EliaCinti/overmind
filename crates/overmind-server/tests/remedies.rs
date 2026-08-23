//! An error Overmind can repair arrives with the repair (ADR-0038 addendum).
//!
//! The owner's principle, in his own words: when the problem shows up, the
//! interface proposes the fix; the user approves; Overmind acts. First case:
//! a start refused because the task carries visual material and the agent is
//! not characterized for it — the refusal names the remedy, and a general
//! traits endpoint applies it.

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

async fn upload_image(app: &axum::Router, uri: &str, filename: &str) -> Value {
    const BOUNDARY: &str = "----overmindtestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: image/jpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"not really a jpeg");
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

async fn setup() -> (axum::Router, String) {
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: std::env::temp_dir()
                .join(format!("overmind-remedy-{}", uuid::Uuid::now_v7().simple())),
            agent_cmd: Some("/usr/bin/true".into()),
            heartbeat_ms: 1_000_000,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = overmind_server::app(state);
    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Remedy Co" })),
    )
    .await;
    (app, co["id"].as_str().expect("id").to_string())
}

/// The refusal carries a machine-readable remedy; the traits endpoint applies
/// it (validated, revisioned); the retry goes through.
#[tokio::test]
async fn a_refused_start_names_its_remedy_and_the_remedy_works() {
    let (app, company) = setup().await;
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company}/agents"),
        Some(json!({ "name": "Tobia", "archetype": "writer",
                     "traits": { "multimodal": false } })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent").to_string();

    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company}/tasks"),
        Some(json!({ "title": "Look at the sketch", "execution_kind": "knowledge" })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task").to_string();
    upload_image(
        &app,
        &format!("/api/tasks/{task_id}/attachments"),
        "sketch.jpeg",
    )
    .await;
    send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;

    // Refused — with the remedy, not just the sentence.
    let (s, v) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["remedy"]["kind"], json!("grant_multimodal"), "{v}");
    assert_eq!(v["remedy"]["agent_id"], json!(agent_id), "{v}");

    // The remedy: one validated traits patch, recorded like any other change.
    let (s, v) = send(
        &app,
        "POST",
        &format!("/api/agents/{agent_id}/traits"),
        Some(json!({ "multimodal": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["traits"]["multimodal"], json!(true));
    let (_, revs) = send(
        &app,
        "GET",
        &format!("/api/agents/{agent_id}/revisions"),
        None,
    )
    .await;
    assert!(
        revs["revisions"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|r| r["source"] == "patch"),
        "a revision records the change: {revs}"
    );

    // And now the same start goes through.
    let (s, v) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
}

/// The traits endpoint is still a gate, not a hole: an unknown tool or model
/// in the patch is refused exactly as at hire.
#[tokio::test]
async fn the_traits_endpoint_validates_like_the_hire() {
    let (app, company) = setup().await;
    let (_, agent) = send(
        &app,
        "POST",
        &format!("/api/companies/{company}/agents"),
        Some(json!({ "name": "Tobia", "archetype": "writer" })),
    )
    .await;
    let agent_id = agent["id"].as_str().expect("agent");
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/agents/{agent_id}/traits"),
        Some(json!({ "tools": ["lathe"] })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/agents/{agent_id}/traits"),
        Some(json!({ "model": "gpt-99" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
