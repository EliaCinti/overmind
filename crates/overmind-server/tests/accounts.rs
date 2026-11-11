//! The accounts Overmind holds, and which one pays (ADR-0051, M34 step 3).
//!
//! No test here spawns the real CLI. The credentials are made-up strings, and
//! deliberately not all shaped like Anthropic's: the point of decision 6 is
//! that a secret is known by its value, not by a vendor's prefix.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use overmind_server::accounts::{self, EnvCredentials, Kind, Source};
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKEN: &str = "sk-ant-oat01-TOKENTOKENTOKENTOKENTOKENTOKENTOKENTOKEN0001";
const KEY_ONE: &str = "sk-ant-api03-KEYONEKEYONEKEYONEKEYONEKEYONEKEYONE-1111";
/// A key no Anthropic-shaped scrubber would recognise.
const KEY_ODD: &str = "AIzaSyD-not-an-anthropic-shape-9f8e7d6c5b4a";
const ENV_KEY: &str = "sk-ant-api03-FROMTHEENVIRONMENTFROMTHEENVIRONMENT-0000";

fn unique_root(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "overmind-accounts-{tag}-{}",
        uuid::Uuid::now_v7().simple()
    ))
}

fn config(data_dir: &Path, env: EnvCredentials) -> overmind_server::Config {
    overmind_server::Config {
        data_dir: data_dir.to_path_buf(),
        agent_cmd: Some("/usr/bin/true".into()),
        env_credentials: env,
        ..overmind_server::Config::default()
    }
}

async fn boot(data_dir: &Path, env: EnvCredentials) -> overmind_server::AppState {
    overmind_server::init_with("sqlite::memory:", config(data_dir, env))
        .await
        .expect("init")
}

/// What an agent's shell would see for the two Anthropic variables, after the
/// spawn has been applied over a server environment that carries `leaked`.
fn what_the_agent_sees(spawn: &accounts::Spawn, leaked: &str) -> String {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg("echo \"${ANTHROPIC_API_KEY:-none} ${CLAUDE_CODE_OAUTH_TOKEN:-none}\"")
        .env("ANTHROPIC_API_KEY", leaked)
        .env("CLAUDE_CODE_OAUTH_TOKEN", leaked);
    spawn.apply(&mut cmd);
    let out = cmd.output().expect("run sh");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn claude() -> &'static dyn overmind_server::provider::Provider {
    overmind_server::provider::current()
}

