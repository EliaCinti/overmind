use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit;
use crate::db::AppState;
use crate::domain::{AgentTraits, DomainPatch, TaskStatus, TraitsPatch, event_kind};

/// The full application: JSON API under `/api`, the live-update WebSocket at
/// `/ws`, and (when built) the SPA served at the root with history fallback.
pub fn app(state: AppState) -> Router {
    let mut router = Router::new()
        .nest(
            "/api",
            api_router()
                // The wall (M24): every /api route requires a session except
                // the door itself and a redacted health. Layered here so /mcp
                // (its own bearer tokens), /ws (guarded below by session too)
                // and the SPA's static files stay outside it.
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::auth::wall,
                )),
        )
        // Overmind's own MCP surface, for the agents it runs (ADR-0027). Not
        // under /api: it is a protocol endpoint, not part of the JSON API the
        // UI speaks, and its caller authenticates with a per-run bearer token
        // rather than being same-origin.
        .merge(crate::mcp_server::router())
        .route(
            "/ws",
            get(crate::ws::handler)
                .layer(axum::middleware::from_fn(crate::ws::guard_origin))
                // The socket authenticates like everything else (M24): the
                // cookie rides the upgrade request. Same wall -- and since
                // ADR-0045 no exception for an unclaimed instance either.
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::auth::ws_wall,
                )),
        );

    // Serve the built frontend if present; unknown paths fall back to
    // index.html so client-side routing works. Absent in API-only/dev mode
    // (Vite's dev server proxies /api and /ws to us instead).
    // CORS is a **development-only** affordance, and deliberately absent in
    // production. Overmind has no authentication and its API starts tasks that
    // run a CLI on this machine, so running "only on localhost" is not the
    // protection it sounds like: the browser is the attack surface, not the
    // network. With a permissive policy, any page you happen to have open can
    // POST to 127.0.0.1:7070 and start a task here.
    //
    // Serving the SPA ourselves means the UI is same-origin and needs no CORS
    // at all; without the layer, a cross-origin JSON request fails its
    // preflight and the browser refuses it. In dev the UI lives on Vite's
    // origin, so we allow exactly that one.
    //
    // If you ever "fix a CORS error" by widening this, put authentication in
    // first (M10).
    if state.config.web_dir.is_dir() {
        let index = state.config.web_dir.join("index.html");
        // The HTML document revalidates on every load; the hashed assets it
        // names may cache forever. Without this, Safari kept serving a
        // deploy-old bundle from cache and a fixed bug looked unfixed —
        // measured on the owner's machine, resolved by a hard reload nobody
        // should need.
        router = router.fallback_service(
            tower_http::services::ServeDir::new(&state.config.web_dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    } else {
        use tower_http::cors::{Any, CorsLayer};
        let dev_origins = [
            "http://localhost:5173".parse().expect("static origin"),
            "http://127.0.0.1:5173".parse().expect("static origin"),
        ];
        router = router.layer(
            CorsLayer::new()
                .allow_origin(dev_origins)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    }

    // Uploads are the point of M17, and axum's 2 MB default is under the size
    // of an ordinary scanned PDF — small enough that "attach anything" would
    // have failed on the first real file. 128 MB is chosen against what a
    // person plausibly hands an agent (a dataset, a recording, a design file),
    // not against what the machine can take.
    router
        .layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        // The SPA's HTML revalidates on every load; its hashed assets may
        // cache forever. Applied to every text/html response (only the SPA
        // serves any): without it, Safari kept a deploy-old bundle and a
        // fixed bug looked unfixed until a hard reload nobody should need.
        .layer(axum::middleware::map_response(
            |mut res: axum::response::Response| async move {
                let is_html = res
                    .headers()
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|ct| ct.starts_with("text/html"));
                if is_html
                    && !res
                        .headers()
                        .contains_key(axum::http::header::CACHE_CONTROL)
                {
                    res.headers_mut().insert(
                        axum::http::header::CACHE_CONTROL,
                        axum::http::HeaderValue::from_static("no-cache"),
                    );
                }
                res
            },
        ))
        .with_state(state)
}

/// The largest single upload accepted. See the note in [`app`].
pub const MAX_UPLOAD_BYTES: usize = 128 * 1024 * 1024;

fn api_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/companies", post(create_company).get(list_companies))
        // Deleting is a member's verb like every other on this surface
        // (ADR-0034); the membership wall already keeps outsiders off it.
        .route("/companies/{company_id}", delete(delete_company))
        .route("/archetypes", get(list_archetypes))
        .route("/domains", get(list_domains))
        .route("/models", get(list_models))
        // The tools an agent may be granted (ADR-0036): what the operator
        // declared, listed so the interface can offer exactly that.
        .route("/tools", get(list_tools))
        .route("/companies/{company_id}/language", post(set_language))
        // Membership (M25): any member brings in a colleague by name.
        .route(
            "/companies/{company_id}/members",
            post(add_member).get(list_members),
        )
        .route(
            "/companies/{company_id}/brain",
            get(brain_status).post(set_brain_enabled),
        )
        .route(
            "/companies/{company_id}/memory/memories",
            get(browse_memories),
        )
        .route(
            "/companies/{company_id}/memory/decisions",
            get(browse_decisions),
        )
        .route("/languages", get(list_languages))
        .route(
            "/companies/{company_id}/tokens",
            post(create_company_token).get(list_company_tokens),
        )
        .route("/tokens/{token_id}/revoke", post(revoke_company_token))
        .route(
            "/companies/{company_id}/agents",
            post(hire_agent).get(list_agents),
        )
        .route("/agents/{agent_id}/reassign", post(reassign_agent))
        .route("/agents/{agent_id}/pause", post(pause_agent))
        .route("/agents/{agent_id}/resume", post(resume_agent))
        .route("/agents/{agent_id}/terminate", post(terminate_agent))
        .route(
            "/agents/{agent_id}/approval-gate",
            post(set_requires_approval),
        )
        .route("/agents/{agent_id}/budget", post(set_agent_budget))
        .route("/agents/{agent_id}/tools", post(set_agent_tools))
        .route("/agents/{agent_id}/traits", post(patch_agent_traits))
        .route("/agents/{agent_id}/revisions", get(list_revisions))
        .route("/agents/{agent_id}/rollback", post(rollback_agent))
        .route("/companies/{company_id}/approvals", get(list_approvals))
        .route("/approvals/{approval_id}/decision", post(decide_approval))
        .route("/companies/{company_id}/budget", get(budget_summary))
        .route(
            "/companies/{company_id}/agents/{agent_id}/conversation",
            get(get_conversation),
        )
        .route(
            "/companies/{company_id}/agents/{agent_id}/conversation/messages",
            post(post_message),
        )
        .route(
            "/companies/{company_id}/agents/{agent_id}/conversation/attachments",
            post(upload_attachment),
        )
        .route(
            "/companies/{company_id}/conversation/attachments/{attachment_id}",
            get(download_attachment),
        )
        .route(
            "/companies/{company_id}/meetings",
            post(convene_meeting).get(list_meetings),
        )
        .route("/meetings/{meeting_id}", get(get_meeting))
        .route(
            "/companies/{company_id}/meetings/{meeting_id}/resume",
            post(resume_meeting),
        )
        .route(
            "/companies/{company_id}/org-proposals",
            get(list_org_proposals),
        )
        .route("/org-proposals/{proposal_id}", get(get_org_proposal))
        .route(
            "/org-proposals/{proposal_id}/members/{member_id}",
            post(set_member_excluded),
        )
        .route(
            "/companies/{company_id}/notifications",
            get(list_notifications),
        )
        .route(
            "/companies/{company_id}/notifications/read",
            post(read_all_notifications),
        )
        .route(
            "/notifications/{notification_id}/read",
            post(read_notification),
        )
        .route(
            "/companies/{company_id}/projects",
            post(create_project).get(list_projects),
        )
        .route("/projects/{project_id}/goals", post(create_goal))
        .route(
            "/projects/{project_id}/workspaces",
            post(create_workspace).get(list_workspaces),
        )
        .route(
            "/companies/{company_id}/tasks",
            post(create_task).get(list_tasks),
        )
        .route("/tasks/{task_id}/transition", post(transition_task))
        .route("/tasks/{task_id}/start", post(start_task))
        .route("/tasks/{task_id}/sessions", get(list_task_sessions))
        .route("/tasks/{task_id}/artifacts", get(list_task_artifacts))
        .route(
            "/tasks/{task_id}/attachments",
            post(upload_task_attachment).get(list_task_attachments),
        )
        .route(
            "/tasks/{task_id}/attachments/{attachment_id}",
            delete(remove_task_attachment),
        )
        .route("/artifacts/{artifact_id}/download", get(download_artifact))
        .route("/agents/{agent_id}/wakeup", post(request_wakeup))
        .route("/sessions/{session_id}", get(get_session))
        .route("/sessions/{session_id}/diff", get(get_session_diff))
        .route("/audit/events", get(list_events))
        .route("/audit/verify", get(verify_chain))
        .route("/memory/status", get(memory_status))
        // The door (M24, ADR-0032).
        .route("/auth", get(auth_state))
        .route("/auth/claim", post(auth_claim))
        .route("/auth/signup", post(auth_signup))
        .route("/auth/invites", post(auth_mint_invite))
        .route("/auth/login", post(auth_login))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/last-company", post(auth_last_company))
        // Signing the agent CLI into a Claude subscription, from the product
        // (M23). A setup surface: loopback-only like everything else today.
        .route("/claude-auth", get(claude_auth_status))
        .route("/claude-auth/start", post(claude_auth_start))
        .route("/claude-auth/code", post(claude_auth_code))
        .route("/economy/pay-with", post(pay_with))
        // The archive is the instance (ADR-0044): the owner's verb, on a
        // claimed instance only -- `require_owner` alone would let anyone on
        // the port of an unclaimed box fill the folder and download it.
        .route("/backup", post(create_backup))
        .route("/backups", get(list_backups))
        .route("/backup/{name}", get(download_backup).delete(delete_backup))
        // The archive is bigger than any upload the rest of the API takes,
        // and it is streamed to disk rather than held: the router's 128 MB
        // limit is lifted for this one route (ADR-0044). It answers on an
        // *empty* instance only -- see `what_makes_the_instance_full`.
        .route(
            "/restore",
            post(restore_instance).layer(axum::extract::DefaultBodyLimit::disable()),
        )
}

#[derive(serde::Deserialize)]
struct AddMember {
    name: String,
}

/// Add a registered user to a company (M25). Any member can: a team invites
/// a colleague, and the owner is not a bottleneck. The wall has already
/// established the caller is a member (or the owner) of this company.
/// Who is inside a company, in the order they came in -- the founder first
/// (M25). The instance role rides along so the interface can mark the
/// administrator without a second request; it is not a per-company role,
/// there are none (ADR-0033, decision 3).
async fn list_members(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT u.id, u.name, u.role, m.added_at
         FROM company_members m JOIN users u ON u.id = m.user_id
         WHERE m.company_id = ? ORDER BY m.added_at, u.name",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let members: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, role, added_at)| {
            json!({ "id": id, "name": name, "role": role, "added_at": added_at })
        })
        .collect();
    Ok(Json(json!({ "members": members })))
}

async fn add_member(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<AddMember>,
) -> Result<StatusCode, ApiError> {
    let user: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE name = ?")
        .bind(req.name.trim())
        .fetch_optional(&state.pool)
        .await?;
    let Some((user_id,)) = user else {
        return Err(ApiError::NotFound("user"));
    };
    let mut tx = state.write_tx().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO company_members (company_id, user_id, added_at) VALUES (?, ?, ?)",
    )
    .bind(&company_id)
    .bind(&user_id)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    crate::audit::append(
        &mut tx,
        Some(&company_id),
        None,
        "company.member_added",
        &json!({ "user": req.name.trim() }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(StatusCode::CREATED)
}

/// Where the door stands (M24): unclaimed, locked, or in.
async fn auth_state(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    Json(crate::auth::state_of(&state, &headers).await)
}

async fn auth_claim(
    State(state): State<AppState>,
    Json(req): Json<crate::auth::Credentials>,
) -> Result<axum::response::Response, ApiError> {
    crate::auth::claim(&state, &req).await
}

async fn auth_signup(
    State(state): State<AppState>,
    Json(req): Json<crate::auth::Credentials>,
) -> Result<axum::response::Response, ApiError> {
    crate::auth::signup(&state, &req).await
}

async fn auth_mint_invite(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(crate::auth::mint_invite(&state, &headers).await?))
}

async fn auth_login(
    State(state): State<AppState>,
    Json(req): Json<crate::auth::Credentials>,
) -> Result<axum::response::Response, ApiError> {
    crate::auth::login(&state, &req).await
}

async fn auth_logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    crate::auth::logout(&state, &headers).await
}

#[derive(serde::Deserialize)]
struct LastCompany {
    company_id: String,
}

/// Where this person is working now (M23, carried): remembered per user, so
/// a fresh browser lands there instead of on the first company.
async fn auth_last_company(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<LastCompany>,
) -> Result<StatusCode, ApiError> {
    crate::auth::remember_last_company(&state, &headers, &req.company_id).await?;
    Ok(StatusCode::OK)
}

/// Where the subscription sign-in stands (M23). Polled by the interface.
async fn claude_auth_status(State(state): State<AppState>) -> Json<Value> {
    Json(crate::claude_auth::status(&state).await)
}

async fn claude_auth_start(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, ApiError> {
    // Billing is the owner's: connecting or replacing the subscription
    // changes who pays, and that is not a member's call (M24 roles).
    crate::auth::require_owner(&state, &headers).await?;
    crate::claude_auth::start(&state).map_err(ApiError::Invalid)?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(serde::Deserialize)]
struct AuthCode {
    code: String,
}

async fn claude_auth_code(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<AuthCode>,
) -> Result<StatusCode, ApiError> {
    crate::auth::require_owner(&state, &headers).await?;
    if req.code.trim().is_empty() {
        return Err(ApiError::Invalid("the code must not be empty".into()));
    }
    crate::claude_auth::submit_code(&req.code).map_err(ApiError::Invalid)?;
    Ok(StatusCode::ACCEPTED)
}

/// Whether organizational memory (Wadachi/MCP) is wired up at all — for the UI
/// badge. Server-wide: what a *given* company's brain is doing is
/// `GET /companies/{id}/brain` (ADR-0024).
#[derive(Deserialize)]
struct PayWith {
    /// `plan` or `detected` (ADR-0037).
    with: String,
}

/// Let the plan pay — or go back to whatever the probe finds (ADR-0037).
///
/// Choosing the plan keeps `ANTHROPIC_API_KEY` out of every command that runs
/// as the agent, then asks the CLI again who pays. If the answer is still a
/// key — it lives somewhere the environment does not reach, a settings file
/// or an `apiKeyHelper` — the choice is withdrawn and the request refused:
/// a setting that disagrees with who is billed is a setting that will cost
/// someone money, and ADR-0030 exists to prevent that.
async fn pay_with(
    State(state): State<AppState>,
    Json(req): Json<PayWith>,
) -> Result<Json<Value>, ApiError> {
    let plan = match req.with.as_str() {
        "plan" => true,
        "detected" => false,
        other => {
            return Err(ApiError::Invalid(format!(
                "pay with \"plan\" or \"detected\", not \"{other}\""
            )));
        }
    };
    crate::economy::prefer_plan(&state.config, plan).map_err(|e| ApiError::Internal(e.into()))?;
    let economy = crate::economy::detect(&state.config).await;
    if plan && economy.is_metered() {
        crate::economy::prefer_plan(&state.config, false)
            .map_err(|e| ApiError::Internal(e.into()))?;
        return Err(ApiError::Conflict(
            "the key still pays: it is not coming from the environment Overmind controls \
             (a settings file or an apiKeyHelper, most likely), so the plan cannot be made to pay from here"
                .into(),
        ));
    }
    state.set_economy(economy.clone());
    eprintln!(
        "economy: chosen pay_with={} — now paying with {}",
        crate::economy::pay_with_slug(&state.config),
        crate::economy::describe(&economy)
    );
    Ok(Json(json!({
        "economy": crate::economy::as_json(&economy),
        "pay_with": crate::economy::pay_with_slug(&state.config),
    })))
}

/// No `Debug`: the one field is a passphrase, and a `{:?}` in a future log
/// line is all it would take.
#[derive(Default, Deserialize)]
struct BackupRequest {
    #[serde(default)]
    passphrase: Option<String>,
}

/// Export, list and download are the owner's, and only once an owner exists:
/// an unclaimed instance has companies and possibly a sign-in already, and
/// nobody to answer for handing them out.
async fn require_claimed_owner(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<(), ApiError> {
    let anyone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
    if anyone == 0 {
        return Err(ApiError::Conflict(
            "this instance has no owner yet: claim it first, then export".into(),
        ));
    }
    crate::auth::require_owner(state, headers).await
}

async fn create_backup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Option<Json<BackupRequest>>,
) -> Result<Json<Value>, ApiError> {
    require_claimed_owner(&state, &headers).await?;
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let report = crate::backup::export(&state, req.passphrase.as_deref())
        .await
        .map_err(|e| match e {
            crate::backup::ExportError::PassphraseRequired
            | crate::backup::ExportError::PassphraseTooShort
            | crate::backup::ExportError::Config(_) => ApiError::Invalid(e.to_string()),
            other => {
                // Paths and SQLite's words, never a credential: the export
                // handles the token only through the seal.
                eprintln!("backup: export failed: {other}");
                ApiError::Internal(other.into())
            }
        })?;
    eprintln!(
        "backup: exported {} ({} bytes, chain {} events{})",
        report.name,
        report.bytes,
        report.chain.events_checked,
        if report.sealed_token {
            ", token sealed"
        } else {
            ""
        }
    );
    Ok(Json(serde_json::to_value(report)?))
}

async fn list_backups(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_claimed_owner(&state, &headers).await?;
    let archives = crate::backup::list(&state.config).map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(json!({
        "archives": archives,
        // Whether an export will need a passphrase, so the interface can ask
        // for one only when there is a sign-in to seal -- and say why.
        "sign_in_travels": crate::claude_auth::stored_token(&state.config).is_some(),
    })))
}

