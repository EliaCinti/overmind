//! The archive is the instance (M31, ADR-0044) -- slice B: the restore.
//!
//! Every test here takes a real archive out of one instance and hands it to
//! another, the way a person would after losing a disk. What is checked is
//! what the ADR promises: an empty instance only, every hash and the audit
//! chain against the manifest before anything moves, the swap at boot rather
//! than under a live pool, and a wrong passphrase refused while a retry is
//! still free.

use std::io::{Read, Write};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKEN: &str = "sk-ant-oat01-THIS-IS-THE-SUBSCRIPTION-TOKEN-0123456789abcdefghijklmnopqrstuv";
const PASSPHRASE: &str = "correct horse battery staple";

struct Instance {
    app: axum::Router,
    state: overmind_server::AppState,
    dir: std::path::PathBuf,
    db_url: String,
}

async fn instance() -> Instance {
    let dir = std::env::temp_dir().join(format!(
        "overmind-restore-{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&dir).expect("data dir");
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        dir.join("overmind.sqlite").display()
    );
    let state = overmind_server::init_with(
        &db_url,
        overmind_server::Config {
            data_dir: dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    Instance {
        app: overmind_server::app(state.clone()),
        state,
        dir,
        db_url,
    }
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> (StatusCode, Value) {
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
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn claim(app: &axum::Router) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/claim")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "name": "elia", "password": "a long enough password" }).to_string(),
        ))
        .expect("build");
    let response = app.clone().oneshot(request).await.expect("responds");
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .to_string()
}

/// An instance with something in it, and the archive of it.
async fn an_instance_worth_restoring() -> (Instance, Vec<u8>) {
    let source = instance().await;
    let cookie = claim(&source.app).await;
    let (s, company) = send(
        &source.app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Casa San Vito" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{company}");
    std::fs::create_dir_all(source.dir.join("artifacts").join("s1")).expect("shelf");
    std::fs::write(
        source.dir.join("artifacts").join("s1").join("ARTIFACT.md"),
        b"# the dollhouse, v7\n",
    )
    .expect("artifact");
    std::fs::write(source.dir.join("claude-oauth-token"), format!("{TOKEN}\n")).expect("token");
    std::fs::write(source.dir.join("pay-with-plan"), b"plan\n").expect("marker");

    let (s, report) = send(
        &source.app,
        "POST",
        "/api/backup",
        Some(json!({ "passphrase": PASSPHRASE })),
        Some(&cookie),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let archive = std::fs::read(
        source
            .dir
            .join("backups")
            .join(report["name"].as_str().expect("name")),
    )
    .expect("archive");
    (source, archive)
}

/// `POST /api/restore`, multipart, the way the browser sends it.
async fn restore(
    app: &axum::Router,
    archive: &[u8],
    fields: &[(&str, &str)],
) -> (StatusCode, Value) {
    restore_as(app, archive, fields, None).await
}

async fn restore_as(
    app: &axum::Router,
    archive: &[u8],
    fields: &[(&str, &str)],
    cookie: Option<&str>,
) -> (StatusCode, Value) {
    let boundary = "----overmindrestoretest";
    let mut body: Vec<u8> = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; \
             filename=\"overmind.tar.gz\"\r\nContent-Type: application/gzip\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(archive);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/restore")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body)).expect("build request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn entries(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let gz = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(gz);
    let mut out = Vec::new();
    for entry in tar.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        let path = entry.path().expect("path").to_string_lossy().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read");
        out.push((path, bytes));
    }
    out
}

/// Rebuild an archive from entries, in order -- for the tests that need a
/// changed manifest or an entry nobody named.
fn rebuild(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut tar = tar::Builder::new(&mut gz);
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(&mut header, path, bytes.as_slice())
                .expect("append");
        }
        tar.finish().expect("finish");
    }
    gz.finish().expect("gz")
}

