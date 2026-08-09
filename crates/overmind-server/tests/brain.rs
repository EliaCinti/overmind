//! M8 slice 1 acceptance tests: every company gets its own brain (ADR-0024).
//!
//! The claim under test is not "the env var differs" — it is that one
//! company's agent **cannot read** another company's memories. So the stub
//! memory server here is a real (tiny) store: it writes memories into the
//! directory `BRAIN_DIR` names and answers `get_context` from that same
//! directory. If the routing were wrong, company B's agent would see company
//! A's secret in its prompt, and the assertion below would say so.
//!
//! Also held here: a fresh company gets its brain directory in one click; a
//! company with its brain switched off runs tasks identically; and
//! `OVERMIND_MANAGED_BRAIN=off` still shares one brain, because someone who
//! deliberately pointed Overmind at their own brain must keep getting it.

use std::path::{Path, PathBuf};
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

fn sh(dir: &Path, cmd: &str) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .output()
        .expect("run shell");
    assert!(out.status.success(), "cmd failed: {cmd}");
}

fn unique_root() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("overmind-brain-{nanos}-{}-{n}", std::process::id()))
}

/// An MCP memory server that actually remembers, per brain. Memories go into
/// `$BRAIN_DIR/memories.txt`, one title per line, and `get_context` reads that
/// file back. With no `BRAIN_DIR` it falls back to `$FALLBACK_BRAIN`, which is
/// what the unmanaged (shared-brain) mode looks like from here.
const BRAIN_MCP: &str = r#"import sys, json, os
brain = os.environ.get("BRAIN_DIR") or os.environ.get("FALLBACK_BRAIN")
os.makedirs(brain, exist_ok=True)
store = os.path.join(brain, "memories.txt")
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"brain-stub","version":"0"}}}), flush=True)
    elif method == "notifications/initialized":
        pass
    elif method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments", {})
        if name == "get_context":
            remembered = ""
            if os.path.exists(store):
                with open(store) as f:
                    remembered = " | ".join(x.strip() for x in f if x.strip())
            text = "BRAIN[%s] KNOWS: %s" % (brain, remembered)
            print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":text}]}}), flush=True)
        elif name in ("store_memory", "store_decision"):
            with open(store, "a") as f:
                f.write(json.dumps(args.get("title") or args.get("decision") or "") + "\n")
            print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"stored"}]}}), flush=True)
        else:
            print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[]}}), flush=True)
    elif mid is not None:
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{}}), flush=True)
"#;

/// Echoes the injected memory context so a test can read what the agent saw.
const MEMORY_AGENT: &str = r#"#!/bin/sh
echo "agent saw memory: $OVERMIND_MEMORY_CONTEXT"
echo done > out.txt
echo '{"total_cost_usd":0.01,"session_id":"s"}'
"#;

fn python() -> &'static str {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "python3"
    } else {
        ""
    }
}

struct Env {
    app: axum::Router,
    root: PathBuf,
}

impl Env {
    fn brain_dir(&self, company: &str) -> PathBuf {
        self.root
            .join("data")
            .join("companies")
            .join(company)
            .join("brain")
    }
    fn fallback_brain(&self) -> PathBuf {
        self.root.join("fallback-brain")
    }
}

