//! A long conversation is compacted before it drowns the turn (ADR-0040).
//!
//! The owner, after his CEO's thread grew to days of briefs: "cosa succede
//! quando la chat diventa un po' lunga? Potremmo fare come fa Anthropic."
//! When the transcript that would ride into a turn exceeds a threshold, the
//! agent first writes a handoff summary of the older part; the summary is
//! stored, the turn (and every later one) reads summary + recent tail, and
//! the person sees a quiet system chip saying it happened.

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

/// An adapter that answers a compaction ask with a recognizable summary and
/// every other turn with a fixed reply — logging each answer-turn prompt.
fn stub(root: &std::path::Path) -> String {
    format!(
        r#"#!/bin/sh
case "$OVERMIND_TASK_PROMPT" in
  *"handoff summary"*)
    echo '{{"type":"result","result":"RIASSUNTO-DI-PASSAGGIO: il filo finora.","total_cost_usd":0.001}}'
    ;;
  *)
    printf '%s' "$OVERMIND_TASK_PROMPT" > "{root}/prompt-$$.log"
    echo '{{"type":"result","result":"{{\"reply\":\"ricevuto\",\"tasks\":[]}}","total_cost_usd":0.001}}'
    ;;
esac
"#,
        root = root.display()
    )
}

struct Env {
    app: axum::Router,
    state: overmind_server::AppState,
    company: String,
    ceo: String,
    root: PathBuf,
}

async fn setup(threshold: usize) -> Env {
    let root = std::env::temp_dir().join(format!(
        "overmind-compact-{}-{}",
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
            chat_compact_chars: threshold,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = overmind_server::app(state.clone());
    let (s, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Long Co" })),
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

async fn tell_and_wait(env: &Env, text: &str) -> Vec<Value> {
    let before = messages(env).await.len();
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
    for _ in 0..100 {
        let m = messages(env).await;
        // Specifically the agent's reply: the compaction chip is a `system`
        // message posted mid-turn, and "anything non-user" returned early on
        // it — the answer turn had not run yet (measured as a flake).
        if m.len() > before + 1 && m.last().map(|x| x["role"] == "ceo").unwrap_or(false) {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no reply arrived");
}

async fn messages(env: &Env) -> Vec<Value> {
    let (_, c) = send(
        &env.app,
        "GET",
        &format!(
            "/api/companies/{}/agents/{}/conversation",
            env.company, env.ceo
        ),
        None,
    )
    .await;
    c["messages"].as_array().cloned().unwrap_or_default()
}

/// Pad the thread with old traffic, straight into the store: posting through
/// the API would run a turn per message. The thread must exist — say hello
/// through the API first. The padded rows carry 2026-08-01 timestamps, so
/// they sort before everything the API writes.
async fn pad(env: &Env, n: usize, each: usize) {
    let convo: (String,) =
        sqlx::query_as("SELECT id FROM conversations WHERE company_id = ? LIMIT 1")
            .bind(&env.company)
            .fetch_one(&env.state.pool)
            .await
            .expect("conversation");
    for i in 0..n {
        let role = if i % 2 == 0 { "user" } else { "ceo" };
        let body = format!("VECCHIO-{i} {}", "x".repeat(each));
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(&convo.0)
        .bind(role)
        .bind(&body)
        .bind(format!("2026-08-01T00:{:02}:{:02}Z", i / 60, i % 60))
        .execute(&env.state.pool)
        .await
        .expect("insert");
    }
}

fn answer_prompts(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(root)
        .expect("root")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("prompt-"))
        .map(|e| {
            let meta = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (meta, std::fs::read_to_string(e.path()).unwrap_or_default())
        })
        .collect();
    v.sort_by_key(|(t, _)| *t);
    v.into_iter().map(|(_, s)| s).collect()
}

/// Below the threshold nothing changes: full history in the prompt, no chip.
#[tokio::test]
async fn a_short_thread_rides_whole() {
    let env = setup(50_000).await;
    tell_and_wait(&env, "SALUTO-INIZIALE").await;
    pad(&env, 4, 100).await;
    let m = tell_and_wait(&env, "Come procediamo?").await;
    assert!(m.iter().all(|x| x["role"] != "system"), "no chip: {m:?}");
    let prompts = answer_prompts(&env.root);
    let p = prompts.last().expect("a turn ran");
    assert!(p.contains("VECCHIO-0"), "the oldest message still rides");
}

/// Past the threshold the turn compacts first: the stored summary rides in
/// place of the old traffic, the recent tail stays verbatim, a system chip
/// says it happened — and the next turn does not compact again.
#[tokio::test]
async fn a_long_thread_is_compacted_before_the_turn() {
    let env = setup(8_000).await;
    tell_and_wait(&env, "SALUTO-INIZIALE").await;
    pad(&env, 30, 500).await; // ~15k chars of old traffic
    let m = tell_and_wait(&env, "DOMANDA-FRESCA: come procediamo?").await;

    let prompts = answer_prompts(&env.root);
    // Picked by content, not by clock: two turns can share an mtime tick on a
    // busy CI runner, and "the last file" then lies (measured: macOS CI).
    let p = prompts
        .iter()
        .find(|p| p.contains("DOMANDA-FRESCA"))
        .expect("the fresh question's turn ran");
    assert!(
        p.contains("RIASSUNTO-DI-PASSAGGIO"),
        "the summary rides in the prompt"
    );
    assert!(
        !p.contains("VECCHIO-0 "),
        "the oldest traffic no longer rides verbatim"
    );
    assert!(
        p.contains("DOMANDA-FRESCA"),
        "the fresh question rides verbatim"
    );
    assert!(
        m.iter().any(|x| x["role"] == "system"),
        "a chip says it happened: {m:?}"
    );

    // Another message: no second compaction (the summary already covers it).
    let before = answer_prompts(&env.root).len();
    tell_and_wait(&env, "SECONDA-DOMANDA").await;
    let prompts = answer_prompts(&env.root);
    assert_eq!(
        prompts.len(),
        before + 1,
        "one answer turn, no re-compaction"
    );
    let p = prompts
        .iter()
        .find(|p| p.contains("SECONDA-DOMANDA"))
        .expect("the second question's turn ran");
    assert!(p.contains("RIASSUNTO-DI-PASSAGGIO"));
}
