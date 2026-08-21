//! The door (M24, ADR-0032): one owner, claimed at first run; argon2id
//! passwords; server-side sessions stored hashed; a wall in front of every
//! route but the door itself.
//!
//! What this module deliberately is NOT: a user system. M25 makes users
//! plural; today the only identity is the owner, and the only thing worth
//! building is the boundary -- built the way the owner asked: properly.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::api::ApiError;
use crate::db::AppState;

/// Session lifetime: 30 days, sliding (each authenticated request renews).
const SESSION_DAYS: i64 = 30;
/// The cookie the browser holds. The value is the raw token; the server
/// stores only its hash.
const COOKIE: &str = "overmind_session";

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// 256 bits from the OS, hex-encoded. The session token itself.
fn new_token() -> Result<String, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| ApiError::Invalid("the OS would not provide randomness".into()))?;
    Ok(hex::encode(bytes))
}

/// What the database stores: the SHA-256 of the token, hex.
fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Hash a password for storage. ~100ms by construction: the work factor is
/// the point.
fn hash_password(password: &str) -> Result<String, ApiError> {
    let salt = SaltString::from_b64(&b64_salt()?)
        .map_err(|_| ApiError::Invalid("salt generation failed".into()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| ApiError::Invalid("password hashing failed".into()))
}

fn b64_salt() -> Result<String, ApiError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| ApiError::Invalid("the OS would not provide randomness".into()))?;
    // Standard unpadded base64 for PHC salts, hand-rolled to avoid a
    // dependency for sixteen bytes.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
    }
    Ok(out)
}

fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

// ── rate limiting ───────────────────────────────────────────────────────────

/// Login attempts per attempted name: a small fixed window on top of
/// argon2's own cost. Per name and not per IP, deliberately: this server
/// lives on loopback or behind a tunnel, where every caller shares an
/// address and an IP bucket is theater -- what actually needs protecting is
/// the owner's password against being guessed at. In-memory on purpose:
/// restarting the server to reset it is a feature an attacker does not
/// control.
static ATTEMPTS: Mutex<Option<HashMap<String, (u32, Instant)>>> = Mutex::new(None);
const MAX_ATTEMPTS: u32 = 5;
const WINDOW: Duration = Duration::from_secs(60);

fn over_limit(source: &str) -> bool {
    let mut guard = match ATTEMPTS.lock() {
        Ok(g) => g,
        // A poisoned limiter fails closed: refusing logins beats waving
        // them through.
        Err(_) => return true,
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(source.to_string()).or_insert((0, Instant::now()));
    if entry.1.elapsed() > WINDOW {
        *entry = (0, Instant::now());
    }
    entry.0 += 1;
    entry.0 > MAX_ATTEMPTS
}

// ── the door's own endpoints ────────────────────────────────────────────────

/// Where the door stands: `unclaimed` (no owner yet), `locked` (owner
/// exists, this request carries no valid session), or `in` (name attached).
pub async fn state_of(state: &AppState, headers: &HeaderMap) -> Value {
    let owner: Option<(String,)> = sqlx::query_as("SELECT name FROM users LIMIT 1")
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    let Some((owner_name,)) = owner else {
        return json!({ "state": "unclaimed" });
    };
    match session_user(state, headers).await {
        Some(_) => json!({ "state": "in", "name": owner_name }),
        None => json!({ "state": "locked" }),
    }
}

#[derive(serde::Deserialize)]
pub struct Credentials {
    pub name: String,
    pub password: String,
}

/// Create the owner -- exactly once, atomically. Of N concurrent claims one
/// INSERT wins: the guard is in the WHERE, not in application logic.
pub async fn claim(state: &AppState, req: &Credentials) -> Result<Response, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::Invalid("the owner needs a name".into()));
    }
    if req.password.len() < 8 {
        return Err(ApiError::Invalid(
            "the password needs at least 8 characters".into(),
        ));
    }
    let hash = hash_password(&req.password)?;
    let done = sqlx::query(
        "INSERT INTO users (id, name, password_hash, created_at)
         SELECT ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users)",
    )
    .bind(new_id())
    .bind(name)
    .bind(&hash)
    .bind(now().to_rfc3339())
    .execute(&state.pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Invalid("the owner is already claimed".into()));
    }
    audit_auth(state, "auth.claimed", name).await;
    open_session(state, name).await
}

/// Log in. The same answer for a wrong name and a wrong password: which of
/// the two was wrong is exactly what an attacker wants to learn.
pub async fn login(state: &AppState, req: &Credentials) -> Result<Response, ApiError> {
    if over_limit(req.name.trim()) {
        audit_auth(state, "auth.rate_limited", req.name.trim()).await;
        return Err(ApiError::Invalid("too many attempts; wait a minute".into()));
    }
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT name, password_hash FROM users WHERE name = ?")
            .bind(req.name.trim())
            .fetch_optional(&state.pool)
            .await?;
    // Verify against a dummy hash when the user is unknown, so timing does
    // not tell names from passwords.
    const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let ok = match &row {
        Some((_, hash)) => verify_password(hash, &req.password),
        None => {
            let _ = verify_password(DUMMY, &req.password);
            false
        }
    };
    if !ok {
        audit_auth(state, "auth.failed", req.name.trim()).await;
        return Err(ApiError::Unauthorized);
    }
    let (name, _) = row.expect("verified row exists");
    audit_auth(state, "auth.login", &name).await;
    open_session(state, &name).await
}