#[tokio::test]
async fn a_second_account_of_the_same_provider_is_linked_inactive() {
    let root = unique_root("second");
    let state = boot(&root, EnvCredentials::default()).await;

    let first = accounts::link(&state, "anthropic", Kind::Key, KEY_ONE, None)
        .await
        .expect("link the first");
    let second = accounts::link(&state, "anthropic", Kind::Key, KEY_ODD, None)
        .await
        .expect("link the second");

    assert!(first.active, "the provider's first account pays");
    assert!(
        !second.active,
        "a second account never changes who pays by arriving"
    );
    // Two files, not one overwritten: the old fixed `claude-oauth-token` path
    // is exactly what a second account would have clobbered.
    assert_ne!(
        accounts::credential_dir(&state.config, "anthropic", &first.id),
        accounts::credential_dir(&state.config, "anthropic", &second.id)
    );
    let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
    assert_eq!(spawn.account_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(
        what_the_agent_sees(&spawn, "leak"),
        format!("{KEY_ONE} none")
    );
}

#[tokio::test]
async fn activating_one_deactivates_the_other() {
    let root = unique_root("activate");
    let state = boot(&root, EnvCredentials::default()).await;
    let first = accounts::link(&state, "anthropic", Kind::Key, KEY_ONE, None)
        .await
        .expect("link");
    let second = accounts::link(&state, "anthropic", Kind::Key, KEY_ODD, None)
        .await
        .expect("link");

    let now = accounts::activate(&state, &second.id)
        .await
        .expect("activate");
    assert!(now.active);
    let all = accounts::list(&state.pool).await.expect("list");
    let active: Vec<_> = all.iter().filter(|a| a.active).map(|a| &a.id).collect();
    assert_eq!(active, vec![&second.id], "exactly one pays: {all:?}");

    let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
    assert_eq!(
        what_the_agent_sees(&spawn, "leak"),
        format!("{KEY_ODD} none")
    );

    // The rule is the database's, not this module's: a second active row is
    // refused even by a write that goes around the code.
    let both = sqlx::query("UPDATE accounts SET active = 1 WHERE id = ?")
        .bind(&first.id)
        .execute(&state.pool)
        .await;
    assert!(
        both.is_err(),
        "the database must refuse two active accounts for one provider"
    );
}

#[tokio::test]
async fn an_env_key_is_kept_away_while_a_subscription_account_is_active() {
    let root = unique_root("keep-away");
    let state = boot(
        &root,
        EnvCredentials::default().with("ANTHROPIC_API_KEY", ENV_KEY),
    )
    .await;

    // The environment's key is an account, read-only, and -- being the only
    // one -- the payer.
    let all = accounts::list(&state.pool).await.expect("list");
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].source, Source::Environment);
    assert_eq!(all[0].kind, Kind::Key);
    assert!(all[0].active);
    let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
    assert_eq!(
        what_the_agent_sees(&spawn, "leak"),
        format!("{ENV_KEY} none")
    );

    let plan = accounts::link(&state, "anthropic", Kind::Subscription, TOKEN, None)
        .await
        .expect("link the plan");
    accounts::activate(&state, &plan.id)
        .await
        .expect("activate");

    // A key left in the environment silently outranks a login (memory #207),
    // so it is removed -- not merely left unset by us.
    let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
    assert_eq!(what_the_agent_sees(&spawn, "leak"), format!("none {TOKEN}"));

    // And it cannot be disconnected from here: it is removed where it lives.
    let env_id = all[0].id.clone();
    assert!(matches!(
        accounts::disconnect(&state, &env_id).await,
        Err(accounts::AccountError::FromEnvironment)
    ));
}

/// Until the accounts panel exists (M34 step 4) an instance with no account
/// runs on the host's own login, as it always has: nothing is injected, and
/// nothing from the server's environment leaks through either.
#[tokio::test]
async fn with_no_account_the_host_login_is_still_used_for_now() {
    let root = unique_root("host");
    let state = boot(&root, EnvCredentials::default()).await;
    let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
    assert_eq!(spawn.account_id, None);
    assert_eq!(what_the_agent_sees(&spawn, "leak"), "none none");
}

#[tokio::test]
async fn a_stored_credential_is_redacted_by_value() {
    let root = unique_root("redact");
    let state = boot(
        &root,
        EnvCredentials::default().with("ANTHROPIC_API_KEY", "envsecret-0123456789-abcdef"),
    )
    .await;
    accounts::link(&state, "anthropic", Kind::Key, KEY_ODD, None)
        .await
        .expect("link");

    let said = format!("error: the key {KEY_ODD} was refused (code 401)");
    let shown = overmind_server::secrets::scrub(&said);
    assert!(!shown.contains(KEY_ODD), "a stored key survived: {shown}");
    assert!(shown.contains("was refused (code 401)"), "{shown}");

    // The environment's credentials are registered at boot, not only the
    // ones Overmind wrote.
    let shown = overmind_server::secrets::scrub("got envsecret-0123456789-abcdef back");
    assert!(
        !shown.contains("envsecret-0123456789-abcdef"),
        "an environment key survived: {shown}"
    );
}