/// Delete one archive. The owner's verb, and audited: an archive is the whole
/// instance, and "where did last month's backup go" deserves an answer.
async fn delete_backup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_claimed_owner(&state, &headers).await?;
    let listed = crate::backup::list(&state.config)
        .map_err(|e| ApiError::Internal(e.into()))?
        .iter()
        .any(|a| a["name"].as_str() == Some(name.as_str()));
    if !crate::backup::is_archive_name(&name) || !listed {
        return Err(ApiError::NotFound("archive"));
    }
    let path = crate::backup::backup_dir(&state.config).join(&name);
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    std::fs::remove_file(&path).map_err(|e| ApiError::Internal(e.into()))?;
    let mut conn = state.pool.acquire().await?;
    crate::audit::append(
        &mut conn,
        None,
        None,
        "backup.deleted",
        &json!({ "name": name, "bytes": bytes }),
    )
    .await?;
    Ok(Json(json!({ "deleted": name })))
}

/// Streamed from the folder; `name` must be a bare entry of it -- the first
/// user-supplied file name this API resolves, so the rule is strict.
async fn download_backup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ApiError> {
    require_claimed_owner(&state, &headers).await?;
    let listed = crate::backup::list(&state.config)
        .map_err(|e| ApiError::Internal(e.into()))?
        .iter()
        .any(|a| a["name"].as_str() == Some(name.as_str()));
    if !crate::backup::is_archive_name(&name) || !listed {
        return Err(ApiError::NotFound("archive"));
    }
    let path = crate::backup::backup_dir(&state.config).join(&name);
    let mut response = tower_http::services::ServeFile::new_with_mime(
        &path,
        &"application/gzip"
            .parse()
            .map_err(|_| ApiError::NotFound("archive"))?,
    )
    .try_call(request)
    .await
    .map_err(|_| ApiError::NotFound("archive file"))?
    .into_response();
    if let Ok(value) = format!("attachment; filename=\"{name}\"").parse() {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

/// Take an archive, check it, stage it, and ask to be restarted.
///
/// No credential guards this one: an instance with no owner, no company and
/// no sign-in is open by design (ADR-0032) -- whoever can reach the port can
/// claim it, and a restore is a claim with a payload. The moment any of the
/// three exists, it answers 409 instead.
async fn restore_instance(
    State(state): State<AppState>,
    mut form: Multipart,
) -> Result<Json<Value>, ApiError> {
    if let Some(reason) = crate::backup::what_makes_the_instance_full(&state).await {
        return Err(ApiError::Conflict(
            crate::backup::RestoreError::NotEmpty(reason).to_string(),
        ));
    }
    let upload = state
        .config
        .data_dir
        .join(format!("upload-{}.tar.gz", new_id()));
    let mut passphrase: Option<String> = None;
    let mut skip_token = false;
    let mut arrived = false;

    let taken = take_upload(
        &mut form,
        &upload,
        &state.config,
        &mut passphrase,
        &mut skip_token,
        &mut arrived,
    )
    .await;
    if let Err(e) = taken {
        let _ = tokio::fs::remove_file(&upload).await;
        return Err(e);
    }
    if !arrived {
        let _ = tokio::fs::remove_file(&upload).await;
        return Err(ApiError::Invalid(
            "no archive came with the request: send the `.tar.gz` as the `archive` field".into(),
        ));
    }

    let outcome = crate::backup::restore(&state, &upload, passphrase.as_deref(), skip_token).await;
    let _ = tokio::fs::remove_file(&upload).await;
    let report = outcome.map_err(|e| match e {
        crate::backup::RestoreError::NotEmpty(_) => ApiError::Conflict(e.to_string()),
        crate::backup::RestoreError::Refused(_) => ApiError::Invalid(e.to_string()),
        other => {
            eprintln!("restore: {other}");
            ApiError::Internal(other.into())
        }
    })?;

    // The swap happens at the next boot; ask for one. In the image the
    // restart policy is the supervisor; natively the person starts it again,
    // and the answer says so.
    let _ = state.restart.send(());
    Ok(Json(serde_json::to_value(report)?))
}

/// Read the multipart form, streaming the archive to disk rather than into
/// memory: this upload is the size of somebody's whole instance.
async fn take_upload(
    form: &mut Multipart,
    upload: &std::path::Path,
    config: &crate::db::Config,
    passphrase: &mut Option<String>,
    skip_token: &mut bool,
    arrived: &mut bool,
) -> Result<(), ApiError> {
    use tokio::io::AsyncWriteExt;
    // A restore is a claim with a payload (ADR-0044), so it costs the same
    // code (ADR-0045). `None` is already right when the instance has no code
    // waiting -- one that was claimed before this existed, or claimed at all.
    let mut code_ok = crate::auth::setup_code_ok(config, None);
    while let Some(mut field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::Invalid(format!("the upload could not be read: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "passphrase" => {
                *passphrase = field.text().await.ok().filter(|t| !t.trim().is_empty());
            }
            "skip_token" => {
                let said = field.text().await.unwrap_or_default();
                *skip_token = matches!(said.trim(), "true" | "1" | "on" | "yes");
            }
            "setup" => {
                let given = field.text().await.unwrap_or_default();
                code_ok = crate::auth::setup_code_ok(config, Some(&given));
            }
            "archive" => {
                // Before a single byte reaches the disk. An archive is the
                // size of somebody's whole instance, and a stranger must not
                // be able to make this machine write one -- so the code is
                // demanded here rather than after the upload, and a client
                // that sends the archive first is refused without it.
                // Wordless 401, like the claim's: a missing credential is not
                // a malformed request, and this is the same credential. The
                // ordering requirement -- `setup` before `archive` -- is in
                // ADR-0045 and the threat model, where a client author reads
                // it, rather than in an answer to somebody who has not proved
                // they may ask.
                if !code_ok {
                    return Err(ApiError::Unauthorized);
                }
                let mut file = tokio::fs::File::create(upload)
                    .await
                    .map_err(|e| ApiError::Internal(e.into()))?;
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| ApiError::Invalid(format!("the archive stopped arriving: {e}")))?
                {
                    file.write_all(&chunk)
                        .await
                        .map_err(|e| ApiError::Internal(e.into()))?;
                }
                file.flush()
                    .await
                    .map_err(|e| ApiError::Internal(e.into()))?;
                *arrived = true;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn memory_status(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "enabled": state.memory.is_enabled(),
        "managed": state.config.managed_brain && state.memory.is_enabled(),
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    /// Refused by a governance policy (e.g. over budget). Maps to 402.
    #[error("{0}")]
    Blocked(String),
    /// No valid session (M24). Deliberately wordless beyond the status:
    /// which part of the credential was wrong is not the caller's to learn.
    /// Refused with a repair Overmind can apply (ADR-0038 addendum): 409,
    /// and the body carries the machine-readable `remedy` beside the message.
    #[error("{message}")]
    Remediable { message: String, remedy: Value },
    #[error("unauthorized")]
    Unauthorized,
    /// A valid session without the standing (M24): today, a member touching
    /// billing. 403, not 401 -- who you are was never in question.
    #[error("this action is the owner's")]
    Forbidden,
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<crate::runner::RunnerError> for ApiError {
    fn from(e: crate::runner::RunnerError) -> Self {
        use crate::runner::RunnerError;
        match e {
            RunnerError::NotFound(what) => ApiError::NotFound(what),
            RunnerError::Invalid(msg) => ApiError::Invalid(msg),
            RunnerError::Conflict => {
                ApiError::Conflict("task is not available for checkout".into())
            }
            RunnerError::Blocked(msg) => ApiError::Conflict(msg),
            RunnerError::Remediable { message, remedy } => ApiError::Remediable { message, remedy },
            RunnerError::OverBudget {
                limit_cents,
                observed_cents,
            } => ApiError::Blocked(format!(
                "agent is over its monthly budget: {observed_cents} of {limit_cents} cents"
            )),
            RunnerError::Git(msg) => ApiError::Internal(msg.into()),
            RunnerError::Db(e) => ApiError::Internal(Box::new(e)),
        }
    }
}

impl From<crate::ceo::CeoError> for ApiError {
    fn from(e: crate::ceo::CeoError) -> Self {
        use crate::ceo::CeoError;
        match e {
            CeoError::NotFound(what) => ApiError::NotFound(what),
            CeoError::Invalid(msg) => ApiError::Invalid(msg),
            // Over budget is a conflict with the world's current state, not a
            // malformed request — the same shape as a refused task checkout.
            CeoError::OverBudget(check) => ApiError::Conflict(format!(
                "monthly budget reached: {} of {} spent",
                crate::governance::euros(check.spent + check.reserved),
                crate::governance::euros(check.cap),
            )),
            // Also a conflict with the world, and also transient — but with a
            // different remedy, so it says which one it is rather than letting
            // a reader assume there is a cap to raise (ADR-0030).
            CeoError::PlanExhausted(window) => ApiError::Conflict(format!(
                "the subscription has run out for its {} window; it resets at {}",
                window.window.replace('_', "-"),
                crate::economy::reset_time(&window),
            )),
            CeoError::Db(e) => ApiError::Internal(Box::new(e)),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(Box::new(e))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::Internal(Box::new(e))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Invalid(_) => StatusCode::BAD_REQUEST,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Remediable { .. } => StatusCode::CONFLICT,
            ApiError::Blocked(_) => StatusCode::PAYMENT_REQUIRED,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::Internal(source) => {
                // The client gets an opaque error; the operator gets the cause.
                eprintln!("internal error: {source}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let mut body = json!({ "error": self.to_string() });
        if let ApiError::Remediable { remedy, .. } = &self {
            body["remedy"] = remedy.clone();
        }
        (status, Json(body)).into_response()
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Deserialize an optional field that also distinguishes explicit `null` from
/// absence: absent → `None` (leave unchanged), `null` → `Some(None)` (clear),
/// value → `Some(Some(v))`.
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        // How this Overmind pays (ADR-0030). A machine-level fact rather than a
        // company one, so it belongs here and not on the budget: two companies
        // on one server cannot be paying different ways. The client reads it
        // once and words the budget accordingly — a cap in dollars promises
        // something under a key that it cannot promise under a plan.
        "economy": crate::economy::as_json(&state.economy()),
        // Whether the person chose who pays (ADR-0037): `plan` when they asked
        // the plan to, `detected` when the economy is whatever the probe found.
        "pay_with": crate::economy::pay_with_slug(&state.config),
        // Where each of the plan's windows stands, as last reported. Empty
        // under an API key, where windows do not apply, and empty before the
        // first run says anything — a window we have not heard about is absent
        // rather than assumed healthy.
        "plan_windows": state
            .plan_windows()
            .iter()
            .map(|(k, w)| (k.clone(), crate::economy::window_as_json(w)))
            .collect::<serde_json::Map<_, _>>(),
    }))
}

// ---------- companies ----------

#[derive(Deserialize)]
struct CreateCompany {
    name: String,
    /// The language this company works in (M16). Optional, and validated the
    /// same way `set_language` validates it.
    ///
    /// It belongs here and not only on the settings endpoint because the very
    /// next thing that happens is a CEO speaking (M15): a company founded
    /// without a language has already answered its first question in the wrong
    /// one by the time you find the setting. Until this existed the field was
    /// simply *dropped* — a request saying `"language": "it"` was accepted,
    /// stored as English, and nothing said otherwise.
    language: Option<String>,
}

async fn create_company(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateCompany>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::Invalid("company name must not be empty".into()));
    }
    // Who is founding, asked BEFORE the write transaction opens: the session
    // lookup slides the expiry with a write of its own, on another pool
    // connection -- inside our IMMEDIATE transaction that write would wait on
    // the very lock we hold (measured: every founding took the full wait).
    let founder = crate::auth::session_identity(&state, &headers).await;
    let language = req.language.as_deref().unwrap_or(crate::i18n::DEFAULT);
    if !crate::i18n::is_supported(language) {
        return Err(ApiError::Invalid(format!(
            "unsupported language `{language}`"
        )));
    }
    let (id, created_at) = (new_id(), now());
    let mut tx = state.write_tx().await?;
    sqlx::query("INSERT INTO companies (id, name, language, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(req.name.trim())
        .bind(language)
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&id),
        None,
        event_kind::COMPANY_CREATED,
        &json!({ "name": req.name.trim(), "language": language }),
    )
    .await?;

    // A company is never empty (M15). It is founded with a CEO — the org
    // leader (`reports_to` NULL, ADR-0019), on the strongest model, allowed to
    // take on anything within its budget. You can talk to it from the first
    // second and have it build the rest of the team, or ignore it and hire
    // everyone yourself.
    let ceo = hire(
        &mut tx,
        &state.config.agent_tools,
        &id,
        &HireAgent {
            name: crate::db::random_ceo_name().to_string(),
            archetype: crate::db::CEO_ARCHETYPE.to_string(),
            // The leader is not a specialist in any one field (ADR-0021).
            domain: None,
            traits: TraitsPatch::default(),
            custom_brief: None,
            title: Some("CEO".to_string()),
            reports_to: None,
        },
    )
    .await?;

    // Whoever founds a company is inside it (M25). Before any user exists
    // (an unclaimed instance) there is nobody to enrol, and the migration's
    // backfill rule applies when accounts arrive.
    if let Some((user_id, _, _)) = founder {
        sqlx::query(
            "INSERT OR IGNORE INTO company_members (company_id, user_id, added_at) VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(&user_id)
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    // "One click and a company has a brain" (ADR-0004), and the click is this
    // one. `memory_for` would create the directory lazily at the first memory
    // call anyway — doing it here is what makes the brain something you can go
    // and look at from the moment the company exists, rather than something
    // that appears once an agent happens to remember something.
    if state.config.managed_brain && state.memory.is_enabled() {
        let dir = state.brain_dir(&id);
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            // Not fatal: memory is best-effort, and a company without a brain
            // is a company that works (ADR-0003).
            eprintln!(
                "could not provision brain at {} (ignored): {e}",
                dir.display()
            );
        }
    }

    // The brain's first memory: who this company is (M21). Before this, a
    // fresh brain was empty and `get_context` had nothing true to say — so an
    // agent asked to "write about the company" reached for world knowledge,
    // and M19's acceptance run got a confident document about somebody else's
    // product of the same name. Best-effort like every memory call, and a
    // no-op when memory is off.
    if state.memory.is_enabled() {
        let name = req.name.trim();
        let language_name = crate::i18n::SUPPORTED
            .iter()
            .find(|(code, _)| *code == language)
            .map(|(_, n)| *n)
            .unwrap_or(language);
        let ceo_name = ceo.get("name").and_then(Value::as_str).unwrap_or("the CEO");
        state
            .memory_for(&id)
            .await
            .store_memory(
                &format!("Who {name} is"),
                &format!(
                    "This company is {name}, founded on {created_at}. It works in \
                     {language_name}. Its CEO is {ceo_name}. When a task or a \
                     conversation says \"the company\", it means {name} — not any \
                     other organization or product with a similar name."
                ),
                &id,
                &["founding", "identity"],
                "context",
                None,
            )
            .await;
    }

    state.notify(&id);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "name": req.name.trim(),
            "language": language,
            "created_at": created_at,
            "brain_enabled": true,
            "ceo": ceo,
        })),
    ))
}

