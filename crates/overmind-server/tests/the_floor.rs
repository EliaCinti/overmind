//! M30 — The CEO runs the floor (ADR-0042).
//!
//! The owner, reading a beautiful relaunch plan the CEO had written *for him
//! to execute by hand*: "il CEO comanda e controlla — deve proporre i
//! rilanci, e i risultati devono passare agli agenti successivi; sempre
//! previa mia autorizzazione." Three verbs make that real: `start` (the CEO
//! starts or relaunches an existing task, through the same autonomy gates),
//! `after` (a task waits for another and inherits its deliverables), and
//! digest-proposed starts (always an approval, never a spend).

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

/// One stub, three voices, switched by marker strings in the prompt:
/// chat turns answer with the plan in `PLAN_FILE`; digest turns with the
/// plan in `DIGEST_FILE`; task runs write a deliverable and finish.
fn stub(root: &std::path::Path) -> String {
    format!(
        r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"finished since your last word"*)
    cat "{root}/digest-plan.json"
    ;;
  *"Respond with a SINGLE JSON"*)
    cat "{root}/chat-plan.json"
    ;;
  *)
    echo "risultato del lavoro" > consegna.md
    echo '{{"type":"result","result":"LAVORO-FATTO","total_cost_usd":0.001,"session_id":"s"}}'
    ;;
esac
"#,
        root = root.display()
    )
}

fn plan_line(plan: &Value) -> String {
    json!({
        "type": "result",
        "result": plan.to_string(),
        "total_cost_usd": 0.001
    })
    .to_string()
}

struct Env {
    app: axum::Router,
    state: overmind_server::AppState,
    company: String,
    ceo: String,
    root: PathBuf,
}

impl Env {
    fn set_chat_plan(&self, plan: &Value) {
        std::fs::write(self.root.join("chat-plan.json"), plan_line(plan)).expect("plan");
    }
    fn set_digest_plan(&self, plan: &Value) {
        std::fs::write(self.root.join("digest-plan.json"), plan_line(plan)).expect("plan");
    }
}

