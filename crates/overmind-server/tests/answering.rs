//! While an agent is answering, the server says so — and a message sent in
//! the meantime is queued, not raced (ADR-0038, addendum).
//!
//! Measured by the owner: send a message, switch to the board, come back —
//! the typing dots were gone and it looked as if nothing was happening,
//! because "answering" lived only in the chat component's memory. And a
//! second message sent before the reply started a second, concurrent turn.

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

struct Env {
    app: axum::Router,
    company: String,
    ceo: String,
    root: PathBuf,
}

/// A CEO that takes a second to think, logs each prompt it is given (one
/// file per turn), and answers with a fixed reply.
async fn setup() -> Env {
    let root = std::env::temp_dir().join(format!(
        "overmind-answering-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&root).expect("mkdir");
    let script = root.join("stub.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 1\nprintf '%s' \"$OVERMIND_TASK_PROMPT\" > \"{}/prompt-$$.log\"\necho '{{\"reply\":\"Thought about it.\",\"tasks\":[]}}'\n",
            root.display()
        ),
    )
    .expect("stub");
    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000,
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    let app = overmind_server::app(state);
    let (s, co) = send(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Slow Co" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{co}");
    Env {
        company: co["id"].as_str().expect("id").to_string(),
        ceo: co["ceo"]["id"].as_str().expect("ceo").to_string(),
        app,
        root,
    }
}

async fn tell(env: &Env, text: &str) {
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

async fn conversation(env: &Env) -> Value {
    let (_, v) = send(
        &env.app,
        "GET",
        &format!(
            "/api/companies/{}/agents/{}/conversation",
            env.company, env.ceo
        ),
        None,
    )
    .await;
    v
}

fn ceo_replies(convo: &Value) -> usize {
    convo["messages"]
        .as_array()
        .map(|m| m.iter().filter(|x| x["role"] == "ceo").count())
        .unwrap_or(0)
}

/// The conversation carries `answering` while a turn is in flight, so a chat
/// that remounts shows the dots again — and drops them when the reply lands.
#[tokio::test]
async fn the_conversation_says_when_the_agent_is_answering() {
    let env = setup().await;
    tell(&env, "Hello?").await;
    let c = conversation(&env).await;
    assert_eq!(c["answering"], json!(true), "{c}");
    for _ in 0..100 {
        let c = conversation(&env).await;
        if ceo_replies(&c) == 1 {
            assert_eq!(c["answering"], json!(false), "{c}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no reply arrived");
}

/// A message sent while the agent is answering does not start a second,
/// concurrent turn. Either the turn under way had not read the thread yet
/// and takes both messages in one go, or it had — and then exactly one more
/// turn runs afterwards, reading both. Never two at once, never a message
/// left unread.
#[tokio::test]
async fn a_message_sent_while_answering_is_read_in_the_next_turn() {
    let env = setup().await;
    tell(&env, "First thing.").await;
    tell(&env, "Second thing, while you think.").await;
    let c = conversation(&env).await;
    assert_eq!(c["answering"], json!(true), "{c}");

    // Settle: answering false and the last message is the agent's.
    let mut settled = Value::Null;
    for _ in 0..150 {
        let c = conversation(&env).await;
        let last_is_agent = c["messages"]
            .as_array()
            .and_then(|m| m.last())
            .map(|m| m["role"] == "ceo")
            .unwrap_or(false);
        if c["answering"] == json!(false) && last_is_agent {
            settled = c;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!settled.is_null(), "never settled");
    let replies = ceo_replies(&settled);

    let mut logs: Vec<String> = std::fs::read_dir(&env.root)
        .expect("root")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("prompt-"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect();
    assert_eq!(logs.len(), replies, "one reply per turn");
    assert!(
        (1..=2).contains(&replies),
        "one or two turns, never more: {replies}"
    );
    logs.sort_by_key(|l| l.contains("Second thing"));
    let last = logs.last().expect("a turn ran");
    assert!(
        last.contains("First thing") && last.contains("Second thing"),
        "the last turn read both messages: {last}"
    );
}
