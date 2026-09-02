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
    match session_identity(state, headers).await {
        Some((user_id, name, role)) => {
            // Where this person left off (M23, carried): the server's to
            // remember, so a fresh browser lands where you were.
            let last: Option<(Option<String>,)> =
                sqlx::query_as("SELECT last_company_id FROM users WHERE id = ?")
                    .bind(&user_id)
                    .fetch_optional(&state.pool)
                    .await
                    .ok()
                    .flatten();
            json!({
                "state": "in",
                "name": name,
                "role": role,
                "last_company_id": last.and_then(|(c,)| c),
            })
        }
        None => json!({ "state": "locked", "owner": owner_name }),
    }
}

/// Remember the company a person is working in (M23, carried). Per user,
/// not per browser; refused for a company they are not a member of -- the
/// owner passes, as everywhere.
pub async fn remember_last_company(
    state: &AppState,
    headers: &HeaderMap,
    company_id: &str,
) -> Result<(), ApiError> {
    let Some((user_id, _, role)) = session_identity(state, headers).await else {
        return Err(ApiError::Unauthorized);
    };
    if role != "owner" {
        let member: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM company_members WHERE company_id = ? AND user_id = ?",
        )
        .bind(company_id)
        .bind(&user_id)
        .fetch_one(&state.pool)
        .await?;
        if member == 0 {
            return Err(ApiError::Forbidden);
        }
    }
    sqlx::query("UPDATE users SET last_company_id = ? WHERE id = ?")
        .bind(company_id)
        .bind(&user_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// The full identity behind a session: (user id, name, role).
pub async fn session_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(String, String, String)> {
    let user_id = session_user(state, headers).await?;
    sqlx::query_as("SELECT id, name, role FROM users WHERE id = ?")
        .bind(&user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// The setup code (ADR-0045)
// ---------------------------------------------------------------------------

/// The file a fresh instance writes its setup code into, under the data dir.
pub const SETUP_CODE_FILE: &str = "setup-code";

/// Where the code lives. A file rather than a log line, so it survives a
/// restart unchanged, can be read by the installer, and is not re-minted under
/// somebody who has already copied it.
pub fn setup_code_path(config: &crate::db::Config) -> std::path::PathBuf {
    config.data_dir.join(SETUP_CODE_FILE)
}

/// The code, if this instance still has one. `None` once it has been claimed.
pub fn stored_setup_code(config: &crate::db::Config) -> Option<String> {
    let c = std::fs::read_to_string(setup_code_path(config)).ok()?;
    let c = c.trim().to_string();
    (!c.is_empty()).then_some(c)
}

/// Compare without letting the clock say how much of the code was right.
fn codes_match(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// Four groups of four, from Crockford's alphabet minus the letters that read
/// as digits: eighty bits, and a person can copy it off a terminal without
/// wondering whether that was an O or a zero.
fn new_setup_code() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw).expect("the operating system has randomness");
    let chars: Vec<char> = raw
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Mint one at boot when nobody owns this instance yet and none is waiting.
///
/// An unclaimed Overmind is open by construction — whoever reaches the port
/// first becomes its owner — and the documented way to share a company is to
/// bind it to a tailnet address. So the claim costs something only a person at
/// the machine can read.
pub async fn mint_setup_code(
    pool: &sqlx::SqlitePool,
    config: &crate::db::Config,
) -> Result<(), std::io::Error> {
    let owner_exists: bool = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .map(|n| n > 0)
        .unwrap_or(false);
    if owner_exists {
        // Nothing to guard, and nothing to leave lying about.
        let _ = std::fs::remove_file(setup_code_path(config));
        return Ok(());
    }
    if stored_setup_code(config).is_some() {
        // Already minted, and somebody may have copied it.
        return Ok(());
    }
    std::fs::create_dir_all(&config.data_dir)?;
    let code = new_setup_code();
    let path = setup_code_path(config);
    std::fs::write(&path, format!("{code}\n"))?;
    keep_to_the_server(&path);
    println!(
        "setup code for the first claim: {code}  (also in {}; it is asked for once, then deleted)",
        path.display()
    );
    Ok(())
}

/// `0600`: the code is the whole of the door until somebody claims it.
fn keep_to_the_server(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Spend it: a claim that succeeded is the last one this code can buy.
pub fn clear_setup_code(config: &crate::db::Config) {
    let _ = std::fs::remove_file(setup_code_path(config));
}

/// Does this request carry the code the instance is waiting for?
///
/// An instance with no code waiting has already been claimed, or was created
/// before this existed; either way the claim's own checks decide.
pub fn setup_code_ok(config: &crate::db::Config, given: Option<&str>) -> bool {
    match stored_setup_code(config) {
        None => true,
        Some(expected) => given.is_some_and(|g| codes_match(g.trim(), &expected)),
    }
}

/// The one thing a role changes today (M24): billing is the owner's.
/// Everything else works identically for every user until M25's real
/// permission model -- a decorative column would be worse than none.
pub async fn require_owner(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    // An unclaimed instance has no owner to protect yet.
    let anyone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
    if anyone == 0 {
        return Ok(());
    }
    match session_identity(state, headers).await {
        Some((_, _, role)) if role == "owner" => Ok(()),
        Some(_) => Err(ApiError::Forbidden),
        None => Err(ApiError::Unauthorized),
    }
}

#[derive(serde::Deserialize)]
pub struct Credentials {
    pub name: String,
    pub password: String,
    /// The invite code (M25): ignored while the instance is empty, required
    /// once anyone exists.
    #[serde(default)]
    pub invite: Option<String>,
    /// The setup code (ADR-0045): what a claim costs while the instance has
    /// nobody to refuse on its behalf.
    #[serde(default)]
    pub setup: Option<String>,
}

/// Create the owner -- exactly once, atomically. Of N concurrent claims one
/// INSERT wins: the guard is in the WHERE, not in application logic.
pub async fn claim(state: &AppState, req: &Credentials) -> Result<Response, ApiError> {
    // Before anything else, and before the password is even hashed: an
    // unclaimed instance has nobody to refuse on its behalf, so the code it
    // minted at boot is the whole of its door (ADR-0045).
    if !setup_code_ok(&state.config, req.setup.as_deref()) {
        return Err(ApiError::Unauthorized);
    }
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
        "INSERT INTO users (id, name, password_hash, created_at, role)
         SELECT ?, ?, ?, ?, 'owner' WHERE NOT EXISTS (SELECT 1 FROM users)",
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
    // A restore only ever checked emptiness at a point in time; this INSERT's
    // `WHERE NOT EXISTS` is the one atomic decision that matters, and once it
    // lands, no staged restore gets to overwrite it at the next boot.
    crate::backup::discard_stale_restore(&state.config);
    // Spent: this instance has an owner, and the code bought the only claim
    // it was ever going to buy.
    clear_setup_code(&state.config);
    audit_auth(state, "auth.claimed", name).await;
    open_session(state, name).await
}

/// Sign up: one more user in the local store (M24, reaching toward M25).
///
/// The store is `overmind.sqlite` itself -- local, created once, reused,
/// and holding argon2id hashes rather than passwords: the "protected file"
/// the owner asked for is the database the server already guards, because a
/// second credentials file beside it would be a second thing to leak.
///
/// Registration is open to whoever reaches the port, and the threat model
/// says so out loud: until M25 brings roles, every user sees everything,
/// and the port is loopback or a tunnel of your own machines.
pub async fn signup(state: &AppState, req: &Credentials) -> Result<Response, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::Invalid("a user needs a name".into()));
    }
    if req.password.len() < 8 {
        return Err(ApiError::Invalid(
            "the password needs at least 8 characters".into(),
        ));
    }
    if over_limit(name) {
        return Err(ApiError::Invalid("too many attempts; wait a minute".into()));
    }
    // One transaction for the whole entry (M25): the invite gate, the user
    // row and the invite's attribution commit together -- a taken name must
    // not leave a code half-burnt, and of two racers on one code exactly
    // one gets in.
    let hash = hash_password(&req.password)?;
    let user_id = new_id();
    let mut tx = state.write_tx().await?;
    let anyone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await
        .unwrap_or(0);
    // Nobody has claimed this instance, so there is nobody who could have
    // invited you -- and signing up here used to make you its *owner*, with
    // neither an invite nor the setup code the claim costs (ADR-0045). The
    // first person in is the owner, through the claim, holding the code.
    if anyone == 0 {
        return Err(ApiError::Unauthorized);
    }
    if anyone > 0
        && req
            .invite
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .is_none()
    {
        return Err(ApiError::Invalid("an invite code is required".into()));
    }
    // The user row first, the invite spend second -- the foreign key on
    // `used_by` checks immediately, so the row it names must exist. Both
    // live or die with the same transaction: a refused spend discards the
    // row, a taken name hands the code back untouched.
    let done = sqlx::query(
        "INSERT OR IGNORE INTO users (id, name, password_hash, created_at, role)
         VALUES (?, ?, ?, ?,
                 CASE WHEN EXISTS (SELECT 1 FROM users) THEN 'member' ELSE 'owner' END)",
    )
    .bind(&user_id)
    .bind(name)
    .bind(&hash)
    .bind(now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Unauthorized);
    }
    if anyone > 0 {
        let code = req.invite.as_deref().map(str::trim).expect("checked above");
        let spent = sqlx::query(
            "UPDATE invites SET used_by = ? WHERE id = ? AND used_by IS NULL AND expires_at > ?",
        )
        .bind(&user_id)
        .bind(token_hash(code))
        .bind(now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        if spent.rows_affected() == 0 {
            // Wordless, and the dropped transaction takes the user row with
            // it: no code, no account.
            return Err(ApiError::Unauthorized);
        }
    }
    tx.commit().await?;
    audit_auth(state, "auth.signed_up", name).await;
    open_session(state, name).await
}

/// Mint an invite (owner only): a single-use code, seven days, stored
/// hashed. The raw code appears exactly once, in this response.
pub async fn mint_invite(state: &AppState, headers: &HeaderMap) -> Result<Value, ApiError> {
    require_owner(state, headers).await?;
    let Some((user_id, _, _)) = session_identity(state, headers).await else {
        return Err(ApiError::Unauthorized);
    };
    let code = new_token()?;
    let n = now();
    sqlx::query("INSERT INTO invites (id, created_by, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(token_hash(&code))
        .bind(&user_id)
        .bind(n.to_rfc3339())
        .bind((n + chrono::Duration::days(7)).to_rfc3339())
        .execute(&state.pool)
        .await?;
    audit_auth(state, "auth.invite_minted", &user_id).await;
    Ok(json!({ "invite": code, "expires_days": 7 }))
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
    let user: Option<(String, String)> =
        sqlx::query_as("SELECT id, role FROM users WHERE name = ?")
            .bind(user_name)
            .fetch_optional(&state.pool)
            .await?;
    let Some((user_id, role)) = user else {
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
        Json(json!({ "state": "in", "name": user_name, "role": role })),
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

/// The company a request is about, when its path (or its query) names one
/// (ADR-0033, slice B). `/companies/{id}/…` names it outright; the bare-id
/// surface -- a task, a session, an agent, an approval, an artifact reached
/// by its own id -- names it through a row, and that row is read here so the
/// wall can ask the same membership question it asks of the company-scoped
/// surface. `None` when the path is not company-scoped at all, or when the
/// id resolves to nothing: an id nobody owns is not a secret, and the handler
/// behind it answers 404 the way it always did.
async fn company_of(pool: &sqlx::SqlitePool, path: &str, query: Option<&str>) -> Option<String> {
    if let Some(rest) = path.strip_prefix("/companies/") {
        return rest
            .split('/')
            .next()
            .filter(|c| !c.is_empty())
            .map(str::to_string);
    }
    // The audit feed filtered by company is the same surface, read sideways.
    if path == "/audit/events" {
        return query?
            .split('&')
            .find_map(|kv| kv.strip_prefix("company_id="))
            .filter(|c| !c.is_empty())
            .map(str::to_string);
    }
    const RESOLVERS: &[(&str, &str)] = &[
        ("/agents/", "SELECT company_id FROM agents WHERE id = ?"),
        (
            "/approvals/",
            "SELECT company_id FROM approvals WHERE id = ?",
        ),
        (
            "/artifacts/",
            "SELECT t.company_id FROM task_artifacts a JOIN tasks t ON t.id = a.task_id WHERE a.id = ?",
        ),
        ("/meetings/", "SELECT company_id FROM meetings WHERE id = ?"),
        (
            "/notifications/",
            "SELECT company_id FROM notifications WHERE id = ?",
        ),
        (
            "/org-proposals/",
            "SELECT company_id FROM org_proposals WHERE id = ?",
        ),
        ("/projects/", "SELECT company_id FROM projects WHERE id = ?"),
        (
            "/sessions/",
            "SELECT t.company_id FROM agent_task_sessions s JOIN tasks t ON t.id = s.task_id WHERE s.id = ?",
        ),
        ("/tasks/", "SELECT company_id FROM tasks WHERE id = ?"),
        (
            "/tokens/",
            "SELECT company_id FROM company_tokens WHERE id = ?",
        ),
    ];
    let (prefix, sql) = RESOLVERS.iter().find(|(p, _)| path.starts_with(p))?;
    let id = path[prefix.len()..]
        .split('/')
        .next()
        .filter(|c| !c.is_empty())?;
    sqlx::query_scalar::<_, String>(sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
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
        "/auth" | "/auth/claim" | "/auth/signup" | "/auth/login" | "/auth/logout"
    );
    if open {
        return next.run(req).await;
    }

    // A restore is a claim with a payload (ADR-0044), so it is reachable
    // exactly while a claim is: on an instance nobody owns yet. On a claimed
    // one it stays behind the wall, which refuses a stranger *before* the
    // upload rather than after it -- and the handler demands the setup code
    // like the claim does (ADR-0045).
    if path == "/restore" {
        let owner_exists: bool = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await
            .map(|n| n > 0)
            .unwrap_or(false);
        if !owner_exists {
            return next.run(req).await;
        }
    }

    // Until ADR-0045 an unclaimed instance waved the whole API through --
    // "the boundary is the credential, and until one exists there is nothing
    // to guard with". But there is: this machine. A stranger reaching the port
    // of an unclaimed instance could found a company and start a task, which
    // runs an agent CLI here; choose who pays; open the subscription sign-in;
    // mint an invite. The door answers about itself and takes a claim. Nothing
    // else answers anybody without a session.

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

    if let Some((user_id, _, role)) = session_identity(&state, req.headers()).await {
        // Membership gates every company-scoped request (M25, ADR-0033):
        // /companies/{id}/... names the company outright, the bare-id
        // surface names it through a row (slice B), and both answer members
        // and the owner. Organizational, not adversarial -- the threat model
        // says so out loud.
        if role != "owner"
            && let Some(company_id) = company_of(&state.pool, path, req.uri().query()).await
        {
            let member: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM company_members WHERE company_id = ? AND user_id = ?",
            )
            .bind(&company_id)
            .bind(&user_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);
            if member == 0 {
                return ApiError::Forbidden.into_response();
            }
        }
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
