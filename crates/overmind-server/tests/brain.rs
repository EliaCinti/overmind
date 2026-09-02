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

mod common;

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
const BRAIN_MCP: &str = r#"import sys, json, os, fcntl
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
        if name == "brain_watermark":
            if os.environ.get("STUB_NO_WATERMARK"):
                reply(mid, "I do not know that tool.")
            else:
                rs = rows()
                reply(mid, json.dumps({
                    "memories": max([r["id"] for r in rs if r["kind"] == "memory"] or [0]),
                    "decisions": max([r["id"] for r in rs if r["kind"] == "decision"] or [0]),
                    "project": args.get("project") or "",
                }))
        elif name == "get_context":
            remembered = " | ".join(r["title"] for r in rows())
            reply(mid, "BRAIN[%s] KNOWS: %s" % (brain, remembered))
        elif name in ("store_memory", "store_decision"):
            # One writer at a time, as a real provider is. Overmind's memory
            # pool runs stores concurrently, and two stub processes that each
            # read the file before the other appended would see neither the
            # other's row nor the collision -- measured as a flake on CI's
            # macOS runner, where the two finalizes landed together.
            lock = open(store + ".lock", "w")
            fcntl.flock(lock, fcntl.LOCK_EX)
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
            # ADR-0026: anything written after the caller's position that shares
            # a word with what it just wrote. Crude on purpose — this stub
            # stands in for a provider, and the real scoring lives in Wadachi.
            collisions = []
            wm = args.get("since_watermark") or {}
            if wm:
                floor = wm.get("memories") or 0
                mine = {w for w in row["title"].lower().split() if len(w) > 3}
                for r in existing:
                    if r["kind"] != "memory" or r["id"] <= floor:
                        continue
                    theirs = {w for w in r["title"].lower().split() if len(w) > 3}
                    if mine & theirs:
                        collisions.append({"kind": "memory", "id": r["id"],
                                           "title": r["title"], "similarity": 0.9})
            fcntl.flock(lock, fcntl.LOCK_UN)
            lock.close()
            if os.environ.get("STUB_NO_IDS"):
                reply(mid, "stored")
            else:
                reply(mid, json.dumps({"id": row["id"], "title": row["title"],
                                       "collisions": collisions}))
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