/// Delete a company: the rows, the brain, the debris on disk (ADR-0034).
///
/// A hard delete, deliberately — the ROADMAP names the wound this heals:
/// the owner's first real test found no way to remove a company, and
/// cleaning up meant surgery on the volume. The audit chain is the one
/// thing that stays: `audit_events` carries no foreign key and its
/// append-only triggers would abort any thinning, so history keeps saying
/// this company existed — and the newest event says it was deleted, which
/// is what separates a deletion from corruption.
///
/// The foreign keys are left ON as a net: the deletes walk children-first,
/// and a table this function forgot fails the whole transaction instead of
/// leaving orphans behind.
async fn delete_company(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM companies WHERE id = ?")
        .bind(&company_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((name,)) = name else {
        return Err(ApiError::NotFound("company"));
    };

    // A queued or running session is an agent mid-thought: deleting the
    // ground under it would leave the runner finalizing into missing rows.
    // The door holds until the work settles (or is terminated).
    let (live,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM agent_task_sessions s
         JOIN tasks t ON t.id = s.task_id
         WHERE t.company_id = ? AND s.status IN ('queued', 'running')",
    )
    .bind(&company_id)
    .fetch_one(&mut *tx)
    .await?;
    if live > 0 {
        return Err(ApiError::Conflict(format!(
            "{live} session(s) still queued or running; wait for them to finish before deleting"
        )));
    }

    // The directories to sweep afterwards are named by row ids, so the ids
    // must be read before the rows go.
    let ids = |rows: Vec<(String,)>| rows.into_iter().map(|(id,)| id).collect::<Vec<_>>();
    let task_ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM tasks WHERE company_id = ?")
        .bind(&company_id)
        .fetch_all(&mut *tx)
        .await?;
    let conversation_ids: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM conversations WHERE company_id = ?")
            .bind(&company_id)
            .fetch_all(&mut *tx)
            .await?;
    let session_ids: Vec<(String,)> = sqlx::query_as(
        "SELECT s.id FROM agent_task_sessions s
         JOIN tasks t ON t.id = s.task_id WHERE t.company_id = ?",
    )
    .bind(&company_id)
    .fetch_all(&mut *tx)
    .await?;
    let meeting_ids: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM meetings WHERE company_id = ?")
            .bind(&company_id)
            .fetch_all(&mut *tx)
            .await?;
    let (task_ids, conversation_ids, session_ids, meeting_ids) = (
        ids(task_ids),
        ids(conversation_ids),
        ids(session_ids),
        ids(meeting_ids),
    );

    // Children first, each level before the one it points at. The two
    // self-references (agents.reports_to, roles.reports_to) are cut before
    // their tables go, because a single DELETE promises nothing about the
    // order it visits rows in.
    for statement in [
        "DELETE FROM task_artifacts WHERE task_id IN (SELECT id FROM tasks WHERE company_id = ?)",
        "DELETE FROM attachments WHERE task_id IN (SELECT id FROM tasks WHERE company_id = ?)
             OR conversation_id IN (SELECT id FROM conversations WHERE company_id = ?)",
        "DELETE FROM messages WHERE conversation_id IN
             (SELECT id FROM conversations WHERE company_id = ?)",
        // Born after the original list (measured 27 Aug 2026, a FK failure):
        // the thread's compaction summaries (ADR-0040)…
        "DELETE FROM conversation_summaries WHERE conversation_id IN
             (SELECT id FROM conversations WHERE company_id = ?)",
        // …and the two references the tasks themselves carry — the birth
        // thread (ADR-0038) and the dependency (M30). Cut before either
        // parent table goes: a single DELETE promises nothing about order.
        "UPDATE tasks SET conversation_id = NULL, depends_on = NULL WHERE company_id = ?",
        "DELETE FROM cost_events WHERE company_id = ?",
        "DELETE FROM agent_task_sessions WHERE task_id IN
             (SELECT id FROM tasks WHERE company_id = ?)",
        "DELETE FROM meeting_turns WHERE meeting_id IN
             (SELECT id FROM meetings WHERE company_id = ?)",
        "DELETE FROM meeting_participants WHERE meeting_id IN
             (SELECT id FROM meetings WHERE company_id = ?)",
        "DELETE FROM org_proposal_members WHERE proposal_id IN
             (SELECT id FROM org_proposals WHERE company_id = ?)",
        "DELETE FROM agent_wakeup_requests WHERE agent_id IN
             (SELECT id FROM agents WHERE company_id = ?)",
        "DELETE FROM agent_turn_reservations WHERE company_id = ?",
        "DELETE FROM budget_incidents WHERE company_id = ?",
        "DELETE FROM agent_config_revisions WHERE company_id = ?",
        "DELETE FROM notifications WHERE company_id = ?",
        "DELETE FROM meetings WHERE company_id = ?",
        "DELETE FROM org_proposals WHERE company_id = ?",
        "DELETE FROM approvals WHERE company_id = ?",
        "DELETE FROM conversations WHERE company_id = ?",
        "DELETE FROM tasks WHERE company_id = ?",
        "DELETE FROM goals WHERE project_id IN (SELECT id FROM projects WHERE company_id = ?)",
        "DELETE FROM project_workspaces WHERE project_id IN
             (SELECT id FROM projects WHERE company_id = ?)",
        "DELETE FROM projects WHERE company_id = ?",
        "UPDATE agents SET reports_to = NULL WHERE company_id = ?",
        "DELETE FROM agents WHERE company_id = ?",
        "DELETE FROM memory_links WHERE company_id = ?",
        // company_tokens and company_members cascade with the row.
        "DELETE FROM companies WHERE id = ?",
    ] {
        let mut query = sqlx::query(statement).bind(&company_id);
        if statement.matches('?').count() == 2 {
            query = query.bind(&company_id);
        }
        query.execute(&mut *tx).await?;
    }

    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::COMPANY_DELETED,
        &json!({ "name": name }),
    )
    .await?;
    tx.commit().await?;

    // The brain's server processes die before their directory does: a live
    // MCP pool holds open handles into the tree about to be removed.
    state.forget_brain(&company_id);
    let mut debris = vec![state.config.data_dir.join("companies").join(&company_id)];
    for task_id in &task_ids {
        debris.push(state.config.data_dir.join("attachments").join(task_id));
    }
    for conversation_id in &conversation_ids {
        debris.push(
            state
                .config
                .data_dir
                .join("attachments")
                .join(conversation_id),
        );
    }
    for session_id in &session_ids {
        for root in ["artifacts", "worktrees", "sessions"] {
            debris.push(state.config.data_dir.join(root).join(session_id));
        }
    }
    for meeting_id in &meeting_ids {
        debris.push(state.config.data_dir.join("meetings").join(meeting_id));
    }
    for dir in debris {
        // Best-effort like every filesystem sweep here: the rows are already
        // gone, and a directory that would not delete is disk to reclaim by
        // hand, not a reason to claim the company still exists.
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("could not sweep {} (ignored): {e}", dir.display());
        }
    }

    state.notify(&company_id);
    Ok(Json(json!({ "ok": true })))
}

async fn list_companies(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    // Membership is the filter (M25, ADR-0033): you see your companies.
    // The instance owner sees all -- the owner is the administrator, and
    // pretending otherwise on a machine they control would be theater. An
    // unclaimed instance has nobody to filter for.
    let rows: Vec<(String, String, String, String, i64)> =
        match crate::auth::session_identity(&state, &headers).await {
            Some((_, _, role)) if role == "owner" => {
                sqlx::query_as(
                    "SELECT id, name, language, created_at, brain_enabled \
                     FROM companies ORDER BY created_at",
                )
                .fetch_all(&state.pool)
                .await?
            }
            Some((user_id, _, _)) => {
                sqlx::query_as(
                    "SELECT c.id, c.name, c.language, c.created_at, c.brain_enabled \
                     FROM companies c JOIN company_members m ON m.company_id = c.id \
                     WHERE m.user_id = ? ORDER BY c.created_at",
                )
                .bind(&user_id)
                .fetch_all(&state.pool)
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT id, name, language, created_at, brain_enabled \
                     FROM companies ORDER BY created_at",
                )
                .fetch_all(&state.pool)
                .await?
            }
        };
    let companies: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, language, created_at, brain)| {
            json!({
                "id": id,
                "name": name,
                "language": language,
                "created_at": created_at,
                "brain_enabled": brain != 0,
            })
        })
        .collect();
    Ok(Json(json!({ "companies": companies })))
}

// ---------- archetypes ----------

async fn list_archetypes(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, slug, name, description, default_traits FROM archetypes ORDER BY slug",
    )
    .fetch_all(&state.pool)
    .await?;
    let archetypes = rows
        .into_iter()
        .map(|(id, slug, name, description, traits)| {
            let traits: Value = serde_json::from_str(&traits)?;
            Ok(json!({
                "id": id,
                "slug": slug,
                "name": name,
                "description": description,
                "default_traits": traits,
            }))
        })
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(Json(json!({ "archetypes": archetypes })))
}

/// The domain catalog — the second axis of characterization (ADR-0021).
async fn list_domains(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, slug, name, description, traits_patch FROM domains ORDER BY slug",
    )
    .fetch_all(&state.pool)
    .await?;
    let domains = rows
        .into_iter()
        .map(|(id, slug, name, description, patch)| {
            let patch: Value = serde_json::from_str(&patch)?;
            Ok(json!({
                "id": id,
                "slug": slug,
                "name": name,
                "description": description,
                "traits_patch": patch,
            }))
        })
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(Json(json!({ "domains": domains })))
}

/// The models an agent may run on. The hire dialog offered three strings that
/// were not model identifiers at all until ADR-0021; it now reads this.
async fn list_models() -> Json<Value> {
    Json(json!({ "models": crate::model::catalog() }))
}

/// The operator's tool registry, by name (ADR-0036): the name, the command
/// it runs (so a person can tell what they are granting), and our one-line
/// description. Empty is the ordinary case and not an error.
async fn list_tools(State(state): State<AppState>) -> Json<Value> {
    let reg = &state.config.agent_tools;
    let tools: Vec<Value> = reg
        .servers
        .iter()
        .map(|(name, def)| {
            let command = def
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| def.get("url").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            json!({
                "name": name,
                "command": command,
                "description": reg.description(name),
                // One hand at a time (the registry's "exclusive" list): the
                // interface says it, the server enforces it.
                "exclusive": reg.is_exclusive(name),
            })
        })
        .collect();
    Json(json!({ "tools": tools }))
}

/// Refuse a characterization the server cannot honour, at the boundary where it
/// enters rather than at the prompt where it would finally break — the rule
/// M16 already applies to language codes (ADR-0021).
/// An exclusive tool fits one hand: refuse the grant when another active
/// agent in the company already holds it, naming the holder (ADR-0036).
async fn refuse_second_hand(
    tx: &mut sqlx::SqliteConnection,
    tools: &crate::db::AgentTools,
    company_id: &str,
    this_agent: Option<&str>,
    granted: &[String],
) -> Result<(), ApiError> {
    for name in granted {
        if !tools.is_exclusive(name) {
            continue;
        }
        let holder: Option<(String,)> = sqlx::query_as(
            "SELECT a.name FROM agents a
             WHERE a.company_id = ? AND a.status = 'active' AND a.id != COALESCE(?, '')
               AND EXISTS (SELECT 1 FROM json_each(json_extract(a.traits, '$.tools'))
                           WHERE json_each.value = ?)",
        )
        .bind(company_id)
        .bind(this_agent)
        .bind(name)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((who,)) = holder {
            return Err(ApiError::Conflict(format!(
                "the tool `{name}` is exclusive and {who} already holds it — take it out of their hands first"
            )));
        }
    }
    Ok(())
}

fn validate_traits(tools: &crate::db::AgentTools, traits: &AgentTraits) -> Result<(), ApiError> {
    // A tool grant names something the operator declared, or it is refused
    // here (ADR-0036) -- never stored and handed to a run later.
    if let Some(unknown) = traits.tools.iter().find(|t| !tools.contains(t)) {
        return Err(ApiError::Invalid(format!(
            "unknown tool `{unknown}`: not declared in OVERMIND_AGENT_TOOLS"
        )));
    }
    if !crate::model::is_known(&traits.model) {
        return Err(ApiError::Invalid(format!(
            "unknown model `{}`",
            traits.model
        )));
    }
    if traits.multimodal && !crate::model::supports_vision(&traits.model) {
        return Err(ApiError::Invalid(format!(
            "`{}` cannot read images, so this agent cannot be characterized as multimodal",
            traits.model
        )));
    }
    Ok(())
}

// ---------- agents ----------

#[derive(Deserialize)]
pub(crate) struct HireAgent {
    pub name: String,
    /// Archetype slug — the *function* (UX Level 1 "pick").
    pub archetype: String,
    /// Domain slug — the *field* the function is applied in (ADR-0021).
    /// `None` means the general domain, which adds nothing.
    pub domain: Option<String>,
    /// Structured overrides on the composed defaults (UX Level 2 "tune").
    #[serde(default)]
    pub traits: TraitsPatch,
    /// Free-form additions (UX Level 3 "expert") — additive only.
    pub custom_brief: Option<String>,
    /// Free-text job title, e.g. "Senior Backend Engineer" (org chart).
    pub title: Option<String>,
    /// The agent this one reports to (must be an agent in the same company).
    /// `None` means "under the org leader" — an org has exactly one root.
    pub reports_to: Option<String>,
}

async fn hire_agent(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<HireAgent>,
) -> Result<impl IntoResponse, ApiError> {
    let mut tx = state.write_tx().await?;
    let company: Option<(String,)> = sqlx::query_as("SELECT id FROM companies WHERE id = ?")
        .bind(&company_id)
        .fetch_optional(&mut *tx)
        .await?;
    if company.is_none() {
        return Err(ApiError::NotFound("company"));
    }
    let hired = hire(&mut tx, &state.config.agent_tools, &company_id, &req).await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok((StatusCode::CREATED, Json(hired)))
}