/// The boot half: what `main` does before the pool is opened.
async fn swap(inst: &Instance) -> Option<Value> {
    overmind_server::backup::swap_pending(&inst.state.config, &inst.db_url)
        .await
        .expect("swap")
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_archive_restores_into_an_empty_instance_and_the_chain_still_verifies() {
    let (source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;

    let (s, v) = restore(&target.app, &archive, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["restarting"], true);
    assert_eq!(v["token"], "restored");
    // Nothing has moved yet: the swap is the boot's job, not the request's.
    assert!(target.dir.join("restore-pending").is_file());
    let (still_empty,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM companies")
        .fetch_one(&target.state.pool)
        .await
        .expect("companies");
    assert_eq!(still_empty, 0, "the live database was touched");
    target.state.pool.close().await;

    let record = swap(&target).await.expect("a restore was pending");
    assert_eq!(record["scope"], "instance");
    assert!(!target.dir.join("restore-pending").exists());

    // The instance that comes back up is the one that was exported.
    let restored = overmind_server::init_with(
        &target.db_url,
        overmind_server::Config {
            data_dir: target.dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init on restored data");
    let (company,): (String,) = sqlx::query_as("SELECT name FROM companies")
        .fetch_one(&restored.pool)
        .await
        .expect("company");
    assert_eq!(company, "Casa San Vito");
    let (owner,): (String,) = sqlx::query_as("SELECT name FROM users")
        .fetch_one(&restored.pool)
        .await
        .expect("owner");
    assert_eq!(owner, "elia");
    let chain = overmind_server::audit::verify(&restored.pool)
        .await
        .expect("verify");
    assert!(chain.valid, "the restored chain does not verify");

    assert_eq!(
        std::fs::read_to_string(target.dir.join("artifacts").join("s1").join("ARTIFACT.md"))
            .expect("artifact"),
        "# the dollhouse, v7\n"
    );
    assert_eq!(
        std::fs::read_to_string(target.dir.join("claude-oauth-token"))
            .expect("token")
            .trim(),
        TOKEN
    );
    assert!(target.dir.join("pay-with-plan").is_file());

    // The staging tree does not outlive the swap.
    let leftovers = std::fs::read_dir(&target.dir)
        .expect("data dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("restore-"))
        .count();
    assert_eq!(leftovers, 0, "staging left behind");

    // And the event says so, on the restored chain.
    overmind_server::backup::note_restored(&restored, &record)
        .await
        .expect("note the restore");
    let (kind, payload): (String, String) =
        sqlx::query_as("SELECT kind, payload FROM audit_events ORDER BY seq DESC LIMIT 1")
            .fetch_one(&restored.pool)
            .await
            .expect("last event");
    assert_eq!(kind, "backup.restored");
    let payload: Value = serde_json::from_str(&payload).expect("payload");
    // A system event: the archive's owner did not act, and is not named as if
    // they had.
    assert!(payload.get("actor").is_none(), "{payload}");
    assert_eq!(payload["scope"], "instance");
    let _ = source;
}

#[tokio::test]
async fn a_restore_lands_only_on_an_empty_instance() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;
    let cookie = claim(&target.app).await;

    // A stranger gets the door's answer, not the instance's state.
    let (s, _) = restore(&target.app, &archive, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // The owner gets the reason, and the way out.
    let (s, v) = restore_as(
        &target.app,
        &archive,
        &[("passphrase", PASSPHRASE)],
        Some(&cookie),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let said = v["error"].as_str().unwrap_or_default();
    assert!(said.contains("owner"), "says what makes it full: {v}");
    assert!(said.contains("empty"), "says the way out: {v}");
    assert!(
        !target.dir.join("restore-pending").exists(),
        "a refused restore staged anyway"
    );
}

#[tokio::test]
async fn a_tampered_archive_is_refused_by_name_and_nothing_is_staged() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let mut entries = entries(&archive);
    let artifact = entries
        .iter_mut()
        .find(|(p, _)| p == "artifacts/s1/ARTIFACT.md")
        .expect("the artifact entry");
    artifact.1 = b"# a different plan entirely\n".to_vec();
    let tampered = rebuild(&entries);

    let target = instance().await;
    let (s, v) = restore(&target.app, &tampered, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let said = v["error"].as_str().unwrap_or_default();
    assert!(
        said.contains("artifacts/s1/ARTIFACT.md"),
        "names the entry that does not match: {v}"
    );
    assert!(!target.dir.join("restore-pending").exists());
    let staged = std::fs::read_dir(&target.dir)
        .expect("data dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("restore-"))
        .count();
    assert_eq!(staged, 0, "staging survived a refusal");
}

#[tokio::test]
async fn an_entry_the_manifest_does_not_name_is_refused() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let mut entries = entries(&archive);
    entries.push((
        "artifacts/s1/smuggled.md".to_string(),
        b"not in the manifest".to_vec(),
    ));
    let smuggled = rebuild(&entries);

    let target = instance().await;
    let (s, v) = restore(&target.app, &smuggled, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("smuggled.md"),
        "names the entry nobody vouched for: {v}"
    );
}

#[tokio::test]
async fn a_wrong_passphrase_refuses_the_whole_restore_while_a_retry_is_free() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;

    let (s, v) = restore(
        &target.app,
        &archive,
        &[("passphrase", "not the one at all")],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("passphrase"),
        "{v}"
    );
    assert!(
        !target.dir.join("restore-pending").exists(),
        "half a restore was staged: the retry below would answer 409 instead"
    );

    // Still empty, so the same archive goes in on the second try.
    let (s, v) = restore(&target.app, &archive, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn a_restore_without_the_sign_in_leaves_nothing_claiming_the_plan_pays() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;

    let (s, v) = restore(&target.app, &archive, &[("skip_token", "true")]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["token"], "skipped");
    target.state.pool.close().await;
    swap(&target).await.expect("pending");

    assert!(
        !target.dir.join("claude-oauth-token").exists(),
        "the token came back without a passphrase"
    );
    // The marker without the token would leave every agent command with
    // neither credential, while the health card said the plan pays.
    assert!(
        !target.dir.join("pay-with-plan").exists(),
        "the who-pays marker outlived the sign-in it points at"
    );
}

#[tokio::test]
async fn an_archive_that_needs_a_passphrase_says_so_instead_of_guessing() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;
    let (s, v) = restore(&target.app, &archive, &[]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let said = v["error"].as_str().unwrap_or_default();
    assert!(said.contains("passphrase"), "{v}");
    assert!(!target.dir.join("restore-pending").exists());
}

#[tokio::test]
async fn an_archive_from_a_newer_overmind_is_refused_before_the_migrations() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let mut entries = entries(&archive);
    let manifest = entries
        .iter_mut()
        .find(|(p, _)| p == "MANIFEST.json")
        .expect("manifest entry");
    let mut parsed: Value = serde_json::from_slice(&manifest.1).expect("manifest json");
    parsed["overmind_version"] = json!("99.0.0");
    manifest.1 = serde_json::to_vec_pretty(&parsed).expect("manifest bytes");
    let from_the_future = rebuild(&entries);

    let target = instance().await;
    let (s, v) = restore(&target.app, &from_the_future, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let said = v["error"].as_str().unwrap_or_default();
    assert!(
        said.contains("99.0.0"),
        "names the version it cannot read: {v}"
    );
}

#[tokio::test]
async fn a_broken_chain_is_refused_even_when_every_hash_matches() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let mut entries = entries(&archive);

    // Rewrite the database inside the archive: change one audit payload, so
    // the chain no longer recomputes, then re-hash the entry in the manifest
    // so the archive is internally consistent about everything except the
    // one thing the chain is for.
    let tmp = std::env::temp_dir().join(format!("chain-{}.sqlite", uuid::Uuid::now_v7().simple()));
    let db_index = entries
        .iter()
        .position(|(p, _)| p == "overmind.sqlite")
        .expect("db entry");
    std::fs::write(&tmp, &entries[db_index].1).expect("write db");
    {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", tmp.display()))
            .await
            .expect("open");
        // The table is append-only by trigger -- which is the point, and
        // which anybody holding the file can simply drop. That is what the
        // chain is for, and what this test proves it catches.
        let triggers: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'audit_events'",
        )
        .fetch_all(&pool)
        .await
        .expect("triggers");
        for (name,) in triggers {
            sqlx::query(&format!("DROP TRIGGER {name}"))
                .execute(&pool)
                .await
                .expect("drop trigger");
        }
        sqlx::query(
            "UPDATE audit_events SET payload = '{\"actor\":\"somebody else\"}' WHERE seq = 1",
        )
        .execute(&pool)
        .await
        .expect("tamper");
        pool.close().await;
    }
    entries[db_index].1 = std::fs::read(&tmp).expect("read back");
    let db_hash = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(&entries[db_index].1))
    };
    let manifest_index = entries
        .iter()
        .position(|(p, _)| p == "MANIFEST.json")
        .expect("manifest");
    let mut parsed: Value =
        serde_json::from_slice(&entries[manifest_index].1).expect("manifest json");
    parsed["entries"]["overmind.sqlite"] = json!(db_hash);
    entries[manifest_index].1 = serde_json::to_vec_pretty(&parsed).expect("manifest bytes");
    let rewritten = rebuild(&entries);

    let target = instance().await;
    let (s, v) = restore(&target.app, &rewritten, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let said = v["error"].as_str().unwrap_or_default();
    assert!(
        said.contains("chain"),
        "the chain is what refuses it, and says so: {v}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn a_boot_with_nothing_pending_is_a_boot_like_any_other() {
    let dir = std::env::temp_dir().join(format!("overmind-nopending-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("dir");
    let config = overmind_server::Config {
        data_dir: dir.clone(),
        ..overmind_server::Config::default()
    };
    let done = overmind_server::backup::swap_pending(
        &config,
        &format!("sqlite://{}", dir.join("overmind.sqlite").display()),
    )
    .await
    .expect("swap");
    assert!(done.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_pending_marker_pointing_nowhere_is_cleared_rather_than_obeyed() {
    let dir = std::env::temp_dir().join(format!("overmind-badpending-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut marker = std::fs::File::create(dir.join("restore-pending")).expect("marker");
    marker
        .write_all(
            json!({ "staging": dir.join("restore-gone").to_string_lossy(), "scope": "instance" })
                .to_string()
                .as_bytes(),
        )
        .expect("write");
    drop(marker);
    let config = overmind_server::Config {
        data_dir: dir.clone(),
        ..overmind_server::Config::default()
    };
    // A boot that cannot find what it was told to swap must come up on the
    // data it has, saying so -- not refuse to start for ever.
    let done = overmind_server::backup::swap_pending(
        &config,
        &format!("sqlite://{}", dir.join("overmind.sqlite").display()),
    )
    .await
    .expect("swap");
    assert!(done.is_none());
    assert!(!dir.join("restore-pending").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// The race a review found: a legitimate claim landing after a restore only
// ever checked emptiness at one point in time.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_claim_that_lands_after_a_restore_was_staged_wins_at_the_next_boot() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;

    // The attacker's request: passes the emptiness check, stages an archive,
    // asks to be restarted -- exactly what the API does.
    let (s, v) = restore(&target.app, &archive, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(target.dir.join("restore-pending").is_file());

    // Before any restart happens, the real operator claims the instance --
    // the race the review's exploit walks through step by step.
    let cookie = claim(&target.app).await;
    let (s, mine) = send(
        &target.app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "The Real Company" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{mine}");
    target.state.pool.close().await;

    // The boot swap must refuse: the live database is no longer empty, and
    // that is the one fact that must survive a slow, merely-checked-earlier
    // restore.
    let swapped = swap(&target).await;
    assert!(
        swapped.is_none(),
        "the staged restore overwrote a real claim: {swapped:?}"
    );
    assert!(
        !target.dir.join("restore-pending").exists(),
        "the stale restore should be discarded, not left to try again"
    );

    let restored = overmind_server::init_with(
        &target.db_url,
        overmind_server::Config {
            data_dir: target.dir.clone(),
            agent_cmd: Some("/usr/bin/true".into()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init on the live data");
    let (owner,): (String,) = sqlx::query_as("SELECT name FROM users")
        .fetch_one(&restored.pool)
        .await
        .expect("owner");
    assert_eq!(owner, "elia", "the real owner survives");
    let (company,): (String,) = sqlx::query_as("SELECT name FROM companies")
        .fetch_one(&restored.pool)
        .await
        .expect("company");
    assert_eq!(company, "The Real Company", "the real company survives");
}

#[tokio::test]
async fn a_claim_clears_a_restore_staged_earlier_the_instant_it_lands() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let target = instance().await;

    let (s, _) = restore(&target.app, &archive, &[("passphrase", PASSPHRASE)]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(target.dir.join("restore-pending").is_file());

    let _cookie = claim(&target.app).await;

    // Not just "the boot refuses it" -- the claim itself clears the note, so
    // an operator inspecting the data directory right after claiming does
    // not find a restore that is merely waiting to be stale.
    assert!(
        !target.dir.join("restore-pending").exists(),
        "claiming should clear a restore staged before it, immediately"
    );
}

// ---------------------------------------------------------------------------
// The manifest is the archive's word for itself, not ours: it must not be
// able to hand the KDF a memory allocation big enough to abort the process.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_manifest_asking_the_kdf_for_a_terabyte_is_refused_before_it_is_spent() {
    let (_source, archive) = an_instance_worth_restoring().await;
    let mut archive_entries = entries(&archive);
    let manifest_index = archive_entries
        .iter()
        .position(|(p, _)| p == "MANIFEST.json")
        .expect("manifest");
    let mut parsed: Value =
        serde_json::from_slice(&archive_entries[manifest_index].1).expect("manifest json");
    // Near u32::MAX KiB is on the order of a terabyte -- exactly the
    // allocation `Argon2::hash_password_into` would otherwise be asked to
    // make, and the one a global allocator does not fail out of gracefully.
    parsed["token"]["m_kib"] = json!(u32::MAX - 1);
    archive_entries[manifest_index].1 = serde_json::to_vec_pretty(&parsed).expect("manifest bytes");
    let greedy = rebuild(&archive_entries);

    // At the API, a bad seal and a wrong passphrase read the same on purpose
    // (ADR-0044: no oracle for which one failed) -- so the request-level
    // check is that it refuses, and refuses fast rather than spending the
    // KDF's time first.
    let target = instance().await;
    let started = std::time::Instant::now();
    let (s, v) = restore(&target.app, &greedy, &[("passphrase", PASSPHRASE)]).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the request took {:?} -- it is spending the KDF's time instead of refusing it",
        started.elapsed()
    );
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(!target.dir.join("restore-pending").exists());

    // Underneath, `unseal` itself names what it refused -- checked directly,
    // since the API layer collapses every unseal failure to one message.
    let (_, sealed) = archive_entries
        .iter()
        .find(|(p, _)| p == "secrets/claude-oauth-token.enc")
        .expect("sealed entry");
    let err = overmind_server::backup::unseal(&parsed, sealed, PASSPHRASE)
        .expect_err("a terabyte request must not be honoured");
    assert!(
        err.to_string().contains("more work"),
        "names what it refused: {err}"
    );
}