/// An agent that waits for the test to let it finish.
///
/// Overlap is the whole point of ADR-0026: a collision test has to keep two runs
/// open at once, and a token test has to catch one while it is still going.
/// Both used to buy that with `sleep 2` and the hope that everything else
/// happened inside those two seconds.
///
/// It did, on a developer's machine. On a loaded macOS runner it did not, and
/// the test failed reporting no collision — a red build that says nothing about
/// the code, which is the worst kind. So the wall clock is gone: the agent
/// blocks until the test creates `gate`, and the test creates it only once the
/// state it wanted to arrange exists. The overlap is now constructed rather
/// than raced for.
///
/// Bounded at roughly thirty seconds so a test that dies before releasing its
/// agents fails there instead of hanging until the session timeout.
fn gated_agent(gate: &std::path::Path) -> String {
    format!(
        r#"#!/bin/sh
i=0
while [ ! -f "{}" ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done
echo "agent saw memory: $OVERMIND_MEMORY_CONTEXT"
echo done > out.txt
echo '{{"total_cost_usd":0.01,"session_id":"s"}}'
"#,
        gate.display()
    )
}

/// A path in temp for [`gated_agent`], which the cage grants on every platform.
fn gate_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("overmind-gate-{}", uuid::Uuid::now_v7().simple()))
}

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
    /// The per-run MCP config file the runner writes (ADR-0027).
    fn mcp_config_path(session: &str) -> PathBuf {
        std::env::temp_dir().join(format!("overmind-mcp-{session}.json"))
    }

    /// The token the agent would use, read from that file — which is the real
    /// artifact under test: if it is not written, or written without a token,
    /// the agent has no way to reach memory at all.
    /// Async, and the `tokio::time::sleep` matters: a `std::thread::sleep`
    /// here blocks the current-thread runtime the test runs on, so the spawned
    /// runner never gets scheduled and the file it would write never appears.
    /// The symptom is a timeout that looks exactly like a bug in the code
    /// under test.
    async fn session_token(&self, session: &str) -> String {
        let path = Self::mcp_config_path(session);
        for _ in 0..60 {
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(v) = serde_json::from_str::<Value>(&text)
                && let Some(t) = v["mcpServers"]["overmind"]["headers"]["Authorization"]
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix("Bearer ")
                && !t.is_empty()
            {
                return t.to_string();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no MCP config was written for session {session} at {path:?}");
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
    setup_full(managed, with_memory, stub_env, MEMORY_AGENT).await
}

async fn setup_full(managed: bool, with_memory: bool, stub_env: &str, agent_body: &str) -> Env {
    let root = unique_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    let agent = root.join("agent.sh");
    std::fs::write(&agent, agent_body).expect("write agent");

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
        app: common::claimed(overmind_server::app(state), &root.join("data")).await,
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
/// Create an agent, a task, and start it. Returns the session id without
/// waiting, so a caller can hold two runs open at once.
async fn start_task(env: &Env, company: &str, goal: &str, title: &str) -> String {
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
    started["session_id"].as_str().expect("session").to_string()
}

/// Wait for a session to finish, and hand back what the agent printed.
async fn await_session(env: &Env, session: &str) -> String {
    for _ in 0..150 {
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

/// Start a task and wait for it — the ordinary case.
async fn run_task(env: &Env, company: &str, goal: &str, title: &str) -> String {
    let session = start_task(env, company, goal, title).await;
    await_session(env, &session).await
}

/// Wait for what the assertion is actually about.
///
/// The runner marks a session `completed` and records what the organization
/// learned *after* that -- deliberately, because a hung memory provider must
/// never hold a run open ("Best-effort; never fatal", `runner.rs`). So
/// `await_session` returning is not "the memory landed", and a test that reads
/// the store the instant it returns is racing a write that is on purpose off
/// the critical path. On a loaded runner that race is lost: seen on macOS CI,
/// where `memories.txt` still held only the founding memory.
async fn await_brain(env: &Env, company: &str, needle: &str) -> String {
    let path = env.brain_dir(company).join("memories.txt");
    for _ in 0..150 {
        if let Ok(stored) = std::fs::read_to_string(&path)
            && stored.contains(needle)
        {
            return stored;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let stored = std::fs::read_to_string(&path).unwrap_or_default();
    panic!("{needle:?} never reached the brain of {company}: {stored}");
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
    let acme_store = await_brain(&env, &acme, "ACME_SECRET").await;
    // Globex's own write is awaited too, so the negative assertion below is
    // made against a store that has finished being written -- otherwise it
    // could pass because nothing had landed yet, which proves nothing.
    let globex_store = await_brain(&env, &globex, "Globex first task").await;
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

/// A brain is born knowing who the company is (M21). Before this, a fresh
/// brain was empty and an agent asked to write about the company reached for
/// world knowledge — M19's acceptance run got a confident document about
/// somebody else's product of the same name.
#[tokio::test]
async fn a_brain_is_born_knowing_who_the_company_is() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (company, _) = found_company(&env, "Aurora").await;

    // On disk: the brain's first memory states the identity.
    let stored =
        std::fs::read_to_string(env.brain_dir(&company).join("memories.txt")).expect("store");
    assert!(
        stored.contains("Who Aurora is"),
        "no founding memory in the brain: {stored}"
    );
    assert!(
        stored.contains("This company is Aurora"),
        "the founding memory does not state the identity: {stored}"
    );

    // Through the browse: present, and honestly without a subject — it was
    // produced by founding the company, not by a task or a meeting.
    let (s, body) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let items = body["items"].as_array().expect("items");
    let item = items
        .iter()
        .find(|i| i["title"] == json!("Who Aurora is"))
        .unwrap_or_else(|| panic!("founding memory not browsable: {body}"));
    assert_eq!(item["subject"], Value::Null, "item: {item}");
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
    // …and nothing new was written into its brain: the founding memory (M21)
    // is all it holds, stored at creation while the brain was still on.
    let stored =
        std::fs::read_to_string(env.brain_dir(&company).join("memories.txt")).expect("store");
    assert!(
        !stored.contains("Work without memory"),
        "a switched-off brain still stored the task: {stored}"
    );

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
    // Wait for the answer this test is about, not for a sign of it. The runner
    // stores the memory and only *then* writes the link row that `subject` is
    // read from, so waiting for the memory to reach the store would move the
    // race one step later instead of removing it: the browse could return the
    // memory with a null subject.
    let mut body = json!(null);
    for _ in 0..150 {
        let (s, v) = send(
            &env.app,
            "GET",
            &format!("/api/companies/{company}/memory/memories"),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["state"], json!("ok"), "browse: {v}");
        let ready = v["items"].as_array().is_some_and(|items| {
            items.iter().any(|i| {
                i["title"] == json!("Rewrite the deploy script")
                    && i["subject"]["type"] == json!("task")
            })
        });
        body = v;
        if ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let items = body["items"].as_array().expect("items");
    // Two memories: the founding one (M21) and the task's. The one under test
    // is found by title, not by position — a browse order is not a contract.
    assert_eq!(items.len(), 2, "expected founding + task memory: {body}");
    let item = items
        .iter()
        .find(|i| i["title"] == json!("Rewrite the deploy script"))
        .unwrap_or_else(|| panic!("task memory not in browse: {body}"));

    let subject = &item["subject"];
    assert_eq!(subject["type"], json!("task"));
    assert_eq!(subject["title"], json!("Rewrite the deploy script"));
    assert!(
        subject["id"].as_str().is_some_and(|s| !s.is_empty()),
        "subject has no task id: {subject}"
    );
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

    let stored = await_brain(&env, &company, "task:").await;
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

    // 2. Provider present. A fresh company is no longer genuinely empty —
    //    its brain is born holding the founding memory (M21) — so the state
    //    is a working browse with one item, not an empty one.
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
    let items = body["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        1,
        "a fresh brain holds its founding memory: {body}"
    );
    assert_eq!(items[0]["title"], json!("Who Fresh is"));

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
    // Founding memory + two tasks (M21).
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(3));

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
    let item = items
        .iter()
        .find(|i| i["title"] == json!("Unattributable work"))
        .unwrap_or_else(|| panic!("the memory should still be stored: {body}"));
    // …and its provenance is honestly absent rather than invented.
    assert_eq!(item["subject"], Value::Null, "item: {item}");

    // Nothing was written to the link table either: a link with no ref to key
    // it on would be a row that can never be matched back.
    let (_, second) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/memory/memories"),
        None,
    )
    .await;
    let second_item = second["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["title"] == json!("Unattributable work"))
        .cloned()
        .expect("still stored");
    assert_eq!(second_item["subject"], Value::Null);
}

// ── M8 slice 4: change awareness across concurrent agents (ADR-0026) ────────
//
// Visibility was never the problem — Wadachi 0.15.0 measures eight processes
// writing one brain and four readers seeing all of it. What was missing is a
// reason to look. These hold the Overmind half: a position taken at checkout,
// handed back at completion, and a human told when two agents wrote about the
// same thing without seeing each other.

async fn notifications(env: &Env, company: &str) -> Vec<Value> {
    let (_, v) = send(
        &env.app,
        "GET",
        &format!("/api/companies/{company}/notifications"),
        None,
    )
    .await;
    v.as_array()
        .cloned()
        .or_else(|| v["notifications"].as_array().cloned())
        .unwrap_or_default()
}

/// The position is taken before the run and kept with the session, so the
/// completion write has something to compare against.
#[tokio::test]
async fn a_run_records_where_the_brain_was_when_it_started() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (acme, goal) = found_company(&env, "Acme").await;

    run_task(&env, &acme, &goal, "First thing").await;

    // The stub answers `brain_watermark`, so the tool log proves it was asked
    // at checkout — not inferred, not defaulted.
    let log = env.tool_log();
    assert!(
        log.iter().any(|t| t == "brain_watermark"),
        "checkout never took a watermark: {log:?}"
    );
}

/// Sequential runs must NOT collide. The second one checks out after the first
/// has already written, so the first is below its watermark — outside the
/// window by construction. A system that reported this would be reporting every
/// pair of related memories ever written, which is the noise that makes people
/// stop reading.
#[tokio::test]
async fn work_that_did_not_overlap_is_not_a_collision() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup(true, true).await;
    let (acme, goal) = found_company(&env, "Acme").await;

    run_task(&env, &acme, &goal, "deprecate the legacy cart").await;
    run_task(&env, &acme, &goal, "extend the legacy cart").await;

    let ns = notifications(&env, &acme).await;
    assert!(
        !ns.iter().any(|n| n["kind"] == "memory.collision"),
        "sequential runs cannot have missed each other: {ns:?}"
    );
}

/// The case the milestone exists for: two runs genuinely open at the same time,
/// writing about the same subject. Whichever commits second finds the other in
/// its window, and a human is told.
#[tokio::test]
async fn two_overlapping_runs_writing_the_same_thing_are_reported() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let gate = gate_path();
    let env = setup_full(true, true, "", &gated_agent(&gate)).await;
    let (acme, goal) = found_company(&env, "Acme").await;

    // Both start before either can finish, by construction rather than by
    // racing a clock: neither agent gets past the gate until both have been
    // checked out, so both watermarks are taken while the brain holds neither
    // write. That is the overlap ADR-0026 is about.
    let a = start_task(&env, &acme, &goal, "deprecate the legacy cart").await;
    let b = start_task(&env, &acme, &goal, "extend the legacy cart").await;
    std::fs::write(&gate, b"go").expect("release both agents");
    await_session(&env, &a).await;
    await_session(&env, &b).await;
    let _ = std::fs::remove_file(&gate);

    // The session is marked complete BEFORE the memory is stored and the
    // collision reported: memory is best-effort and comes after, by design.
    // So the notification is eventual, and the test waits for it the way a
    // person would -- measured as a flake on CI's macOS runner, where the
    // store landed after the first read of the inbox.
    let mut ns = Vec::new();
    let mut collision = None;
    for _ in 0..100 {
        ns = notifications(&env, &acme).await;
        collision = ns.iter().find(|n| n["kind"] == "memory.collision").cloned();
        if collision.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(c) = collision else {
        panic!("two overlapping runs on the same subject went unreported: {ns:?}");
    };
    let params = &c["params"];
    assert!(
        params["collisions"]
            .as_array()
            .is_some_and(|xs| !xs.is_empty()),
        "the notification must name the other side: {c}"
    );
    assert!(
        c["approval_id"].is_null(),
        "a collision is informational — both writes already happened: {c}"
    );
}

/// A company whose provider does not implement the tool keeps working, with no
/// window and no candidates. ADR-0003 keeps memory optional; this keeps the
/// *awareness* optional too.
#[tokio::test]
async fn a_provider_without_watermarks_changes_nothing() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let env = setup_with(true, true, "STUB_NO_WATERMARK=1").await;
    let (acme, goal) = found_company(&env, "Acme").await;

    let saw = run_task(&env, &acme, &goal, "Works without watermarks").await;
    assert!(!saw.is_empty(), "the run must complete exactly as before");

    let ns = notifications(&env, &acme).await;
    assert!(
        !ns.iter().any(|n| n["kind"] == "memory.collision"),
        "no watermark means no window, so nothing to report: {ns:?}"
    );
}

// ── M8 slice 3 / M9 foundation: agents reach memory through Overmind ────────
//
// ADR-0027. The direct route — a Wadachi over stdio inside the agent's cage —
// cannot work: ADR-0023's profile reaches neither the brain directory nor
// anything outside the run dir. So the agent talks to Overmind. These hold the
// three boundaries that decision rests on.

async fn mcp(env: &Env, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let response = env
        .app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).expect("build"))
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

/// A caller Overmind cannot identify gets nothing — and gets the same nothing
/// whether the token is absent, malformed or retired.
#[tokio::test]
async fn the_mcp_endpoint_refuses_a_caller_it_cannot_identify() {
    let env = setup(true, true).await;
    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });

    for token in [None, Some(""), Some("not-a-token")] {
        let (status, _) = mcp(&env, token, call.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "token {token:?} was let through"
        );
    }
}

/// Agents read; Overmind writes. The write tools are not on the list, and
/// asking for one says so rather than failing vaguely.
#[tokio::test]
async fn agents_are_offered_reads_and_refused_writes() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    // A gated agent: the config file exists only while the run does, and an
    // ordinary stub finishes in milliseconds — too fast to observe what we are
    // testing. The gate holds the run open until we are done looking, instead
    // of a sleep that has to outlast everything else the test does.
    let gate = gate_path();
    let env = setup_full(true, true, "", &gated_agent(&gate)).await;
    let (acme, goal) = found_company(&env, "Acme").await;
    let session = start_task(&env, &acme, &goal, "A task with a live token").await;
    let token = env.session_token(&session).await;

    let (status, v) = mcp(
        &env,
        Some(&token),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<String> = v["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        names.contains(&"recall".to_string()),
        "no recall: {names:?}"
    );
    assert!(names.contains(&"why".to_string()), "no why: {names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("store_")),
        "a write tool is on offer: {names:?}"
    );

    // And asking anyway is answered, not swallowed.
    let (_, v) = mcp(
        &env,
        Some(&token),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "store_memory", "arguments": {} } }),
    )
    .await;
    assert_eq!(v["result"]["isError"], json!(true));
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("provenance"),
        "the refusal should say why: {text}"
    );

    std::fs::write(&gate, b"go").expect("release the agent");
    await_session(&env, &session).await;
    let _ = std::fs::remove_file(&gate);
}

/// The token is the identity: a request names no company, and once the run is
/// over the token stops working. A key left in a door is the failure ADR-0015
/// flagged and ADR-0027 had to answer.
#[tokio::test]
async fn a_token_dies_with_its_run() {
    if python().is_empty() {
        eprintln!("skipping: python3 not available");
        return;
    }
    // A gated agent, so "during the run" is a fact rather than a bet on the
    // next few lines executing inside a two-second sleep.
    let gate = gate_path();
    let env = setup_full(true, true, "", &gated_agent(&gate)).await;
    let (acme, goal) = found_company(&env, "Acme").await;
    let session = start_task(&env, &acme, &goal, "A task that will finish").await;
    let token = env.session_token(&session).await;

    let live = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let (status, _) = mcp(&env, Some(&token), live.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the token should work during the run"
    );

    std::fs::write(&gate, b"go").expect("release the agent");
    await_session(&env, &session).await;
    let _ = std::fs::remove_file(&gate);

    let (status, _) = mcp(&env, Some(&token), live).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the token outlived its run"
    );
    assert!(
        !Env::mcp_config_path(&session).exists(),
        "the config file outlived its run — a token left on disk"
    );
}
