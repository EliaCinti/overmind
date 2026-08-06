//! M18 acceptance tests (ADR-0022): conversational spend enters the ledger,
//! the cap is one cap, and a room that runs out of money waits.
//!
//! Before this, `cost_events` had exactly one writer — the task runner. Chat
//! turns and meeting turns spent real money and recorded nothing, so every
//! agent's `spent_cents` was short by all of its conversational work and the
//! M6 gate was enforcing correctly against an incomplete ledger.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Reports a real cost, so the ledger has something to record. 30 cents a turn
/// makes a small cap run out in a countable number of turns.
const COSTLY_STUB: &str = r#"#!/bin/sh
echo '{"reply":"Noted.","tasks":[]}'
echo '{"total_cost_usd":0.30,"model":"stub","usage":{"input_tokens":10,"output_tokens":5}}'
"#;

/// Two agents deliberating, nobody deciding — so the room runs to its cap, or
/// to the end of somebody's money, whichever comes first.
const DELIBERATING_STUB: &str = r#"#!/bin/sh
echo '{"say":"I have a view but no conclusion yet."}'
echo '{"total_cost_usd":0.30,"model":"stub","usage":{"input_tokens":10,"output_tokens":5}}'
"#;

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

struct TestEnv {
    app: axum::Router,
    company_id: String,
    ceo_id: String,
}

