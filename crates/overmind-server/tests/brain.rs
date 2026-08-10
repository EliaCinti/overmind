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

/// An MCP memory server that actually remembers, per brain, and answers in the
/// shapes real Wadachi answers in: `store_memory` returns `{"id": n, …}`,
/// `list_memories` returns `{"memories": […]}`, `recall` returns
/// `{"results": […]}`. Rows live in `$BRAIN_DIR/memories.txt`, one JSON object
/// per line, so a test can read a brain off disk and see whose it is.
///
/// With no `BRAIN_DIR` it falls back to `$FALLBACK_BRAIN` — what the unmanaged
/// (shared-brain) mode looks like from here. `$STUB_UNBROWSABLE` makes the read
/// tools answer prose instead, standing in for a conforming server that simply
/// does not expose a list. `$STUB_TOOL_LOG` records which tool was called, so a
/// test can tell searching apart from listing.
const BRAIN_MCP: &str = r#"import sys, json, os
brain = os.environ.get("BRAIN_DIR") or os.environ.get("FALLBACK_BRAIN")
os.makedirs(brain, exist_ok=True)
store = os.path.join(brain, "memories.txt")
unbrowsable = os.environ.get("STUB_UNBROWSABLE")
tool_log = os.environ.get("STUB_TOOL_LOG")

def rows():
    if not os.path.exists(store):
        return []
    out = []
    with open(store) as f:
        for line in f:
            line = line.strip()
            if line:
                out.append(json.loads(line))
    return out