/// A server with the remembering stub wired up. `managed` mirrors
/// `OVERMIND_MANAGED_BRAIN`.
async fn setup(managed: bool, with_memory: bool) -> Env {
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let agent = root.join("agent.sh");
    std::fs::write(&agent, MEMORY_AGENT).expect("write agent");

    let memory_cmd = with_memory.then(|| {
        let stub = root.join("brain_mcp.py");
        std::fs::write(&stub, BRAIN_MCP).expect("write stub");
        format!(
            "FALLBACK_BRAIN={} {} {}",
            root.join("fallback-brain").display(),
            python(),
            stub.display()
        )
    });

    let config = overmind_server::Config {
        agent_cmd: Some(format!("sh {}", agent.display())),
        data_dir: root.join("data"),
        heartbeat_ms: 1_000_000,
        memory_cmd,
        managed_brain: managed,
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init");
    Env {
        app: overmind_server::app(state),
        root,
    }
}

/// A company with a repo, a project, a workspace and a goal — enough to run a
/// task. Returns `(company_id, goal_id)`.
async fn found_company(env: &Env, name: &str) -> (String, String) {
    let repo = env.root.join(format!("repo-{name}"));
    std::fs::create_dir_all(&repo).expect("mkdir repo");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo x > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );

    let (_, co) = send(
        &env.app,
        "POST",
        "/api/companies",
        Some(json!({ "name": name })),
    )
    .await;
    let company = co["id"].as_str().expect("company id").to_string();
    let (_, pr) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{company}/projects"),
        Some(json!({ "title": "P" })),
    )
    .await;
    let project = pr["id"].as_str().expect("project id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/projects/{project}/workspaces"),
        Some(json!({ "name": "w", "cwd": repo.to_string_lossy() })),
    )
    .await;
    let (_, goal) = send(
        &env.app,
        "POST",
        &format!("/api/projects/{project}/goals"),
        Some(json!({ "title": "G" })),
    )
    .await;
    (company, goal["id"].as_str().expect("goal id").to_string())
}

/// Run one task to completion and return what the agent printed — which
/// includes the memory context it was given.
async fn run_task(env: &Env, company: &str, goal: &str, title: &str) -> String {
    let (_, agent) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{company}/agents"),
        Some(json!({ "name": "Worker", "archetype": "builder" })),
    )
    .await;
    let agent = agent["id"].as_str().expect("agent id").to_string();
    let (_, task) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{company}/tasks"),
        Some(json!({ "title": title, "description": "do it", "goal_id": goal })),
    )
    .await;
    let task = task["id"].as_str().expect("task id").to_string();
    send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (s, started) = send(
        &env.app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "start: {started}");
    let session = started["session_id"].as_str().expect("session").to_string();
    for _ in 0..100 {
        let (_, sv) = send(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        match sv["status"].as_str().unwrap_or("") {
            "completed" => return sv["output"].as_str().unwrap_or("").to_string(),
            "failed" => panic!("session failed: {sv}"),
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session never finished");
}

/// The heart of the milestone: what one company remembers, another cannot read.
#[tokio::test]
async fn one_companys_memories_are_invisible_to_another() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (acme, acme_goal) = found_company(&env, "Acme").await;
    let (globex, globex_goal) = found_company(&env, "Globex").await;

    // Acme learns something. The title is what the stub stores.
    run_task(
        &env,
        &acme,
        &acme_goal,
        "ACME_SECRET_the_key_is_under_the_mat",
    )
    .await;

    // Globex starts work. Its agent is handed Globex's brain, which is empty.
    let globex_saw = run_task(&env, &globex, &globex_goal, "Globex first task").await;
    assert!(
        !globex_saw.contains("ACME_SECRET"),
        "Globex's agent read Acme's memory: {globex_saw}"
    );
    assert!(
        globex_saw.contains(&*env.brain_dir(&globex).to_string_lossy()),
        "Globex's agent was not pointed at Globex's brain: {globex_saw}"
    );

    // …while Acme's next agent does remember, so the isolation is not just the
    // memory being broken for everyone.
    let acme_saw = run_task(&env, &acme, &acme_goal, "Acme second task").await;
    assert!(
        acme_saw.contains("ACME_SECRET"),
        "Acme's agent lost Acme's own memory: {acme_saw}"
    );

    // And on disk they are two stores, each holding only its own company's
    // work. Globex has a file too — it completed a task, so it remembered
    // something — and what matters is that Acme's secret is not in it.
    let acme_store =
        std::fs::read_to_string(env.brain_dir(&acme).join("memories.txt")).expect("acme store");
    let globex_store =
        std::fs::read_to_string(env.brain_dir(&globex).join("memories.txt")).expect("globex store");
    assert!(acme_store.contains("ACME_SECRET"), "acme: {acme_store}");
    assert!(
        !globex_store.contains("ACME_SECRET"),
        "acme's memory leaked into globex's store: {globex_store}"
    );
    assert!(
        globex_store.contains("Globex first task"),
        "globex: {globex_store}"
    );
}

/// "A fresh company gets a working brain in one click" — the click being
/// company creation, not the first task.
#[tokio::test]
async fn founding_a_company_provisions_its_brain() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (company, _) = found_company(&env, "Fresh").await;

    assert!(
        env.brain_dir(&company).is_dir(),
        "no brain directory at {}",
        env.brain_dir(&company).display()
    );

    let (s, status) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/brain"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(status["managed"], json!(true));
    assert_eq!(status["enabled"], json!(true));
    assert_eq!(
        status["brain_dir"],
        json!(env.brain_dir(&company).to_string_lossy())
    );
}