/// Hire one agent inside an open transaction, and return the row as the API
/// shapes it. Shared by the endpoint, the founding CEO (M15) and — later — the
/// CEO's proposed organization, so all three produce identical records:
/// archetype defaults + patch, first config revision, audit event.
pub(crate) async fn hire(
    tx: &mut sqlx::SqliteConnection,
    tools: &crate::db::AgentTools,
    company_id: &str,
    req: &HireAgent,
) -> Result<Value, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::Invalid("agent name must not be empty".into()));
    }
    let archetype: Option<(String, String)> =
        sqlx::query_as("SELECT id, default_traits FROM archetypes WHERE slug = ?")
            .bind(&req.archetype)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((archetype_id, default_traits)) = archetype else {
        return Err(ApiError::NotFound("archetype"));
    };

    // The field the function is applied in (ADR-0021). Unknown is a 404 for
    // the same reason an unknown archetype is: a characterization we cannot
    // resolve is worse than a refused hire.
    let domain_slug = req.domain.as_deref().unwrap_or(crate::db::GENERAL_DOMAIN);
    let domain: Option<(String, String)> =
        sqlx::query_as("SELECT id, traits_patch FROM domains WHERE slug = ?")
            .bind(domain_slug)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((domain_id, domain_patch)) = domain else {
        return Err(ApiError::NotFound("domain"));
    };
    let domain_patch: DomainPatch = serde_json::from_str(&domain_patch).unwrap_or_default();

    // Most general to most specific: the function's defaults, what the field
    // adds, then what you tuned by hand.
    let defaults: AgentTraits = serde_json::from_str(&default_traits)?;
    let traits = defaults
        .with_domain(&domain_patch)
        .apply(req.traits.clone());
    validate_traits(tools, &traits)?;
    refuse_second_hand(tx, tools, company_id, None, &traits.tools).await?;
    let traits_json = serde_json::to_string(&traits)?;

    // A manager, if given, must be an existing agent in this company.
    // If none is given, the new hire reports to the org leader — an
    // organization has one root (ADR-0019 reads the leader as `reports_to IS
    // NULL`), so defaulting to "no manager" would quietly create a second one.
    // Only the founding CEO itself is allowed to have no manager.
    let reports_to: Option<String> = match &req.reports_to {
        Some(mgr) => {
            let ok: Option<(String,)> =
                sqlx::query_as("SELECT id FROM agents WHERE id = ? AND company_id = ?")
                    .bind(mgr)
                    .bind(company_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if ok.is_none() {
                return Err(ApiError::NotFound("manager agent"));
            }
            Some(mgr.clone())
        }
        None => sqlx::query_as::<_, (String,)>(
            "SELECT id FROM agents
             WHERE company_id = ? AND status = 'active' AND reports_to IS NULL
             ORDER BY created_at LIMIT 1",
        )
        .bind(company_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(|(id,)| id),
    };
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO agents (id, company_id, archetype_id, domain_id, name, traits, custom_brief, title, reports_to, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?)",
    )
    .bind(&id)
    .bind(company_id)
    .bind(&archetype_id)
    .bind(&domain_id)
    .bind(req.name.trim())
    .bind(&traits_json)
    .bind(&req.custom_brief)
    .bind(title)
    .bind(&reports_to)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    // First config revision: from nothing to the hired configuration.
    let snapshot = crate::governance::agent_snapshot(
        req.name.trim(),
        title,
        reports_to.as_deref(),
        &serde_json::to_value(&traits)?,
        req.custom_brief.as_deref(),
        false,
    );
    crate::governance::record_revision(tx, company_id, &id, "hire", &json!({}), &snapshot).await?;
    audit::append(
        tx,
        Some(company_id),
        None,
        event_kind::AGENT_HIRED,
        &json!({
            "agent_id": id,
            "name": req.name.trim(),
            "archetype": req.archetype,
            "domain": domain_slug,
            "title": title,
            "reports_to": reports_to,
            "traits": serde_json::to_value(&traits)?,
        }),
    )
    .await?;
    Ok(json!({
            "id": id,
            "company_id": company_id,
            "name": req.name.trim(),
            "archetype": req.archetype,
            "domain": domain_slug,
            "traits": serde_json::to_value(&traits)?,
            "custom_brief": req.custom_brief,
            "title": title,
            "reports_to": reports_to,
            "status": "active",
            "created_at": created_at,
    }))
}

/// (id, name, traits, custom_brief, status, title, reports_to, requires_approval, archetype slug)
type AgentRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    i64,
    String,
    Option<String>,
);

async fn list_agents(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<AgentRow> = sqlx::query_as(
        "SELECT a.id, a.name, a.traits, a.custom_brief, a.status, a.title, a.reports_to, a.requires_approval, ar.slug, d.slug
         FROM agents a
         JOIN archetypes ar ON ar.id = a.archetype_id
         LEFT JOIN domains d ON d.id = a.domain_id
         WHERE a.company_id = ? ORDER BY a.created_at",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let agents = rows
        .into_iter()
        .map(
            |(
                id,
                name,
                traits,
                custom_brief,
                status,
                title,
                reports_to,
                requires_approval,
                archetype,
                domain,
            )| {
                let traits: Value = serde_json::from_str(&traits)?;
                Ok(json!({
                    "id": id,
                    "name": name,
                    "archetype": archetype,
                    "domain": domain,
                    "traits": traits,
                    "custom_brief": custom_brief,
                    "title": title,
                    "reports_to": reports_to,
                    "requires_approval": requires_approval != 0,
                    "status": status,
                }))
            },
        )
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(Json(json!({ "agents": agents })))
}

#[derive(Deserialize)]
struct ReassignAgent {
    /// New manager agent id, or null to move the agent to the top (reports to
    /// the human owner). Omitted → unchanged.
    #[serde(default, deserialize_with = "double_option")]
    reports_to: Option<Option<String>>,
    title: Option<String>,
}

/// Set an agent's manager and/or title. Enforces the reporting DAG: a manager
/// must be another agent in the same company, and the change must not create
/// a cycle (an agent cannot end up reporting, directly or transitively, to
/// itself).
async fn reassign_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<ReassignAgent>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let Some((company_id, before)) = agent_snapshot_by_id(&mut tx, &agent_id).await? else {
        return Err(ApiError::NotFound("agent"));
    };

    if let Some(new_mgr) = &req.reports_to {
        if let Some(mgr) = new_mgr {
            if mgr == &agent_id {
                return Err(ApiError::Invalid("an agent cannot report to itself".into()));
            }
            let ok: Option<(String,)> =
                sqlx::query_as("SELECT id FROM agents WHERE id = ? AND company_id = ?")
                    .bind(mgr)
                    .bind(&company_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if ok.is_none() {
                return Err(ApiError::NotFound("manager agent"));
            }
            // Walk up from the proposed manager; if we reach this agent, the
            // edge would close a cycle.
            let mut cursor = Some(mgr.clone());
            while let Some(cur) = cursor {
                if cur == agent_id {
                    return Err(ApiError::Invalid(
                        "that change would create a reporting cycle".into(),
                    ));
                }
                let next: Option<(Option<String>,)> =
                    sqlx::query_as("SELECT reports_to FROM agents WHERE id = ?")
                        .bind(&cur)
                        .fetch_optional(&mut *tx)
                        .await?;
                cursor = next.and_then(|(r,)| r);
            }
        }
        sqlx::query("UPDATE agents SET reports_to = ? WHERE id = ?")
            .bind(new_mgr)
            .bind(&agent_id)
            .execute(&mut *tx)
            .await?;
    }

    if let Some(title) = &req.title {
        let title = title.trim();
        sqlx::query("UPDATE agents SET title = ? WHERE id = ?")
            .bind(if title.is_empty() { None } else { Some(title) })
            .bind(&agent_id)
            .execute(&mut *tx)
            .await?;
    }

    // Snapshot the new config as a revision (forward-only history).
    if let Some((_, after)) = agent_snapshot_by_id(&mut tx, &agent_id).await? {
        crate::governance::record_revision(
            &mut tx,
            &company_id,
            &agent_id,
            "patch",
            &before,
            &after,
        )
        .await?;
    }
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::AGENT_REASSIGNED,
        &json!({ "agent_id": agent_id, "reports_to": req.reports_to, "title": req.title }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({ "id": agent_id })))
}

// ---------- projects & goals ----------

#[derive(Deserialize)]
struct CreateProject {
    title: String,
    #[serde(default)]
    description: String,
}

async fn create_project(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<CreateProject>,
) -> Result<impl IntoResponse, ApiError> {
    if req.title.trim().is_empty() {
        return Err(ApiError::Invalid("project title must not be empty".into()));
    }
    let mut tx = state.write_tx().await?;
    let company: Option<(String,)> = sqlx::query_as("SELECT id FROM companies WHERE id = ?")
        .bind(&company_id)
        .fetch_optional(&mut *tx)
        .await?;
    if company.is_none() {
        return Err(ApiError::NotFound("company"));
    }
    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO projects (id, company_id, title, description, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&company_id)
    .bind(req.title.trim())
    .bind(&req.description)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::PROJECT_CREATED,
        &json!({ "project_id": id, "title": req.title.trim() }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "company_id": company_id, "title": req.title.trim(), "created_at": created_at }),
        ),
    ))
}

/// A company's projects, each with its goals and workspaces nested — enough
/// for the UI to attach tasks to a goal and know a runnable workspace exists,
/// without a fistful of round-trips.
async fn list_projects(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let projects: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, title, created_at FROM projects WHERE company_id = ? ORDER BY created_at",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::with_capacity(projects.len());
    for (id, title, created_at) in projects {
        let goals: Vec<(String, String)> =
            sqlx::query_as("SELECT id, title FROM goals WHERE project_id = ? ORDER BY created_at")
                .bind(&id)
                .fetch_all(&state.pool)
                .await?;
        let workspaces: Vec<(String, String, String, bool)> = sqlx::query_as(
            "SELECT id, name, cwd, is_primary FROM project_workspaces WHERE project_id = ? ORDER BY created_at",
        )
        .bind(&id)
        .fetch_all(&state.pool)
        .await?;
        out.push(json!({
            "id": id,
            "title": title,
            "created_at": created_at,
            "goals": goals.into_iter().map(|(gid, gt)| json!({ "id": gid, "title": gt })).collect::<Vec<_>>(),
            "workspaces": workspaces.into_iter().map(|(wid, wn, cwd, primary)| json!({ "id": wid, "name": wn, "cwd": cwd, "is_primary": primary })).collect::<Vec<_>>(),
        }));
    }
    Ok(Json(json!({ "projects": out })))
}

#[derive(Deserialize)]
struct CreateGoal {
    title: String,
    #[serde(default)]
    description: String,
}

async fn create_goal(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateGoal>,
) -> Result<impl IntoResponse, ApiError> {
    if req.title.trim().is_empty() {
        return Err(ApiError::Invalid("goal title must not be empty".into()));
    }
    let mut tx = state.write_tx().await?;
    let project: Option<(String,)> = sqlx::query_as("SELECT company_id FROM projects WHERE id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((company_id,)) = project else {
        return Err(ApiError::NotFound("project"));
    };
    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO goals (id, project_id, title, description, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(req.title.trim())
    .bind(&req.description)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::GOAL_CREATED,
        &json!({ "goal_id": id, "project_id": project_id, "title": req.title.trim() }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "project_id": project_id, "title": req.title.trim(), "created_at": created_at }),
        ),
    ))
}

// ---------- tasks ----------

#[derive(Deserialize)]
pub(crate) struct CreateTask {
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) goal_id: Option<String>,
    pub(crate) priority: Option<String>,
    /// `code` (default) or `knowledge` (ADR-0017).
    pub(crate) execution_kind: Option<String>,
}

async fn create_task(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<CreateTask>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(open_task(&state, &company_id, &req).await?),
    ))
}

/// File a task in a company's backlog, and say what was filed.
///
/// One definition of what a valid new task is, because there are now two doors
/// into it: this one and the MCP tool an outside caller uses (ADR-0028). Split
/// out rather than copied — parallel copies of a rule is a mistake this project
/// has already paid for once, in `agent_command` (ADR-0021), where the second
/// copy named no model and nobody noticed for a milestone.
pub(crate) async fn open_task(
    state: &AppState,
    company_id: &str,
    req: &CreateTask,
) -> Result<Value, ApiError> {
    let company_id = company_id.to_string();
    if req.title.trim().is_empty() {
        return Err(ApiError::Invalid("task title must not be empty".into()));
    }
    let priority = req.priority.as_deref().unwrap_or("medium");
    if !matches!(priority, "low" | "medium" | "high" | "urgent") {
        return Err(ApiError::Invalid(format!("unknown priority '{priority}'")));
    }
    let execution_kind = req.execution_kind.as_deref().unwrap_or("code");
    if crate::domain::ExecutionKind::parse(execution_kind).is_none() {
        return Err(ApiError::Invalid(format!(
            "unknown execution_kind '{execution_kind}'"
        )));
    }
    let mut tx = state.write_tx().await?;
    let company: Option<(String,)> = sqlx::query_as("SELECT id FROM companies WHERE id = ?")
        .bind(&company_id)
        .fetch_optional(&mut *tx)
        .await?;
    if company.is_none() {
        return Err(ApiError::NotFound("company"));
    }
    if let Some(goal_id) = &req.goal_id {
        let goal: Option<(String,)> = sqlx::query_as(
            "SELECT g.id FROM goals g JOIN projects p ON p.id = g.project_id WHERE g.id = ? AND p.company_id = ?",
        )
        .bind(goal_id)
        .bind(&company_id)
        .fetch_optional(&mut *tx)
        .await?;
        if goal.is_none() {
            return Err(ApiError::NotFound("goal"));
        }
    }
    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO tasks (id, company_id, goal_id, title, description, status, priority, execution_kind, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'backlog', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&company_id)
    .bind(&req.goal_id)
    .bind(req.title.trim())
    .bind(&req.description)
    .bind(priority)
    .bind(execution_kind)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(&id),
        event_kind::TASK_CREATED,
        &json!({ "title": req.title.trim(), "goal_id": req.goal_id, "priority": priority, "execution_kind": execution_kind }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(json!({
        "id": id,
        "company_id": company_id,
        "goal_id": req.goal_id,
        "title": req.title.trim(),
        "status": "backlog",
        "priority": priority,
        "execution_kind": execution_kind,
        "created_at": created_at,
    }))
}

/// (id, goal_id, title, status, priority, assignee_agent_id, execution_kind, updated_at)
type TaskRow = (
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
);

async fn list_tasks(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<TaskRow> = sqlx::query_as(
        "SELECT id, goal_id, title, status, priority, assignee_agent_id, execution_kind, updated_at
         FROM tasks WHERE company_id = ? ORDER BY created_at",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let tasks: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, goal_id, title, status, priority, assignee, execution_kind, updated_at)| {
                json!({
                    "id": id,
                    "goal_id": goal_id,
                    "title": title,
                    "status": status,
                    "priority": priority,
                    "assignee_agent_id": assignee,
                    "execution_kind": execution_kind,
                    "updated_at": updated_at,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "tasks": tasks })))
}

#[derive(Deserialize)]
struct TransitionTask {
    to: String,
    /// When moving to `in_progress`, optionally (re)assign the task.
    agent_id: Option<String>,
}

async fn transition_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<TransitionTask>,
) -> Result<Json<Value>, ApiError> {
    let Some(to) = TaskStatus::parse(&req.to) else {
        return Err(ApiError::Invalid(format!("unknown status '{}'", req.to)));
    };

    let mut tx = state.write_tx().await?;
    let task: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT company_id, status, assignee_agent_id FROM tasks WHERE id = ?")
            .bind(&task_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((company_id, status_str, current_assignee)) = task else {
        return Err(ApiError::NotFound("task"));
    };
    let Some(from) = TaskStatus::parse(&status_str) else {
        return Err(ApiError::Internal(
            format!("corrupt task status '{status_str}'").into(),
        ));
    };
    if !from.can_transition(to) {
        return Err(ApiError::Invalid(format!(
            "invalid transition {} -> {}",
            from.as_str(),
            to.as_str()
        )));
    }

    let assignee = match (&req.agent_id, to) {
        (Some(agent_id), TaskStatus::InProgress) => {
            let agent: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM agents WHERE id = ? AND company_id = ? AND status = 'active'",
            )
            .bind(agent_id)
            .bind(&company_id)
            .fetch_optional(&mut *tx)
            .await?;
            if agent.is_none() {
                return Err(ApiError::NotFound("agent"));
            }
            Some(agent_id.clone())
        }
        (Some(_), _) => {
            return Err(ApiError::Invalid(
                "agent_id may only be set when transitioning to in_progress".into(),
            ));
        }
        (None, _) => current_assignee,
    };

    sqlx::query("UPDATE tasks SET status = ?, assignee_agent_id = ?, updated_at = ? WHERE id = ?")
        .bind(to.as_str())
        .bind(&assignee)
        .bind(now())
        .bind(&task_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(&task_id),
        event_kind::TASK_TRANSITIONED,
        &json!({ "from": from.as_str(), "to": to.as_str(), "assignee_agent_id": assignee }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({
        "id": task_id,
        "status": to.as_str(),
        "assignee_agent_id": assignee,
    })))
}

