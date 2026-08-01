//! M13 acceptance tests (ADR-0020): agents collaborating on their own work ask
//! for a meeting, you are notified and decide, and only then do they
//! deliberate — bounded, to a recorded decision that follows every participant
//! back into its work.

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

struct TestEnv {
    app: axum::Router,
    company_id: String,
    /// The org leader (reports to nobody) — chairs any meeting it sits in.
    leader_id: String,
    /// A specialist reporting to the leader — the one we talk to.
    specialist_id: String,
    /// A second specialist, the teammate a meeting request names.
    guard_id: String,
}

/// A company with three agents and a stub adapter. Meetings never touch git.
async fn setup(stub_script: &str) -> TestEnv {
    let root =
        std::env::temp_dir().join(format!("overmind-meet-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("create test root");
    let script_path = root.join("stub-agent.sh");
    std::fs::write(&script_path, stub_script).expect("write stub script");

    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script_path.display())),
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
        Some(json!({ "name": "Deliberating Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company id").to_string();

    let hire = |name: &'static str, reports_to: Option<String>| {
        let app = app.clone();
        let company_id = company_id.clone();
        async move {
            let mut body = json!({ "name": name, "archetype": "backend-developer" });
            if let Some(boss) = reports_to {
                body["reports_to"] = json!(boss);
            }
            let (_, agent) = send(
                &app,
                "POST",
                &format!("/api/companies/{company_id}/agents"),
                Some(body),
            )
            .await;
            agent["id"].as_str().expect("agent id").to_string()
        }
    };
    let leader_id = hire("Ada", None).await;
    let specialist_id = hire("Bruno", Some(leader_id.clone())).await;
    let guard_id = hire("Guard", Some(leader_id.clone())).await;

    TestEnv {
        app,
        company_id,
        leader_id,
        specialist_id,
        guard_id,
    }
}

/// Poll a company's meetings until one exists, and return it.
async fn wait_for_meeting(app: &axum::Router, company_id: &str) -> Value {
    for _ in 0..100 {
        let (_, list) = send(
            app,
            "GET",
            &format!("/api/companies/{company_id}/meetings"),
            None,
        )
        .await;
        if let Some(first) = list["meetings"].as_array().and_then(|m| m.first()) {
            return first.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no meeting was ever requested");
}

/// Poll one meeting until it reaches `status`.
async fn wait_for_status(app: &axum::Router, meeting_id: &str, status: &str) -> Value {
    for _ in 0..100 {
        let (_, m) = send(app, "GET", &format!("/api/meetings/{meeting_id}"), None).await;
        if m["meeting"]["status"] == json!(status) {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (_, m) = send(app, "GET", &format!("/api/meetings/{meeting_id}"), None).await;
    panic!("meeting never reached {status}: {m}");
}

async fn audit_kinds(app: &axum::Router) -> Vec<String> {
    let (_, events) = send(app, "GET", "/api/audit/events?limit=200", None).await;
    events["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|e| e["kind"].as_str().map(str::to_string))
        .collect()
}

/// Talks normally, but asks for a meeting the moment you raise security; in a
/// meeting, settles it on the first turn.
const ASKS_FOR_A_MEETING_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"in a meeting with your colleagues"*)
    echo '{"say":"Agreed, rotate them.","decision":"Rotate the tokens and ship the auth fix on Friday."}' ;;
  *)
    echo '{"reply":"This one needs Guard and the boss in a room.","tasks":[],"meeting":{"topic":"How do we secure the login flow","reason":"The fix changes auth and deploy, so it lands on Guard work too","participants":["Guard"],"turn_cap":3}}' ;;
esac
"#;

#[tokio::test]
async fn an_agent_asks_for_a_meeting_and_nothing_runs_until_you_say_so() {
    let env = setup(ASKS_FOR_A_MEETING_STUB).await;

    // Talking to Bruno about security makes him ask for a room.
    let (status, _) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.specialist_id
        ),
        Some(json!({ "content": "Can you secure the login?" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    assert_eq!(meeting["status"], json!("requested"), "meeting: {meeting}");
    assert_eq!(meeting["convener_name"], json!("Bruno"));
    assert_eq!(meeting["topic"], json!("How do we secure the login flow"));
    assert!(
        meeting["reason"]
            .as_str()
            .unwrap_or("")
            .contains("Guard work"),
        "the reason must reach you: {meeting}"
    );
    let meeting_id = meeting["id"].as_str().expect("meeting id").to_string();

    // Not a single turn has been spoken: it is only a request.
    let (_, detail) = send(
        &env.app,
        "GET",
        &format!("/api/meetings/{meeting_id}"),
        None,
    )
    .await;
    assert_eq!(detail["turns"].as_array().map(Vec::len), Some(0));
    // The convener put itself in the room, plus the teammate it named.
    let room: Vec<&str> = detail["participants"]
        .as_array()
        .expect("participants")
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert_eq!(room, vec!["Bruno", "Guard"], "room: {detail}");

    // You were notified — actionable, with the approval attached.
    let (_, inbox) = send(
        &env.app,
        "GET",
        &format!(
            "/api/companies/{}/notifications?unread=true",
            env.company_id
        ),
        None,
    )
    .await;
    assert_eq!(inbox["unread"], json!(1), "inbox: {inbox}");
    let n = &inbox["notifications"][0];
    assert_eq!(n["kind"], json!("meeting.requested"));
    assert!(
        n["title"].as_str().unwrap_or("").contains("Bruno"),
        "the notification says who is asking: {n}"
    );
    assert!(n["body"].as_str().unwrap_or("").contains("Why:"), "{n}");
    assert_eq!(n["subject_type"], json!("meeting"));
    assert_eq!(n["subject_id"], json!(meeting_id));
    let approval_id = n["approval_id"].as_str().expect("actionable").to_string();

    // And the same request is a pending approval.
    let (_, approvals) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/approvals", env.company_id),
        None,
    )
    .await;
    let a = approvals["approvals"]
        .as_array()
        .expect("approvals")
        .iter()
        .find(|a| a["id"] == json!(approval_id))
        .expect("the meeting approval");
    assert_eq!(a["type"], json!("meeting_request"));
    assert_eq!(a["status"], json!("pending"));

    let kinds = audit_kinds(&env.app).await;
    assert!(
        kinds.iter().any(|k| k == "meeting.requested"),
        "kinds: {kinds:?}"
    );
    assert!(
        !kinds.iter().any(|k| k == "meeting.convened"),
        "nothing may convene before approval: {kinds:?}"
    );
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn approving_lets_them_deliberate_and_sends_the_decision_back_to_work() {
    let env = setup(ASKS_FOR_A_MEETING_STUB).await;
    send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.specialist_id
        ),
        Some(json!({ "content": "Can you secure the login?" })),
    )
    .await;
    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    let meeting_id = meeting["id"].as_str().expect("meeting id").to_string();
    let approval_id = meeting["approval_id"]
        .as_str()
        .expect("approval id")
        .to_string();

    // You approve. Now — and only now — they meet.
    let (status, decided) = send(
        &env.app,
        "POST",
        &format!("/api/approvals/{approval_id}/decision"),
        Some(json!({ "decision": "approve" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "decision failed: {decided}");
    assert_eq!(decided["meeting_id"], json!(meeting_id));

    let m = wait_for_status(&env.app, &meeting_id, "decided").await;
    assert_eq!(
        m["meeting"]["decision"],
        json!("Rotate the tokens and ship the auth fix on Friday.")
    );
    assert_eq!(
        m["turns"].as_array().map(Vec::len),
        Some(1),
        "settled on the first turn: {m}"
    );

    // The outcome comes back to you as a notification.
    let (_, inbox) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/notifications", env.company_id),
        None,
    )
    .await;
    let decided_note = inbox["notifications"]
        .as_array()
        .expect("notifications")
        .iter()
        .find(|n| n["kind"] == json!("meeting.decided"))
        .expect("a decision notification");
    assert!(
        decided_note["body"]
            .as_str()
            .unwrap_or("")
            .contains("Rotate the tokens"),
        "note: {decided_note}"
    );

    // Everyone who was in the room is sent back to work with the decision.
    let (_, events) = send(&env.app, "GET", "/api/audit/events?limit=200", None).await;
    let woken: Vec<&str> = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| {
            e["kind"] == json!("agent.wakeup_requested")
                && e["payload"]["source"] == json!("meeting")
        })
        .filter_map(|e| e["payload"]["agent_id"].as_str())
        .collect();
    assert!(
        woken.contains(&env.specialist_id.as_str()) && woken.contains(&env.guard_id.as_str()),
        "both participants must be woken, got {woken:?}"
    );

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn declining_stops_the_meeting_before_it_starts() {
    let env = setup(ASKS_FOR_A_MEETING_STUB).await;
    send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.specialist_id
        ),
        Some(json!({ "content": "Can you secure the login?" })),
    )
    .await;
    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    let meeting_id = meeting["id"].as_str().expect("meeting id").to_string();
    let approval_id = meeting["approval_id"].as_str().expect("approval id");

    let (status, _) = send(
        &env.app,
        "POST",
        &format!("/api/approvals/{approval_id}/decision"),
        Some(json!({ "decision": "reject", "note": "handle it in the task" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let m = wait_for_status(&env.app, &meeting_id, "declined").await;
    assert_eq!(m["turns"].as_array().map(Vec::len), Some(0), "{m}");
    assert!(m["meeting"]["decision"].is_null());

    let (_, inbox) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/notifications", env.company_id),
        None,
    )
    .await;
    let note = inbox["notifications"]
        .as_array()
        .expect("notifications")
        .iter()
        .find(|n| n["kind"] == json!("meeting.declined"))
        .expect("the agent is told");
    assert!(
        note["body"]
            .as_str()
            .unwrap_or("")
            .contains("handle it in the task"),
        "your note reaches it: {note}"
    );

    let kinds = audit_kinds(&env.app).await;
    assert!(kinds.iter().any(|k| k == "meeting.declined"));
    assert!(
        !kinds.iter().any(|k| k == "meeting.convened"),
        "a declined meeting never convenes: {kinds:?}"
    );
}

/// Working on a task, the agent hits a call it should not make alone and
/// leaves a MEETING_REQUEST.json behind. Once a decision exists, its next run
/// works from it.
const WORKING_AGENT_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"Decisions from meetings you took part in"*)
    echo 'Written with the meeting decision in hand.' > ARTIFACT.md
    echo 'USED_THE_DECISION' >> ARTIFACT.md
    ;;
  *"You are working on the task"*)
    echo 'Partial notes.' > ARTIFACT.md
    cat > MEETING_REQUEST.json <<'JSON'
{"topic":"Do we drop the legacy path","reason":"Removing it changes what Guard is building","participants":["Guard"],"turn_cap":2}
JSON
    ;;
  *)
    echo '{"say":"Fine by me.","decision":"Drop the legacy path in the next release."}'
    ;;
esac
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
"#;

/// Create a knowledge task (no git needed, ADR-0017) and run it with `agent_id`.
async fn run_knowledge_task(
    app: &axum::Router,
    company_id: &str,
    agent_id: &str,
    title: &str,
) -> String {
    let (status, task) = send(
        app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({
            "title": title,
            "description": "Do the thing.",
            "execution_kind": "knowledge",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {task}");
    let task_id = task["id"].as_str().expect("task id").to_string();
    send(
        app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (status, started) = send(
        app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");
    let session_id = started["session_id"].as_str().expect("session id");
    for _ in 0..100 {
        let (_, s) = send(app, "GET", &format!("/api/sessions/{session_id}"), None).await;
        match s["status"].as_str().unwrap_or("") {
            "completed" | "failed" => return task_id,
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("session {session_id} did not finish in time");
}

#[tokio::test]
async fn an_agent_at_work_can_ask_for_a_meeting() {
    let env = setup(WORKING_AGENT_STUB).await;
    let task_id = run_knowledge_task(
        &env.app,
        &env.company_id,
        &env.specialist_id,
        "Clean up the legacy path",
    )
    .await;

    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    assert_eq!(meeting["status"], json!("requested"));
    assert_eq!(meeting["convener_name"], json!("Bruno"));
    assert_eq!(meeting["topic"], json!("Do we drop the legacy path"));
    assert!(meeting["approval_id"].is_string(), "it waits on you");

    // The control file is not a deliverable — it must not show up as an artifact.
    let (_, artifacts) = send(
        &env.app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    let titles: Vec<&str> = artifacts["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .filter_map(|a| a["title"].as_str())
        .collect();
    assert!(titles.contains(&"ARTIFACT.md"), "titles: {titles:?}");
    assert!(
        !titles.contains(&"MEETING_REQUEST.json"),
        "the request is a control file, not a deliverable: {titles:?}"
    );

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn the_decision_follows_a_participant_into_its_next_task() {
    let env = setup(WORKING_AGENT_STUB).await;

    // A meeting Bruno sat in on, already decided.
    let (_, convened) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/meetings", env.company_id),
        Some(json!({
            "topic": "The legacy path",
            "participants": [env.specialist_id, env.leader_id],
            "turn_cap": 2,
        })),
    )
    .await;
    let meeting_id = convened["id"].as_str().expect("meeting id").to_string();
    let m = wait_for_status(&env.app, &meeting_id, "decided").await;
    assert_eq!(
        m["meeting"]["decision"],
        json!("Drop the legacy path in the next release.")
    );

    // Bruno's next task run carries that decision into the work.
    let task_id = run_knowledge_task(
        &env.app,
        &env.company_id,
        &env.specialist_id,
        "Finish the cleanup",
    )
    .await;
    let (_, artifacts) = send(
        &env.app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    let doc = artifacts["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .find(|a| a["title"] == json!("ARTIFACT.md"))
        .expect("the deliverable");
    assert!(
        doc["content"]
            .as_str()
            .unwrap_or("")
            .contains("USED_THE_DECISION"),
        "the agent must work from the decision: {doc}"
    );
}

/// Reports which instruction each turn was given, so the test can check the
/// room is actually being pushed to deliberate rather than to nod along.
const CONSTRUCTIVE_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"A decision is REQUIRED"*)
    echo '{"say":"Closing it.","decision":"Ship it behind a flag."}' ;;
  *"You speak first"*)
    case "$OVERMIND_TASK_PROMPT" in
      *"Why this room was called: it lands on Guard deploy work too"*)
        echo '{"say":"FRAMED_KNOWING_WHY"}' ;;
      *)
        echo '{"say":"FRAMED_BLIND"}' ;;
    esac ;;
  *"Do not agree without adding something"*)
    echo '{"say":"CHALLENGED"}' ;;
  *"in a meeting with your colleagues"*)
    echo '{"say":"UNGUIDED"}' ;;
  *)
    echo '{"reply":"Asking for a room.","tasks":[],"meeting":{"topic":"The login flow","reason":"it lands on Guard deploy work too","participants":["Guard"],"turn_cap":2}}' ;;
esac
"#;

#[tokio::test]
async fn the_room_is_pushed_to_deliberate_not_to_nod_along() {
    let env = setup(CONSTRUCTIVE_STUB).await;
    send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.specialist_id
        ),
        Some(json!({ "content": "Look at the login flow." })),
    )
    .await;
    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    let meeting_id = meeting["id"].as_str().expect("meeting id").to_string();
    send(
        &env.app,
        "POST",
        &format!(
            "/api/approvals/{}/decision",
            meeting["approval_id"].as_str().expect("approval id")
        ),
        Some(json!({ "decision": "approve" })),
    )
    .await;

    let m = wait_for_status(&env.app, &meeting_id, "decided").await;
    let said: Vec<&str> = m["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .filter_map(|t| t["content"].as_str())
        .collect();

    // The opener frames the choice — and knows *why* the room was called, in
    // the convener's own words. The next speaker is pushed to add, not agree.
    // The chair closes with a decision.
    assert_eq!(
        said,
        vec!["FRAMED_KNOWING_WHY", "CHALLENGED", "Closing it."],
        "each turn must get the instruction its position calls for: {m}"
    );
    assert_eq!(m["meeting"]["decision"], json!("Ship it behind a flag."));
}

/// Never settles on its own — only when told the decision is required.
const NEVER_DECIDES_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"A decision is REQUIRED"*)
    echo '{"say":"Time is up, here is the call.","decision":"Ship option A next week."}' ;;
  *)
    echo '{"say":"I lean toward option A, but let us hear the others."}' ;;
esac
"#;

#[tokio::test]
async fn a_meeting_that_never_converges_is_closed_by_the_chair() {
    let env = setup(NEVER_DECIDES_STUB).await;

    let (status, convened) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/meetings", env.company_id),
        Some(json!({
            "topic": "How do we split the work?",
            "participants": [env.specialist_id, env.leader_id],
            "turn_cap": 3,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let meeting_id = convened["id"].as_str().expect("meeting id").to_string();

    let m = wait_for_status(&env.app, &meeting_id, "decided").await;
    assert_eq!(
        m["meeting"]["decision"],
        json!("Ship option A next week."),
        "the chair's closing turn must produce the decision"
    );
    // The cap held: exactly turn_cap turns, plus the chair's closing turn.
    let speakers: Vec<&str> = m["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .filter_map(|t| t["agent_name"].as_str())
        .collect();
    // Round-robin in the order given — then Ada closes, because the leader
    // chairs even though she was listed second.
    assert_eq!(speakers, vec!["Bruno", "Ada", "Bruno", "Ada"]);

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}

#[tokio::test]
async fn the_turn_cap_is_clamped() {
    let env = setup(NEVER_DECIDES_STUB).await;
    let (_, convened) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/meetings", env.company_id),
        Some(json!({
            "topic": "Can we talk forever?",
            "participants": [env.leader_id, env.specialist_id],
            "turn_cap": 500,
        })),
    )
    .await;
    let meeting_id = convened["id"].as_str().expect("meeting id").to_string();

    let m = wait_for_status(&env.app, &meeting_id, "decided").await;
    assert_eq!(m["meeting"]["turn_cap"], json!(12));
    assert_eq!(m["turns"].as_array().map(Vec::len), Some(13));
}

#[tokio::test]
async fn a_meeting_needs_a_topic_and_two_participants() {
    let env = setup(NEVER_DECIDES_STUB).await;
    let uri = format!("/api/companies/{}/meetings", env.company_id);

    let (status, _) = send(
        &env.app,
        "POST",
        &uri,
        Some(json!({ "topic": "  ", "participants": [env.leader_id, env.specialist_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty topic");

    let (status, _) = send(
        &env.app,
        "POST",
        &uri,
        Some(json!({ "topic": "Alone in a room", "participants": [env.leader_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "one participant");

    // The same agent twice is still one participant.
    let (status, _) = send(
        &env.app,
        "POST",
        &uri,
        Some(
            json!({ "topic": "Talking to myself", "participants": [env.leader_id, env.leader_id] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "duplicate participant");

    let (status, _) = send(
        &env.app,
        "POST",
        &uri,
        Some(json!({
            "topic": "Who is that?",
            "participants": [env.leader_id, "no-such-agent"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown participant");
}

/// M14 / ADR-0005: the agent's characterization must reach the work itself.
/// This stub writes the prompt it was handed into the deliverable, so the test
/// can assert on what the agent actually saw.
const ECHO_PROMPT_STUB: &str = r#"#!/bin/sh
printf '%s' "$OVERMIND_TASK_PROMPT" > ARTIFACT.md
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
"#;

#[tokio::test]
async fn a_working_agent_is_told_who_it_is() {
    let env = setup(ECHO_PROMPT_STUB).await;

    // Two agents, deliberately different: role, focus areas and brief.
    let (_, media) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company_id),
        Some(json!({
            "name": "Nova",
            "archetype": "researcher",
            "title": "Media & A/V quality",
            "reports_to": env.leader_id,
            "custom_brief": "Judge everything by what reaches the viewer's eyes and ears.",
            "traits": { "focus_areas": ["calibration", "acoustics", "picture quality"] },
        })),
    )
    .await;
    let media_id = media["id"].as_str().expect("media agent id").to_string();

    let prompt_of = |agent_id: String, title: &'static str| {
        let app = env.app.clone();
        let company_id = env.company_id.clone();
        async move {
            let task_id = run_knowledge_task(&app, &company_id, &agent_id, title).await;
            let (_, artifacts) = send(
                &app,
                "GET",
                &format!("/api/tasks/{task_id}/artifacts"),
                None,
            )
            .await;
            artifacts["artifacts"]
                .as_array()
                .expect("artifacts")
                .iter()
                .find(|a| a["title"] == json!("ARTIFACT.md"))
                .and_then(|a| a["content"].as_str())
                .expect("the echoed prompt")
                .to_string()
        }
    };

    let media_prompt = prompt_of(media_id, "Pick the projector").await;
    let plain_prompt = prompt_of(env.specialist_id.clone(), "Pick the projector").await;

    // The specialist knows who it is...
    assert!(
        media_prompt.contains("You are Nova, the Media & A/V quality"),
        "no persona in the prompt: {media_prompt}"
    );
    assert!(
        media_prompt.contains("calibration") && media_prompt.contains("acoustics"),
        "focus areas missing: {media_prompt}"
    );
    assert!(
        media_prompt.contains("viewer's eyes and ears"),
        "custom_brief missing: {media_prompt}"
    );

    // ...and two different agents no longer get the same instructions.
    assert_ne!(
        media_prompt, plain_prompt,
        "role-blind: different agents received an identical prompt"
    );
    assert!(
        plain_prompt.contains("You are Bruno,"),
        "the plain agent has a persona too: {plain_prompt}"
    );
}

// ---------- M13.5: restraint — autonomous, but not free to flood you ----------

/// Asks for a meeting on every single turn. Without restraint this is the
/// "10k requests" agent.
const ALWAYS_ASKS_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"in a meeting with your colleagues"*)
    echo '{"say":"Fine.","decision":"Do it."}' ;;
  *)
    echo '{"reply":"We should meet.","tasks":[],"meeting":{"topic":"Yet another room","reason":"I would like company","participants":["Guard"],"turn_cap":2}}' ;;
esac
"#;

#[tokio::test]
async fn an_agent_may_keep_only_one_request_waiting_on_you() {
    let env = setup(ALWAYS_ASKS_STUB).await;
    let uri = format!(
        "/api/companies/{}/agents/{}/conversation/messages",
        env.company_id, env.specialist_id
    );

    // Three turns, three attempts to convene.
    for i in 0..3 {
        send(
            &env.app,
            "POST",
            &uri,
            Some(json!({ "content": format!("turn {i}") })),
        )
        .await;
        wait_for_meeting(&env.app, &env.company_id).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    let (_, list) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/meetings", env.company_id),
        None,
    )
    .await;
    let pending = list["meetings"]
        .as_array()
        .expect("meetings")
        .iter()
        .filter(|m| m["status"] == json!("requested"))
        .count();
    assert_eq!(
        pending, 1,
        "one pending request per agent, got {pending}: {list}"
    );
}

/// Asks for a meeting from chat, and echoes the prompt it is given when it
/// works a task — so the test can read what the agent actually saw.
const ASKS_THEN_ECHOES_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"You are working on the task"*)
    printf '%s' "$OVERMIND_TASK_PROMPT" > ARTIFACT.md ;;
  *"in a meeting with your colleagues"*)
    echo '{"say":"Fine.","decision":"Do it."}' ;;
  *)
    echo '{"reply":"We should meet.","tasks":[],"meeting":{"topic":"Which font for the label","reason":"I would like company","participants":["Guard"],"turn_cap":2}}' ;;
esac
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
"#;

#[tokio::test]
async fn a_declined_request_reaches_the_agent_that_asked() {
    let env = setup(ASKS_THEN_ECHOES_STUB).await;
    send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.specialist_id
        ),
        Some(json!({ "content": "start" })),
    )
    .await;
    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    let meeting_id = meeting["id"].as_str().expect("id").to_string();

    // You decline, with a reason.
    send(
        &env.app,
        "POST",
        &format!(
            "/api/approvals/{}/decision",
            meeting["approval_id"].as_str().expect("approval")
        ),
        Some(json!({ "decision": "reject", "note": "decide it yourself, it is your call" })),
    )
    .await;
    let m = wait_for_status(&env.app, &meeting_id, "declined").await;
    assert_eq!(
        m["meeting"]["decline_note"],
        json!("decide it yourself, it is your call"),
        "the reason must be stored on the meeting, not only in the notification"
    );

    // The real point: your refusal, and your reason, are in front of the agent
    // the next time it works. Without this it re-asks on its very next turn.
    let task_id = run_knowledge_task(
        &env.app,
        &env.company_id,
        &env.specialist_id,
        "Carry on alone",
    )
    .await;
    let (_, artifacts) = send(
        &env.app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    let prompt = artifacts["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .find(|a| a["title"] == json!("ARTIFACT.md"))
        .and_then(|a| a["content"].as_str())
        .expect("the echoed prompt")
        .to_string();

    assert!(
        prompt.contains("Meetings you asked for that did NOT happen"),
        "the refusal never reached the agent: {prompt}"
    );
    assert!(
        prompt.contains("Which font for the label"),
        "which meeting was refused: {prompt}"
    );
    assert!(
        prompt.contains("decide it yourself"),
        "your reason must travel with it: {prompt}"
    );
    assert!(
        prompt.contains("only ONE request waiting"),
        "the agent is told the limit exists, not just blocked by it: {prompt}"
    );
}

/// The room is asked to meet, looks at the topic, and says it is pointless.
const DROPS_THE_ROOM_STUB: &str = r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"in a meeting with your colleagues"*)
    echo '{"say":"This is my call to make, we do not need a room.","no_decision_needed":"it is a single-owner decision"}' ;;
  *)
    echo '{"reply":"Calling a room.","tasks":[],"meeting":{"topic":"Which font for the label","reason":"seemed worth discussing","participants":["Guard"],"turn_cap":4}}' ;;
esac
"#;

#[tokio::test]
async fn a_pointless_room_closes_without_inventing_a_decision() {
    let env = setup(DROPS_THE_ROOM_STUB).await;
    send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company_id, env.specialist_id
        ),
        Some(json!({ "content": "go" })),
    )
    .await;
    let meeting = wait_for_meeting(&env.app, &env.company_id).await;
    let meeting_id = meeting["id"].as_str().expect("id").to_string();
    send(
        &env.app,
        "POST",
        &format!(
            "/api/approvals/{}/decision",
            meeting["approval_id"].as_str().expect("approval")
        ),
        Some(json!({ "decision": "approve" })),
    )
    .await;

    let m = wait_for_status(&env.app, &meeting_id, "dropped").await;
    assert!(
        m["meeting"]["decision"].is_null(),
        "a dropped room must NOT produce a decision: {m}"
    );
    assert!(
        m["meeting"]["decline_note"]
            .as_str()
            .unwrap_or("")
            .contains("single-owner"),
        "the reason it was dropped is recorded: {m}"
    );
    assert_eq!(
        m["turns"].as_array().map(Vec::len),
        Some(1),
        "it closes on the first turn, not at the cap"
    );

    let kinds = audit_kinds(&env.app).await;
    assert!(kinds.iter().any(|k| k == "meeting.dropped"), "{kinds:?}");
    assert!(
        !kinds.iter().any(|k| k == "meeting.decided"),
        "nothing was decided: {kinds:?}"
    );

    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));
}
