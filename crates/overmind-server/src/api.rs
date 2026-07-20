use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit;
use crate::db::AppState;
use crate::domain::{AgentTraits, TaskStatus, TraitsPatch, event_kind};

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/companies", post(create_company).get(list_companies))
        .route("/archetypes", get(list_archetypes))
        .route(
            "/companies/{company_id}/agents",
            post(hire_agent).get(list_agents),
        )
        .route("/companies/{company_id}/projects", post(create_project))
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
        .route("/agents/{agent_id}/wakeup", post(request_wakeup))
        .route("/sessions/{session_id}", get(get_session))
        .route("/sessions/{session_id}/diff", get(get_session_diff))
        .route("/audit/events", get(list_events))
        .route("/audit/verify", get(verify_chain))
        .with_state(state)
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<crate::runner::RunnerError> for ApiError {
    fn from(e: crate::runner::RunnerError) -> Self {
        use crate::runner::RunnerError;
        match e {
            RunnerError::NotFound(what) => ApiError::NotFound(what),
            RunnerError::Invalid(msg) => ApiError::Invalid(msg),
            RunnerError::Conflict => ApiError::Conflict(e.to_string()),
            RunnerError::Git(msg) => ApiError::Internal(msg.into()),
            RunnerError::Db(e) => ApiError::Internal(Box::new(e)),
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
            ApiError::Internal(source) => {
                // The client gets an opaque error; the operator gets the cause.
                eprintln!("internal error: {source}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ---------- companies ----------

#[derive(Deserialize)]
struct CreateCompany {
    name: String,
}

async fn create_company(
    State(state): State<AppState>,
    Json(req): Json<CreateCompany>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::Invalid("company name must not be empty".into()));
    }
    let (id, created_at) = (new_id(), now());
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO companies (id, name, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(req.name.trim())
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&id),
        None,
        event_kind::COMPANY_CREATED,
        &json!({ "name": req.name.trim() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "name": req.name.trim(), "created_at": created_at })),
    ))
}

async fn list_companies(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, name, created_at FROM companies ORDER BY created_at")
            .fetch_all(&state.pool)
            .await?;
    let companies: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, created_at)| json!({ "id": id, "name": name, "created_at": created_at }))
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

// ---------- agents ----------

#[derive(Deserialize)]
struct HireAgent {
    name: String,
    /// Archetype slug (UX Level 1 "pick").
    archetype: String,
    /// Structured overrides on the archetype defaults (UX Level 2 "tune").
    #[serde(default)]
    traits: TraitsPatch,
    /// Free-form additions (UX Level 3 "expert") — additive only.
    custom_brief: Option<String>,
    role_id: Option<String>,
}

async fn hire_agent(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<HireAgent>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::Invalid("agent name must not be empty".into()));
    }
    let mut tx = state.pool.begin().await?;

    let company: Option<(String,)> = sqlx::query_as("SELECT id FROM companies WHERE id = ?")
        .bind(&company_id)
        .fetch_optional(&mut *tx)
        .await?;
    if company.is_none() {
        return Err(ApiError::NotFound("company"));
    }

    let archetype: Option<(String, String)> =
        sqlx::query_as("SELECT id, default_traits FROM archetypes WHERE slug = ?")
            .bind(&req.archetype)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((archetype_id, default_traits)) = archetype else {
        return Err(ApiError::NotFound("archetype"));
    };

    let defaults: AgentTraits = serde_json::from_str(&default_traits)?;
    let traits = defaults.apply(req.traits);
    let traits_json = serde_json::to_string(&traits)?;

    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO agents (id, company_id, role_id, archetype_id, name, traits, custom_brief, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?)",
    )
    .bind(&id)
    .bind(&company_id)
    .bind(&req.role_id)
    .bind(&archetype_id)
    .bind(req.name.trim())
    .bind(&traits_json)
    .bind(&req.custom_brief)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::AGENT_HIRED,
        &json!({
            "agent_id": id,
            "name": req.name.trim(),
            "archetype": req.archetype,
            "traits": serde_json::to_value(&traits)?,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "company_id": company_id,
            "name": req.name.trim(),
            "archetype": req.archetype,
            "traits": serde_json::to_value(&traits)?,
            "custom_brief": req.custom_brief,
            "status": "active",
            "created_at": created_at,
        })),
    ))
}