// ---------- workspaces & execution ----------

#[derive(Deserialize)]
struct CreateWorkspace {
    name: String,
    /// Path of the git repository agents will work on.
    cwd: String,
    default_ref: Option<String>,
    /// Defaults to true: the primary workspace is the one task sessions use.
    is_primary: Option<bool>,
}

/// Why a workspace path is not there, said in terms the person can act on.
///
/// `cwd '/Users/me/code/thing' is not a directory` is true and useless in a
/// container: the path exists perfectly well, on the other side of a boundary
/// the message never mentions. The path a workspace needs is the **in-container**
/// one, and until now the only place that was written down was a comment in
/// `docker-compose.yml` — so the way you found out was by getting this error and
/// guessing.
///
/// So when a mount point is configured, name it and say what is actually
/// mounted. An empty one is worth saying too: "nothing is mounted" is a
/// different problem from "you named the wrong path", and they are indeed the
/// two things that happen.
fn unreachable_cwd(repos_dir: Option<&std::path::Path>, cwd: &str) -> String {
    let plain = format!("cwd '{cwd}' is not a directory");
    let Some(repos) = repos_dir else {
        return plain;
    };
    let mounted: Vec<String> = std::fs::read_dir(repos)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let repos = repos.display();
    if mounted.is_empty() {
        return format!(
            "{plain}. Overmind is running in a container, so a workspace path has to be one \
             *it* can see — host paths are not reachable from in here. Nothing is mounted at \
             {repos} yet: add your repository to the `volumes` of docker-compose.yml, e.g. \
             `- ${{HOME}}/code:{repos}:rw`, and use the {repos}/… path here."
        );
    }
    let mut names: Vec<String> = mounted;
    names.sort();
    let listed = names
        .iter()
        .map(|n| format!("{repos}/{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{plain}. Overmind is running in a container, so a workspace path has to be one *it* \
         can see. Mounted right now: {listed}."
    )
}

async fn create_workspace(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateWorkspace>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::Invalid("workspace name must not be empty".into()));
    }
    if !std::path::Path::new(&req.cwd).is_dir() {
        return Err(ApiError::Invalid(unreachable_cwd(
            state.config.repos_dir.as_deref(),
            &req.cwd,
        )));
    }
    let is_primary = req.is_primary.unwrap_or(true);
    let mut tx = state.write_tx().await?;
    let project: Option<(String,)> = sqlx::query_as("SELECT company_id FROM projects WHERE id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((company_id,)) = project else {
        return Err(ApiError::NotFound("project"));
    };
    if is_primary {
        sqlx::query("UPDATE project_workspaces SET is_primary = 0 WHERE project_id = ?")
            .bind(&project_id)
            .execute(&mut *tx)
            .await?;
    }
    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO project_workspaces (id, project_id, name, source_type, cwd, default_ref, is_primary, created_at)
         VALUES (?, ?, ?, 'local_path', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(req.name.trim())
    .bind(&req.cwd)
    .bind(&req.default_ref)
    .bind(is_primary)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::WORKSPACE_CREATED,
        &json!({ "workspace_id": id, "project_id": project_id, "cwd": req.cwd, "is_primary": is_primary }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "project_id": project_id,
            "name": req.name.trim(),
            "cwd": req.cwd,
            "default_ref": req.default_ref,
            "is_primary": is_primary,
            "created_at": created_at,
        })),
    ))
}

async fn list_workspaces(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String, Option<String>, bool, String)> = sqlx::query_as(
        "SELECT id, name, cwd, default_ref, is_primary, created_at
         FROM project_workspaces WHERE project_id = ? ORDER BY created_at",
    )
    .bind(&project_id)
    .fetch_all(&state.pool)
    .await?;
    let workspaces: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, cwd, default_ref, is_primary, created_at)| {
            json!({
                "id": id,
                "name": name,
                "cwd": cwd,
                "default_ref": default_ref,
                "is_primary": is_primary,
                "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "workspaces": workspaces })))
}

#[derive(Deserialize)]
struct StartTask {
    agent_id: String,
}

async fn start_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<StartTask>,
) -> Result<impl IntoResponse, ApiError> {
    match crate::runner::start_task(&state, &task_id, &req.agent_id, false).await? {
        crate::runner::StartResult::Started(outcome) => Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "status": "started",
                "session_id": outcome.session_id,
                "branch": outcome.branch,
                "workspace_path": outcome.workspace_path,
            })),
        )),
        crate::runner::StartResult::ApprovalRequired { approval_id } => Ok((
            StatusCode::ACCEPTED,
            Json(json!({ "status": "approval_required", "approval_id": approval_id })),
        )),
    }
}

#[derive(Deserialize, Default)]
struct RequestWakeup {
    reason: Option<String>,
    source: Option<String>,
}

async fn request_wakeup(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    payload: Option<Json<RequestWakeup>>,
) -> Result<impl IntoResponse, ApiError> {
    let req = payload.map(|Json(r)| r).unwrap_or_default();
    let mut tx = state.write_tx().await?;
    let agent: Option<(String,)> = sqlx::query_as("SELECT company_id FROM agents WHERE id = ?")
        .bind(&agent_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((company_id,)) = agent else {
        return Err(ApiError::NotFound("agent"));
    };
    let (id, requested_at) = (new_id(), now());
    let source = req.source.as_deref().unwrap_or("manual");
    sqlx::query(
        "INSERT INTO agent_wakeup_requests (id, agent_id, source, reason, requested_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&agent_id)
    .bind(source)
    .bind(&req.reason)
    .bind(&requested_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::WAKEUP_REQUESTED,
        &json!({ "request_id": id, "agent_id": agent_id, "source": source, "reason": req.reason }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok((
        StatusCode::ACCEPTED,
        Json(
            json!({ "id": id, "agent_id": agent_id, "status": "queued", "requested_at": requested_at }),
        ),
    ))
}

/// (task_id, agent_id, adapter_type, status, branch, workspace_path, base_sha,
///  output, exit_code, last_error, created_at, started_at, finished_at)
type SessionRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<SessionRow> = sqlx::query_as(
        "SELECT task_id, agent_id, adapter_type, status, branch, workspace_path, base_sha,
                output, exit_code, last_error, created_at, started_at, finished_at
         FROM agent_task_sessions WHERE id = ?",
    )
    .bind(&session_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((
        task_id,
        agent_id,
        adapter_type,
        status,
        branch,
        workspace_path,
        base_sha,
        output,
        exit_code,
        last_error,
        created_at,
        started_at,
        finished_at,
    )) = row
    else {
        return Err(ApiError::NotFound("session"));
    };
    let cost: Option<(i64,)> =
        sqlx::query_as("SELECT COALESCE(SUM(cost_cents), 0) FROM cost_events WHERE session_id = ?")
            .bind(&session_id)
            .fetch_optional(&state.pool)
            .await?;
    Ok(Json(json!({
        "activity": state.activity(&session_id),
        "id": session_id,
        "task_id": task_id,
        "agent_id": agent_id,
        "adapter_type": adapter_type,
        "status": status,
        "branch": branch,
        "workspace_path": workspace_path,
        "base_sha": base_sha,
        "output": output,
        // What the agent actually said, lifted out of the adapter's envelope.
        // The envelope is diagnostic and stays in `output`; it is not a report.
        // A person opening a finished run wants the agent's own summary, not
        // `ephemeral_1h_input_tokens` — the same confusion that once showed a
        // raw envelope as a chat reply.
        "said": output.as_deref().map(crate::ceo::agent_text).filter(|s| {
            !s.trim().is_empty() && output.as_deref() != Some(s.as_str())
        }),
        "exit_code": exit_code,
        "last_error": last_error,
        "cost_cents": cost.map(|(c,)| c).unwrap_or(0),
        "created_at": created_at,
        "started_at": started_at,
        "finished_at": finished_at,
    })))
}

async fn get_session_diff(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<String, ApiError> {
    Ok(crate::runner::session_diff(&state, &session_id).await?)
}

/// (id, agent_id, status, exit_code, last_error, created_at)
type TaskSessionRow = (String, String, String, Option<i64>, Option<String>, String);

/// Sessions for a task, newest first — lets the UI find a task's run(s) after
/// a reload without the client having to remember session ids.
async fn list_task_sessions(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<TaskSessionRow> = sqlx::query_as(
        "SELECT id, agent_id, status, exit_code, last_error, created_at
         FROM agent_task_sessions WHERE task_id = ? ORDER BY created_at DESC",
    )
    .bind(&task_id)
    .fetch_all(&state.pool)
    .await?;
    let sessions: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, agent_id, status, exit_code, last_error, created_at)| {
                json!({
                    "id": id,
                    "agent_id": agent_id,
                    "status": status,
                    "exit_code": exit_code,
                    "last_error": last_error,
                    "created_at": created_at,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "sessions": sessions })))
}

/// (id, session_id, kind, title, mime, content, file_path, created_at)
/// (id, session_id, kind, title, mime, content, file_path, size_bytes,
/// relative_path, created_at)
type TaskArtifactRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    String,
);

/// Artifacts a task's knowledge run(s) produced (ADR-0017), newest first. The
/// drawer shows these instead of a diff for `knowledge` tasks.
async fn list_task_artifacts(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<TaskArtifactRow> = sqlx::query_as(
        "SELECT id, session_id, kind, title, mime, content, file_path, size_bytes, relative_path, created_at
         FROM task_artifacts WHERE task_id = ? ORDER BY created_at DESC",
    )
    .bind(&task_id)
    .fetch_all(&state.pool)
    .await?;
    let artifacts: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                session_id,
                kind,
                title,
                mime,
                content,
                file_path,
                size_bytes,
                relative_path,
                created_at,
            )| {
                json!({
                    "id": id,
                    "session_id": session_id,
                    "kind": kind,
                    "title": title,
                    "mime": mime,
                    "content": content,
                    // Whether the bytes can be fetched — the client shows a
                    // download only when there is something behind it.
                    "downloadable": file_path.is_some(),
                    "size_bytes": size_bytes,
                    "relative_path": relative_path,
                    "created_at": created_at,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "artifacts": artifacts })))
}

// ---------- conversation: talk to the CEO (M12 / ADR-0018) ----------

#[derive(Deserialize)]
struct PostMessage {
    content: String,
    /// Ids of attachments already uploaded to this thread (see upload_attachment).
    #[serde(default)]
    attachment_ids: Vec<String>,
}

/// Post a user message to an agent's thread. The agent's reply and any tasks it
/// opens arrive asynchronously (announced over `/ws`).
async fn post_message(
    State(state): State<AppState>,
    Path((company_id, agent_id)): Path<(String, String)>,
    Json(req): Json<PostMessage>,
) -> Result<impl IntoResponse, ApiError> {
    let conversation_id = crate::ceo::post_user_message(
        &state,
        &company_id,
        &agent_id,
        &req.content,
        &req.attachment_ids,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "conversation_id": conversation_id })),
    ))
}

/// Upload a file/image to an agent's thread. Multipart with a `file` part.
/// Returns the attachment id to include in the next `post_message`.
async fn upload_attachment(
    State(state): State<AppState>,
    Path((company_id, agent_id)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut file: Option<(String, String, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::Invalid(format!("malformed upload: {e}")))?
    {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or("file").to_string();
            let mime = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::Invalid(format!("bad file: {e}")))?
                .to_vec();
            file = Some((filename, mime, bytes));
        }
    }
    let (filename, mime, bytes) =
        file.ok_or_else(|| ApiError::Invalid("a file part is required".into()))?;
    let meta =
        crate::ceo::store_attachment(&state, &company_id, &agent_id, &filename, &mime, &bytes)
            .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": meta.id,
            "filename": meta.filename,
            "mime": meta.mime,
            "size_bytes": meta.size_bytes,
        })),
    ))
}

/// Serve an attachment's bytes (for the UI to render images / offer downloads).
async fn download_attachment(
    State(state): State<AppState>,
    Path((company_id, attachment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT a.filename, a.mime, a.path FROM attachments a
         JOIN conversations c ON c.id = a.conversation_id
         WHERE a.id = ? AND c.company_id = ?",
    )
    .bind(&attachment_id)
    .bind(&company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((filename, mime, path)) = row else {
        return Err(ApiError::NotFound("attachment"));
    };
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound("attachment file"))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{filename}\""),
            ),
        ],
        bytes,
    ))
}

/// Serve an artifact's bytes — the only way anything non-text a run produced
/// gets off this machine and into the user's hands (M17).
///
/// `attachment` (not `inline`): an artifact is a deliverable, and the browser
/// should save it rather than try to display a spreadsheet. The one exception
/// is an image, which the drawer renders from this same URL.
async fn download_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let row: Option<(String, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT title, mime, file_path, content FROM task_artifacts WHERE id = ?")
            .bind(&artifact_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((title, mime, file_path, content)) = row else {
        return Err(ApiError::NotFound("artifact"));
    };
    // Prefer the file; fall back to the inline text, so an artifact that was
    // only ever stored as content is still downloadable.
    let bytes = match file_path {
        Some(p) => tokio::fs::read(&p)
            .await
            .map_err(|_| ApiError::NotFound("artifact file"))?,
        None => content
            .ok_or(ApiError::NotFound("artifact bytes"))?
            .into_bytes(),
    };
    let filename = crate::files::safe_name(&title);
    let disposition = if mime.starts_with("image/") {
        format!("inline; filename=\"{filename}\"")
    } else {
        format!("attachment; filename=\"{filename}\"")
    };
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    ))
}

