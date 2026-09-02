//! What every suite that is *not* about the door now needs: an instance
//! somebody owns.
//!
//! Until ADR-0045 an unclaimed Overmind waved the whole API through, so a test
//! could found a company and run work without anybody ever having claimed the
//! instance. That state no longer exists in production -- a fresh install
//! answers about its door and takes a claim, and nothing else -- so a test
//! that relied on it was exercising a configuration nobody can reach.
//!
//! [`claimed`] claims the instance with the code the server minted at boot and
//! hands back a router that carries the owner's session on every request. The
//! suites keep their `send(&app, …)` helpers exactly as they were: what
//! changed is that there is now somebody on the other side of them.

use axum::extract::Request;
use axum::http::{StatusCode, header};

/// The owner's name and password, the same in every suite: what matters here
/// is that somebody owns the instance, never who.
const OWNER: &str = "the owner";
const PASSWORD: &str = "a long enough password";

/// Claim `app`, and return it wrapping every later request in the session.
///
/// A second server over the same database -- which is how the restart and
/// recovery suites are written -- finds the instance already claimed and no
/// code left to spend, because a successful claim deletes it. So this does
/// what the person would do: claim the first time, sign in after a restart.
pub async fn claimed(app: axum::Router, data_dir: &std::path::Path) -> axum::Router {
    let code = std::fs::read_to_string(data_dir.join("setup-code"))
        .ok()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());

    let (uri, body) = match code {
        Some(code) => (
            "/api/auth/claim",
            serde_json::json!({ "name": OWNER, "password": PASSWORD, "setup": code }),
        ),
        None => (
            "/api/auth/login",
            serde_json::json!({ "name": OWNER, "password": PASSWORD }),
        ),
    };
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("build the request");

    let response = tower::ServiceExt::oneshot(app.clone(), request)
        .await
        .expect("the router answers");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the test instance could not be entered"
    );
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .expect("entering sets the session cookie")
        .split(';')
        .next()
        .expect("the cookie's k=v")
        .to_string();

    // Carried by the router rather than by every call site: the suites below
    // are about companies, agents and runs, not about who is holding the door.
    app.layer(axum::middleware::map_request(move |mut req: Request| {
        let cookie = cookie.clone();
        async move {
            if !req.headers().contains_key(header::COOKIE) {
                req.headers_mut().insert(
                    header::COOKIE,
                    header::HeaderValue::from_str(&cookie).expect("a cookie header"),
                );
            }
            req
        }
    }))
}