#[tokio::test]
async fn the_old_token_file_migrates_to_an_account() {
    let root = unique_root("migrate");
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::write(root.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("write token");

    let state = boot(&root, EnvCredentials::default()).await;

    let all = accounts::list(&state.pool).await.expect("list");
    assert_eq!(all.len(), 1, "{all:?}");
    let a = &all[0];
    assert_eq!(
        (a.provider.as_str(), a.kind, a.source, a.active),
        ("anthropic", Kind::Subscription, Source::Stored, true)
    );
    assert!(
        !root.join("claude-oauth-token").exists(),
        "the old file is gone once the account holds it"
    );
    let dir = accounts::credential_dir(&state.config, "anthropic", &a.id);
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&root.join("credentials")), 0o700);
    for entry in std::fs::read_dir(&dir).expect("read dir") {
        assert_eq!(mode(&entry.expect("entry").path()), 0o600);
    }
    let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
    assert_eq!(what_the_agent_sees(&spawn, "leak"), format!("none {TOKEN}"));
}

/// ADR-0037's choice survives the move. With the marker, the plan was paying
/// over the key; without it, the key was.
#[tokio::test]
async fn the_old_choice_of_payer_survives_the_migration() {
    for (marker, pays) in [(true, "token"), (false, "key")] {
        let root = unique_root("choice");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("claude-oauth-token"), TOKEN).expect("write token");
        if marker {
            std::fs::write(root.join("pay-with-plan"), "plan\n").expect("write marker");
        }
        let state = boot(
            &root,
            EnvCredentials::default().with("ANTHROPIC_API_KEY", ENV_KEY),
        )
        .await;
        let spawn = accounts::for_spawn(&state, claude()).await.expect("spawn");
        let expected = match pays {
            "token" => format!("none {TOKEN}"),
            _ => format!("{ENV_KEY} none"),
        };
        assert_eq!(
            what_the_agent_sees(&spawn, "leak"),
            expected,
            "marker={marker}"
        );
        assert!(
            !root.join("pay-with-plan").exists(),
            "the marker is retired"
        );
    }
}

/// A restart must not change who pays: the environment's key is not
/// re-promoted over an account somebody chose.
#[tokio::test]
async fn a_restart_keeps_the_payer_somebody_chose() {
    let root = unique_root("restart");
    let db = root.join("overmind.sqlite");
    std::fs::create_dir_all(&root).expect("mkdir");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let env = || EnvCredentials::default().with("ANTHROPIC_API_KEY", ENV_KEY);

    let state = overmind_server::init_with(&url, config(&root, env()))
        .await
        .expect("init");
    let plan = accounts::link(&state, "anthropic", Kind::Subscription, TOKEN, None)
        .await
        .expect("link");
    accounts::activate(&state, &plan.id)
        .await
        .expect("activate");
    state.pool.close().await;

    let again = overmind_server::init_with(&url, config(&root, env()))
        .await
        .expect("init again");
    let active = accounts::active(&again.pool, "anthropic")
        .await
        .expect("active")
        .expect("someone pays");
    assert_eq!(active.id, plan.id);
}

// ---------- the API ----------

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
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
    let raw = String::from_utf8_lossy(&bytes).to_string();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, raw)
}

async fn audit_kinds_and_payloads(state: &overmind_server::AppState) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT kind, payload FROM audit_events WHERE kind LIKE 'account.%' ORDER BY seq",
    )
    .fetch_all(&state.pool)
    .await
    .expect("audit")
}