/// Attach a file to a task, so an agent that picks it up gets it too (M17).
/// Multipart with a `file` part, same shape as the conversation upload.
async fn upload_task_attachment(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let exists: Option<(String,)> = sqlx::query_as("SELECT company_id FROM tasks WHERE id = ?")
        .bind(&task_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some((company_id,)) = exists else {
        return Err(ApiError::NotFound("task"));
    };
    let (filename, mime, bytes) = read_upload(multipart).await?;
    let name = crate::files::safe_name(&filename);
    // Trust the extension over the browser's content type: a browser reports
    // `application/octet-stream` for anything it does not recognise, and the
    // extension is what the agent will see anyway.
    let mime = match crate::files::mime_for(&name) {
        "application/octet-stream" => mime,
        known => known.to_string(),
    };
    let id = uuid::Uuid::now_v7().to_string();
    let dir = state.config.data_dir.join("attachments").join(&task_id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Invalid(format!("cannot create attachments dir: {e}")))?;
    let path = dir.join(format!("{id}_{name}"));
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| ApiError::Invalid(format!("cannot write attachment: {e}")))?;
    let size = bytes.len() as i64;
    let mut tx = state.write_tx().await?;
    sqlx::query(
        "INSERT INTO attachments
         (id, conversation_id, task_id, message_id, origin, filename, mime, size_bytes, path, created_at)
         VALUES (?, NULL, ?, NULL, 'user', ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&task_id)
    .bind(&name)
    .bind(&mime)
    .bind(size)
    .bind(path.to_string_lossy().as_ref())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    crate::audit::append(
        &mut tx,
        Some(&company_id),
        Some(&task_id),
        crate::domain::event_kind::ATTACHMENT_ADDED,
        &json!({ "task_id": task_id, "attachment_id": id, "filename": name, "mime": mime }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({
        "id": id, "filename": name, "mime": mime, "size_bytes": size,
    })))
}

/// Files attached to a task — what an agent picking it up will be handed.
async fn list_task_attachments(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // The task's own files plus its birth-thread's (ADR-0038) — the same
    // set the run receives, so what the interface lists is what the agent got.
    let rows: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT id, filename, mime, size_bytes, created_at FROM attachments
         WHERE task_id = ?1
            OR (message_id IS NOT NULL AND conversation_id IS NOT NULL
                AND conversation_id = (SELECT conversation_id FROM tasks WHERE id = ?1))
         ORDER BY created_at",
    )
    .bind(&task_id)
    .fetch_all(&state.pool)
    .await?;
    let attachments: Vec<Value> = rows
        .into_iter()
        .map(|(id, filename, mime, size_bytes, created_at)| {
            json!({
                "id": id, "filename": filename, "mime": mime,
                "size_bytes": size_bytes, "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "attachments": attachments })))
}

/// Detach a file from a task. The row goes; the bytes on disk are left for the
/// audit trail to point at, and cost nothing to keep.
async fn remove_task_attachment(
    State(state): State<AppState>,
    Path((task_id, attachment_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let done = sqlx::query("DELETE FROM attachments WHERE id = ? AND task_id = ?")
        .bind(&attachment_id)
        .bind(&task_id)
        .execute(&state.pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("attachment"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// The single `file` part of a multipart upload.
async fn read_upload(mut multipart: Multipart) -> Result<(String, String, Vec<u8>), ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::Invalid(format!("malformed upload: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::Invalid(format!("bad file: {e}")))?
            .to_vec();
        if bytes.is_empty() {
            return Err(ApiError::Invalid("attachment is empty".into()));
        }
        return Ok((filename, mime, bytes));
    }
    Err(ApiError::Invalid("no file part in upload".into()))
}

/// An agent's thread and its messages (null until the first message).
async fn get_conversation(
    State(state): State<AppState>,
    Path((company_id, agent_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let convo: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, agent_id, title, created_at FROM conversations WHERE company_id = ? AND agent_id = ?",
    )
    .bind(&company_id)
    .bind(&agent_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((id, agent_id, title, created_at)) = convo else {
        return Ok(Json(json!({ "conversation": null, "messages": [] })));
    };
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, role, content, created_at FROM messages WHERE conversation_id = ? ORDER BY created_at",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    // Attachments, grouped by their message.
    let atts: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, message_id, filename, mime, size_bytes FROM attachments
         WHERE conversation_id = ? AND message_id IS NOT NULL ORDER BY created_at",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    let mut by_msg: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for (aid, mid, fname, mime, size) in atts {
        by_msg.entry(mid).or_default().push(json!({
            "id": aid, "filename": fname, "mime": mime, "size_bytes": size,
        }));
    }
    let messages: Vec<Value> = rows
        .into_iter()
        .map(|(mid, role, content, created)| {
            let attachments = by_msg.remove(&mid).unwrap_or_default();
            json!({ "id": mid, "role": role, "content": content, "created_at": created, "attachments": attachments })
        })
        .collect();
    Ok(Json(json!({
        "conversation": { "id": id, "agent_id": agent_id, "title": title, "created_at": created_at },
        "messages": messages,
        // Whether a turn is in flight (ADR-0038 addendum): the chat asks on
        // every load, so the typing dots survive a page switch and vanish
        // the moment the reply lands, whichever page you were on.
        "answering": state.is_answering(&id),
        // What the turn is doing right now (ADR-0039), when it said.
        "activity": state.activity(&id),
    })))
}

// ---------- meetings: bounded deliberation (M13 / ADR-0020) ----------

#[derive(Deserialize)]
struct ConveneMeeting {
    topic: String,
    /// Agent ids, in speaking order. At least two.
    participants: Vec<String>,
    /// How many turns before the chair must close it (clamped to [1, 12]).
    #[serde(default = "default_turn_cap")]
    turn_cap: i64,
}

fn default_turn_cap() -> i64 {
    6
}

/// Convene a meeting. The deliberation runs in the background; the transcript
/// and the decision arrive as they happen (announced over `/ws`).
async fn convene_meeting(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<ConveneMeeting>,
) -> Result<impl IntoResponse, ApiError> {
    let id = crate::meeting::convene(
        &state,
        &company_id,
        &req.topic,
        &req.participants,
        req.turn_cap,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

/// Pick a paused room back up (ADR-0022).
///
/// Runs in the background like the original deliberation: resuming can take as
/// long as the remaining turns do, and the caller should not hold a request
/// open for it. The room's status is the progress indicator.
async fn resume_meeting(
    State(state): State<AppState>,
    Path((company_id, meeting_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    // Fail fast on "not paused" so the caller gets a real answer, then let the
    // deliberation itself run detached.
    let paused: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM meetings WHERE id = ? AND company_id = ? AND status = 'paused'",
    )
    .bind(&meeting_id)
    .bind(&company_id)
    .fetch_optional(&state.pool)
    .await?;
    if paused.is_none() {
        return Err(ApiError::Conflict("this meeting is not paused".into()));
    }
    let id = meeting_id.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::meeting::resume(&state, &company_id, &meeting_id).await {
            eprintln!("resuming meeting {meeting_id} failed: {e}");
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

/// A company's meetings, newest first — including the ones still waiting on you.
/// The human behind a meeting's fate, read off the audit chain (M25): the one
/// who declined it, or -- for a room that was let to convene -- the one who
/// decided the approval that opened it. `m` is the meetings alias.
const MEETING_DECIDED_BY: &str = "COALESCE(
    (SELECT u.name FROM audit_events e
       JOIN users u ON u.id = json_extract(e.payload, '$.actor')
      WHERE e.kind = 'meeting.declined'
        AND json_extract(e.payload, '$.meeting_id') = m.id
      ORDER BY e.seq DESC LIMIT 1),
    (SELECT u.name FROM audit_events e
       JOIN users u ON u.id = json_extract(e.payload, '$.actor')
      WHERE e.kind = 'approval.decided'
        AND json_extract(e.payload, '$.approval_id') = m.approval_id
      ORDER BY e.seq DESC LIMIT 1)) AS decided_by";

async fn list_meetings(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    type Row = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT m.id, m.topic, m.reason, m.convener_agent_id, a.name, m.turn_cap, m.status,
                m.decision, m.decline_note, m.approval_id, m.created_at, m.decided_at,
                {MEETING_DECIDED_BY}
         FROM meetings m LEFT JOIN agents a ON a.id = m.convener_agent_id
         WHERE m.company_id = ? ORDER BY m.created_at DESC"
    ))
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let meetings: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                topic,
                reason,
                convener_agent_id,
                convener_name,
                turn_cap,
                status,
                decision,
                decline_note,
                approval_id,
                created_at,
                decided_at,
                decided_by,
            )| {
                json!({
                    "id": id, "topic": topic, "reason": reason,
                    "convener_agent_id": convener_agent_id, "convener_name": convener_name,
                    "turn_cap": turn_cap, "status": status, "decision": decision,
                    "decline_note": decline_note, "approval_id": approval_id,
                    "created_at": created_at, "decided_at": decided_at,
                    "decided_by": decided_by,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "meetings": meetings })))
}

/// One meeting: who is in the room, the transcript, and the decision.
async fn get_meeting(
    State(state): State<AppState>,
    Path(meeting_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    type MeetingRow = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Option<MeetingRow> = sqlx::query_as(&format!(
        "SELECT m.id, m.company_id, m.topic, m.reason, m.convener_agent_id, a.name, m.turn_cap,
                m.status, m.decision, m.decline_note, m.approval_id, m.created_at, m.decided_at,
                m.paused_note, {MEETING_DECIDED_BY}
         FROM meetings m LEFT JOIN agents a ON a.id = m.convener_agent_id
         WHERE m.id = ?"
    ))
    .bind(&meeting_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((
        id,
        company_id,
        topic,
        reason,
        convener_agent_id,
        convener_name,
        turn_cap,
        status,
        decision,
        decline_note,
        approval_id,
        created_at,
        decided_at,
        paused_note,
        decided_by,
    )) = row
    else {
        return Err(ApiError::NotFound("meeting"));
    };
    let participants: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT a.id, a.name, a.title FROM meeting_participants mp
         JOIN agents a ON a.id = mp.agent_id
         WHERE mp.meeting_id = ? ORDER BY mp.position",
    )
    .bind(&meeting_id)
    .fetch_all(&state.pool)
    .await?;
    let turns: Vec<(String, String, String, i64, String, String)> = sqlx::query_as(
        "SELECT t.id, t.agent_id, a.name, t.ordinal, t.content, t.created_at
         FROM meeting_turns t JOIN agents a ON a.id = t.agent_id
         WHERE t.meeting_id = ? ORDER BY t.ordinal",
    )
    .bind(&meeting_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({
        "meeting": {
            "id": id, "company_id": company_id, "topic": topic, "reason": reason,
            "convener_agent_id": convener_agent_id, "convener_name": convener_name,
            "turn_cap": turn_cap, "status": status, "decision": decision,
            "decline_note": decline_note, "approval_id": approval_id,
            // Why the room is waiting, when it is (ADR-0022).
            "paused_note": paused_note,
            "created_at": created_at, "decided_at": decided_at,
            // The human who let it convene, or who declined it (M25).
            "decided_by": decided_by,
        },
        "participants": participants.into_iter().map(|(id, name, title)| {
            json!({ "id": id, "name": name, "title": title })
        }).collect::<Vec<_>>(),
        "turns": turns.into_iter().map(|(id, agent_id, name, ordinal, content, created_at)| {
            json!({
                "id": id, "agent_id": agent_id, "agent_name": name,
                "ordinal": ordinal, "content": content, "created_at": created_at,
            })
        }).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct SetLanguage {
    language: String,
}

/// Choose the language the company works in (M16). It governs the interface
/// *and* what the agents write, which is why it lives here and not in the
/// browser: a per-tab preference cannot instruct an agent.
async fn set_language(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<SetLanguage>,
) -> Result<Json<Value>, ApiError> {
    if !crate::i18n::is_supported(&req.language) {
        return Err(ApiError::Invalid(format!(
            "unsupported language `{}`",
            req.language
        )));
    }
    let done = sqlx::query("UPDATE companies SET language = ? WHERE id = ?")
        .bind(&req.language)
        .bind(&company_id)
        .execute(&state.pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("company"));
    }
    state.notify(&company_id);
    Ok(Json(json!({ "id": company_id, "language": req.language })))
}

// ---------- integration tokens (M9, ADR-0028) ----------

#[derive(Deserialize)]
struct CreateToken {
    label: String,
}

/// Issue a credential for a caller outside Overmind — a Claude Code session, a
/// script — so it can file work and read the board over MCP (ADR-0028).
///
/// The secret is in this response and nowhere else afterwards. Not because the
/// store is untrusted (it is plaintext in `overmind.sqlite`, and the threat
/// model says the machine is the boundary) but because a credential you can
/// re-read is one nobody bothers to keep track of, and the label is what makes
/// revoking it later a decision rather than a guess.
async fn create_company_token(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<CreateToken>,
) -> Result<impl IntoResponse, ApiError> {
    let label = req.label.trim();
    if label.is_empty() {
        return Err(ApiError::Invalid("a token needs a label".into()));
    }
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM companies WHERE id = ?")
        .bind(&company_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("company"));
    }
    // v4, not the v7 used for ids: a v7 encodes the time it was minted, and a
    // secret should not be predictable in any dimension (ADR-0027).
    let token = uuid::Uuid::new_v4().to_string();
    let (id, created_at) = (new_id(), now());
    let mut tx = state.write_tx().await?;
    sqlx::query(
        "INSERT INTO company_tokens (id, company_id, label, token, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&company_id)
    .bind(label)
    .bind(&token)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    // The label, never the token: an audit log is read by people, and a log
    // that quotes secrets is a place secrets leak from.
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::TOKEN_ISSUED,
        &json!({ "token_id": id, "label": label }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "label": label,
            "token": token,
            "created_at": created_at,
        })),
    ))
}

/// (id, label, created_at, last_used_at, revoked_at)
type TokenRow = (String, String, String, Option<String>, Option<String>);

/// The credentials this company has issued — what they are for, whether they
/// have ever been used, and whether they still work. Never the secrets.
async fn list_company_tokens(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<TokenRow> = sqlx::query_as(
        "SELECT id, label, created_at, last_used_at, revoked_at
           FROM company_tokens WHERE company_id = ? ORDER BY created_at DESC",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({
        "tokens": rows
            .into_iter()
            .map(|(id, label, created_at, last_used_at, revoked_at)| json!({
                "id": id,
                "label": label,
                "created_at": created_at,
                "last_used_at": last_used_at,
                "revoked_at": revoked_at,
            }))
            .collect::<Vec<_>>()
    })))
}

/// Withdraw a credential. A timestamp, not a delete: the audit log names the
/// token that filed a task, and a row that vanished would leave that name
/// pointing at nothing.
async fn revoke_company_token(
    State(state): State<AppState>,
    Path(token_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT company_id, label, revoked_at FROM company_tokens WHERE id = ?")
            .bind(&token_id)
            .fetch_optional(&state.pool)
            .await?;
    let (company_id, label, revoked_at) = row.ok_or(ApiError::NotFound("token"))?;
    if let Some(at) = revoked_at {
        // Already gone, and saying so beats appending a second revocation event
        // for a credential that stopped working the first time.
        return Ok(Json(json!({ "id": token_id, "revoked_at": at })));
    }
    let at = now();
    let mut tx = state.write_tx().await?;
    sqlx::query("UPDATE company_tokens SET revoked_at = ? WHERE id = ?")
        .bind(&at)
        .bind(&token_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::TOKEN_REVOKED,
        &json!({ "token_id": token_id, "label": label }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": token_id, "revoked_at": at })))
}

// ---------- the company's brain (M8, ADR-0024) ----------

#[derive(Deserialize)]
struct SetBrainEnabled {
    enabled: bool,
}

/// What this company's memory actually is right now: whether a provider is
/// configured at all, whether this company's brain is switched on, and — when
/// brains are managed — where it lives. The path is worth returning: a managed
/// brain is a directory, and being able to open it in Obsidian or back it up is
/// most of the point of it being one.
async fn brain_status(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let enabled: Option<i64> =
        sqlx::query_scalar("SELECT brain_enabled FROM companies WHERE id = ?")
            .bind(&company_id)
            .fetch_optional(&state.pool)
            .await?;
    let enabled = enabled.ok_or(ApiError::NotFound("company"))? != 0;
    let managed = state.config.managed_brain && state.memory.is_enabled();
    Ok(Json(json!({
        "provider_configured": state.memory.is_enabled(),
        "managed": managed,
        "enabled": enabled,
        // Only meaningful when managed: otherwise the brain is wherever the
        // memory command points, which is the user's business and not ours to
        // report as if we chose it.
        "brain_dir": managed.then(|| state.brain_dir(&company_id).to_string_lossy().into_owned()),
    })))
}

/// Switch this company's brain on or off. Off is the no-provider path: agents
/// keep working, they just stop remembering — which is a thing you may
/// legitimately want, and is an acceptance criterion of M8.
async fn set_brain_enabled(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<SetBrainEnabled>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let done = sqlx::query("UPDATE companies SET brain_enabled = ? WHERE id = ?")
        .bind(i64::from(req.enabled))
        .bind(&company_id)
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("company"));
    }
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::BRAIN_TOGGLED,
        &json!({ "enabled": req.enabled }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({ "id": company_id, "enabled": req.enabled })))
}

// ---------- browsing what the organization remembers (M8, ADR-0025) ----------

/// How many rows a browse returns. The brain can hold thousands; a page that
/// tries to render all of them is not a browser, it is a download.
const BROWSE_LIMIT: u32 = 100;

#[derive(Deserialize)]
struct BrowseQuery {
    /// Present → semantic `recall`; absent → enumerate. Two different
    /// operations, not one filtered by the other (ADR-0025).
    q: Option<String>,
}

/// Why a browse came back with nothing — four situations that a careless
/// implementation renders identically as an empty page, and that need
/// different things from the reader (ADR-0025).
async fn browse_state(
    state: &AppState,
    company_id: &str,
) -> Result<Option<&'static str>, ApiError> {
    if !state.memory.is_enabled() {
        return Ok(Some("no_provider"));
    }
    let enabled: Option<i64> =
        sqlx::query_scalar("SELECT brain_enabled FROM companies WHERE id = ?")
            .bind(company_id)
            .fetch_optional(&state.pool)
            .await?;
    match enabled.ok_or(ApiError::NotFound("company"))? {
        0 => Ok(Some("brain_off")),
        _ => Ok(None),
    }
}

/// The subject each memory came from, keyed by the provider's identifier.
/// One query for the page rather than one per row.
async fn subjects_for(
    state: &AppState,
    company_id: &str,
    kind: &str,
) -> Result<std::collections::HashMap<String, Value>, ApiError> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT memory_ref, subject_type, subject_id, subject_title
           FROM memory_links WHERE company_id = ? AND kind = ?",
    )
    .bind(company_id)
    .bind(kind)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(r, stype, sid, title)| (r, json!({ "type": stype, "id": sid, "title": title })))
        .collect())
}

/// Take from a provider's row only the fields we recognize, and attach the
/// subject if we recorded one. Everything else the provider chose to send is
/// dropped: rendering fields we have not designed for is how a UI starts
/// depending on one implementation's shape.
fn normalize(item: &Value, subjects: &std::collections::HashMap<String, Value>) -> Value {
    let id = match item.get("id") {
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
    json!({
        "id": id,
        // `decision` is what the decisions table calls its headline; a memory
        // calls it `title`. One field to the reader, either way.
        "title": text("title").or_else(|| text("decision")),
        // Three names for the body, all seen coming out of the real provider:
        // a memory has `content`, a decision has `rationale`, and a search hit
        // has `preview`. Verified against Wadachi rather than guessed — an
        // enumerated memory carries no body at all, only a `filepath`, so a
        // listed row showing just its title is correct and not a bug.
        "content": text("content")
            .or_else(|| text("rationale"))
            .or_else(|| text("preview")),
        "category": text("category"),
        "project": text("project"),
        "created_at": text("created_at"),
        "subject": id.as_ref().and_then(|i| subjects.get(i)).cloned(),
    })
}

async fn browse(
    state: &AppState,
    company_id: &str,
    kind: &str,
    query: Option<String>,
) -> Result<Json<Value>, ApiError> {
    if let Some(reason) = browse_state(state, company_id).await? {
        return Ok(Json(json!({ "state": reason, "items": [] })));
    }
    let memory = state.memory_for(company_id).await;
    let scope = Some(company_id);
    let found = match (kind, query.as_deref()) {
        // A search asks the whole brain, memories and decisions alike — the
        // provider ranks by meaning and does not split by kind.
        (_, Some(q)) if !q.trim().is_empty() => memory.recall(q.trim(), scope, BROWSE_LIMIT).await,
        ("decision", _) => memory.list_decisions(scope, BROWSE_LIMIT).await,
        _ => memory.list_memories(scope).await,
    };
    let Some(items) = found else {
        // The provider answered something we cannot read. Saying so beats an
        // empty list, which would read as "this company knows nothing".
        return Ok(Json(json!({ "state": "not_browsable", "items": [] })));
    };
    let subjects = subjects_for(state, company_id, kind).await?;
    let items: Vec<Value> = items
        .iter()
        .take(BROWSE_LIMIT as usize)
        .map(|i| normalize(i, &subjects))
        .collect();
    Ok(Json(json!({ "state": "ok", "items": items })))
}

async fn browse_memories(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Query(q): Query<BrowseQuery>,
) -> Result<Json<Value>, ApiError> {
    browse(&state, &company_id, "memory", q.q).await
}

async fn browse_decisions(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Query(q): Query<BrowseQuery>,
) -> Result<Json<Value>, ApiError> {
    browse(&state, &company_id, "decision", q.q).await
}

/// The languages on offer, each named in its own language.
async fn list_languages() -> Json<Value> {
    Json(json!({
        "languages": crate::i18n::SUPPORTED
            .iter()
            .map(|(code, name)| json!({ "code": code, "name": name }))
            .collect::<Vec<_>>(),
    }))
}

// ---------- org proposals: the CEO designs a team, you decide (M15) ----------

/// The proposals of a company, newest first, each with its members.
async fn list_org_proposals(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT p.id, p.summary, a.name, p.status, p.decline_note, p.approval_id,
                p.created_at, p.decided_at
         FROM org_proposals p LEFT JOIN agents a ON a.id = p.proposed_by
         WHERE p.company_id = ? ORDER BY p.created_at DESC",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let mut proposals = Vec::new();
    for (id, summary, by, status, note, approval_id, created_at, decided_at) in rows {
        let members = proposal_members(&state, &id).await?;
        proposals.push(json!({
            "id": id, "summary": summary, "proposed_by_name": by, "status": status,
            "decline_note": note, "approval_id": approval_id,
            "created_at": created_at, "decided_at": decided_at,
            "members": members,
        }));
    }
    Ok(Json(json!({ "proposals": proposals })))
}

async fn get_org_proposal(
    State(state): State<AppState>,
    Path(proposal_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT p.id, p.summary, a.name, p.status, p.decline_note, p.approval_id,
                p.created_at, p.decided_at
         FROM org_proposals p LEFT JOIN agents a ON a.id = p.proposed_by
         WHERE p.id = ?",
    )
    .bind(&proposal_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((id, summary, by, status, note, approval_id, created_at, decided_at)) = row else {
        return Err(ApiError::NotFound("org proposal"));
    };
    let members = proposal_members(&state, &id).await?;
    Ok(Json(json!({
        "proposal": {
            "id": id, "summary": summary, "proposed_by_name": by, "status": status,
            "decline_note": note, "approval_id": approval_id,
            "created_at": created_at, "decided_at": decided_at,
        },
        "members": members,
    })))
}

/// (id, position, name, archetype, domain, title, reports_to, brief, rationale, excluded, hired)
type ProposalMemberRow = (
    String,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

async fn proposal_members(state: &AppState, proposal_id: &str) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<ProposalMemberRow> = sqlx::query_as(
        "SELECT id, position, name, archetype, domain, title, reports_to, brief, rationale,
                excluded, hired_agent_id
         FROM org_proposal_members WHERE proposal_id = ? ORDER BY position",
    )
    .bind(proposal_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                position,
                name,
                archetype,
                domain,
                title,
                reports_to,
                brief,
                rationale,
                excluded,
                hired,
            )| {
                json!({
                    "id": id, "position": position, "name": name, "archetype": archetype,
                    "domain": domain, "title": title, "reports_to": reports_to, "brief": brief,
                    "rationale": rationale, "excluded": excluded != 0, "hired_agent_id": hired,
                })
            },
        )
        .collect())
}

#[derive(Deserialize)]
struct SetExcluded {
    excluded: bool,
}

/// Drop a member from a proposal (or put them back) before accepting the rest.
/// Only while it is still `proposed`: an accepted chart is history.
async fn set_member_excluded(
    State(state): State<AppState>,
    Path((proposal_id, member_id)): Path<(String, String)>,
    Json(req): Json<SetExcluded>,
) -> Result<Json<Value>, ApiError> {
    let status: Option<(String,)> = sqlx::query_as("SELECT status FROM org_proposals WHERE id = ?")
        .bind(&proposal_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some((status,)) = status else {
        return Err(ApiError::NotFound("org proposal"));
    };
    if status != "proposed" {
        return Err(ApiError::Conflict(format!("proposal is already {status}")));
    }
    let done = sqlx::query(
        "UPDATE org_proposal_members SET excluded = ? WHERE id = ? AND proposal_id = ?",
    )
    .bind(i64::from(req.excluded))
    .bind(&member_id)
    .bind(&proposal_id)
    .execute(&state.pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("proposal member"));
    }
    Ok(Json(json!({ "id": member_id, "excluded": req.excluded })))
}

// ---------- notifications: how the company reaches you (ADR-0020) ----------

#[derive(Deserialize)]
struct NotificationQuery {
    /// Only what you haven't seen yet.
    #[serde(default)]
    unread: bool,
    #[serde(default = "default_notification_limit")]
    limit: i64,
}

fn default_notification_limit() -> i64 {
    50
}

/// A company's notifications, newest first, with the unread count for the badge.
async fn list_notifications(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Query(q): Query<NotificationQuery>,
) -> Result<Json<Value>, ApiError> {
    type Row = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    );
    let sql = if q.unread {
        "SELECT id, kind, title, body, params, agent_id, subject_type, subject_id, approval_id, read_at, created_at
         FROM notifications WHERE company_id = ? AND read_at IS NULL
         ORDER BY created_at DESC LIMIT ?"
    } else {
        "SELECT id, kind, title, body, params, agent_id, subject_type, subject_id, approval_id, read_at, created_at
         FROM notifications WHERE company_id = ? ORDER BY created_at DESC LIMIT ?"
    };
    let rows: Vec<Row> = sqlx::query_as(sql)
        .bind(&company_id)
        .bind(q.limit.clamp(1, 500))
        .fetch_all(&state.pool)
        .await?;
    let (unread,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notifications WHERE company_id = ? AND read_at IS NULL",
    )
    .bind(&company_id)
    .fetch_one(&state.pool)
    .await?;
    let notifications: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                kind,
                title,
                body,
                params,
                agent_id,
                subject_type,
                subject_id,
                approval_id,
                read_at,
                created_at,
            )| {
                json!({
                    "id": id, "kind": kind, "title": title, "body": body,
                    // Rows written before M16 have no params; the client falls
                    // back to the stored title and body for those.
                    "params": params
                        .and_then(|p: String| serde_json::from_str::<Value>(&p).ok()),
                    "agent_id": agent_id, "subject_type": subject_type, "subject_id": subject_id,
                    "approval_id": approval_id, "read_at": read_at, "created_at": created_at,
                })
            },
        )
        .collect();
    Ok(Json(
        json!({ "notifications": notifications, "unread": unread }),
    ))
}

