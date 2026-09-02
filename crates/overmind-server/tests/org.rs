//! M5 acceptance tests: reporting hierarchy (agent → manager), title/reporting
//! reassignment, and the reporting-DAG invariant (no cycles), all enforced
//! server-side.

mod common;

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

async fn app() -> axum::Router {
    // A data directory of its own: the setup code is written there at boot,
    // and `claimed` reads it the way the person at the machine would.
    let data_dir =
        std::env::temp_dir().join(format!("overmind-org-{}", uuid::Uuid::now_v7().simple()));
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init db");
    common::claimed(overmind_server::app(state), &data_dir).await
}

async fn hire(
    app: &axum::Router,
    company: &str,
    name: &str,
    reports_to: Option<&str>,
    title: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({ "name": name, "archetype": "builder" });
    if let Some(m) = reports_to {
        body["reports_to"] = json!(m);
    }
    if let Some(t) = title {
        body["title"] = json!(t);
    }
    send(
        app,
        "POST",
        &format!("/api/companies/{company}/agents"),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn builds_a_reporting_tree() {
    let app = app().await;
    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Org Co" })),
    )
    .await;
    let company = co["id"].as_str().expect("company id").to_string();
    // Since M15 the company is founded with a CEO, and it is the only root:
    // hiring without a manager attaches under it rather than creating a second.
    let ceo_id = co["ceo"]["id"].as_str().expect("founding CEO").to_string();
    assert_eq!(co["ceo"]["reports_to"], Value::Null, "the CEO is the root");

    // A lead hired with no manager given, and two reports under it.
    let (s, lead) = hire(&app, &company, "Lead", None, Some("Tech Lead")).await;
    assert_eq!(s, StatusCode::CREATED);
    let lead_id = lead["id"].as_str().expect("lead id").to_string();
    assert_eq!(lead["title"], "Tech Lead");
    assert_eq!(
        lead["reports_to"],
        json!(ceo_id),
        "an org has one root: a manager-less hire lands under the CEO"
    );

    let (s, dev) = hire(&app, &company, "Dev", Some(&lead_id), None).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(dev["reports_to"], json!(lead_id));

    // Hiring under a manager from another company is rejected.
    let (_, other) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Other" })),
    )
    .await;
    let other_id = other["id"].as_str().expect("id").to_string();
    let (s, _) = hire(&app, &other_id, "Stray", Some(&lead_id), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // The tree is visible through list_agents.
    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company}/agents"),
        None,
    )
    .await;
    let list = agents["agents"].as_array().expect("agents");
    assert_eq!(list.len(), 3, "CEO + Lead + Dev");
    let dev_row = list.iter().find(|a| a["name"] == "Dev").expect("dev");
    assert_eq!(dev_row["reports_to"], json!(lead_id));
}

#[tokio::test]
async fn reassignment_enforces_the_dag() {
    let app = app().await;
    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "DAG Co" })),
    )
    .await;
    let company = co["id"].as_str().expect("id").to_string();

    let (_, a) = hire(&app, &company, "A", None, None).await;
    let a_id = a["id"].as_str().expect("id").to_string();
    let (_, b) = hire(&app, &company, "B", Some(&a_id), None).await;
    let b_id = b["id"].as_str().expect("id").to_string();
    let (_, c) = hire(&app, &company, "C", Some(&b_id), None).await;
    let c_id = c["id"].as_str().expect("id").to_string();

    // A → B → C. Making A report to C would close the cycle A→C→B→A.
    let (s, err) = send(
        &app,
        "POST",
        &format!("/api/agents/{a_id}/reassign"),
        Some(json!({ "reports_to": c_id })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(
        err["error"].as_str().unwrap_or("").contains("cycle"),
        "err: {err}"
    );

    // Self-reporting is rejected.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/agents/{b_id}/reassign"),
        Some(json!({ "reports_to": b_id })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // A valid reassignment: move C up to report to A, and retitle it.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/agents/{c_id}/reassign"),
        Some(json!({ "reports_to": a_id, "title": "Senior Dev" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Clearing reports_to (explicit null) moves an agent to the top.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/agents/{b_id}/reassign"),
        Some(json!({ "reports_to": null })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company}/agents"),
        None,
    )
    .await;
    let list = agents["agents"].as_array().expect("agents");
    let c_row = list.iter().find(|a| a["name"] == "C").expect("c");
    assert_eq!(c_row["reports_to"], json!(a_id));
    assert_eq!(c_row["title"], "Senior Dev");
    let b_row = list.iter().find(|a| a["name"] == "B").expect("b");
    assert_eq!(b_row["reports_to"], Value::Null);

    // The audit chain still verifies after all the org mutations.
    let (_, report) = send(&app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

/// M15: the org is a tree rooted at the founding CEO, of any depth. Everyone
/// reaches the CEO by walking up; nobody is a second root.
#[tokio::test]
async fn the_org_is_a_tree_rooted_at_the_ceo() {
    let app = app().await;
    let (_, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Deep Co" })),
    )
    .await;
    let company = co["id"].as_str().expect("company id").to_string();
    let ceo = co["ceo"]["id"].as_str().expect("founding CEO").to_string();

    // CEO → Director → Lead → Engineer: four levels, built explicitly.
    let (_, director) = hire(&app, &company, "Director", Some(&ceo), Some("Director")).await;
    let director_id = director["id"].as_str().expect("id").to_string();
    let (_, lead) = hire(&app, &company, "Lead", Some(&director_id), Some("Lead")).await;
    let lead_id = lead["id"].as_str().expect("id").to_string();
    let (s, eng) = hire(&app, &company, "Engineer", Some(&lead_id), None).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(eng["reports_to"], json!(lead_id));

    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company}/agents"),
        None,
    )
    .await;
    let list = agents["agents"].as_array().expect("agents");
    assert_eq!(list.len(), 4, "CEO + three levels under it");

    // Exactly one root, and it is the CEO.
    let roots: Vec<&str> = list
        .iter()
        .filter(|a| a["reports_to"].is_null())
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(roots, vec![ceo.as_str()], "one root, the CEO");

    // Every agent reaches the CEO by walking up its managers.
    let manager_of = |id: &str| -> Option<String> {
        list.iter()
            .find(|a| a["id"] == json!(id))
            .and_then(|a| a["reports_to"].as_str())
            .map(str::to_string)
    };
    for a in list.iter() {
        let id = a["id"].as_str().expect("id");
        let mut cursor = id.to_string();
        let mut hops = 0;
        while let Some(mgr) = manager_of(&cursor) {
            cursor = mgr;
            hops += 1;
            assert!(hops < 10, "walking up {id} did not terminate");
        }
        assert_eq!(cursor, ceo, "{id} does not reach the CEO");
    }

    // And a cycle is still refused, however deep: the CEO cannot report to a leaf.
    let eng_id = eng["id"].as_str().expect("id");
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/agents/{ceo}/reassign"),
        Some(json!({ "reports_to": eng_id })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "that would close a cycle");
}