/// Connect a key, make it active, disconnect it: each step is the owner's,
/// each is audited, and no answer and no audit row ever carries the secret.
#[tokio::test]
async fn an_account_is_connected_activated_and_disconnected_without_its_secret_leaving() {
    let root = unique_root("api");
    let state = boot(&root, EnvCredentials::default()).await;
    let app = common::claimed(overmind_server::app(state.clone()), &root).await;

    let (s, first, raw) = send(
        &app,
        "POST",
        "/api/accounts",
        Some(json!({ "provider": "anthropic", "kind": "key", "key": KEY_ONE })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{raw}");
    assert!(!raw.contains(KEY_ONE), "the answer carried the key: {raw}");
    assert_eq!(first["last_four"], json!("1111"));
    assert_eq!(first["active"], json!(true));

    let (_, second, _) = send(
        &app,
        "POST",
        "/api/accounts",
        Some(json!({ "provider": "anthropic", "kind": "key", "key": KEY_ODD })),
    )
    .await;
    let second_id = second["id"].as_str().expect("id").to_string();

    let (s, list, raw) = send(&app, "GET", "/api/accounts", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!raw.contains(KEY_ONE) && !raw.contains(KEY_ODD), "{raw}");
    assert_eq!(list["accounts"].as_array().map(Vec::len), Some(2), "{list}");

    let (s, _, raw) = send(
        &app,
        "POST",
        &format!("/api/accounts/{second_id}/activate"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{raw}");

    let dir = accounts::credential_dir(&state.config, "anthropic", &second_id);
    assert!(dir.exists());
    let (s, _, raw) = send(&app, "DELETE", &format!("/api/accounts/{second_id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{raw}");
    assert!(!dir.exists(), "disconnecting deletes the secret");

    let audit = audit_kinds_and_payloads(&state).await;
    let kinds: Vec<&str> = audit.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "account.connected",
            "account.connected",
            "account.activated",
            "account.disconnected"
        ]
    );
    for (_, payload) in &audit {
        assert!(
            !payload.contains(KEY_ONE) && !payload.contains(KEY_ODD),
            "an audit row carried a secret: {payload}"
        );
        assert!(payload.contains("anthropic"), "{payload}");
    }
}

#[tokio::test]
async fn a_bad_account_request_is_refused() {
    let root = unique_root("api-bad");
    let state = boot(
        &root,
        EnvCredentials::default().with("ANTHROPIC_API_KEY", ENV_KEY),
    )
    .await;
    let app = common::claimed(overmind_server::app(state.clone()), &root).await;
    for body in [
        json!({ "provider": "anthropic", "kind": "key", "key": "   " }),
        json!({ "provider": "nobody", "kind": "key", "key": KEY_ONE }),
        // A subscription is connected by signing in, never pasted.
        json!({ "provider": "anthropic", "kind": "subscription", "key": TOKEN }),
    ] {
        let (s, _, raw) = send(&app, "POST", "/api/accounts", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} → {raw}");
    }
    let env = accounts::list(&state.pool).await.expect("list");
    let (s, _, raw) = send(
        &app,
        "DELETE",
        &format!("/api/accounts/{}", env[0].id),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{raw}");
    let (s, _, _) = send(&app, "POST", "/api/accounts/no-such-id/activate", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// ---------- a run in flight ----------

fn sh(dir: &Path, cmd: &str) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .output()
        .expect("run shell");
    assert!(out.status.success(), "cmd failed: {cmd}");
}

/// An instance whose agent is the shell script `stub`, with a repository to
/// work in. Every run here is the stub; nothing spawns the real CLI.
async fn instance_with_stub(
    tag: &str,
    stub: &str,
) -> (overmind_server::AppState, axum::Router, PathBuf) {
    let root = unique_root(tag);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    sh(&repo, "git init -q -b main");
    sh(
        &repo,
        "echo x > README.md && git add . && git -c user.email=t@t -c user.name=T commit -qm init",
    );
    let script = root.join("stub.sh");
    std::fs::write(&script, stub).expect("write stub");
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            agent_cmd: Some(format!("sh {}", script.display())),
            data_dir: root.join("data"),
            heartbeat_ms: 1_000_000,
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = common::claimed(overmind_server::app(state.clone()), &root.join("data")).await;
    (state, app, root)
}

/// Found a company with one agent and one task, and start the task.
async fn start_a_task(app: &axum::Router, root: &Path) -> String {
    let repo = root.join("repo");
    let (_, co, _) = send(app, "POST", "/api/companies", Some(json!({ "name": "Co" }))).await;
    let company = co["id"].as_str().expect("id").to_string();
    let (_, pr, _) = send(
        app,
        "POST",
        &format!("/api/companies/{company}/projects"),
        Some(json!({ "title": "P" })),
    )
    .await;
    let project = pr["id"].as_str().expect("id").to_string();
    send(
        app,
        "POST",
        &format!("/api/projects/{project}/workspaces"),
        Some(json!({ "name": "main", "cwd": repo.to_string_lossy() })),
    )
    .await;
    let (_, goal, _) = send(
        app,
        "POST",
        &format!("/api/projects/{project}/goals"),
        Some(json!({ "title": "G" })),
    )
    .await;
    let goal = goal["id"].as_str().expect("id").to_string();
    let (s, agent, raw) = send(
        app,
        "POST",
        &format!("/api/companies/{company}/agents"),
        Some(json!({ "name": "A", "archetype": "builder", "traits": { "monthly_budget_cents": 1000 } })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{raw}");
    let agent = agent["id"].as_str().expect("id").to_string();
    let (_, task, _) = send(
        app,
        "POST",
        &format!("/api/companies/{company}/tasks"),
        Some(json!({ "title": "T", "goal_id": goal })),
    )
    .await;
    let task = task["id"].as_str().expect("id").to_string();
    send(
        app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
    )
    .await;
    let (s, _, raw) = send(
        app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": agent })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{raw}");

    task
}

/// Switching takes effect at the next spawn: a run already going finishes on
/// the account it started with, and its cost is recorded against that one.
#[tokio::test]
async fn a_run_in_flight_is_billed_to_the_account_it_started_on() {
    let gate = unique_root("gate");
    let (state, app, root) = instance_with_stub(
        "in-flight",
        &format!(
            "#!/bin/sh\nwhile [ ! -e '{}' ]; do sleep 0.1; done\necho work > out.txt\necho '{{\"model\":\"stub\",\"session_id\":\"s\",\"total_cost_usd\":0.05,\"usage\":{{\"input_tokens\":10,\"output_tokens\":5}}}}'\n",
            gate.display()
        ),
    )
    .await;
    let first = accounts::link(&state, "anthropic", Kind::Key, KEY_ONE, None)
        .await
        .expect("link");
    let second = accounts::link(&state, "anthropic", Kind::Key, KEY_ODD, None)
        .await
        .expect("link");

    let task = start_a_task(&app, &root).await;

    // Wait until the stub is actually running, then switch under it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let running: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM agent_task_sessions WHERE task_id = ? AND status = 'running'",
        )
        .bind(&task)
        .fetch_optional(&state.pool)
        .await
        .expect("poll");
        if running.is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the run never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    accounts::activate(&state, &second.id)
        .await
        .expect("switch");
    std::fs::write(&gate, "go").expect("open the gate");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let billed: String = loop {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT account_id FROM cost_events WHERE task_id = ?")
                .bind(&task)
                .fetch_optional(&state.pool)
                .await
                .expect("poll");
        if let Some((account,)) = row {
            break account.unwrap_or_default();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the run never finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(billed, first.id, "billed to the account it started on");
}

/// An agent can print its own credential -- `env`, a CLI's error that echoes
/// its key -- and what it prints becomes the run's stored output, its error
/// and its live narration. Known by value, the key is gone from all of them,
/// shaped like Anthropic's or not.
#[tokio::test]
async fn a_credential_an_agent_prints_never_reaches_the_record() {
    let (state, app, root) = instance_with_stub(
        "prints",
        "#!/bin/sh\necho \"my key is $ANTHROPIC_API_KEY\"\necho \"refused: $ANTHROPIC_API_KEY\" >&2\nexit 1\n",
    )
    .await;
    accounts::link(&state, "anthropic", Kind::Key, KEY_ODD, None)
        .await
        .expect("link");
    let task = start_a_task(&app, &root).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let (output, error) = loop {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT output, last_error FROM agent_task_sessions WHERE task_id = ? AND finished_at IS NOT NULL",
        )
        .bind(&task)
        .fetch_optional(&state.pool)
        .await
        .expect("poll");
        if let Some((o, e)) = row {
            break (o.unwrap_or_default(), e.unwrap_or_default());
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the run never finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(output.contains("my key is"), "the run did speak: {output}");
    assert!(
        !output.contains(KEY_ODD),
        "the stored output kept the key: {output}"
    );
    assert!(
        !error.contains(KEY_ODD),
        "the stored error kept the key: {error}"
    );
}