/// Mark one notification read. Idempotent: the first read wins.
async fn read_notification(
    State(state): State<AppState>,
    Path(notification_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT company_id FROM notifications WHERE id = ?")
            .bind(&notification_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id,)) = row else {
        return Err(ApiError::NotFound("notification"));
    };
    sqlx::query("UPDATE notifications SET read_at = ? WHERE id = ? AND read_at IS NULL")
        .bind(now())
        .bind(&notification_id)
        .execute(&state.pool)
        .await?;
    state.notify(&company_id);
    Ok(Json(json!({ "id": notification_id, "read": true })))
}

/// Clear the badge: mark everything in this company read.
async fn read_all_notifications(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let done = sqlx::query(
        "UPDATE notifications SET read_at = ? WHERE company_id = ? AND read_at IS NULL",
    )
    .bind(now())
    .bind(&company_id)
    .execute(&state.pool)
    .await?;
    state.notify(&company_id);
    Ok(Json(json!({ "read": done.rows_affected() })))
}

// ---------- governance: lifecycle, approvals, budgets, config revisions ----------

/// (company_id, name, title, reports_to, traits, custom_brief, requires_approval)
type AgentConfigRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    i64,
);

/// The full config snapshot of an agent as stored in a revision.
async fn agent_snapshot_by_id(
    conn: &mut sqlx::sqlite::SqliteConnection,
    agent_id: &str,
) -> Result<Option<(String, Value)>, ApiError> {
    let row: Option<AgentConfigRow> = sqlx::query_as(
        "SELECT company_id, name, title, reports_to, traits, custom_brief, requires_approval
             FROM agents WHERE id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((company_id, name, title, reports_to, traits, custom_brief, requires_approval)) = row
    else {
        return Ok(None);
    };
    let traits: Value = serde_json::from_str(&traits)?;
    let snap = crate::governance::agent_snapshot(
        &name,
        title.as_deref(),
        reports_to.as_deref(),
        &traits,
        custom_brief.as_deref(),
        requires_approval != 0,
    );
    Ok(Some((company_id, snap)))
}

/// Change an agent's lifecycle status. `paused`/`terminated` stop it from
/// taking new work; `terminated` is permanent.
async fn set_agent_status(
    state: &AppState,
    agent_id: &str,
    to: &str,
    event: &'static str,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT company_id, status FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((company_id, current)) = row else {
        return Err(ApiError::NotFound("agent"));
    };
    if current == "terminated" {
        return Err(ApiError::Conflict("agent is terminated".into()));
    }
    sqlx::query("UPDATE agents SET status = ? WHERE id = ?")
        .bind(to)
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event,
        &json!({ "agent_id": agent_id }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({ "id": agent_id, "status": to })))
}

async fn pause_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_agent_status(&state, &agent_id, "paused", event_kind::AGENT_PAUSED).await
}

async fn resume_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_agent_status(&state, &agent_id, "active", event_kind::AGENT_RESUMED).await
}

async fn terminate_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_agent_status(
        &state,
        &agent_id,
        "terminated",
        event_kind::AGENT_TERMINATED,
    )
    .await
}

#[derive(Deserialize)]
struct SetApproval {
    requires_approval: bool,
}

