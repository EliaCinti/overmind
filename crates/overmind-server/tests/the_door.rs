//! The adversarial suite a door deserves (M24, ADR-0032). Every test here is
//! an attack that must fail, or a legitimate path that must keep working --
//! never trust a door that was only ever pushed from the inside.

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
) -> (StatusCode, Value, Option<String>) {
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
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value, set_cookie)
}

/// The app, and the directory the server writes its setup code into -- a test
/// that claims legitimately has to read that code the way the person at the
/// machine does. Derefs to the router so every `send_raw(&app, …)` still reads
/// the way it did before the code existed.
struct Door {
    app: axum::Router,
    data_dir: std::path::PathBuf,
}

impl std::ops::Deref for Door {
    type Target = axum::Router;
    fn deref(&self) -> &axum::Router {
        &self.app
    }
}

impl Door {
    /// What the server minted at boot, as the installer would read it.
    fn setup_code(&self) -> String {
        std::fs::read_to_string(self.data_dir.join("setup-code"))
            .expect("the server minted a setup code")
            .trim()
            .to_string()
    }
}

async fn setup() -> Door {
    setup_with_data_dir().await
}

async fn setup_with_data_dir() -> Door {
    let data_dir =
        std::env::temp_dir().join(format!("overmind-door-{}", uuid::Uuid::now_v7().simple()));
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: data_dir.clone(),
            // Never the real CLI: on a machine that has it, a test touching
            // the subscription sign-in would open the owner's browser.
            agent_cmd: Some("/usr/bin/true".into()),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    Door {
        app: overmind_server::app(state),
        data_dir,
    }
}

/// An unclaimed instance is open by construction -- whoever reaches the port
/// first owns it -- and the wiki teaches binding to a tailnet address, because
/// sharing a company is the product. So the claim costs a code the server
/// minted and only somebody at the machine can read (ADR-0045).
#[tokio::test]
async fn a_claim_without_the_setup_code_is_refused() {
    let door = setup_with_data_dir().await;

    let (s, v, cookie) = send_raw(
        &door,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": "a stranger", "password": "a long enough password" })),
        None,
    )
    .await;

    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert!(
        cookie.is_none(),
        "a refused claim must not hand out a session: {cookie:?}"
    );
}

/// Claim the owner and hand back the session cookie's `k=v` pair.
async fn claim(door: &Door, name: &str, pass: &str) -> String {
    let (s, _, cookie) = send_raw(
        door,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": name, "password": pass, "setup": door.setup_code() })),
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

/// Mint an invite as the given session and hand back the raw code.
async fn mint(app: &axum::Router, cookie: &str) -> String {
    let (s, v, _) = send_raw(app, "POST", "/api/auth/invites", None, Some(cookie)).await;
    assert_eq!(s, StatusCode::OK, "mint should succeed");
    v["invite"].as_str().expect("the raw code").to_string()
}

/// Before an owner exists the door answers about itself -- a fresh install has
/// to be able to see its own state and claim itself. **Nothing else does.**
/// Until ADR-0045 an unclaimed instance waved the whole API through, so
/// whoever reached the port first could found a company and start a task,
/// which runs an agent CLI on this machine.
#[tokio::test]
async fn an_unclaimed_instance_says_so_and_answers_nothing_else() {
    let app = setup().await;

    let (s, v, _) = send_raw(&app, "GET", "/api/auth", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], json!("unclaimed"));

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Pre-door" })),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "a passer-by must not found a company on an instance nobody has claimed"
    );
}

/// The code guards the claim, so signup must not be the way around it. While
/// nobody has claimed the instance there is nobody to invite anybody: the
/// first person in is the owner, and the owner pays the code. Until ADR-0045
/// the invite gate was skipped entirely while `users` was empty, so a stranger
/// could sign up, get a session, and walk through the wall.
#[tokio::test]
async fn signup_is_not_a_way_around_the_claim() {
    let app = setup().await;

    let (s, v, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "a stranger", "password": "a long enough password" })),
        None,
    )
    .await;

    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert!(
        cookie.is_none(),
        "a refused signup must not hand out a session: {cookie:?}"
    );
}