def reply(mid, text):
    print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":text}]}}), flush=True)

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
        if tool_log:
            with open(tool_log, "a") as f:
                f.write(name + "\n")
        if name == "get_context":
            remembered = " | ".join(r["title"] for r in rows())
            reply(mid, "BRAIN[%s] KNOWS: %s" % (brain, remembered))
        elif name in ("store_memory", "store_decision"):
            existing = rows()
            row = {
                "id": len(existing) + 1,
                "kind": "decision" if name == "store_decision" else "memory",
                "title": args.get("title") or args.get("decision") or "",
                "content": args.get("content") or args.get("rationale") or "",
                "project": args.get("project") or "",
                "tags": args.get("tags") or [],
                "category": args.get("category") or "",
                "created_at": "2026-08-09T00:00:00Z",
            }
            with open(store, "a") as f:
                f.write(json.dumps(row) + "\n")
            if os.environ.get("STUB_NO_IDS"):
                reply(mid, "stored")
            else:
                reply(mid, json.dumps({"id": row["id"], "title": row["title"]}))
        elif name in ("list_memories", "list_decisions", "recall"):
            if unbrowsable:
                reply(mid, "I hold a few things but cannot list them.")
            else:
                wanted = "decision" if name == "list_decisions" else "memory"
                if name == "recall":
                    q = (args.get("query") or "").lower()
                    found = [r for r in rows() if q in r["title"].lower()]
                    reply(mid, json.dumps({"results": found, "count": len(found)}))
                else:
                    found = [r for r in rows() if r["kind"] == wanted]
                    key = "decisions" if wanted == "decision" else "memories"
                    reply(mid, json.dumps({key: found, "count": len(found)}))
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
    fn tool_log(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("tools.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// A server with the remembering stub wired up. `managed` mirrors
/// `OVERMIND_MANAGED_BRAIN`.
async fn setup(managed: bool, with_memory: bool) -> Env {
    setup_with(managed, with_memory, "").await
}

/// `stub_env` goes in front of the stub's command line — the hook for
/// `STUB_UNBROWSABLE`, which stands in for a conforming provider that exposes
/// no browsable list.
async fn setup_with(managed: bool, with_memory: bool, stub_env: &str) -> Env {
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let agent = root.join("agent.sh");
    std::fs::write(&agent, MEMORY_AGENT).expect("write agent");

    let memory_cmd = with_memory.then(|| {
        let stub = root.join("brain_mcp.py");
        std::fs::write(&stub, BRAIN_MCP).expect("write stub");
        format!(
            "FALLBACK_BRAIN={} STUB_TOOL_LOG={} {stub_env} {} {}",
            root.join("fallback-brain").display(),
            root.join("tools.log").display(),
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

// ---------- M8 slice 2: provenance and browsing (ADR-0025) ----------

/// The acceptance criterion of the milestone: a memory the organization stored
/// names the task that produced it. ADR-0015 promised this in words in July;
/// this is the test that makes the promise checkable.
#[tokio::test]
async fn a_memory_names_the_task_that_produced_it() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (company, goal) = found_company(&env, "Provenance").await;
    run_task(&env, &company, &goal, "Rewrite the deploy script").await;

    let (s, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["state"], json!("ok"), "browse: {body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "expected one memory: {body}");

    let subject = &items[0]["subject"];
    assert_eq!(subject["type"], json!("task"));
    assert_eq!(subject["title"], json!("Rewrite the deploy script"));
    assert!(
        subject["id"].as_str().is_some_and(|s| !s.is_empty()),
        "subject has no task id: {subject}"
    );
    assert_eq!(items[0]["title"], json!("Rewrite the deploy script"));
}

/// The provenance also goes into the brain as a tag, so a vault opened outside
/// Overmind still says where a memory came from (ADR-0025).
#[tokio::test]
async fn the_brain_itself_records_which_task_a_memory_came_from() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (company, goal) = found_company(&env, "Tagged").await;
    run_task(&env, &company, &goal, "Tag me").await;

    let stored =
        std::fs::read_to_string(env.brain_dir(&company).join("memories.txt")).expect("store");
    assert!(
        stored.contains("task:"),
        "no task tag written into the brain: {stored}"
    );
}

/// Four reasons a browse can be empty, and they must not look alike: a reader
/// who cannot tell "not set up" from "nothing stored yet" cannot act on either.
#[tokio::test]
async fn an_empty_browse_says_why_it_is_empty() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    // 1. No provider at all.
    let env = setup(true, false).await;
    let (company, _) = found_company(&env, "Nothing").await;
    let (_, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(body["state"], json!("no_provider"));

    // 2. Provider present, nothing stored yet — genuinely empty.
    let env = setup(true, true).await;
    let (company, _) = found_company(&env, "Fresh").await;
    let (_, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(body["state"], json!("ok"));
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

    // 3. This company's brain switched off.
    send(
        &env.app,
        "POST",
        &format!("/api/companies/{company}/brain"),
        Some(json!({ "enabled": false })),
    )
    .await;
    let (_, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(body["state"], json!("brain_off"));

    // 4. A conforming provider that exposes no browsable list. The memory loop
    //    still works — only the browsing does not.
    let env = setup_with(true, true, "STUB_UNBROWSABLE=1").await;
    let (company, goal) = found_company(&env, "Opaque").await;
    let saw = run_task(&env, &company, &goal, "Still remembers").await;
    assert!(saw.contains("BRAIN["), "the loop should still work: {saw}");
    let (_, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(body["state"], json!("not_browsable"));
}

/// Search is `recall`, not a filtered list — two different operations, and
/// calling one the other would misrepresent both (ADR-0025).
#[tokio::test]
async fn searching_recalls_instead_of_filtering_a_list() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (company, goal) = found_company(&env, "Searcher").await;
    run_task(&env, &company, &goal, "Harden the deploy script").await;
    run_task(&env, &company, &goal, "Rename a variable").await;

    let (_, listed) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(2));

    let (_, found) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories?q=deploy"),
        None,
    )
    .await;
    assert_eq!(found["state"], json!("ok"));
    let items = found["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "search returned: {found}");
    assert_eq!(items[0]["title"], json!("Harden the deploy script"));
    // …and the provenance survives the search path too.
    assert_eq!(items[0]["subject"]["type"], json!("task"));

    let tools = env.tool_log();
    assert!(
        tools.contains(&"recall".to_string()),
        "a query should reach recall: {tools:?}"
    );
    assert!(
        tools.contains(&"list_memories".to_string()),
        "a bare browse should reach list_memories: {tools:?}"
    );
}

/// A provider that stores but returns no identifier leaves the memory without a
/// subject — degraded, not broken, and never a guessed link (ADR-0025).
#[tokio::test]
async fn a_memory_with_no_identifier_is_shown_without_a_subject() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup_with(true, true, "STUB_NO_IDS=1").await;
    let (company, goal) = found_company(&env, "Anonymous").await;
    run_task(&env, &company, &goal, "Unattributable work").await;

    let (_, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    // The memory is there — the work was still remembered…
    assert_eq!(body["state"], json!("ok"));
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "the memory should still be stored: {body}");
    assert_eq!(items[0]["title"], json!("Unattributable work"));
    // …and its provenance is honestly absent rather than invented.
    assert_eq!(items[0]["subject"], Value::Null, "item: {}", items[0]);

    // Nothing was written to the link table either: a link with no ref to key
    // it on would be a row that can never be matched back.
    let (_, second) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(second["items"][0]["subject"], Value::Null);
}