/// Toggle the governance gate: whether starting this agent's tasks needs a
/// human approval.
async fn set_requires_approval(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<SetApproval>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let agent: Option<(String,)> = sqlx::query_as("SELECT company_id FROM agents WHERE id = ?")
        .bind(&agent_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((company_id,)) = agent else {
        return Err(ApiError::NotFound("agent"));
    };
    sqlx::query("UPDATE agents SET requires_approval = ? WHERE id = ?")
        .bind(req.requires_approval as i64)
        .bind(&agent_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::CONFIG_REVISED,
        &json!({ "agent_id": agent_id, "requires_approval": req.requires_approval }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(
        json!({ "id": agent_id, "requires_approval": req.requires_approval }),
    ))
}

/// Edit an agent's characterization after hire (ADR-0038 addendum): the same
/// validated `TraitsPatch` the hire takes, applied to the current traits,
/// recorded as a `patch` revision. This is also how a remediable refusal's
/// repair is applied — the interface offers it, the user approves, this acts.
async fn patch_agent_traits(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(patch): Json<TraitsPatch>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let Some((company_id, before)) = agent_snapshot_by_id(&mut tx, &agent_id).await? else {
        return Err(ApiError::NotFound("agent"));
    };
    let current: AgentTraits = serde_json::from_value(before["traits"].clone())?;
    let traits = current.apply(patch);
    validate_traits(&state.config.agent_tools, &traits)?;
    refuse_second_hand(
        &mut tx,
        &state.config.agent_tools,
        &company_id,
        Some(&agent_id),
        &traits.tools,
    )
    .await?;
    sqlx::query("UPDATE agents SET traits = ? WHERE id = ?")
        .bind(serde_json::to_string(&traits)?)
        .bind(&agent_id)
        .execute(&mut *tx)
        .await?;
    if let Some((_, after)) = agent_snapshot_by_id(&mut tx, &agent_id).await? {
        crate::governance::record_revision(
            &mut tx,
            &company_id,
            &agent_id,
            "patch",
            &before,
            &after,
        )
        .await?;
    }
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::CONFIG_REVISED,
        &json!({ "agent_id": agent_id, "via": "traits_patch" }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({ "id": agent_id, "traits": traits })))
}

#[derive(Deserialize)]
struct SetTools {
    /// The whole hand, not a delta: what this agent holds after the call.
    tools: Vec<String>,
}

/// Put tools in the hand of an agent who is already hired — or take them out
/// (ADR-0036). The CEO proposes a team and hires it without tools; the person
/// then grants Blender to the one modeler from the org chart. Same rules as at
/// hire: names are validated against the operator's registry (an unknown one
/// is 400 and nothing changes), the grant is the agent's trait, and the change
/// is a config revision like any other characterization change.
async fn set_agent_tools(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<SetTools>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let Some((company_id, before)) = agent_snapshot_by_id(&mut tx, &agent_id).await? else {
        return Err(ApiError::NotFound("agent"));
    };
    let mut traits: AgentTraits = serde_json::from_value(before["traits"].clone())?;
    let mut hand: Vec<String> = Vec::new();
    for t in req.tools {
        let t = t.trim().to_string();
        if !t.is_empty() && !hand.contains(&t) {
            hand.push(t);
        }
    }
    traits.tools = hand.clone();
    validate_traits(&state.config.agent_tools, &traits)?;
    refuse_second_hand(
        &mut tx,
        &state.config.agent_tools,
        &company_id,
        Some(&agent_id),
        &hand,
    )
    .await?;
    sqlx::query("UPDATE agents SET traits = ? WHERE id = ?")
        .bind(serde_json::to_string(&traits)?)
        .bind(&agent_id)
        .execute(&mut *tx)
        .await?;
    if let Some((_, after)) = agent_snapshot_by_id(&mut tx, &agent_id).await? {
        crate::governance::record_revision(
            &mut tx,
            &company_id,
            &agent_id,
            "tools",
            &before,
            &after,
        )
        .await?;
    }
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::CONFIG_REVISED,
        &json!({ "agent_id": agent_id, "tools": hand }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(json!({ "id": agent_id, "traits": traits })))
}

#[derive(Deserialize)]
struct SetBudget {
    monthly_budget_cents: i64,
}

/// Raise (or lower) an agent's monthly cap.
///
/// ADR-0022 tells the human "raise the cap or wait for the new month" when a
/// turn is refused — which needs somewhere to raise it. Recorded as a config
/// revision like any other characterization change, so the change to a
/// governance control is itself governed and roll-backable (ADR-0012).
async fn set_agent_budget(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<SetBudget>,
) -> Result<Json<Value>, ApiError> {
    /// (company_id, name, traits, title, custom_brief, requires_approval)
    type BudgetAgentRow = (String, String, String, Option<String>, Option<String>, i64);

    if req.monthly_budget_cents < 0 {
        return Err(ApiError::Invalid("a budget cannot be negative".into()));
    }
    let mut tx = state.write_tx().await?;
    let agent: Option<BudgetAgentRow> = sqlx::query_as(
        "SELECT company_id, name, traits, title, custom_brief, requires_approval
         FROM agents WHERE id = ?",
    )
    .bind(&agent_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((company_id, name, traits_json, title, brief, requires_approval)) = agent else {
        return Err(ApiError::NotFound("agent"));
    };
    let mut traits: AgentTraits = serde_json::from_str(&traits_json)?;
    let before = serde_json::to_value(&traits)?;
    traits.monthly_budget_cents = req.monthly_budget_cents;
    let after = serde_json::to_value(&traits)?;
    sqlx::query("UPDATE agents SET traits = ? WHERE id = ?")
        .bind(serde_json::to_string(&traits)?)
        .bind(&agent_id)
        .execute(&mut *tx)
        .await?;
    let snapshot = |t: &Value| {
        crate::governance::agent_snapshot(
            &name,
            title.as_deref(),
            None,
            t,
            brief.as_deref(),
            requires_approval != 0,
        )
    };
    crate::governance::record_revision(
        &mut tx,
        &company_id,
        &agent_id,
        "budget",
        &snapshot(&before),
        &snapshot(&after),
    )
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::CONFIG_REVISED,
        &json!({ "agent_id": agent_id, "monthly_budget_cents": req.monthly_budget_cents }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(
        json!({ "id": agent_id, "monthly_budget_cents": req.monthly_budget_cents }),
    ))
}

async fn list_approvals(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // Who decided is read off the audit chain, never off a second column
    // (M25): the actor has ridden inside every hashed payload since M24, and
    // a column beside it could only ever drift from the chain.
    type Row = (
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, type, status, summary, decision_note, created_at, decided_at,
                (SELECT u.name FROM audit_events e
                   JOIN users u ON u.id = json_extract(e.payload, '$.actor')
                  WHERE e.kind = 'approval.decided'
                    AND json_extract(e.payload, '$.approval_id') = approvals.id
                  ORDER BY e.seq DESC LIMIT 1) AS decided_by
             FROM approvals WHERE company_id = ? ORDER BY created_at DESC",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let approvals: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, kind, status, summary, note, created_at, decided_at, decided_by)| {
                json!({
                    "id": id,
                    "type": kind,
                    "status": status,
                    "summary": summary,
                    "decision_note": note,
                    "created_at": created_at,
                    "decided_at": decided_at,
                    "decided_by": decided_by,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "approvals": approvals })))
}

#[derive(Deserialize)]
struct DecideApproval {
    /// "approve" or "reject".
    decision: String,
    note: Option<String>,
}

/// Decide a pending approval. Approving a `task_start` runs the gated start
/// (bypassing the gate this time); rejecting leaves the task untouched.
async fn decide_approval(
    State(state): State<AppState>,
    Path(approval_id): Path<String>,
    Json(req): Json<DecideApproval>,
) -> Result<Json<Value>, ApiError> {
    let approve = match req.decision.as_str() {
        "approve" => true,
        "reject" => false,
        _ => {
            return Err(ApiError::Invalid(
                "decision must be approve or reject".into(),
            ));
        }
    };
    let row: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT company_id, type, status, payload FROM approvals WHERE id = ?")
            .bind(&approval_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, kind, status, payload)) = row else {
        return Err(ApiError::NotFound("approval"));
    };
    if status != "pending" {
        return Err(ApiError::Conflict(format!("approval already {status}")));
    }

    let mut tx = state.write_tx().await?;
    sqlx::query("UPDATE approvals SET status = ?, decision_note = ?, decided_at = ? WHERE id = ?")
        .bind(if approve { "approved" } else { "rejected" })
        .bind(&req.note)
        .bind(now())
        .bind(&approval_id)
        .execute(&mut *tx)
        .await?;
    // The decision is the read: an ask that has been answered cannot stay
    // "unread" (measured: the bell said 6 with nothing left to do — five of
    // them were asks whose approvals were long decided, approved from a
    // toast or by someone else).
    sqlx::query("UPDATE notifications SET read_at = ? WHERE approval_id = ? AND read_at IS NULL")
        .bind(now())
        .bind(&approval_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::APPROVAL_DECIDED,
        &json!({ "approval_id": approval_id, "decision": req.decision }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);

    // Carry out an approved task_start.
    let mut result =
        json!({ "id": approval_id, "status": if approve { "approved" } else { "rejected" } });
    if approve && kind == "task_start" {
        let p: Value = serde_json::from_str(&payload)?;
        if let (Some(task_id), Some(agent_id)) = (p["task_id"].as_str(), p["agent_id"].as_str()) {
            match crate::runner::start_task(&state, task_id, agent_id, true).await? {
                crate::runner::StartResult::Started(o) => {
                    result["session_id"] = json!(o.session_id);
                }
                crate::runner::StartResult::ApprovalRequired { .. } => {}
            }
        }
    }
    // A team the CEO drew up (M15): approving hires everyone still on the list,
    // rejecting tells the CEO why so it does not propose the same shape again.
    if kind == "org_proposal" {
        let p: Value = serde_json::from_str(&payload)?;
        if let Some(proposal_id) = p["proposal_id"].as_str() {
            if approve {
                let hired = crate::org::accept(&state, proposal_id).await?;
                result["hired"] = json!(hired.len());
            } else {
                crate::org::reject(&state, proposal_id, req.note.as_deref()).await?;
            }
            result["proposal_id"] = json!(proposal_id);
        }
    }
    // A meeting an agent asked for: approving opens the room, rejecting closes
    // the request and tells the agent that raised it (ADR-0020).
    if kind == "meeting_request" {
        let p: Value = serde_json::from_str(&payload)?;
        if let Some(meeting_id) = p["meeting_id"].as_str() {
            if approve {
                crate::meeting::approve(&state, meeting_id).await?;
            } else {
                crate::meeting::decline(&state, meeting_id, req.note.as_deref()).await?;
            }
            result["meeting_id"] = json!(meeting_id);
        }
    }
    Ok(Json(result))
}

async fn list_revisions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, source, changed_keys, after_config, created_at
         FROM agent_config_revisions WHERE agent_id = ? ORDER BY created_at DESC",
    )
    .bind(&agent_id)
    .fetch_all(&state.pool)
    .await?;
    let revisions = rows
        .into_iter()
        .map(|(id, source, changed, after, created_at)| {
            Ok(json!({
                "id": id,
                "source": source,
                "changed_keys": serde_json::from_str::<Value>(&changed).unwrap_or(json!([])),
                "config": serde_json::from_str::<Value>(&after)?,
                "created_at": created_at,
            }))
        })
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(Json(json!({ "revisions": revisions })))
}

#[derive(Deserialize)]
struct Rollback {
    revision_id: String,
}

/// Roll an agent's config back to the state a past revision produced. Appends
/// a new `rollback` revision — history is never rewritten.
async fn rollback_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<Rollback>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.write_tx().await?;
    let target: Option<(String, String)> = sqlx::query_as(
        "SELECT company_id, after_config FROM agent_config_revisions WHERE id = ? AND agent_id = ?",
    )
    .bind(&req.revision_id)
    .bind(&agent_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((company_id, after_config)) = target else {
        return Err(ApiError::NotFound("revision"));
    };
    let cfg: Value = serde_json::from_str(&after_config)?;

    let before = match agent_snapshot_by_id(&mut tx, &agent_id).await? {
        Some((_, snap)) => snap,
        None => return Err(ApiError::NotFound("agent")),
    };

    let name = cfg["name"].as_str().unwrap_or("");
    let title = cfg["title"].as_str();
    let reports_to = cfg["reports_to"].as_str();
    let traits = serde_json::to_string(&cfg["traits"])?;
    let custom_brief = cfg["custom_brief"].as_str();
    let requires_approval = cfg["requires_approval"].as_bool().unwrap_or(false);
    sqlx::query(
        "UPDATE agents SET name = ?, title = ?, reports_to = ?, traits = ?, custom_brief = ?, requires_approval = ? WHERE id = ?",
    )
    .bind(name)
    .bind(title)
    .bind(reports_to)
    .bind(&traits)
    .bind(custom_brief)
    .bind(requires_approval as i64)
    .bind(&agent_id)
    .execute(&mut *tx)
    .await?;
    crate::governance::record_revision(&mut tx, &company_id, &agent_id, "rollback", &before, &cfg)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::CONFIG_ROLLED_BACK,
        &json!({ "agent_id": agent_id, "to_revision": req.revision_id }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(Json(
        json!({ "id": agent_id, "rolled_back_to": req.revision_id }),
    ))
}

/// Per-agent month-to-date budget usage for the company (for the UI).
async fn budget_summary(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let window = crate::governance::month_window_start();
    let agents: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, name, traits FROM agents WHERE company_id = ? AND status != 'terminated' ORDER BY created_at")
            .bind(&company_id)
            .fetch_all(&state.pool)
            .await?;
    let mut out = Vec::with_capacity(agents.len());
    for (id, name, traits) in agents {
        let budget = serde_json::from_str::<Value>(&traits)
            .ok()
            .and_then(|v| v.get("monthly_budget_cents").and_then(Value::as_i64))
            .unwrap_or(0);
        // Through governance, not a private copy: this view used to run its own
        // `reserved` query, which now would not see conversational turns at all
        // (ADR-0022) — a summary that disagrees with the gate is worse than no
        // summary.
        let mut conn = state.pool.acquire().await?;
        let spent = (crate::governance::spent_cents(&mut conn, &id, &window).await?,);
        let reserved = (crate::governance::reserved_cents(&mut conn, &id).await?,);
        // What the next run will reserve, and on how much it rests (M26):
        // the person steering by the bar should know whether the number is
        // the agent's or the default's.
        let default = state.config.start_estimate_cents;
        let task = crate::governance::estimate_cents(
            &mut conn,
            &id,
            crate::governance::SpendKind::Task,
            default,
        )
        .await?;
        let turn = crate::governance::estimate_cents(
            &mut conn,
            &id,
            crate::governance::SpendKind::Turn,
            default,
        )
        .await?;
        drop(conn);
        out.push(json!({
            "agent_id": id,
            "name": name,
            "budget_cents": budget,
            "spent_cents": spent.0,
            "reserved_cents": reserved.0,
            "estimates": { "task": task, "turn": turn },
        }));
    }
    Ok(Json(json!({ "budgets": out, "window_start": window })))
}

// ---------- audit ----------

#[derive(Deserialize)]
struct EventsFilter {
    company_id: Option<String>,
}

/// (seq, company_id, task_id, kind, payload, created_at, prev_hash, hash)
type EventRow = (
    i64,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

/// The chain's feed, with the actor resolved to a name beside the payload
/// (M25). The payload itself is returned verbatim -- it is what the hash
/// covers -- and the name rides outside it, a convenience, never a claim.
const EVENT_COLUMNS: &str = "e.seq, e.company_id, e.task_id, e.kind, e.payload, e.created_at,
     e.prev_hash, e.hash, u.name
     FROM audit_events e LEFT JOIN users u ON u.id = json_extract(e.payload, '$.actor')";

async fn list_events(
    State(state): State<AppState>,
    Query(filter): Query<EventsFilter>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<EventRow> = match &filter.company_id {
        Some(company_id) => {
            sqlx::query_as(&format!(
                "SELECT {EVENT_COLUMNS} WHERE e.company_id = ? ORDER BY e.seq"
            ))
            .bind(company_id)
            .fetch_all(&state.pool)
            .await?
        }
        None => {
            sqlx::query_as(&format!("SELECT {EVENT_COLUMNS} ORDER BY e.seq"))
                .fetch_all(&state.pool)
                .await?
        }
    };
    let events = rows
        .into_iter()
        .map(
            |(seq, company_id, task_id, kind, payload, created_at, prev_hash, hash, actor_name)| {
                let payload: Value = serde_json::from_str(&payload)?;
                Ok(json!({
                    "seq": seq,
                    "company_id": company_id,
                    "task_id": task_id,
                    "kind": kind,
                    "payload": payload,
                    "created_at": created_at,
                    "prev_hash": prev_hash,
                    "hash": hash,
                    "actor_name": actor_name,
                }))
            },
        )
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(Json(json!({ "events": events })))
}

async fn verify_chain(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let report = audit::verify(&state.pool).await?;
    Ok(Json(serde_json::to_value(report)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outside a container the old message is still the right one: there is no
    /// mount point to name, and inventing advice about one would be noise.
    #[test]
    fn without_a_mount_point_the_message_stays_plain() {
        let said = unreachable_cwd(None, "/nope");
        assert_eq!(said, "cwd '/nope' is not a directory");
    }

    /// The whole point: a host path is not wrong so much as *unreachable*, and
    /// the message has to say which paths are reachable instead.
    #[test]
    fn in_a_container_the_message_names_what_is_mounted() {
        let repos = std::env::temp_dir().join(format!("overmind-repos-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(repos.join("my-project")).expect("mount point");
        std::fs::create_dir_all(repos.join("another")).expect("second repo");
        // A loose file is not a repository anyone can point a workspace at.
        std::fs::write(repos.join("notes.txt"), b"x").expect("stray file");

        let said = unreachable_cwd(Some(&repos), "/Users/me/code/my-project");
        let _ = std::fs::remove_dir_all(&repos);

        assert!(said.contains("/Users/me/code/my-project"), "{said}");
        assert!(said.contains("container"), "{said}");
        assert!(said.contains("my-project"), "{said}");
        assert!(said.contains("another"), "{said}");
        assert!(
            !said.contains("notes.txt"),
            "only directories are workspaces: {said}"
        );
    }

    /// "Nothing is mounted" and "you named the wrong path" are the two things
    /// that actually happen, and they need different advice.
    #[test]
    fn an_empty_mount_point_says_so_and_shows_how_to_fill_it() {
        let repos = std::env::temp_dir().join(format!("overmind-empty-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&repos).expect("mount point");

        let said = unreachable_cwd(Some(&repos), "/Users/me/code/thing");
        let _ = std::fs::remove_dir_all(&repos);

        assert!(said.contains("Nothing is mounted"), "{said}");
        assert!(said.contains("docker-compose.yml"), "{said}");
    }
}