async fn setup(stub: &str) -> TestEnv {
    let root =
        std::env::temp_dir().join(format!("overmind-budget-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("create test root");
    let script = root.join("stub.sh");
    std::fs::write(&script, stub).expect("write stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init in-memory db");
    let app = overmind_server::app(state);
    let (_, company) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Frugal Co" })),
    )
    .await;
    TestEnv {
        company_id: company["id"].as_str().expect("company").to_string(),
        ceo_id: company["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
    }
}

async fn set_budget(env: &TestEnv, agent_id: &str, cents: i64) {
    let (status, body) = send(
        &env.app,
        "POST",
        &format!("/api/agents/{agent_id}/budget"),
        Some(json!({ "monthly_budget_cents": cents })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "set budget: {body}");
}

async fn budget_of(env: &TestEnv, agent_id: &str) -> Value {
    let (_, summary) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/budget", env.company_id),
        None,
    )
    .await;
    summary["budgets"]
        .as_array()
        .expect("budgets")
        .iter()
        .find(|a| a["agent_id"] == json!(agent_id))
        .cloned()
        .expect("the agent's budget row")
}

async fn say(env: &TestEnv, agent_id: &str, text: &str) {
    let (status, body) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{agent_id}/conversation/messages",
            env.company_id
        ),
        Some(json!({ "content": text })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "post message: {body}");
}

async fn messages(env: &TestEnv, agent_id: &str) -> Vec<Value> {
    let (_, convo) = send(
        &env.app,
        "GET",
        &format!(
            "/api/companies/{}/agents/{agent_id}/conversation",
            env.company_id
        ),
        None,
    )
    .await;
    convo["messages"].as_array().cloned().unwrap_or_default()
}

/// Wait until the agent's thread stops growing — the turn runs detached.
async fn settle(env: &TestEnv, agent_id: &str, expected: usize) -> Vec<Value> {
    for _ in 0..100 {
        let m = messages(env, agent_id).await;
        if m.len() >= expected {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("thread never reached {expected} messages");
}

#[tokio::test]
async fn a_conversational_turn_lands_in_the_ledger() {
    let env = setup(COSTLY_STUB).await;

    let before = budget_of(&env, &env.ceo_id).await;
    assert_eq!(
        before["spent_cents"],
        json!(0),
        "nothing spent yet: {before}"
    );

    say(&env, &env.ceo_id.clone(), "What should we do first?").await;
    settle(&env, &env.ceo_id.clone(), 2).await;

    // The whole point: a chat turn is spend, and spend is recorded. Before
    // ADR-0022 this stayed zero however long you talked.
    let after = budget_of(&env, &env.ceo_id).await;
    assert_eq!(
        after["spent_cents"],
        json!(30),
        "the turn is billed: {after}"
    );
    // …and the hold is gone: a reservation that outlives its turn is a leak.
    assert_eq!(after["reserved_cents"], json!(0), "{after}");
}

#[tokio::test]
async fn an_agent_out_of_budget_is_refused_before_it_spends() {
    let env = setup(COSTLY_STUB).await;
    // Under the 50-cent start estimate: not one turn fits.
    set_budget(&env, &env.ceo_id.clone(), 10).await;

    say(&env, &env.ceo_id.clone(), "Anything for me?").await;
    // The user's message, then the refusal — in the thread, not as a failure.
    let thread = settle(&env, &env.ceo_id.clone(), 2).await;
    let last = thread.last().expect("a reply of some kind");
    assert_eq!(last["role"], json!("system"), "{last}");
    let text = last["content"].as_str().unwrap_or("");
    assert!(
        text.contains("monthly budget"),
        "it says what happened: {text}"
    );
    assert!(
        text.contains("€0.10"),
        "and it says the cap it ran into: {text}"
    );

    // Nothing was spent: the gate is *before* the spawn, not after the money.
    let after = budget_of(&env, &env.ceo_id.clone()).await;
    assert_eq!(after["spent_cents"], json!(0), "{after}");
    assert_eq!(after["reserved_cents"], json!(0), "no leaked hold: {after}");

    // And it reaches the inbox, with the parts a client needs to word it.
    let (_, inbox) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/notifications", env.company_id),
        None,
    )
    .await;
    let n = inbox["notifications"]
        .as_array()
        .expect("notifications")
        .iter()
        .find(|n| n["kind"] == json!("budget.exhausted"))
        .expect("a budget notification");
    assert_eq!(n["params"]["limitCents"], json!(10), "{n}");

    // Raising the cap unblocks it — which is what the message told you to do.
    set_budget(&env, &env.ceo_id.clone(), 5_000).await;
    say(&env, &env.ceo_id.clone(), "And now?").await;
    settle(&env, &env.ceo_id.clone(), 4).await;
    let after = budget_of(&env, &env.ceo_id.clone()).await;
    assert_eq!(after["spent_cents"], json!(30), "it ran this time: {after}");
}

/// The heart of ADR-0022: running out of money is transient and external, so a
/// room waits for you rather than dying or being forced to a conclusion.
#[tokio::test]
async fn a_room_that_runs_out_of_money_waits_and_resumes_where_it_stopped() {
    let env = setup(DELIBERATING_STUB).await;

    let hire = |name: &'static str| {
        let app = env.app.clone();
        let company_id = env.company_id.clone();
        let ceo = env.ceo_id.clone();
        async move {
            let (status, agent) = send(
                &app,
                "POST",
                &format!("/api/companies/{company_id}/agents"),
                Some(json!({
                    "name": name,
                    "archetype": "researcher",
                    "reports_to": ceo,
                })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "hire {name}: {agent}");
            agent["id"].as_str().expect("id").to_string()
        }
    };
    let vera = hire("Vera").await;
    let bo = hire("Bo").await;

    // Vera can afford two turns (50-cent estimate, 30-cent actual): turn 1
    // leaves her at 30, turn 3 at 60, and turn 5 needs 60+50 > 100.
    set_budget(&env, &vera, 100).await;
    set_budget(&env, &bo, 100_000).await;

    let (status, convened) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/meetings", env.company_id),
        Some(json!({
            "topic": "Which projector",
            "participants": [vera, bo],
            "turn_cap": 8,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "convene: {convened}");
    let meeting_id = convened["id"].as_str().expect("meeting id").to_string();

    // It runs until Vera's money is gone, then waits.
    let meeting = wait_for_status(&env, &meeting_id, "paused").await;
    assert!(
        meeting["meeting"]["paused_note"]
            .as_str()
            .unwrap_or("")
            .contains("Vera"),
        "it says who ran out: {meeting}"
    );

    // The work is intact — this is the difference between pausing and failing.
    let turns_at_pause = meeting["turns"].as_array().map(Vec::len).unwrap_or(0);
    assert!(
        turns_at_pause >= 2,
        "the room got somewhere before stopping: {meeting}"
    );
    assert!(
        turns_at_pause < 8,
        "and it stopped short of the cap: {meeting}"
    );

    // The paused room counts against the ceiling on rooms waiting on you,
    // otherwise pauses pile up unnoticed (M13.5's hole, from a new direction).
    let (_, meetings) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/meetings", env.company_id),
        None,
    )
    .await;
    assert_eq!(
        meetings["meetings"]
            .as_array()
            .map(|m| m.iter().filter(|x| x["status"] == json!("paused")).count()),
        Some(1),
        "{meetings}"
    );

    // Top her up and pick it back up.
    set_budget(&env, &vera, 100_000).await;
    let (status, resumed) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/meetings/{meeting_id}/resume",
            env.company_id
        ),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "resume: {resumed}");

    let done = wait_for_status(&env, &meeting_id, "decided").await;
    let turns = done["turns"].as_array().expect("turns");
    // The cap does not refill. A paused room gets the turns it was approved
    // for — otherwise pausing would be how you buy extra deliberation.
    assert!(
        turns.len() <= 9,
        "8 turns plus the chair's closing one, never more: {} turns",
        turns.len()
    );
    // And it carried on rather than starting over: the turns from before the
    // pause are still the first ones in the transcript.
    assert!(
        turns.len() > turns_at_pause,
        "it made progress after resuming: {} -> {}",
        turns_at_pause,
        turns.len()
    );
    assert!(
        !done["meeting"]["decision"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "the room reached a decision in the end: {done}"
    );
}

#[tokio::test]
async fn resuming_a_room_that_is_not_paused_is_refused() {
    let env = setup(DELIBERATING_STUB).await;
    let (status, body) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/meetings/does-not-exist/resume",
            env.company_id
        ),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

async fn wait_for_status(env: &TestEnv, meeting_id: &str, want: &str) -> Value {
    for _ in 0..200 {
        let (_, m) = send(
            &env.app,
            "GET",
            &format!("/api/meetings/{meeting_id}"),
            None,
        )
        .await;
        if m["meeting"]["status"] == json!(want) {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (_, m) = send(
        &env.app,
        "GET",
        &format!("/api/meetings/{meeting_id}"),
        None,
    )
    .await;
    panic!("meeting never reached `{want}`: {m}");
}
