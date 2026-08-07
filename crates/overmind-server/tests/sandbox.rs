//! M10 acceptance tests (ADR-0023): the agent runs in a cage.
//!
//! Until now an agent ran as the user, with the whole machine reachable —
//! `~/.ssh`, the browser profile, and Overmind's own `overmind.sqlite` with the
//! audit chain in it. The declared capabilities said as much and M14 was honest
//! about it; this is the milestone that stops the honesty from being expensive.
//!
//! Every probe here comes in a pair: the same script under the cage and with
//! the cage off. The pair is the point — a denial only proves something if the
//! identical run succeeds without the sandbox, otherwise the test could be
//! passing because the script was broken.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Probes what it can reach and reports each answer as an artifact.
///
/// `$HOME` is the target because it exists on every machine, is never granted
/// wholesale by the profile, and nothing has to be created to test it — a
/// security test that litters someone's home directory is not a good trade.
const PROBING_STUB: &str = r#"#!/bin/sh
( ls "$HOME" >/dev/null 2>&1 && echo REACHABLE || echo DENIED ) > home.txt
( ls /Users >/dev/null 2>&1 && echo REACHABLE || echo DENIED ) > users.txt
echo inside > mine.txt
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
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

/// Run one knowledge task with the given sandbox setting and return the
/// artifacts the stub left behind, keyed by filename.
async fn probe(sandbox: bool) -> std::collections::HashMap<String, String> {
    probe_with(sandbox, PROBING_STUB).await
}

async fn probe_with(sandbox: bool, stub: &str) -> std::collections::HashMap<String, String> {
    let root = std::env::temp_dir().join(format!("overmind-sb-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&root).expect("create test root");
    let script = root.join("stub.sh");
    std::fs::write(&script, stub).expect("write stub");

    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", script.display())),
        data_dir: root.join("data"),
        sandbox,
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
        Some(json!({ "name": "Locked Co" })),
    )
    .await;
    let company_id = company["id"].as_str().expect("company").to_string();
    let agent_id = company["ceo"]["id"].as_str().expect("ceo").to_string();

    let (_, task) = send(
        &app,
        "POST",
        &format!("/api/companies/{company_id}/tasks"),
        Some(json!({
            "title": "Look around",
            "description": "Report what you can reach.",
            "execution_kind": "knowledge",
        })),
    )
    .await;
    let task_id = task["id"].as_str().expect("task id").to_string();
    send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (status, started) = send(
        &app,
        "POST",
        &format!("/api/tasks/{task_id}/start"),
        Some(json!({ "agent_id": agent_id })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "start failed: {started}");

    let session_id = started["session_id"].as_str().expect("session id");
    for _ in 0..100 {
        let (_, s) = send(&app, "GET", &format!("/api/sessions/{session_id}"), None).await;
        if matches!(s["status"].as_str().unwrap_or(""), "completed" | "failed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let (_, artifacts) = send(
        &app,
        "GET",
        &format!("/api/tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    artifacts["artifacts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| {
            Some((
                a["title"].as_str()?.to_string(),
                a["content"].as_str().unwrap_or("").trim().to_string(),
            ))
        })
        .collect()
}

/// The M10 criterion, for the half a sandbox can carry.
#[tokio::test]
async fn a_caged_agent_cannot_reach_the_machine_it_runs_on() {
    if !overmind_server::sandbox::available() {
        eprintln!("no sandbox on this platform — skipping");
        return;
    }
    let caged = probe(true).await;

    assert_eq!(
        caged.get("home.txt").map(String::as_str),
        Some("DENIED"),
        "the home directory must be out of reach: {caged:?}"
    );
    assert_eq!(
        caged.get("users.txt").map(String::as_str),
        Some("DENIED"),
        "and so must every other home on the machine: {caged:?}"
    );
    // Note what is *not* asserted: listing `/` itself succeeds, because the
    // system base profile we import grants it. It reveals the names of
    // top-level directories and nothing inside them — `/Users` above is the
    // probe that matters, and it is denied.
    // The cage is not a wall around everything: the agent still does its job.
    assert_eq!(
        caged.get("mine.txt").map(String::as_str),
        Some("inside"),
        "its own run directory stays writable: {caged:?}"
    );
}

/// The control. Without this, the test above could be passing because the
/// probe script never ran at all.
#[tokio::test]
async fn the_same_agent_uncaged_reaches_everything() {
    let free = probe(false).await;

    assert_eq!(
        free.get("home.txt").map(String::as_str),
        Some("REACHABLE"),
        "with the cage off the home is reachable — which is what the cage is for: {free:?}"
    );
    assert_eq!(
        free.get("mine.txt").map(String::as_str),
        Some("inside"),
        "{free:?}"
    );
}

/// Slice 2: git works, and has nothing to push with.
///
/// The stub is deliberately hostile — it configures a credential helper in its
/// *own* repository, which is the one place an agent can write config. If the
/// isolation only overrode the user's global file, that would be the way round it.
const GIT_STUB: &str = r#"#!/bin/sh
git init -q . 2>/dev/null
git config credential.helper osxkeychain
echo hello > f.txt && git add f.txt
( git commit -q -m probe >/dev/null 2>&1 && echo WORKS || echo BROKEN ) > commit.txt
git config --get-all credential.helper | tail -1 > lasthelper.txt
( printf 'protocol=https\nhost=github.com\n\n' | git credential fill 2>/dev/null | grep -q '^password=' && echo GOTCREDS || echo NOCREDS ) > creds.txt
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
"#;

#[tokio::test]
async fn git_still_works_and_has_no_credentials_to_push_with() {
    let out = probe_with(true, GIT_STUB).await;

    // The cage alone would have broken git outright: it reads ~/.gitconfig
    // before doing anything, and a denial there is fatal. Breaking the tool is
    // not securing it.
    assert_eq!(
        out.get("commit.txt").map(String::as_str),
        Some("WORKS"),
        "an agent must still be able to use git on its own worktree: {out:?}"
    );

    // Our reset is applied at command-line precedence, so it lands *after* the
    // repository's own helper and empties the list rather than joining it.
    assert_eq!(
        out.get("lasthelper.txt").map(String::as_str),
        Some(""),
        "the repo configured a helper and it must not be the last word: {out:?}"
    );
    assert_eq!(
        out.get("creds.txt").map(String::as_str),
        Some("NOCREDS"),
        "no credentials reach the agent, whatever the repo asked for: {out:?}"
    );
}