/// Mint a session and hand the browser its cookie.
async fn open_session(state: &AppState, user_name: &str) -> Result<Response, ApiError> {
    let user: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE name = ?")
        .bind(user_name)
        .fetch_optional(&state.pool)
        .await?;
    let Some((user_id,)) = user else {
        return Err(ApiError::Unauthorized);
    };
    let token = new_token()?;
    let n = now();
    sqlx::query(
        "INSERT INTO auth_sessions (id, user_id, created_at, last_seen, expires_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(token_hash(&token))
    .bind(&user_id)
    .bind(n.to_rfc3339())
    .bind(n.to_rfc3339())
    .bind((n + chrono::Duration::days(SESSION_DAYS)).to_rfc3339())
    .execute(&state.pool)
    .await?;
    let cookie = format!(
        "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{}",
        SESSION_DAYS * 24 * 3600,
        if state.config.cookie_secure {
            "; Secure"
        } else {
            ""
        },
    );
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "state": "in", "name": user_name })),
    )
        .into_response())
}

/// Log out: the session dies server-side, and the cookie is told to.
pub async fn logout(state: &AppState, headers: &HeaderMap) -> Response {
    if let Some(token) = cookie_token(headers) {
        let _ = sqlx::query("DELETE FROM auth_sessions WHERE id = ?")
            .bind(token_hash(&token))
            .execute(&state.pool)
            .await;
    }
    let cookie = format!("{COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "state": "locked" })),
    )
        .into_response()
}

/// The token in the request's cookies, if any.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("overmind_session=") {
            return Some(v.to_string());
        }
    }
    None
}

/// The user this request belongs to, if its session is real and unexpired.
/// A hit slides the expiry forward.
pub async fn session_user(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let token = cookie_token(headers)?;
    let hash = token_hash(&token);
    let n = now();
    let row: Option<(String,)> =
        sqlx::query_as("SELECT user_id FROM auth_sessions WHERE id = ? AND expires_at > ?")
            .bind(&hash)
            .bind(n.to_rfc3339())
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
    let (user_id,) = row?;
    let _ = sqlx::query("UPDATE auth_sessions SET last_seen = ?, expires_at = ? WHERE id = ?")
        .bind(n.to_rfc3339())
        .bind((n + chrono::Duration::days(SESSION_DAYS)).to_rfc3339())
        .bind(&hash)
        .execute(&state.pool)
        .await;
    Some(user_id)
}

/// Auth events go to the audit chain like everything else -- with the actor,
/// which is the whole reason M25 can mean anything.
async fn audit_auth(state: &AppState, kind: &str, who: &str) {
    if let Ok(mut tx) = state.write_tx().await {
        // `who` is the *attempted* name -- on a failed login it names the
        // target, not an authenticated actor, which is why the key differs
        // from the injected `actor`.
        let _ = crate::audit::append(&mut tx, None, None, kind, &json!({ "who": who })).await;
        let _ = tx.commit().await;
    }
}

/// The wall for the live socket: an unauthenticated upgrade is refused
/// before it becomes a stream. Same unclaimed-instance exception as the API.
pub async fn ws_wall(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let owner_exists: bool = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await
        .map(|n| n > 0)
        .unwrap_or(false);
    if !owner_exists || session_user(&state, req.headers()).await.is_some() {
        return next.run(req).await;
    }
    ApiError::Unauthorized.into_response()
}

// ── the wall ────────────────────────────────────────────────────────────────

/// The middleware in front of `/api`: everything requires a session except
/// the door itself and a redacted health.
pub async fn wall(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path();

    // The door, and the door only. The wall is layered on the nested /api
    // router, which sees paths with the prefix already stripped -- measured
    // by the door suite: matching "/api/auth" here let nobody log in.
    let open = matches!(
        path,
        "/auth" | "/auth/claim" | "/auth/login" | "/auth/logout"
    );
    if open {
        return next.run(req).await;
    }

    // No owner yet: the whole API stays open, exactly as before M24. The
    // boundary is the credential, and until one exists there is nothing to
    // guard with -- and a fresh install must be able to claim itself.
    let owner_exists: bool = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await
        .map(|n| n > 0)
        .unwrap_or(false);
    if !owner_exists {
        return next.run(req).await;
    }

    // The CSRF belt on top of SameSite=Strict (ADR-0032): a state-changing
    // request that carries a body must declare it JSON or multipart --
    // shapes a cross-site form cannot produce with credentials attached.
    // Bodyless mutations are covered by the cookie's SameSite alone.
    let mutating = matches!(req.method().as_str(), "POST" | "PUT" | "PATCH" | "DELETE");
    if mutating {
        let has_body = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|n| n > 0)
            .unwrap_or(false);
        if has_body {
            let ct = req
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !ct.starts_with("application/json") && !ct.starts_with("multipart/form-data") {
                return ApiError::Invalid("unexpected content type".into()).into_response();
            }
        }
    }

    if let Some(user_id) = session_user(&state, req.headers()).await {
        // Every audit event this request appends carries who did it (M24):
        // the field M25's "who approved this" is made of.
        return crate::audit::ACTOR
            .scope(Some(user_id), next.run(req))
            .await;
    }

    // Redacted health: probes and orchestrators keep their liveness answer,
    // the economy and plan windows stop leaking.
    if path == "/health" {
        return Json(json!({ "status": "ok" })).into_response();
    }

    ApiError::Unauthorized.into_response()
}