async fn setup() -> Env {
    let root = std::env::temp_dir().join(format!(
        "overmind-floor-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&root).expect("mkdir");
    let script = root.join("stub.sh");
    std::fs::write(&script, stub(&root)).expect("stub");
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            agent_cmd: Some(format!("sh {}", script.display())),
            data_dir: root.join("data"),
            heartbeat_ms: 1_000_000,
            digest_debounce_secs: 0,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = common::claimed(overmind_server::app(state.clone()), &root.join("data")).await;
    let (s, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Floor Co" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{co}");
    Env {
        company: co["id"].as_str().expect("id").to_string(),
        ceo: co["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
        state,
        root,
    }
}

async fn hire(env: &Env, name: &str, autonomy: &str) -> String {
    let (s, a) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/agents", env.company),
        Some(json!({ "name": name, "archetype": "writer",
                     "traits": { "autonomy": autonomy } })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    a["id"].as_str().expect("id").to_string()
}

/// A task created by hand, in `todo`, assigned.
async fn make_task(env: &Env, title: &str, assignee: &str) -> String {
    let (s, t) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{}/tasks", env.company),
        Some(json!({ "title": title, "execution_kind": "knowledge" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().expect("task").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    sqlx::query("UPDATE tasks SET assignee_agent_id = ? WHERE id = ?")
        .bind(assignee)
        .bind(&id)
        .execute(&env.state.pool)
        .await
        .expect("assign");
    id
}

async fn tell_the_ceo(env: &Env, text: &str) {
    let (s, v) = send(
        &env.app,
        "POST",
        &format!(
            "/api/companies/{}/agents/{}/conversation/messages",
            env.company, env.ceo
        ),
        Some(json!({ "content": text })),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
}

async fn task_status(env: &Env, id: &str) -> String {
    let row: (String,) = sqlx::query_as("SELECT status FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_one(&env.state.pool)
        .await
        .expect("task");
    row.0
}

async fn wait_status(env: &Env, id: &str, wanted: &str) -> bool {
    for _ in 0..150 {
        if task_status(env, id).await == wanted {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn pending_start_approvals(env: &Env) -> usize {
    let (_, v) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{}/approvals", env.company),
        None,
    )
    .await;
    v["approvals"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|x| x["type"] == "task_start" && x["status"] == "pending")
                .count()
        })
        .unwrap_or(0)
}

/// The CEO starts an existing task by title: within-budget it runs at once;
/// with-approval the start lands in the inbox; a blocked task is brought
/// back to the queue first (a relaunch is exactly this).
#[tokio::test]
async fn the_ceo_starts_and_relaunches_existing_tasks() {
    let env = setup().await;
    let free = hire(&env, "Libera", "act_within_budget").await;
    let gated = hire(&env, "Cauta", "act_with_approval").await;
    let t1 = make_task(&env, "Riconciliazione metrica", &free).await;
    let t2 = make_task(&env, "Specifica schermo", &gated).await;
    // A blocked task — the relaunch case.
    sqlx::query("UPDATE tasks SET status = 'blocked' WHERE id = ?")
        .bind(&t1)
        .execute(&env.state.pool)
        .await
        .expect("block");

    env.set_chat_plan(&json!({
        "reply": "Rilancio la riconciliazione e metto in fila la specifica.",
        "tasks": [],
        "start": ["Riconciliazione metrica", "Specifica schermo"]
    }));
    tell_the_ceo(&env, "Rilancia quello che serve.").await;

    // The stub delivers in milliseconds, so `in_progress` can be a blink:
    // any move off blocked/todo proves the relaunch.
    let mut moved = false;
    for _ in 0..150 {
        let st = task_status(&env, &t1).await;
        if st != "blocked" && st != "todo" {
            moved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        moved,
        "the blocked task was relaunched: {}",
        task_status(&env, &t1).await
    );
    let mut asked = 0;
    for _ in 0..100 {
        asked = pending_start_approvals(&env).await;
        if asked >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(asked, 1, "the gated start waits in the inbox");
    assert_eq!(task_status(&env, &t2).await, "todo");
}

/// `after`: the dependent task waits; when its dependency delivers, it
/// inherits the deliverables as inputs and goes to work by its autonomy.
#[tokio::test]
async fn a_dependent_task_inherits_the_deliverable_and_starts() {
    let env = setup().await;
    let free = hire(&env, "Libera", "act_within_budget").await;
    env.set_chat_plan(&json!({
        "reply": "Apro il cancello e il dipendente.",
        "tasks": [
            { "title": "Il cancello", "description": "Misura.", "execution_kind": "knowledge", "assignee": "Libera" },
            { "title": "Il dipendente", "description": "Usa le misure del cancello.", "execution_kind": "knowledge", "assignee": "Libera", "after": "Il cancello" }
        ]
    }));
    tell_the_ceo(&env, "Vai.").await;

    // Both exist (the turn is asynchronous: poll for them).
    let mut ids: Vec<(String, String)> = Vec::new();
    for _ in 0..150 {
        ids = sqlx::query_as("SELECT id, title FROM tasks ORDER BY created_at")
            .fetch_all(&env.state.pool)
            .await
            .expect("tasks");
        if ids.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(ids.len(), 2, "{ids:?}");
    let gate = ids
        .iter()
        .find(|(_, t)| t == "Il cancello")
        .expect("gate")
        .0
        .clone();
    let dep = ids
        .iter()
        .find(|(_, t)| t == "Il dipendente")
        .expect("dep")
        .0
        .clone();

    // The gate runs and completes (stub finishes fast).
    assert!(
        wait_status(&env, &gate, "in_review").await,
        "gate delivered"
    );
    // The dependent then starts on its own…
    assert!(
        wait_status(&env, &dep, "in_progress").await || wait_status(&env, &dep, "in_review").await,
        "the dependent went to work: {}",
        task_status(&env, &dep).await
    );
    // …and its inputs carry the gate's deliverable.
    let (_, atts) = send(
        &env.app,
        "GET",
        &format!("/api/tasks/{dep}/attachments"),
        None,
    )
    .await;
    let names: Vec<String> = atts["attachments"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["filename"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.iter().any(|n| n.contains("consegna")),
        "the gate's deliverable rides into the dependent: {names:?}"
    );
    let _ = free;
}

/// A digest may propose starts — they land as approvals, never as spend,
/// whatever the agent's autonomy says.
#[tokio::test]
async fn a_digest_proposed_start_is_an_approval_never_a_spend() {
    let env = setup().await;
    let free = hire(&env, "Libera", "act_within_budget").await;
    // A finished task in the thread (born from chat), so a digest is due.
    env.set_chat_plan(&json!({
        "reply": "Apro il lavoro.",
        "tasks": [{ "title": "Primo lavoro", "description": "Fai.", "execution_kind": "knowledge", "assignee": "Libera" }]
    }));
    tell_the_ceo(&env, "Vai.").await;
    let mut first: Option<(String,)> = None;
    for _ in 0..150 {
        first = sqlx::query_as("SELECT id FROM tasks LIMIT 1")
            .fetch_optional(&env.state.pool)
            .await
            .expect("query");
        if first.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let first = first.expect("the task was opened");
    assert!(
        wait_status(&env, &first.0, "in_review").await,
        "first delivered"
    );

    // A second open task the digest will propose to start.
    let follow = make_task(&env, "Il seguito", &free).await;
    env.set_digest_plan(&json!({
        "reply": "Il primo lavoro è consegnato. Propongo di avviare il seguito.",
        "tasks": [],
        "start": ["Il seguito"]
    }));
    overmind_server::scheduler::beat(&env.state)
        .await
        .expect("beat");
    let mut asked = 0;
    for _ in 0..150 {
        asked = pending_start_approvals(&env).await;
        if asked >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(asked, 1, "the digest's start is an approval");
    assert_eq!(
        task_status(&env, &follow).await,
        "todo",
        "and nothing was spent unprompted"
    );
}

/// Everything the person can read in the thread after a turn, waited for
/// rather than guessed at: the turn runs off the request, so the messages land
/// after `tell_the_ceo` returns. Every role, because what became of a start is
/// the server's word, not the CEO's -- the CEO's sentence was committed before
/// the start was tried (ADR-0046).
async fn await_thread(env: &Env) -> String {
    for _ in 0..150 {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT group_concat(content, '\n') FROM messages WHERE role IN ('ceo', 'system')",
        )
        .fetch_optional(&env.state.pool)
        .await
        .expect("q");
        if let Some((content,)) = row
            && !content.trim().is_empty()
        {
            return content;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("nothing was ever said in the thread");
}

/// The reply and the `start` list are one JSON object: the words are delivered,
/// and only then does the server try the starts. So the CEO cannot have seen
/// what happened — and until ADR-0046 the outcome was discarded, `Ok(None)`
/// and all. Measured on a real company on 2 Sep 2026: three claimed starts,
/// zero in progress, and a CEO apologising for forgetting to press a button it
/// had in fact pressed.
#[tokio::test]
async fn a_start_that_matched_nothing_is_not_reported_as_running() {
    let env = setup().await;

    env.set_chat_plan(&json!({
        "reply": "Ho messo in run il calendario delle condizioni.",
        "tasks": [],
        "start": ["Un titolo che sulla lavagna non esiste"]
    }));
    tell_the_ceo(&env, "Vai.").await;

    let thread = await_thread(&env).await;
    assert!(
        thread.contains("Un titolo che sulla lavagna non esiste"),
        "the CEO claimed a start, the title matched nothing, and the person was \
         told none of it: {thread}"
    );
}

/// The refusal that actually stopped a real company. On *TravelAgency*, 2 Sep
/// 2026, one agent had spent EUR 50.38 against a EUR 50.00 cap; six starts in
/// two hours were refused, each recorded as `budget.blocked` on the chain and
/// told to nobody — while the CEO reported "now I have put three in run". The
/// gate was right. Its silence was the bug (ADR-0046).
#[tokio::test]
async fn a_start_refused_for_budget_says_so_with_the_numbers() {
    let env = setup().await;
    let agent = hire(&env, "Spendacciona", "act_within_budget").await;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO tasks (id, company_id, title, description, status, priority,
                            execution_kind, assignee_agent_id, created_at, updated_at)
         VALUES (?, ?, ?, '', 'todo', 'medium', 'knowledge', ?, ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&env.company)
    .bind("Il lavoro che non partirà")
    .bind(&agent)
    .bind(&now)
    .bind(&now)
    .execute(&env.state.pool)
    .await
    .expect("task");
    // One cent: smaller than any run's estimate, the way EUR 50.00 was smaller
    // than EUR 50.38. (Zero would not do it — `governance::check` reads a cap
    // of zero or less as "no cap at all".)
    sqlx::query(
        "UPDATE agents SET traits = json_set(traits, '$.monthly_budget_cents', 1) WHERE id = ?",
    )
    .bind(&agent)
    .execute(&env.state.pool)
    .await
    .expect("cap");

    env.set_chat_plan(&json!({
        "reply": "Messo in run.",
        "tasks": [],
        "start": ["Il lavoro che non partirà"]
    }));
    tell_the_ceo(&env, "Vai.").await;

    let thread = await_thread(&env).await;
    assert!(
        thread.contains("tetto mensile") || thread.contains("monthly cap"),
        "the budget gate refused the start and the person was told nothing: {thread}"
    );
}

/// Deleting a company still works after the schema grew (measured 27 Aug
/// 2026: FOREIGN KEY constraint failed — three references born after
/// ADR-0034's children-first list: conversation_summaries (ADR-0040),
/// tasks.depends_on (M30), and tasks.conversation_id (ADR-0038, the sneaky
/// one: conversations were deleted before the tasks pointing at them).
#[tokio::test]
async fn a_company_with_the_new_references_can_still_be_deleted() {
    let env = setup().await;
    hire(&env, "Libera", "act_within_budget").await;
    // A thread-born dependent pair (depends_on + conversation_id set)…
    env.set_chat_plan(&json!({
        "reply": "Apro coppia.",
        "tasks": [
            { "title": "Padre", "description": "x", "execution_kind": "knowledge", "assignee": "Libera" },
            { "title": "Figlio", "description": "y", "execution_kind": "knowledge", "assignee": "Libera", "after": "Padre" }
        ]
    }));
    tell_the_ceo(&env, "Vai.").await;
    for _ in 0..150 {
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks")
            .fetch_one(&env.state.pool)
            .await
            .expect("q");
        if n.0 == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // …and a compaction summary on the thread.
    let convo: (String,) = sqlx::query_as("SELECT id FROM conversations LIMIT 1")
        .fetch_one(&env.state.pool)
        .await
        .expect("convo");
    sqlx::query(
        "INSERT INTO conversation_summaries (id, conversation_id, content, covers_until, created_at)
         VALUES (?, ?, 'riassunto', '2026-01-01', '2026-01-01')",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&convo.0)
    .execute(&env.state.pool)
    .await
    .expect("summary");

    // Deletion refuses while a session is queued or running, and this is a
    // dependent pair: between Padre finishing and Figlio being picked up there
    // is a moment with no live session and work still to come -- the scheduler
    // takes tasks in `todo` (`scheduler.rs`), and a `blocked` task becomes
    // `todo` the instant its dependency lands. So rather than guess when the
    // floor is quiet, wait on the verb under test: a 409 means "not yet",
    // anything else is the answer. This also keeps a legitimately stuck task
    // (empty-handed, or a timed-out session) from failing the wait instead of
    // the assertion. (Caught on macOS CI, 1 Sep 2026, from a settle loop that
    // had already seen zero live sessions.)
    let (mut s, mut v) = (StatusCode::CONFLICT, json!(null));
    for _ in 0..200 {
        (s, v) = send(
            &env.app,
            "DELETE",
            &format!("/api/companies/{}", env.company),
            None,
        )
        .await;
        if s != StatusCode::CONFLICT {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(s, StatusCode::OK, "{v}");
}