/// The four routes `docs/NEXT.md` named as open before an owner exists, each
/// of which does something a stranger should not be able to do: choose who
/// pays, start the subscription sign-in that opens a browser, and mint an
/// invite to an instance that is not theirs.
#[tokio::test]
async fn the_owner_only_routes_are_shut_before_an_owner_exists() {
    let app = setup().await;

    for (method, path, body) in [
        (
            "POST",
            "/api/economy/pay-with",
            Some(json!({ "payer": "plan" })),
        ),
        ("POST", "/api/claude-auth/start", None),
        (
            "POST",
            "/api/claude-auth/code",
            Some(json!({ "code": "whatever" })),
        ),
        ("POST", "/api/auth/invites", None),
    ] {
        let (s, v, _) = send_raw(&app, method, path, body, None).await;
        assert_eq!(
            s,
            StatusCode::UNAUTHORIZED,
            "{method} {path} answered a stranger on an unclaimed instance: {v}"
        );
    }
}

/// Once claimed, the wall stands: no session, no API -- and the health
/// answer is redacted down to liveness.
#[tokio::test]
async fn a_claimed_instance_refuses_the_sessionless() {
    let app = setup().await;
    let _cookie = claim(&app, "cl_own", "correct-horse-battery").await;

    let (s, _, _) = send_raw(&app, "GET", "/api/companies", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (s, v, _) = send_raw(&app, "GET", "/api/health", None, None).await;
    assert_eq!(s, StatusCode::OK, "liveness stays answerable");
    assert_eq!(v["status"], json!("ok"));
    assert!(
        v.get("economy").is_none(),
        "the economy must not leak through an unauthenticated health: {v}"
    );
}

/// The cookie works, and a forged one does not. A forgery is
/// indistinguishable in shape and refused all the same.
#[tokio::test]
async fn a_real_session_enters_and_a_forged_one_does_not() {
    let app = setup().await;
    let cookie = claim(&app, "rs_own", "correct-horse-battery").await;

    let (s, _, _) = send_raw(&app, "GET", "/api/companies", None, Some(&cookie)).await;
    assert_eq!(s, StatusCode::OK);

    let forged = format!("overmind_session={}", "ab".repeat(32));
    let (s, _, _) = send_raw(&app, "GET", "/api/companies", None, Some(&forged)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// Exactly one owner: the second claim loses, atomically, and racing does
/// not help.
#[tokio::test]
async fn the_owner_is_claimed_exactly_once_even_racing() {
    let app = setup().await;
    // Every racer holds the same code: what must be exactly-once is the
    // claim, not the knowledge of the code.
    let code = app.setup_code();
    let mut wins = 0;
    let mut handles = Vec::new();
    for i in 0..6 {
        let app = app.clone();
        let code = code.clone();
        handles.push(tokio::spawn(async move {
            let (s, _, _) = send_raw(
                &app,
                "POST",
                "/api/auth/claim",
                Some(json!({ "name": format!("racer{i}"), "password": "long-enough-pass", "setup": code })),
                None,
            )
            .await;
            s
        }));
    }
    for h in handles {
        if h.await.expect("join") == StatusCode::OK {
            wins += 1;
        }
    }
    assert_eq!(wins, 1, "of six concurrent claims exactly one may win");
}

/// Login: right password in, wrong password out -- with the same wordless
/// refusal for a wrong name, and a rate limit that closes the guessing
/// window.
#[tokio::test]
async fn wrong_credentials_are_refused_and_guessing_is_rate_limited() {
    let app = setup().await;
    let _ = claim(&app, "rl_own", "correct-horse-battery").await;

    let (s, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "rl_own", "password": "correct-horse-battery" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(cookie.is_some());

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "rl_own", "password": "wrong" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "nobody", "password": "wrong" })),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "an unknown name refuses identically to a wrong password"
    );

    // Hammer one name past the window: the answer stops being 401 and
    // becomes the limiter's, before argon2 is even consulted.
    let mut limited = false;
    for _ in 0..8 {
        let (s, _, _) = send_raw(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "name": "hammered", "password": "guess" })),
            None,
        )
        .await;
        if s == StatusCode::BAD_REQUEST {
            limited = true;
            break;
        }
    }
    assert!(limited, "guessing one name must hit the rate limit");
}

/// Logout kills the session server-side: the same cookie stops working.
#[tokio::test]
async fn logout_revokes_the_session_not_just_the_cookie() {
    let app = setup().await;
    let cookie = claim(&app, "lo_own", "correct-horse-battery").await;

    let (s, _, _) = send_raw(&app, "POST", "/api/auth/logout", None, Some(&cookie)).await;
    assert_eq!(s, StatusCode::OK);

    let (s, _, _) = send_raw(&app, "GET", "/api/companies", None, Some(&cookie)).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "a logged-out session must be dead on the server, not only in the browser"
    );
}

