//! M15 acceptance tests: a company is founded with a CEO; you tell it your
//! idea and it proposes a team; nothing is hired until you accept; and the
//! manual road — hiring everyone yourself — is never blocked by any of it.

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
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A CEO that answers an idea with a three-person team, two levels deep.
const PROPOSES_A_TEAM_STUB: &str = r#"#!/bin/sh
echo '{"reply":"Here is the team I would build.","tasks":[],"team":{"summary":"A home cinema needs one person on picture and sound, one on the room itself, and someone to keep the budget honest.","members":[{"name":"Vera","archetype":"researcher","title":"Media & A/V quality","why":"picks the projector and calibrates it"},{"name":"Bo","archetype":"technical-writer","title":"Room & acoustics","reports_to":"Vera","why":"writes up the treatment plan"},{"name":"Sam","archetype":"code-reviewer","title":"Budget control","why":"checks every choice against what you said you would spend"}]}}'
"#;

async fn setup(stub: &str) -> (axum::Router, String, String) {
    let root = std::env::temp_dir().join(format!("overmind-org-{}", uuid::Uuid::now_v7().simple()));
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
    let app = overmind_server::app(state);
    let (status, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Home Cinema Co" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let company_id = company["id"].as_str().expect("company").to_string();
    let ceo_id = company["ceo"]["id"].as_str().expect("ceo").to_string();
    (app, company_id, ceo_id)
}

#[tokio::test]
async fn a_company_is_founded_with_a_ceo() {
    let (app, company_id, ceo_id) = setup(PROPOSES_A_TEAM_STUB).await;

    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/agents"),
        None,
    )
    .await;
    let list = agents["agents"].as_array().expect("agents");
    assert_eq!(list.len(), 1, "exactly the CEO: {agents}");
    let ceo = &list[0];
    assert_eq!(ceo["id"], json!(ceo_id));
    assert_eq!(ceo["title"], json!("CEO"));
    assert_eq!(ceo["archetype"], json!("chief-executive"));
    assert!(ceo["reports_to"].is_null(), "the CEO is the root");
    assert!(
        !ceo["name"].as_str().unwrap_or("").is_empty(),
        "the system gives it a name: {ceo}"
    );
    assert_eq!(ceo["traits"]["model"], json!("claude-opus-4-8"));
    assert_eq!(ceo["traits"]["monthly_budget_cents"], json!(2000));
    // Within budget it may take on anything.
    let perms = ceo["traits"]["permissions"]
        .as_array()
        .expect("permissions");
    assert!(perms.iter().any(|p| p == "task:code"));
    assert!(perms.iter().any(|p| p == "task:knowledge"));
}

