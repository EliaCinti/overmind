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

async fn setup() -> axum::Router {
    let state = overmind_server::init_with(
        "sqlite::memory:",
        overmind_server::Config {
            data_dir: std::env::temp_dir()
                .join(format!("overmind-door-{}", uuid::Uuid::now_v7().simple())),
            ..overmind_server::Config::default()
        },
    )
    .await
    .expect("init");
    overmind_server::app(state)
}

/// Claim the owner and hand back the session cookie's `k=v` pair.
async fn claim(app: &axum::Router, name: &str, pass: &str) -> String {
    let (s, _, cookie) = send_raw(
        app,
        "POST",
        "/api/auth/claim",
        Some(json!({ "name": name, "password": pass })),
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

/// Before an owner exists the API is exactly as open as it was before M24:
/// a fresh install must be able to work and to claim itself.
#[tokio::test]
async fn an_unclaimed_instance_is_open_and_says_so() {
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
    assert_eq!(s, StatusCode::CREATED);
}

/// Once claimed, the wall stands: no session, no API -- and the health
/// answer is redacted down to liveness.
#[tokio::test]
async fn a_claimed_instance_refuses_the_sessionless() {
    let app = setup().await;
    let _cookie = claim(&app, "elia", "correct-horse-battery").await;

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
    let cookie = claim(&app, "elia", "correct-horse-battery").await;

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
    let mut wins = 0;
    let mut handles = Vec::new();
    for i in 0..6 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let (s, _, _) = send_raw(
                &app,
                "POST",
                "/api/auth/claim",
                Some(json!({ "name": format!("racer{i}"), "password": "long-enough-pass" })),
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
    let _ = claim(&app, "elia", "correct-horse-battery").await;

    let (s, _, cookie) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "elia", "password": "correct-horse-battery" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(cookie.is_some());

    let (s, _, _) = send_raw(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({ "name": "elia", "password": "wrong" })),
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
    let cookie = claim(&app, "elia", "correct-horse-battery").await;

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
        Some(json!({ "name": "elia", "password": "correct-horse-battery" })),
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
        Some(json!({ "name": "elia", "password": "short" })),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