/// The cookie's own contract: HttpOnly, SameSite=Strict, and no Secure on
/// plain http -- the browser would drop it and logins would silently not
/// stick.
#[tokio::test]
async fn the_cookie_wears_its_armor() {
    let app = setup().await;
    let (_, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": "ck_own", "password": "correct-horse-battery", "setup": app.setup_code() })),
        None,
    )
    .await;
    let cookie = cookie.expect("set-cookie");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(
        !cookie.contains("Secure"),
        "Secure without TLS silently loses every login: {cookie}"
    );
}

/// The claim refuses weak material outright.
#[tokio::test]
async fn a_short_password_is_refused_at_the_door() {
    let app = setup().await;
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": "elia", "password": "short", "setup": app.setup_code() })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// A cross-site form's shapes are refused: with a body, the content type
/// must be one a forger cannot send with credentials attached.
#[tokio::test]
async fn a_forms_content_type_is_refused_at_the_wall() {
    let app = setup().await;
    let cookie = claim(&app, "ct_own", "correct-horse-battery").await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/companies")
        .header(header::COOKIE, &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .header(header::CONTENT_LENGTH, "9")
        .body(Body::from("name=Evil"))
        .expect("build");
    let response = app.clone().oneshot(request).await.expect("responds");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Every event an authenticated request appends carries the actor -- the
/// field M25's "who approved this" is made of. Injected into the payload,
/// which is hashed as stored: the attribution is tamper-evident too.
#[tokio::test]
async fn audit_events_carry_who_did_it() {
    let app = setup().await;
    let cookie = claim(&app, "ae_own", "correct-horse-battery").await;

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Attribuita" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (_, events, _) = send_raw(&app, "GET", "/api/audit/events", None, Some(&cookie)).await;
    let created = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|e| e["kind"] == "company.created")
        .expect("the founding event exists")
        .clone();
    // The endpoint may hand the payload parsed or as its stored string.
    let payload: Value = match &created["payload"] {
        Value::String(raw) => serde_json::from_str(raw).unwrap_or(Value::Null),
        other => other.clone(),
    };
    assert!(
        payload["actor"].as_str().is_some_and(|a| !a.is_empty()),
        "the event must name its actor: {payload}"
    );

    // And the chain still verifies with the actor inside the hashed payload.
    let (_, report, _) = send_raw(&app, "GET", "/api/audit/verify", None, Some(&cookie)).await;
    assert_eq!(report["valid"], json!(true));
}

/// Sign up: more than one user can hold the same local store, each with
/// their own password -- and a taken name refuses in the same wordless
/// shape as a failed login, because whether a name exists is not for an
/// anonymous caller to enumerate.
#[tokio::test]
async fn signup_adds_users_and_does_not_enumerate_names() {
    let app = setup().await;
    let owner = claim(&app, "sa_own", "correct-horse-battery").await;

    let code = mint(&app, &owner).await;
    let (s, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "sa_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "a second user signs up");
    assert!(cookie.is_some(), "signup logs the new user in");

    // Both can log in with their own credentials.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "sa_mem", "password": "another-long-pass" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // A taken name answers exactly like a bad credential.
    let code2 = mint(&app, &owner).await;
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "sa_own", "password": "whatever-else-long", "invite": code2 })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// Roles are real from birth: the first account owns the instance, everyone
/// after is a member -- and today the difference is exactly one thing,
/// billing. A member touching the subscription gets 403: who they are was
/// never in question, their standing was.
///
/// The first account is the *claim*, holding the setup code. It used to be a
/// signup, which is precisely the bypass ADR-0045 closed: an unclaimed
/// instance handed ownership to whoever signed up, no invite, no code.
#[tokio::test]
async fn the_first_user_owns_and_billing_is_the_owners() {
    let app = setup().await;

    let (s, v, owner_cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": "fo_own", "password": "correct-horse-battery", "setup": app.setup_code() })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["role"], json!("owner"), "the first account owns: {v}");
    let owner_cookie = owner_cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    let code = mint(&app, &owner_cookie).await;
    let (s, v, member_cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "fo_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v["role"],
        json!("member"),
        "everyone after is a member: {v}"
    );
    let member_cookie = member_cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/claude-auth/start",
        None,
        Some(&member_cookie),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "billing is not a member's call");

    // The owner passes the role gate -- and meets the next honest refusal:
    // this suite runs with a custom agent command, and a custom adapter is
    // not the Claude CLI, so there is no subscription to sign into. Measured
    // on the owner's desk (22 Aug): before this, the spawn SUCCEEDED on a
    // machine with the real CLI installed, and every `cargo test` opened
    // the owner's browser on an OAuth page -- a dozen times a day.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/claude-auth/start",
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "a custom adapter has no subscription sign-in to start"
    );
}

// ── M25: invites and membership (ADR-0033) ──────────────────────────────────

/// Once anyone exists, sign-up spends an invite: no code, no entry; a spent
/// or invented code refuses wordlessly; the owner mints them one at a time.
#[tokio::test]
async fn signup_after_the_first_needs_an_invite_spent_once() {
    let app = setup().await;
    let owner = claim(&app, "si_own", "correct-horse-battery").await;

    // Without a code: told a code is required (that much is not a secret).
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "si_mem", "password": "another-long-pass" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The owner mints one.
    let (s, v, _) = send_raw(&app, "POST", "/api/auth/invites", None, Some(&owner)).await;
    assert_eq!(s, StatusCode::OK);
    let code = v["invite"]
        .as_str()
        .expect("the raw code, once")
        .to_string();

    // It lets exactly one person in.
    let (s, v, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "si_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["role"], json!("member"));

    // Spent: the same code lets nobody else in.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "terzo", "password": "yet-another-pass", "invite": code })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // And a member cannot mint invites: entry is the owner's to grant.
    let (s, _, member_cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "si_mem", "password": "another-long-pass" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let member_cookie = member_cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/invites",
        None,
        Some(&member_cookie),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

/// A taken name hands the invite back: the transaction that failed to
/// create the user must not have burnt the code.
#[tokio::test]
async fn a_failed_signup_does_not_burn_the_invite() {
    let app = setup().await;
    let owner = claim(&app, "fb_own", "correct-horse-battery").await;
    let (_, v, _) = send_raw(&app, "POST", "/api/auth/invites", None, Some(&owner)).await;
    let code = v["invite"].as_str().expect("code").to_string();

    // Try to take the owner's name: refused, code untouched.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "fb_own", "password": "whatever-long-pass", "invite": code })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // The same code still works for a fresh name.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "fb_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "the invite survived the failed attempt");
}

/// Membership is the filter: a member sees their companies and not the
/// others; the company-scoped surface refuses non-members; adding a member
/// opens it. The owner passes everywhere -- the administrator of the box.
#[tokio::test]
async fn members_see_their_companies_and_only_theirs() {
    let app = setup().await;
    let owner = claim(&app, "ms_own", "correct-horse-battery").await;

    // The owner founds two companies.
    for name in ["Alfa", "Beta"] {
        let (s, _, _) = send_raw(
            &app,
            "POST",
            "/api/companies",
            Some(json!({ "name": name })),
            Some(&owner),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED);
    }

    // A member joins the instance.
    let (_, v, _) = send_raw(&app, "POST", "/api/auth/invites", None, Some(&owner)).await;
    let code = v["invite"].as_str().expect("code").to_string();
    let (_, _, member_cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "ms_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    let member = member_cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    // The member sees nothing yet; the owner sees both.
    let (_, v, _) = send_raw(&app, "GET", "/api/companies", None, Some(&member)).await;
    assert_eq!(v["companies"].as_array().map(Vec::len), Some(0), "{v}");
    let (_, v, _) = send_raw(&app, "GET", "/api/companies", None, Some(&owner)).await;
    assert_eq!(v["companies"].as_array().map(Vec::len), Some(2), "{v}");

    // The company-scoped surface refuses the non-member.
    let alfa = v["companies"].as_array().expect("companies")[0]["id"]
        .as_str()
        .expect("id")
        .to_string();
    let (s, _, _) = send_raw(
        &app,
        "GET",
        &format!("/api/companies/{alfa}/tasks"),
        None,
        Some(&member),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // The owner brings the member in; the same surface opens.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/companies/{alfa}/members"),
        Some(json!({ "name": "ms_mem" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _, _) = send_raw(
        &app,
        "GET",
        &format!("/api/companies/{alfa}/tasks"),
        None,
        Some(&member),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v, _) = send_raw(&app, "GET", "/api/companies", None, Some(&member)).await;
    assert_eq!(
        v["companies"].as_array().map(Vec::len),
        Some(1),
        "one company now: {v}"
    );
}

/// Deleting a company is a member's verb (ADR-0034): membership is the
/// filter here as everywhere on the company-scoped surface -- an outsider
/// gets the same wordless 403 as for any other room they are not in.
#[tokio::test]
async fn deleting_a_company_is_its_members_verb() {
    let app = setup().await;
    let owner = claim(&app, "del_own", "correct-horse-battery").await;

    let (s, v, _) = send_raw(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Alfa" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let alfa = v["id"].as_str().expect("id").to_string();

    // A second person joins the instance but not the company.
    let code = mint(&app, &owner).await;
    let (_, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "del_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    let member = cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    // Not their room, not their verb.
    let (s, _, _) = send_raw(
        &app,
        "DELETE",
        &format!("/api/companies/{alfa}"),
        None,
        Some(&member),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Brought inside, the verb is theirs like any other.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/companies/{alfa}/members"),
        Some(json!({ "name": "del_mem" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _, _) = send_raw(
        &app,
        "DELETE",
        &format!("/api/companies/{alfa}"),
        None,
        Some(&member),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Gone for everyone, the founder included.
    let (_, v, _) = send_raw(&app, "GET", "/api/companies", None, Some(&owner)).await;
    assert_eq!(v["companies"].as_array().map(Vec::len), Some(0), "{v}");
}

/// Slice B of ADR-0033: a task, a session, an agent, an approval reached by
/// bare id belongs to a company, and the wall resolves the id to that
/// company before letting a member through -- the same wordless 403 an
/// outsider gets on the company-scoped surface. The audit feed filtered by
/// company is part of the same surface. An id nobody owns passes the wall
/// and meets the handler's 404: membership is organizational, and a member
/// asking about a vanished task deserves the truth, not a refusal.
#[tokio::test]
async fn the_bare_id_surface_is_gated_by_membership_too() {
    let app = setup().await;
    let owner = claim(&app, "bare_own", "correct-horse-battery").await;

    let (_, v, _) = send_raw(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Alfa" })),
        Some(&owner),
    )
    .await;
    let alfa = v["id"].as_str().expect("company id").to_string();
    let ceo = v["ceo"]["id"].as_str().expect("ceo id").to_string();
    let (_, v, _) = send_raw(
        &app,
        "POST",
        &format!("/api/companies/{alfa}/tasks"),
        Some(json!({ "title": "Private work" })),
        Some(&owner),
    )
    .await;
    let task = v["id"].as_str().expect("task id").to_string();

    let code = mint(&app, &owner).await;
    let (_, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "bare_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    let member = cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    // Reached by bare id, still not their room.
    for (method, uri) in [
        ("GET", format!("/api/tasks/{task}/sessions")),
        ("POST", format!("/api/agents/{ceo}/pause")),
        ("GET", format!("/api/audit/events?company_id={alfa}")),
    ] {
        let (s, _, _) = send_raw(&app, method, &uri, None, Some(&member)).await;
        assert_eq!(
            s,
            StatusCode::FORBIDDEN,
            "{method} {uri} should refuse an outsider"
        );
    }

    // An id nobody owns is not a secret: the wall lets it reach the 404.
    let (s, _, _) = send_raw(
        &app,
        "GET",
        "/api/sessions/not-a-session",
        None,
        Some(&member),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Brought inside, the same ids answer.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/companies/{alfa}/members"),
        Some(json!({ "name": "bare_mem" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    for uri in [
        format!("/api/tasks/{task}/sessions"),
        format!("/api/audit/events?company_id={alfa}"),
    ] {
        let (s, _, _) = send_raw(&app, "GET", &uri, None, Some(&member)).await;
        assert_eq!(s, StatusCode::OK, "GET {uri} should answer a member");
    }
}

/// The members surface (M25): a company can say who is inside it, in the
/// order they came in -- the founder first. Adding a colleague is already a
/// member's verb; this is the list the interface needs to show it.
#[tokio::test]
async fn a_company_lists_its_members_founder_first() {
    let app = setup().await;
    let owner = claim(&app, "list_own", "correct-horse-battery").await;
    let (_, v, _) = send_raw(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Alfa" })),
        Some(&owner),
    )
    .await;
    let alfa = v["id"].as_str().expect("company id").to_string();

    let (s, v, _) = send_raw(
        &app,
        "GET",
        &format!("/api/companies/{alfa}/members"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let names: Vec<&str> = v["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| m["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["list_own"]);

    let code = mint(&app, &owner).await;
    let (_, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "list_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/companies/{alfa}/members"),
        Some(json!({ "name": "list_mem" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (_, v, _) = send_raw(
        &app,
        "GET",
        &format!("/api/companies/{alfa}/members"),
        None,
        Some(&owner),
    )
    .await;
    let names: Vec<&str> = v["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| m["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["list_own", "list_mem"]);
    assert!(
        v["members"][1]["added_at"].as_str().is_some(),
        "each member says when they came in: {v}"
    );
}

/// The actor made legible (M25): the id has ridden inside every hashed
/// payload since M24; now the surfaces where decisions show say *who* beside
/// *what* -- resolved from the chain itself, the one source of truth, never
/// from a second column that could drift from it.
#[tokio::test]
async fn a_decision_says_who_made_it() {
    let app = setup().await;
    let owner = claim(&app, "dec_own", "correct-horse-battery").await;
    let (_, v, _) = send_raw(
        &app,
        "POST",
        "/api/companies",
        Some(json!({ "name": "Alfa" })),
        Some(&owner),
    )
    .await;
    let alfa = v["id"].as_str().expect("company id").to_string();
    let ceo = v["ceo"]["id"].as_str().expect("ceo id").to_string();

    // Gate the CEO, file a task, ask to start it: an approval is born.
    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/agents/{ceo}/approval-gate"),
        Some(json!({ "requires_approval": true })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v, _) = send_raw(
        &app,
        "POST",
        &format!("/api/companies/{alfa}/tasks"),
        Some(json!({ "title": "Needs sign-off", "execution_kind": "knowledge" })),
        Some(&owner),
    )
    .await;
    let task = v["id"].as_str().expect("task id").to_string();
    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/tasks/{task}/transition"),
        Some(json!({ "to": "todo" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v, _) = send_raw(
        &app,
        "POST",
        &format!("/api/tasks/{task}/start"),
        Some(json!({ "agent_id": ceo })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let approval = v["approval_id"].as_str().expect("approval id").to_string();

    // Undecided: nobody yet.
    let (_, v, _) = send_raw(
        &app,
        "GET",
        &format!("/api/companies/{alfa}/approvals"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(v["approvals"][0]["decided_by"], Value::Null, "{v}");

    let (s, _, _) = send_raw(
        &app,
        "POST",
        &format!("/api/approvals/{approval}/decision"),
        Some(json!({ "decision": "reject", "note": "not now" })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Decided: the approval says who, by name.
    let (_, v, _) = send_raw(
        &app,
        "GET",
        &format!("/api/companies/{alfa}/approvals"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(v["approvals"][0]["status"], json!("rejected"));
    assert_eq!(v["approvals"][0]["decided_by"], json!("dec_own"), "{v}");

    // And the chain's own feed names the actor beside every event it has.
    let (_, v, _) = send_raw(
        &app,
        "GET",
        &format!("/api/audit/events?company_id={alfa}"),
        None,
        Some(&owner),
    )
    .await;
    let decided = v["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|e| e["kind"] == "approval.decided")
        .expect("the decision is on the chain");
    assert_eq!(decided["actor_name"], json!("dec_own"), "{decided}");
}

/// The last company a person worked in is the server's to remember (M23,
/// carried): a fresh browser used to land on the first company rather than
/// the one you left. It is per user -- two people on one instance have two
/// answers -- and it is refused for a company you are not in.
#[tokio::test]
async fn the_server_remembers_where_each_person_left_off() {
    let app = setup().await;
    let owner = claim(&app, "lc_own", "correct-horse-battery").await;
    let mut ids = Vec::new();
    for name in ["Alfa", "Beta"] {
        let (_, v, _) = send_raw(
            &app,
            "POST",
            "/api/companies",
            Some(json!({ "name": name })),
            Some(&owner),
        )
        .await;
        ids.push(v["id"].as_str().expect("id").to_string());
    }
    let beta = ids[1].clone();

    // Nothing remembered yet.
    let (_, v, _) = send_raw(&app, "GET", "/api/auth", None, Some(&owner)).await;
    assert_eq!(v["last_company_id"], Value::Null, "{v}");

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/last-company",
        Some(json!({ "company_id": beta })),
        Some(&owner),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v, _) = send_raw(&app, "GET", "/api/auth", None, Some(&owner)).await;
    assert_eq!(v["last_company_id"], json!(beta), "{v}");

    // A member who is not in Beta cannot be remembered as being there.
    let code = mint(&app, &owner).await;
    let (_, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/signup",
        Some(json!({ "name": "lc_mem", "password": "another-long-pass", "invite": code })),
        None,
    )
    .await;
    let member = cookie
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();
    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/last-company",
        Some(json!({ "company_id": beta })),
        Some(&member),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