#[tokio::test]
async fn the_ceo_proposes_a_team_and_nothing_is_hired_until_you_accept() {
    let (app, company_id, _ceo) = setup(PROPOSES_A_TEAM_STUB).await;

    send(
        &app,
        "POST",
        &format!(
            "/api/companies/{company_id}/agents/{}/conversation/messages",
            _ceo
        ),
        Some(json!({ "content": "I want to build a home cinema in the new flat." })),
    )
    .await;

    // Wait for the proposal.
    let mut proposal = Value::Null;
    for _ in 0..100 {
        let (_, list) = send(
            &app,
            "GET",
            &format!("/api/companies/{company_id}/org-proposals"),
            None,
        )
        .await;
        if let Some(p) = list["proposals"].as_array().and_then(|a| a.first()) {
            proposal = p.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!proposal.is_null(), "the CEO never proposed a team");
    assert_eq!(proposal["status"], json!("proposed"));
    assert!(
        proposal["summary"]
            .as_str()
            .unwrap_or("")
            .contains("picture and sound"),
        "its reasoning reaches you: {proposal}"
    );

    let members = proposal["members"].as_array().expect("members");
    assert_eq!(members.len(), 3);
    assert_eq!(members[0]["name"], json!("Vera"));
    assert_eq!(members[0]["title"], json!("Media & A/V quality"));
    assert_eq!(members[1]["reports_to"], json!("Vera"), "two levels deep");
    assert!(
        members[0]["rationale"]
            .as_str()
            .unwrap_or("")
            .contains("projector"),
        "each hire says why: {}",
        members[0]
    );

    // Still only the CEO: a proposal hires nobody.
    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/agents"),
        None,
    )
    .await;
    assert_eq!(agents["agents"].as_array().map(Vec::len), Some(1));

    // Drop one before accepting.
    let sam = members
        .iter()
        .find(|m| m["name"] == json!("Sam"))
        .expect("Sam");
    let (s, _) = send(
        &app,
        "POST",
        &format!(
            "/api/org-proposals/{}/members/{}",
            proposal["id"].as_str().expect("id"),
            sam["id"].as_str().expect("member id")
        ),
        Some(json!({ "excluded": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Accept the rest.
    let (s, decided) = send(
        &app,
        "POST",
        &format!(
            "/api/approvals/{}/decision",
            proposal["approval_id"].as_str().expect("approval")
        ),
        Some(json!({ "decision": "approve" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "decision: {decided}");
    assert_eq!(
        decided["hired"],
        json!(2),
        "the dropped member is not hired"
    );

    // The tree is wired: Vera under the CEO, Bo under Vera, no Sam.
    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/agents"),
        None,
    )
    .await;
    let list = agents["agents"].as_array().expect("agents");
    assert_eq!(list.len(), 3, "CEO + Vera + Bo: {agents}");
    let by_name = |n: &str| list.iter().find(|a| a["name"] == json!(n)).cloned();
    let vera = by_name("Vera").expect("Vera hired");
    let bo = by_name("Bo").expect("Bo hired");
    assert!(
        by_name("Sam").is_none(),
        "the dropped member stayed dropped"
    );
    assert_eq!(vera["reports_to"], json!(_ceo), "Vera reports to the CEO");
    assert_eq!(bo["reports_to"], vera["id"], "Bo reports to Vera");
    assert_eq!(vera["title"], json!("Media & A/V quality"));

    let (_, report) = send(&app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn refusing_a_team_tells_the_ceo_why() {
    let (app, company_id, ceo) = setup(PROPOSES_A_TEAM_STUB).await;
    send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/agents/{ceo}/conversation/messages"),
        Some(json!({ "content": "Home cinema, please." })),
    )
    .await;
    let mut proposal = Value::Null;
    for _ in 0..100 {
        let (_, list) = send(
            &app,
            "GET",
            &format!("/api/companies/{company_id}/org-proposals"),
            None,
        )
        .await;
        if let Some(p) = list["proposals"].as_array().and_then(|a| a.first()) {
            proposal = p.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, _) = send(
        &app,
        "POST",
        &format!(
            "/api/approvals/{}/decision",
            proposal["approval_id"].as_str().expect("approval")
        ),
        Some(json!({ "decision": "reject", "note": "too many people, start with one" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, list) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/org-proposals"),
        None,
    )
    .await;
    let p = &list["proposals"][0];
    assert_eq!(p["status"], json!("rejected"));
    assert_eq!(p["decline_note"], json!("too many people, start with one"));

    // Nobody was hired, and the CEO is told why in its own inbox notification.
    let (_, agents) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/agents"),
        None,
    )
    .await;
    assert_eq!(agents["agents"].as_array().map(Vec::len), Some(1));
    let (_, inbox) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/notifications"),
        None,
    )
    .await;
    assert!(
        inbox["notifications"]
            .as_array()
            .expect("notifications")
            .iter()
            .any(|n| n["kind"] == json!("org.rejected")),
        "inbox: {inbox}"
    );
}

/// The other road: ignore the CEO entirely and build the org by hand.
#[tokio::test]
async fn you_can_build_the_team_yourself_instead() {
    let (app, company_id, ceo) = setup(PROPOSES_A_TEAM_STUB).await;

    let hire = |name: &'static str, mgr: Option<String>| {
        let app = app.clone();
        let company_id = company_id.clone();
        async move {
            let mut body = json!({ "name": name, "archetype": "researcher" });
            if let Some(m) = mgr {
                body["reports_to"] = json!(m);
            }
            let (s, a) = send(
                &app,
                "POST",
                &format!("/api/companies/{company_id}/agents"),
                Some(body),
            )
            .await;
            assert_eq!(s, StatusCode::CREATED);
            a
        }
    };
    let lead = hire("Lead", None).await;
    let lead_id = lead["id"].as_str().expect("id").to_string();
    let junior = hire("Junior", Some(lead_id.clone())).await;

    assert_eq!(
        lead["reports_to"],
        json!(ceo),
        "manager-less hire lands under the CEO"
    );
    assert_eq!(junior["reports_to"], json!(lead_id));

    // No proposal was ever involved.
    let (_, list) = send(
        &app,
        "GET",
        &format!("/api/companies/{company_id}/org-proposals"),
        None,
    )
    .await;
    assert!(list["proposals"].as_array().expect("proposals").is_empty());
}
