//! The archive is the instance (M31, ADR-0044) -- slice A: the export.
//!
//! Every test here reads the archive back the way a stranger would: as bytes,
//! as tar entries, as a database opened cold. What the manifest claims is
//! checked against what the entries are; what the threat model claims about
//! credentials is checked by searching the bytes for them.

use std::io::Read;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn send_raw(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> (StatusCode, Vec<u8>, Option<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, bytes, set_cookie)
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> (StatusCode, Value) {
    let (status, bytes, _) = send_raw(app, method, uri, body, cookie).await;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A server on a data dir of its own, with a database on disk -- the export
/// snapshots a file (sqlx's in-memory database answers `VACUUM INTO` with
/// success and no file), and a file in WAL mode is what production has.
async fn setup() -> (axum::Router, overmind_server::AppState, std::path::PathBuf) {
    let data_dir =
        std::env::temp_dir().join(format!("overmind-backup-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        data_dir.join("overmind.sqlite").display()
    );
    let state = overmind_server::init_with(
        &db_url,
        overmind_server::Config {
            data_dir: data_dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    (overmind_server::app(state.clone()), state, data_dir)
}

/// Claim the owner and hand back the session cookie's `k=v` pair.
async fn claim(app: &axum::Router) -> String {
    let (s, _, cookie) = send_raw(
        app,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": "elia", "password": "a long enough password" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "claim should succeed");
    cookie
        .expect("claim sets the session cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

async fn found(app: &axum::Router, cookie: &str, name: &str) -> String {
    let (s, v) = send(
        app,
        "POST",
        "/api/companies",
        Some(json!({ "name": name })),
        Some(cookie),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().expect("company id").to_string()
}

/// Every entry of the archive, in order, as (path, bytes).
fn entries(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let gz = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(gz);
    let mut out = Vec::new();
    for entry in tar.entries().expect("tar entries") {
        let mut entry = entry.expect("entry");
        let path = entry.path().expect("path").to_string_lossy().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read entry");
        out.push((path, bytes));
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

async fn export(app: &axum::Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    send(app, "POST", "/api/backup", Some(body), Some(cookie)).await
}

async fn download(app: &axum::Router, cookie: &str, name: &str) -> (StatusCode, Vec<u8>) {
    let (s, bytes, _) = send_raw(
        app,
        "GET",
        &format!("/api/backup/{name}"),
        None,
        Some(cookie),
    )
    .await;
    (s, bytes)
}

// ---------------------------------------------------------------------------
// Who may export
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unclaimed_instance_has_nobody_to_export_for() {
    let (app, _state, _dir) = setup().await;
    // `require_owner` waves an unclaimed instance through everywhere else;
    // here that would let anyone on the port fill the backup folder.
    let (s, v) = send(&app, "POST", "/api/backup", Some(json!({})), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (s, _) = send(&app, "GET", "/api/backups", None, None).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn once_claimed_the_export_is_the_owners_alone() {
    let (app, _state, _dir) = setup().await;
    let _cookie = claim(&app).await;
    let (s, _) = send(&app, "POST", "/api/backup", Some(json!({})), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = send(&app, "GET", "/api/backups", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// What the archive is
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_export_is_a_consistent_snapshot_that_carries_its_own_chain_report() {
    let (app, state, dir) = setup().await;
    let cookie = claim(&app).await;
    let _company = found(&app, &cookie, "Casa San Vito").await;
    std::fs::create_dir_all(dir.join("artifacts").join("s1")).expect("artifacts dir");
    std::fs::write(
        dir.join("artifacts").join("s1").join("ARTIFACT.md"),
        b"# the plan\n",
    )
    .expect("artifact");
    std::fs::write(dir.join("pay-with-plan"), b"plan\n").expect("marker");

    let (s, report) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let name = report["name"].as_str().expect("archive name").to_string();
    assert!(
        name.starts_with("overmind-instance-") && name.ends_with(".tar.gz"),
        "{name}"
    );

    // The archive is in the folder, and the download is the same bytes.
    let on_disk = std::fs::read(dir.join("backups").join(&name)).expect("archive in the folder");
    let (s, downloaded) = download(&app, &cookie, &name).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(downloaded, on_disk);

    let entries = entries(&on_disk);
    assert_eq!(entries[0].0, "MANIFEST.json", "the manifest comes first");
    let manifest: Value = serde_json::from_slice(&entries[0].1).expect("manifest json");
    assert_eq!(manifest["format"], 1);
    assert_eq!(manifest["scope"], "instance");
    assert_eq!(manifest["overmind_version"], env!("CARGO_PKG_VERSION"));

    // Every entry after the manifest is named there with its hash -- and
    // nothing is named there that is not in the archive.
    let hashes = manifest["entries"].as_object().expect("entries map");
    assert_eq!(hashes.len(), entries.len() - 1, "{hashes:?}");
    for (path, bytes) in &entries[1..] {
        let claimed = hashes[path].as_str().expect("hash");
        assert_eq!(claimed, sha256_hex(bytes), "hash of {path}");
    }
    assert!(hashes.contains_key("artifacts/s1/ARTIFACT.md"));
    assert!(hashes.contains_key("pay-with-plan"));
    assert!(hashes.contains_key("overmind.sqlite"));
    assert!(
        !hashes
            .keys()
            .any(|k| k.starts_with("sessions/") || k.starts_with("chat/")),
        "scratch stays out: {hashes:?}"
    );

    // The snapshot opens cold and holds the company; its chain verifies and
    // is exactly what the manifest says it is.
    let snap_path = dir.join("snapshot-under-test.sqlite");
    let (_, db_bytes) = entries
        .iter()
        .find(|(p, _)| p == "overmind.sqlite")
        .expect("db entry");
    std::fs::write(&snap_path, db_bytes).expect("write snapshot");
    let snap = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=ro", snap_path.display()))
        .await
        .expect("open snapshot");
    let names: Vec<(String,)> = sqlx::query_as("SELECT name FROM companies")
        .fetch_all(&snap)
        .await
        .expect("companies");
    assert_eq!(names, vec![("Casa San Vito".to_string(),)]);
    let chain = overmind_server::audit::verify(&snap).await.expect("verify");
    assert!(chain.valid);
    assert_eq!(manifest["chain"]["valid"], true);
    assert_eq!(manifest["chain"]["events_checked"], chain.events_checked);
    let (last_seq, last_hash): (i64, String) =
        sqlx::query_as("SELECT seq, hash FROM audit_events ORDER BY seq DESC LIMIT 1")
            .fetch_one(&snap)
            .await
            .expect("last event");
    assert_eq!(manifest["chain"]["last_seq"], last_seq);
    assert_eq!(manifest["chain"]["last_hash"], last_hash);
    snap.close().await;

    // The export itself is on the live chain, with the actor -- after the
    // snapshot, so the snapshot's chain does not contain it.
    let (kind, payload): (String, String) =
        sqlx::query_as("SELECT kind, payload FROM audit_events ORDER BY seq DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .expect("live last event");
    assert_eq!(kind, "backup.exported");
    let payload: Value = serde_json::from_str(&payload).expect("payload json");
    // The actor rides as the user's id; the name is read off the chain by
    // whoever renders it (M25), never stored twice.
    let (owner_id,): (String,) = sqlx::query_as("SELECT id FROM users WHERE name = 'elia'")
        .fetch_one(&state.pool)
        .await
        .expect("owner row");
    assert_eq!(payload["actor"], owner_id);
    assert_eq!(payload["name"], name);
    assert_eq!(payload["chain"]["events_checked"], chain.events_checked);

    // The staging tree did not outlive the export -- and it is staged inside
    // the backup folder, which is where a leak would be found. (Written the
    // way it is because the first version of this assertion looked for
    // `export-` under the data dir, where staging has not lived since the
    // security review, and could never have failed.)
    let leftovers: Vec<String> = std::fs::read_dir(dir.join("backups"))
        .expect("backup folder")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(".staging-"))
        .collect();
    assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");
}

#[tokio::test]
async fn the_folder_lists_what_it_holds_and_names_only_bare_entries() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    let (s, first) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::OK);
    let name = first["name"].as_str().expect("name").to_string();

    let (s, list) = send(&app, "GET", "/api/backups", None, Some(&cookie)).await;
    assert_eq!(s, StatusCode::OK);
    let archives = list["archives"].as_array().expect("archives");
    let names: Vec<&str> = archives
        .iter()
        .map(|a| a["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec![name.as_str()]);
    assert!(archives[0]["bytes"].as_u64().expect("size") > 0);
    // The interface asks a passphrase only when there is a sign-in to seal:
    // a field nobody needs is a field nobody should be shown (UX.md).
    assert_eq!(list["sign_in_travels"], false);

    // A name that is not a bare entry of the folder is nothing.
    std::fs::write(dir.join("not-an-archive.sqlite"), b"x").expect("bait");
    for bad in [
        "..%2Fnot-an-archive.sqlite",
        "..%2F..%2Fetc%2Fpasswd",
        "nope.tar.gz",
        "%2Fetc%2Fpasswd",
    ] {
        let (s, _) = download(&app, &cookie, bad).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{bad}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn the_backup_folder_is_the_servers_alone() {
    use std::os::unix::fs::PermissionsExt;
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    let (s, report) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::OK);
    let folder = dir.join("backups");
    let archive = folder.join(report["name"].as_str().expect("name"));
    let mode =
        |p: &std::path::Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode(&folder), 0o700, "the folder");
    assert_eq!(mode(&archive), 0o600, "the archive");
}

// ---------------------------------------------------------------------------
// What never leaves
// ---------------------------------------------------------------------------

const TOKEN: &str = "sk-ant-oat01-THIS-IS-THE-SUBSCRIPTION-TOKEN-0123456789abcdefghijklmnopqrstuv";

#[tokio::test]
async fn a_token_on_the_box_needs_a_passphrase_to_travel() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    std::fs::write(dir.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("token file");
    let (s, v) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"].as_str().unwrap_or("").contains("passphrase"),
        "says what is missing: {v}"
    );
    // And a blank one is no passphrase.
    let (s, _) = export(&app, &cookie, json!({ "passphrase": "   " })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn no_credential_is_readable_in_the_archive_bytes() {
    let (app, state, dir) = setup().await;
    let cookie = claim(&app).await;
    let company = found(&app, &cookie, "Acme").await;
    std::fs::write(dir.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("token file");

    // An editor's integration token, and a run in flight holding its bearer.
    let integration = "integration-token-that-must-not-travel-9f8e7d6c";
    sqlx::query(
        "INSERT INTO company_tokens (id, company_id, label, token, created_at)
         VALUES ('ct1', ?, 'my editor', ?, '2026-08-29T00:00:00Z')",
    )
    .bind(&company)
    .bind(integration)
    .execute(&state.pool)
    .await
    .expect("integration token");
    let bearer = "bearer-of-a-run-in-flight-0123456789";
    let (archetype,): (String,) = sqlx::query_as("SELECT id FROM archetypes LIMIT 1")
        .fetch_one(&state.pool)
        .await
        .expect("a seeded archetype");
    sqlx::query(
        "INSERT INTO agents (id, company_id, archetype_id, name, traits, status, created_at)
         VALUES ('a1', ?, ?, 'Tobia', '{}', 'active', '2026-08-29T00:00:00Z')",
    )
    .bind(&company)
    .bind(&archetype)
    .execute(&state.pool)
    .await
    .expect("agent");
    sqlx::query(
        "INSERT INTO tasks (id, company_id, title, description, status, priority, created_at, updated_at)
         VALUES ('t1', ?, 'a task', '', 'in_progress', 'medium', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z')",
    )
    .bind(&company)
    .execute(&state.pool)
    .await
    .expect("task");
    sqlx::query(
        "INSERT INTO agent_task_sessions
            (id, task_id, agent_id, status, branch, workspace_path, created_at, started_at, mcp_token)
         VALUES ('s1', 't1', 'a1', 'running', 'b', '/tmp/w', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z', ?)",
    )
    .bind(bearer)
    .execute(&state.pool)
    .await
    .expect("session with bearer");

    let (s, report) = export(
        &app,
        &cookie,
        json!({ "passphrase": "correct horse battery" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{report}");
    assert_eq!(report["sealed_token"], true);
    let archive = std::fs::read(
        dir.join("backups")
            .join(report["name"].as_str().expect("name")),
    )
    .expect("archive");

    // The bytes, compressed and not: no credential, in any form.
    let mut all = archive.clone();
    let entries = entries(&archive);
    for (_, bytes) in &entries {
        all.extend_from_slice(bytes);
    }
    let haystack = String::from_utf8_lossy(&all);
    assert!(
        !haystack.contains("sk-ant-"),
        "the subscription token leaked"
    );
    assert!(
        !haystack.contains(integration),
        "the integration token leaked"
    );
    assert!(!haystack.contains(bearer), "the run's bearer leaked");

    // In the snapshot the bearer is gone and the integration token is
    // revoked -- a restored instance does not honour tokens that lived in a
    // file.
    let snap_path = dir.join("snapshot-under-test.sqlite");
    let (_, db_bytes) = entries
        .iter()
        .find(|(p, _)| p == "overmind.sqlite")
        .expect("db entry");
    std::fs::write(&snap_path, db_bytes).expect("write snapshot");
    let snap = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=ro", snap_path.display()))
        .await
        .expect("open snapshot");
    let (bearer_in_snapshot,): (Option<String>,) =
        sqlx::query_as("SELECT mcp_token FROM agent_task_sessions WHERE id = 's1'")
            .fetch_one(&snap)
            .await
            .expect("session row");
    assert_eq!(bearer_in_snapshot, None);
    let (token_in_snapshot, revoked_at): (String, Option<String>) =
        sqlx::query_as("SELECT token, revoked_at FROM company_tokens WHERE id = 'ct1'")
            .fetch_one(&snap)
            .await
            .expect("token row");
    assert_ne!(token_in_snapshot, integration);
    assert!(revoked_at.is_some(), "revoked in the snapshot");
    snap.close().await;

    // The live database is untouched by the scrub.
    let (live_bearer,): (Option<String>,) =
        sqlx::query_as("SELECT mcp_token FROM agent_task_sessions WHERE id = 's1'")
            .fetch_one(&state.pool)
            .await
            .expect("live session row");
    assert_eq!(live_bearer.as_deref(), Some(bearer));

    // The sealed token opens with the passphrase, and with nothing else.
    let manifest: Value = serde_json::from_slice(&entries[0].1).expect("manifest");
    let (_, sealed) = entries
        .iter()
        .find(|(p, _)| p == "secrets/claude-oauth-token.enc")
        .expect("sealed token entry");
    let opened = overmind_server::backup::unseal(&manifest, sealed, "correct horse battery")
        .expect("the passphrase opens it");
    assert_eq!(opened, TOKEN);
    assert!(overmind_server::backup::unseal(&manifest, sealed, "wrong").is_err());
    let mut tampered = sealed.clone();
    tampered[0] ^= 0x01;
    assert!(
        overmind_server::backup::unseal(&manifest, &tampered, "correct horse battery").is_err(),
        "a flipped bit is refused, not decrypted"
    );
}

#[tokio::test]
async fn without_a_token_no_passphrase_is_asked_and_nothing_is_sealed() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    let (s, report) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(report["sealed_token"], false);
    let archive = std::fs::read(
        dir.join("backups")
            .join(report["name"].as_str().expect("name")),
    )
    .expect("archive");
    let entries = entries(&archive);
    assert!(!entries.iter().any(|(p, _)| p.starts_with("secrets/")));
    let manifest: Value = serde_json::from_slice(&entries[0].1).expect("manifest");
    assert!(manifest["token"].is_null());
}

// ---------------------------------------------------------------------------
// What an agent can put in the way
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn the_room_an_agent_works_in_stays_out_of_the_archive() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    // A meeting room is handed to the agent uid for every turn and outlives
    // the meeting; what the meeting decided is in the database.
    std::fs::create_dir_all(dir.join("meetings").join("m1")).expect("room");
    std::fs::write(dir.join("meetings").join("m1").join("notes.md"), b"scratch")
        .expect("room file");

    let (s, report) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let archive = std::fs::read(
        dir.join("backups")
            .join(report["name"].as_str().expect("name")),
    )
    .expect("archive");
    let entries = entries(&archive);
    assert!(
        !entries.iter().any(|(p, _)| p.starts_with("meetings/")),
        "the agent's room rode along: {:?}",
        entries.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_planted_in_a_copied_shelf_is_not_followed() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    // The credential the whole seal exists to keep out of an archive.
    std::fs::write(dir.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("token file");
    let shelf = dir.join("artifacts").join("s1");
    std::fs::create_dir_all(&shelf).expect("shelf");
    std::fs::write(shelf.join("ARTIFACT.md"), b"# a real deliverable\n").expect("artifact");
    std::os::unix::fs::symlink(dir.join("claude-oauth-token"), shelf.join("notes.md"))
        .expect("symlink");
    // The who-pays marker was the one entry still copied by name.
    std::os::unix::fs::symlink(dir.join("claude-oauth-token"), dir.join("pay-with-plan"))
        .expect("marker symlink");

    let (s, report) = export(
        &app,
        &cookie,
        json!({ "passphrase": "correct horse battery" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let archive = std::fs::read(
        dir.join("backups")
            .join(report["name"].as_str().expect("name")),
    )
    .expect("archive");
    let entries = entries(&archive);
    assert!(
        !entries.iter().any(|(p, _)| p == "artifacts/s1/notes.md"),
        "the link was copied"
    );
    assert!(
        !entries.iter().any(|(p, _)| p == "pay-with-plan"),
        "the marker was read through a link"
    );
    assert!(
        entries.iter().any(|(p, _)| p == "artifacts/s1/ARTIFACT.md"),
        "the real file beside it was not"
    );
    let mut all = archive.clone();
    for (_, bytes) in &entries {
        all.extend_from_slice(bytes);
    }
    assert!(
        !String::from_utf8_lossy(&all).contains("sk-ant-"),
        "the token travelled through a symlink"
    );
}

#[tokio::test]
async fn a_passphrase_that_leaves_the_machine_has_a_floor() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    std::fs::write(dir.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("token file");
    // The door refuses under eight characters for a password that never
    // leaves the box; this one is carried away on a disk, where nothing
    // rate-limits an attempt on it.
    let (s, v) = export(&app, &cookie, json!({ "passphrase": "short" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"].as_str().unwrap_or("").contains("characters"),
        "says what is wrong with it: {v}"
    );
    let written = std::fs::read_dir(dir.join("backups"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "an archive was written anyway");
    let (s, _) = export(&app, &cookie, json!({ "passphrase": "twelve chars" })).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn a_backup_folder_that_holds_the_data_dir_is_refused() {
    // `OVERMIND_BACKUP_DIR=/data` is a reasonable thing to type: the folder
    // would be forced 0700, and the data dir is 0755 precisely so a caged
    // agent can walk through it to its own run.
    let data_dir =
        std::env::temp_dir().join(format!("overmind-backup-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        data_dir.join("overmind.sqlite").display()
    );
    let state = overmind_server::init_with(
        &db_url,
        overmind_server::Config {
            data_dir: data_dir.clone(),
            backup_dir: Some(data_dir.clone()),
            agent_cmd: Some("/usr/bin/true".into()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    let app = overmind_server::app(state);
    let cookie = claim(&app).await;

    let (s, v) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or("")
            .contains("OVERMIND_BACKUP_DIR"),
        "names the setting to change: {v}"
    );
}

#[tokio::test]
async fn the_listing_says_whether_a_sign_in_would_travel() {
    let (app, _state, dir) = setup().await;
    let cookie = claim(&app).await;
    let (s, before) = send(&app, "GET", "/api/backups", None, Some(&cookie)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(before["sign_in_travels"], false);

    std::fs::write(dir.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("token file");
    let (_, after) = send(&app, "GET", "/api/backups", None, Some(&cookie)).await;
    assert_eq!(after["sign_in_travels"], true);
}

#[tokio::test]
async fn an_archive_is_deleted_by_its_owner_and_the_chain_says_so() {
    let (app, state, dir) = setup().await;
    let cookie = claim(&app).await;
    let (s, report) = export(&app, &cookie, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let name = report["name"].as_str().expect("name").to_string();

    // Not a stranger's verb, and not a member's.
    let (s, _) = send(&app, "DELETE", &format!("/api/backup/{name}"), None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(dir.join("backups").join(&name).is_file());

    // Nor a name that is not an archive of ours.
    std::fs::write(dir.join("backups").join("notes.txt"), b"mine").expect("bait");
    let (s, _) = send(&app, "DELETE", "/api/backup/notes.txt", None, Some(&cookie)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(dir.join("backups").join("notes.txt").is_file());

    let (s, v) = send(
        &app,
        "DELETE",
        &format!("/api/backup/{name}"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!dir.join("backups").join(&name).exists());
    let (_, list) = send(&app, "GET", "/api/backups", None, Some(&cookie)).await;
    assert!(list["archives"].as_array().expect("archives").is_empty());

    let (kind, payload): (String, String) =
        sqlx::query_as("SELECT kind, payload FROM audit_events ORDER BY seq DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .expect("last event");
    assert_eq!(kind, "backup.deleted");
    let payload: Value = serde_json::from_str(&payload).expect("payload");
    assert_eq!(payload["name"], name);
    assert!(payload["actor"].is_string(), "deleting names who did it");
}