async fn list_agents(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT a.id, a.name, a.traits, a.custom_brief, a.status, ar.slug
         FROM agents a JOIN archetypes ar ON ar.id = a.archetype_id
         WHERE a.company_id = ? ORDER BY a.created_at",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let agents = rows
        .into_iter()
        .map(|(id, name, traits, custom_brief, status, archetype)| {
            let traits: Value = serde_json::from_str(&traits)?;
            Ok(json!({
                "id": id,
                "name": name,
                "archetype": archetype,
                "traits": traits,
                "custom_brief": custom_brief,
                "status": status,
            }))
        })
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    Ok(Json(json!({ "agents": agents })))
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
    let mut tx = state.pool.begin().await?;
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
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "company_id": company_id, "title": req.title.trim(), "created_at": created_at }),
        ),
    ))
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
    let mut tx = state.pool.begin().await?;
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
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "project_id": project_id, "title": req.title.trim(), "created_at": created_at }),
        ),
    ))
}

// ---------- tasks ----------

#[derive(Deserialize)]
struct CreateTask {
    title: String,
    #[serde(default)]
    description: String,
    goal_id: Option<String>,
    priority: Option<String>,
}

async fn create_task(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Json(req): Json<CreateTask>,
) -> Result<impl IntoResponse, ApiError> {
    if req.title.trim().is_empty() {
        return Err(ApiError::Invalid("task title must not be empty".into()));
    }
    let priority = req.priority.as_deref().unwrap_or("medium");
    if !matches!(priority, "low" | "medium" | "high" | "urgent") {
        return Err(ApiError::Invalid(format!("unknown priority '{priority}'")));
    }
    let mut tx = state.pool.begin().await?;
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
        "INSERT INTO tasks (id, company_id, goal_id, title, description, status, priority, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'backlog', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&company_id)
    .bind(&req.goal_id)
    .bind(req.title.trim())
    .bind(&req.description)
    .bind(priority)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(&id),
        event_kind::TASK_CREATED,
        &json!({ "title": req.title.trim(), "goal_id": req.goal_id, "priority": priority }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "company_id": company_id,
            "goal_id": req.goal_id,
            "title": req.title.trim(),
            "status": "backlog",
            "priority": priority,
            "created_at": created_at,
        })),
    ))
}

/// (id, goal_id, title, status, priority, assignee_agent_id, updated_at)
type TaskRow = (
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    String,
);

async fn list_tasks(
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<TaskRow> = sqlx::query_as(
        "SELECT id, goal_id, title, status, priority, assignee_agent_id, updated_at
         FROM tasks WHERE company_id = ? ORDER BY created_at",
    )
    .bind(&company_id)
    .fetch_all(&state.pool)
    .await?;
    let tasks: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, goal_id, title, status, priority, assignee, updated_at)| {
                json!({
                    "id": id,
                    "goal_id": goal_id,
                    "title": title,
                    "status": status,
                    "priority": priority,
                    "assignee_agent_id": assignee,
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

    let mut tx = state.pool.begin().await?;
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

async fn create_workspace(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateWorkspace>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::Invalid("workspace name must not be empty".into()));
    }
    if !std::path::Path::new(&req.cwd).is_dir() {
        return Err(ApiError::Invalid(format!(
            "cwd '{}' is not a directory",
            req.cwd
        )));
    }
    let is_primary = req.is_primary.unwrap_or(true);
    let mut tx = state.pool.begin().await?;
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
    let outcome = crate::runner::start_task(&state, &task_id, &req.agent_id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "session_id": outcome.session_id,
            "branch": outcome.branch,
            "workspace_path": outcome.workspace_path,
        })),
    ))
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
    let mut tx = state.pool.begin().await?;
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
        "id": session_id,
        "task_id": task_id,
        "agent_id": agent_id,
        "adapter_type": adapter_type,
        "status": status,
        "branch": branch,
        "workspace_path": workspace_path,
        "base_sha": base_sha,
        "output": output,
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
);

async fn list_events(
    State(state): State<AppState>,
    Query(filter): Query<EventsFilter>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<EventRow> = match &filter.company_id {
        Some(company_id) => {
            sqlx::query_as(
                "SELECT seq, company_id, task_id, kind, payload, created_at, prev_hash, hash
                 FROM audit_events WHERE company_id = ? ORDER BY seq",
            )
            .bind(company_id)
            .fetch_all(&state.pool)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT seq, company_id, task_id, kind, payload, created_at, prev_hash, hash
                 FROM audit_events ORDER BY seq",
            )
            .fetch_all(&state.pool)
            .await?
        }
    };
    let events = rows
        .into_iter()
        .map(
            |(seq, company_id, task_id, kind, payload, created_at, prev_hash, hash)| {
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