/// Switching a company's brain off must leave the organization fully
/// functional — the graceful-degradation rule, and an M8 acceptance criterion.
#[tokio::test]
async fn a_company_with_its_brain_off_works_and_remembers_nothing() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (company, goal) = found_company(&env, "Amnesiac").await;

    let (s, _) = send(
        &env.app,
        "POST",
        &format!("/api/companies/{company}/brain"),
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // The task runs to completion exactly as it would with no provider at all…
    let saw = run_task(&env, &company, &goal, "Work without memory").await;
    assert!(
        !saw.contains("BRAIN["),
        "a switched-off brain still answered: {saw}"
    );
    // …and nothing was written into its brain.
    assert!(!env.brain_dir(&company).join("memories.txt").exists());

    // The audit chain records the switch and still verifies.
    let (_, events) = send(&env.app, "GET", "/api/audit/events", None).await;
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"company.brain_toggled"),
        "brain toggle not audited: {kinds:?}"
    );
    let (_, report) = send(&env.app, "GET", "/api/audit/verify", None).await;
    assert_eq!(report["valid"], json!(true));

    // Switching it back on restores memory: the cache must not have pinned the
    // disabled handle.
    send(
        &env.app,
        "POST",
        &format!("/api/companies/{company}/brain"),
        Some(json!({ "enabled": true })),
    )
    .await;
    let saw_again = run_task(&env, &company, &goal, "Work with memory again").await;
    assert!(
        saw_again.contains("BRAIN["),
        "brain did not come back on: {saw_again}"
    );
}

/// `OVERMIND_MANAGED_BRAIN=off` is the escape hatch for someone who pointed
/// Overmind at a brain of their own: no per-company routing, one shared brain,
/// exactly as M7 behaved.
#[tokio::test]
async fn unmanaged_mode_keeps_one_shared_brain() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(false, true).await;
    let (acme, acme_goal) = found_company(&env, "Acme").await;
    let (globex, globex_goal) = found_company(&env, "Globex").await;

    run_task(&env, &acme, &acme_goal, "SHARED_NOTE_from_acme").await;
    let globex_saw = run_task(&env, &globex, &globex_goal, "Globex task").await;

    assert!(
        globex_saw.contains("SHARED_NOTE_from_acme"),
        "unmanaged mode should share one brain: {globex_saw}"
    );
    assert!(
        !env.brain_dir(&acme).exists(),
        "unmanaged mode must not provision per-company brains"
    );
    assert!(env.fallback_brain().join("memories.txt").exists());

    let (_, status) = send(&env.app, "GET", "/api/memory/status", None).await;
    assert_eq!(status["enabled"], json!(true));
    assert_eq!(status["managed"], json!(false));
}

/// With no provider configured at all, the per-company endpoint says so rather
/// than inventing a brain directory nobody will ever write to.
#[tokio::test]
async fn no_provider_means_no_managed_brain() {
    let env = setup(true, false).await;
    let (company, goal) = found_company(&env, "Provider-less").await;

    let (_, status) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/brain"),
        None,
    )
    .await;
    assert_eq!(status["provider_configured"], json!(false));
    assert_eq!(status["managed"], json!(false));
    assert_eq!(status["brain_dir"], Value::Null);
    assert!(!env.brain_dir(&company).exists());

    // And the org is fully functional, which is the whole contract (ADR-0003).
    let saw = run_task(&env, &company, &goal, "Fine without a brain").await;
    assert!(!saw.contains("BRAIN["));
}
