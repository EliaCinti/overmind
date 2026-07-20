//! Agent execution: atomic task checkout, isolated git worktrees, process
//! supervision with timeouts, session resume, output capture and cost
//! recording.
//!
//! Design follows Paperclip's session model (`agent_task_sessions`,
//! `cost_events`) and Vibe Kanban's worktree-per-run isolation
//! (ADR-0008, ADR-0009).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::audit;
use crate::db::AppState;
use crate::domain::event_kind;
use crate::governance;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("task is not available for checkout")]
    Conflict,
    #[error("{0}")]
    Blocked(String),
    #[error("agent is over its monthly budget")]
    OverBudget,
    #[error("git error: {0}")]
    Git(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// What a start attempt did: actually launched a session, or (when the agent
/// is governance-gated) filed an approval and launched nothing.
pub enum StartResult {
    Started(StartOutcome),
    ApprovalRequired { approval_id: String },
}

pub struct StartOutcome {
    pub session_id: String,
    pub branch: String,
    pub workspace_path: String,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Last 12 hex chars of a UUID — the random tail. UUIDv7's *leading* bytes
/// are a millisecond timestamp, so ids minted in the same millisecond share
/// their prefix; the tail is what actually distinguishes them. Used only for
/// human-readable branch names; uniqueness rides on the full session id.
fn tag(id: &str) -> &str {
    let trimmed = id.trim_end_matches('-');
    trimmed
        .get(trimmed.len().saturating_sub(12)..)
        .unwrap_or(id)
}

async fn git(cwd: &Path, args: &[&str]) -> Result<String, RunnerError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| RunnerError::Git(format!("failed to run git: {e}")))?;
    if !out.status.success() {
        return Err(RunnerError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Check a task out for an agent and launch the session.
///
/// The checkout is a single conditional UPDATE (`status = 'todo'` →
/// `in_progress`): of N concurrent attempts exactly one affects a row and
/// wins; the others get `Conflict`. Budget enforcement, checkout, session
/// creation and audit events commit in one transaction; the process itself
/// runs afterwards in a background task that finalizes the session.
///
/// Governance (ADR-0012): a `requires_approval` agent files an approval and
/// launches nothing unless `bypass_approval` (i.e. the approval was granted);
/// a start that would push the agent past its monthly budget is stopped here,
/// atomically, and recorded as a budget incident.
pub async fn start_task(
    state: &AppState,
    task_id: &str,
    agent_id: &str,
    bypass_approval: bool,
) -> Result<StartResult, RunnerError> {
    // Resolve task -> goal -> project -> primary workspace.
    let task: Option<(String, Option<String>, String, String, String)> = sqlx::query_as(
        "SELECT company_id, goal_id, title, description, status FROM tasks WHERE id = ?",
    )
    .bind(task_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((company_id, goal_id, title, description, status)) = task else {
        return Err(RunnerError::NotFound("task"));
    };
    if status != "todo" {
        return Err(RunnerError::Conflict);
    }
    let Some(goal_id) = goal_id else {
        return Err(RunnerError::Invalid(
            "task has no goal: attach it to a project with a workspace first".into(),
        ));
    };
    let workspace: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT w.cwd, w.default_ref FROM project_workspaces w
         JOIN goals g ON g.project_id = w.project_id
         WHERE g.id = ? AND w.is_primary = 1",
    )
    .bind(&goal_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((repo_cwd, default_ref)) = workspace else {
        return Err(RunnerError::Invalid(
            "the task's project has no primary workspace".into(),
        ));
    };

    let agent: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT traits, status, requires_approval FROM agents WHERE id = ? AND company_id = ?",
    )
    .bind(agent_id)
    .bind(&company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((agent_traits, agent_status, requires_approval)) = agent else {
        return Err(RunnerError::NotFound("agent"));
    };
    if agent_status != "active" {
        return Err(RunnerError::Blocked(format!("agent is {agent_status}")));
    }

    // Governance gate: file an approval and launch nothing.
    if requires_approval != 0 && !bypass_approval {
        let approval_id = uuid::Uuid::now_v7().to_string();
        let mut tx = state.pool.begin().await?;
        sqlx::query(
            "INSERT INTO approvals (id, company_id, type, status, payload, summary, created_at)
             VALUES (?, ?, 'task_start', 'pending', ?, ?, ?)",
        )
        .bind(&approval_id)
        .bind(&company_id)
        .bind(json!({ "task_id": task_id, "agent_id": agent_id }).to_string())
        .bind(format!("Start \"{title}\""))
        .bind(now())
        .execute(&mut *tx)
        .await?;
        audit::append(
            &mut tx,
            Some(&company_id),
            Some(task_id),
            event_kind::APPROVAL_REQUESTED,
            &json!({ "approval_id": approval_id, "agent_id": agent_id, "type": "task_start" }),
        )
        .await?;
        tx.commit().await?;
        state.notify(&company_id);
        return Ok(StartResult::ApprovalRequired { approval_id });
    }

    let budget = trait_budget_cents(&agent_traits);
    let estimate = state.config.start_estimate_cents;

    let session_id = uuid::Uuid::now_v7().to_string();
    // Branch uniqueness comes from the full session id (globally unique); the
    // task tag is only there to make the branch human-recognizable.
    let branch = format!("overmind/task-{}-sess-{}", tag(task_id), session_id);
    let worktree_dir = state
        .config
        .data_dir
        .join("worktrees")
        .join(&session_id)
        .to_string_lossy()
        .into_owned();

    let mut tx = state.pool.begin().await?;

    // Budget check, atomic with checkout. spent (this month) + reserved
    // (in-flight) + this run's estimate must fit under the cap.
    if budget > 0 {
        let window = governance::month_window_start();
        let spent = governance::spent_cents(&mut tx, agent_id, &window).await?;
        let reserved = governance::reserved_cents(&mut tx, agent_id).await?;
        if spent + reserved + estimate > budget {
            // Record the incident and commit that alone; the task is untouched.
            sqlx::query(
                "INSERT INTO budget_incidents (id, company_id, agent_id, window_start, threshold_type, amount_limit, amount_observed, status, created_at)
                 VALUES (?, ?, ?, ?, 'hard', ?, ?, 'open', ?)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&company_id)
            .bind(agent_id)
            .bind(&window)
            .bind(budget)
            .bind(spent + reserved + estimate)
            .bind(now())
            .execute(&mut *tx)
            .await?;
            audit::append(
                &mut tx,
                Some(&company_id),
                Some(task_id),
                event_kind::BUDGET_BLOCKED,
                &json!({ "agent_id": agent_id, "limit_cents": budget, "observed_cents": spent + reserved + estimate }),
            )
            .await?;
            tx.commit().await?;
            state.notify(&company_id);
            return Err(RunnerError::OverBudget);
        }
    }

    // Atomic checkout: exactly one concurrent caller wins this UPDATE.
    let checked_out =
        sqlx::query("UPDATE tasks SET status = 'in_progress', assignee_agent_id = ?, updated_at = ? WHERE id = ? AND status = 'todo'")
            .bind(agent_id)
            .bind(now())
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
    if checked_out.rows_affected() != 1 {
        return Err(RunnerError::Conflict);
    }
    sqlx::query(
        "INSERT INTO agent_task_sessions (id, task_id, agent_id, adapter_type, status, branch, workspace_path, reserved_cents, created_at)
         VALUES (?, ?, ?, 'claude_code', 'queued', ?, ?, ?, ?)",
    )
    .bind(&session_id)
    .bind(task_id)
    .bind(agent_id)
    .bind(&branch)
    .bind(&worktree_dir)
    .bind(estimate)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(task_id),
        event_kind::TASK_TRANSITIONED,
        &json!({ "from": "todo", "to": "in_progress", "assignee_agent_id": agent_id }),
    )
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(task_id),
        event_kind::SESSION_STARTED,
        &json!({ "session_id": session_id, "agent_id": agent_id, "branch": branch }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);

    let ctx = SessionContext {
        state: state.clone(),
        session_id: session_id.clone(),
        task_id: task_id.to_string(),
        company_id,
        worktree_dir: PathBuf::from(&worktree_dir),
        title,
        description,
        agent_traits,
    };
    let spec = WorktreeSpec {
        repo_cwd: PathBuf::from(repo_cwd),
        default_ref,
        branch: branch.clone(),
    };
    register(state, &session_id);
    tokio::spawn(async move {
        run_session(ctx, Mode::Fresh(spec)).await;
    });

    Ok(StartResult::Started(StartOutcome {
        session_id,
        branch,
        workspace_path: worktree_dir,
    }))
}

/// The monthly budget cap from an agent's serialized traits (0 = uncapped).
fn trait_budget_cents(traits_json: &str) -> i64 {
    serde_json::from_str::<Value>(traits_json)
        .ok()
        .and_then(|v| v.get("monthly_budget_cents").and_then(Value::as_i64))
        .unwrap_or(0)
}

/// Resume a session that is marked queued/running in the DB but has no live
/// runner in this process (server restart, crashed runner). Called by the
/// heartbeat scheduler.
pub async fn resume_session(state: &AppState, session_id: &str) -> Result<(), RunnerError> {
    let session: Option<(String, String, String)> = sqlx::query_as(
        "SELECT task_id, agent_id, workspace_path FROM agent_task_sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((task_id, agent_id, workspace_path)) = session else {
        return Err(RunnerError::NotFound("session"));
    };
    let task: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT company_id, title, description, status FROM tasks WHERE id = ?")
            .bind(&task_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, title, description, task_status)) = task else {
        return Err(RunnerError::NotFound("task"));
    };
    let agent_traits: Option<(String,)> = sqlx::query_as("SELECT traits FROM agents WHERE id = ?")
        .bind(&agent_id)
        .fetch_optional(&state.pool)
        .await?;
    let agent_traits = agent_traits.map(|(t,)| t).unwrap_or_default();

    let ctx = SessionContext {
        state: state.clone(),
        session_id: session_id.to_string(),
        task_id: task_id.clone(),
        company_id: company_id.clone(),
        worktree_dir: PathBuf::from(&workspace_path),
        title,
        description,
        agent_traits,
    };

    // A session whose task is no longer in progress, or whose worktree is
    // gone, cannot be resumed: fail it and release the task.
    if task_status != "in_progress" || !ctx.worktree_dir.is_dir() {
        let error = if task_status != "in_progress" {
            format!("cannot resume: task is '{task_status}'")
        } else {
            "cannot resume: worktree is missing".to_string()
        };
        let release = task_status == "in_progress";
        finalize(&ctx, Outcome::Infra { error, release }).await?;
        return Ok(());
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE agent_task_sessions SET status = 'running', resumed_count = resumed_count + 1 WHERE id = ?",
    )
    .bind(session_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(&task_id),
        event_kind::SESSION_RESUMED,
        &json!({ "session_id": session_id, "agent_id": agent_id }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);

    register(state, session_id);
    tokio::spawn(async move {
        run_session(ctx, Mode::Resume).await;
    });
    Ok(())
}

fn register(state: &AppState, session_id: &str) {
    if let Ok(mut running) = state.running.lock() {
        running.insert(session_id.to_string());
    }
}

fn deregister(state: &AppState, session_id: &str) {
    if let Ok(mut running) = state.running.lock() {
        running.remove(session_id);
    }
}

pub(crate) struct SessionContext {
    state: AppState,
    session_id: String,
    task_id: String,
    company_id: String,
    worktree_dir: PathBuf,
    title: String,
    description: String,
    agent_traits: String,
}

pub(crate) struct WorktreeSpec {
    repo_cwd: PathBuf,
    default_ref: Option<String>,
    branch: String,
}

pub(crate) enum Mode {
    Fresh(WorktreeSpec),
    Resume,
}

enum Outcome {
    Success { output: String },
    AgentFailure { output: String, exit_code: i32 },
    TimedOut { timeout_secs: u64 },
    Infra { error: String, release: bool },
}

async fn run_session(ctx: SessionContext, mode: Mode) {
    let outcome = execute(&ctx, mode).await;
    if let Err(e) = finalize(&ctx, outcome).await {
        eprintln!("session {}: failed to finalize: {e}", ctx.session_id);
    }
    deregister(&ctx.state, &ctx.session_id);
}

async fn execute(ctx: &SessionContext, mode: Mode) -> Outcome {
    let resume = matches!(mode, Mode::Resume);
    if let Mode::Fresh(spec) = &mode
        && let Err(e) = prepare_worktree(ctx, spec).await
    {
        return Outcome::Infra {
            error: e.to_string(),
            release: false,
        };
    }
    run_process(ctx, resume).await
}

async fn prepare_worktree(ctx: &SessionContext, spec: &WorktreeSpec) -> Result<(), RunnerError> {
    if let Some(parent) = ctx.worktree_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| RunnerError::Git(format!("cannot create worktree dir: {e}")))?;
    }
    let worktree = ctx.worktree_dir.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "add", worktree.as_str(), "-b", &spec.branch];
    if let Some(r) = &spec.default_ref {
        args.push(r.as_str());
    }
    git(&spec.repo_cwd, &args).await?;
    let base_sha = git(&ctx.worktree_dir, &["rev-parse", "HEAD"]).await?;
    sqlx::query(
        "UPDATE agent_task_sessions SET status = 'running', base_sha = ?, started_at = ? WHERE id = ?",
    )
    .bind(&base_sha)
    .bind(now())
    .bind(&ctx.session_id)
    .execute(&ctx.state.pool)
    .await?;
    Ok(())
}

async fn run_process(ctx: &SessionContext, resume: bool) -> Outcome {
    // The adapter command is configurable (tests use a stub); the default
    // drives the Claude Code CLI headless with a JSON result.
    let agent_cmd =
        ctx.state.config.agent_cmd.clone().unwrap_or_else(|| {
            "claude -p \"$OVERMIND_TASK_PROMPT\" --output-format json".to_string()
        });
    // Load what the organization remembers about this kind of work, and put
    // it in front of the agent (and in an env var). A no-op when memory is off.
    let memory_context = ctx
        .state
        .memory
        .get_context(
            &ctx.worktree_dir.to_string_lossy(),
            &format!("{}\n{}", ctx.title, ctx.description),
        )
        .await;
    let memory_block = memory_context
        .as_deref()
        .map(|m| {
            format!(
                "\n\nWhat the organization remembers (use it, don't repeat past mistakes):\n{m}"
            )
        })
        .unwrap_or_default();

    let prompt = if resume {
        format!(
            "You are resuming interrupted work on the task \"{}\".\n\n{}{}\n\nThe current directory may contain partial work from the interrupted run — inspect it first, then finish the task. Leave the changes uncommitted.",
            ctx.title, ctx.description, memory_block
        )
    } else {
        format!(
            "You are working on the task \"{}\".\n\n{}{}\n\nWork in the current directory. When done, leave the changes uncommitted.",
            ctx.title, ctx.description, memory_block
        )
    };

    let adapter_session_id: Option<String> = if resume {
        sqlx::query_as::<_, (Option<String>,)>(
            "SELECT adapter_session_id FROM agent_task_sessions WHERE id = ?",
        )
        .bind(&ctx.session_id)
        .fetch_optional(&ctx.state.pool)
        .await
        .ok()
        .flatten()
        .and_then(|(s,)| s)
    } else {
        None
    };

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&agent_cmd)
        .current_dir(&ctx.worktree_dir)
        .env("OVERMIND_TASK_PROMPT", &prompt)
        .env("OVERMIND_TASK_TITLE", &ctx.title)
        .env("OVERMIND_TASK_DESCRIPTION", &ctx.description)
        .env("OVERMIND_AGENT_TRAITS", &ctx.agent_traits)
        .env(
            "OVERMIND_MEMORY_CONTEXT",
            memory_context.as_deref().unwrap_or(""),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(sid) = &adapter_session_id {
        cmd.env("OVERMIND_RESUME_SESSION_ID", sid);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome::Infra {
                error: format!("failed to spawn agent: {e}"),
                release: false,
            };
        }
    };

    let timeout_secs = ctx.state.config.session_timeout_secs;
    let waited =
        tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await;
    match waited {
        Err(_elapsed) => Outcome::TimedOut { timeout_secs },
        Ok(Err(e)) => Outcome::Infra {
            error: format!("failed to read agent output: {e}"),
            release: false,
        },
        Ok(Ok(out)) => {
            let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.trim().is_empty() {
                output.push_str("\n--- stderr ---\n");
                output.push_str(stderr.trim());
            }
            let exit_code = out.status.code().unwrap_or(-1);
            if exit_code == 0 {
                Outcome::Success { output }
            } else {
                Outcome::AgentFailure { output, exit_code }
            }
        }
    }
}

async fn finalize(ctx: &SessionContext, outcome: Outcome) -> Result<(), RunnerError> {
    struct Final {
        session_status: &'static str,
        output: String,
        exit_code: Option<i32>,
        last_error: Option<String>,
        /// `Some(status)` moves the task; release additionally clears the assignee.
        task_to: &'static str,
        release: bool,
    }
    let f = match outcome {
        Outcome::Success { output } => Final {
            session_status: "completed",
            output,
            exit_code: Some(0),
            last_error: None,
            task_to: "in_review",
            release: false,
        },
        Outcome::AgentFailure { output, exit_code } => Final {
            session_status: "failed",
            output,
            exit_code: Some(exit_code),
            last_error: Some(format!("agent exited with code {exit_code}")),
            task_to: "blocked",
            release: false,
        },
        Outcome::TimedOut { timeout_secs } => Final {
            session_status: "failed",
            output: String::new(),
            exit_code: None,
            last_error: Some(format!("session timed out after {timeout_secs}s")),
            task_to: "todo",
            release: true,
        },
        Outcome::Infra { error, release } => Final {
            session_status: "failed",
            output: String::new(),
            exit_code: None,
            last_error: Some(error),
            task_to: if release { "todo" } else { "blocked" },
            release,
        },
    };

    let mut tx = ctx.state.pool.begin().await?;
    // Releasing the reservation (→ 0): once the run is over, its actual cost
    // is a cost_event and counts as spent; the in-flight reservation is gone.
    sqlx::query(
        "UPDATE agent_task_sessions SET status = ?, output = ?, exit_code = ?, last_error = ?, reserved_cents = 0, finished_at = ? WHERE id = ?",
    )
    .bind(f.session_status)
    .bind(&f.output)
    .bind(f.exit_code)
    .bind(&f.last_error)
    .bind(now())
    .bind(&ctx.session_id)
    .execute(&mut *tx)
    .await?;

    if let Some(adapter_sid) = parse_adapter_session_id(&f.output) {
        sqlx::query("UPDATE agent_task_sessions SET adapter_session_id = ? WHERE id = ?")
            .bind(&adapter_sid)
            .bind(&ctx.session_id)
            .execute(&mut *tx)
            .await?;
    }

    if f.release {
        sqlx::query(
            "UPDATE tasks SET status = 'todo', assignee_agent_id = NULL, updated_at = ? WHERE id = ?",
        )
        .bind(now())
        .bind(&ctx.task_id)
        .execute(&mut *tx)
        .await?;
        audit::append(
            &mut tx,
            Some(&ctx.company_id),
            Some(&ctx.task_id),
            event_kind::TASK_RELEASED,
            &json!({ "from": "in_progress", "to": "todo", "reason": f.last_error }),
        )
        .await?;
    } else {
        sqlx::query("UPDATE tasks SET status = ?, updated_at = ? WHERE id = ?")
            .bind(f.task_to)
            .bind(now())
            .bind(&ctx.task_id)
            .execute(&mut *tx)
            .await?;
        audit::append(
            &mut tx,
            Some(&ctx.company_id),
            Some(&ctx.task_id),
            event_kind::TASK_TRANSITIONED,
            &json!({ "from": "in_progress", "to": f.task_to }),
        )
        .await?;
    }
    audit::append(
        &mut tx,
        Some(&ctx.company_id),
        Some(&ctx.task_id),
        event_kind::SESSION_FINISHED,
        &json!({
            "session_id": ctx.session_id,
            "status": f.session_status,
            "exit_code": f.exit_code,
            "error": f.last_error,
        }),
    )
    .await?;

    // Cost capture: the Claude Code CLI (and our stubs) print a final JSON
    // object with total_cost_usd and usage. Missing/unparseable cost is not
    // an error — the session already carries the full output.
    if let Some(cost) = parse_cost(&f.output) {
        let agent_id: Option<(String,)> =
            sqlx::query_as("SELECT agent_id FROM agent_task_sessions WHERE id = ?")
                .bind(&ctx.session_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((agent_id,)) = agent_id {
            sqlx::query(
                "INSERT INTO cost_events (id, company_id, agent_id, task_id, session_id, provider, model,
                 input_tokens, cached_input_tokens, output_tokens, cost_cents, occurred_at, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&ctx.company_id)
            .bind(&agent_id)
            .bind(&ctx.task_id)
            .bind(&ctx.session_id)
            .bind("anthropic")
            .bind(&cost.model)
            .bind(cost.input_tokens)
            .bind(cost.cached_input_tokens)
            .bind(cost.output_tokens)
            .bind(cost.cost_cents)
            .bind(now())
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    ctx.state.notify(&ctx.company_id);

    // Record what the organization just learned. Best-effort; never fatal.
    if f.session_status == "completed" {
        ctx.state
            .memory
            .store_memory(
                &ctx.title,
                &format!(
                    "Task \"{}\" completed by an agent.\n\n{}",
                    ctx.title, ctx.description
                ),
                &ctx.company_id,
                &["task-completed"],
                "note",
            )
            .await;
    }

    Ok(())
}

struct ParsedCost {
    model: String,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    cost_cents: i64,
}

fn last_json_object(output: &str) -> Option<Value> {
    for line in output.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            return Some(v);
        }
    }
    None
}

/// The adapter's own session id (e.g. Claude Code's), used for `--resume`.
fn parse_adapter_session_id(output: &str) -> Option<String> {
    last_json_object(output)?
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Find the last line of output that is a JSON object carrying
/// `total_cost_usd`, and extract cost + usage from it.
fn parse_cost(output: &str) -> Option<ParsedCost> {
    let v = last_json_object(output)?;
    let usd = v.get("total_cost_usd").and_then(Value::as_f64)?;
    let usage = v.get("usage").cloned().unwrap_or_else(|| json!({}));
    let tok = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0);
    Some(ParsedCost {
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        input_tokens: tok("input_tokens"),
        cached_input_tokens: tok("cache_read_input_tokens"),
        output_tokens: tok("output_tokens"),
        cost_cents: (usd * 100.0).round() as i64,
    })
}

/// Diff of everything the session changed (committed or not) against the
/// commit its worktree started from.
pub async fn session_diff(state: &AppState, session_id: &str) -> Result<String, RunnerError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT workspace_path, base_sha FROM agent_task_sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((workspace_path, base_sha)) = row else {
        return Err(RunnerError::NotFound("session"));
    };
    let Some(base_sha) = base_sha else {
        return Err(RunnerError::Invalid("session has not started yet".into()));
    };
    let worktree = Path::new(&workspace_path);
    // `git diff` ignores untracked files; intent-to-add makes new files
    // created by the agent show up in the diff without staging content.
    git(worktree, &["add", "--intent-to-add", "--all"]).await?;
    git(worktree, &["diff", &base_sha]).await
}

#[cfg(test)]
mod tests {
    use super::{parse_adapter_session_id, parse_cost};

    #[test]
    fn parses_cost_from_final_json_line() {
        let output = "doing work...\n{\"model\":\"claude-sonnet\",\"session_id\":\"abc-123\",\"total_cost_usd\":0.0525,\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":10,\"output_tokens\":50}}";
        let cost = parse_cost(output).expect("cost parsed");
        assert_eq!(cost.cost_cents, 5);
        assert_eq!(cost.input_tokens, 100);
        assert_eq!(cost.cached_input_tokens, 10);
        assert_eq!(cost.output_tokens, 50);
        assert_eq!(cost.model, "claude-sonnet");
        assert_eq!(parse_adapter_session_id(output).as_deref(), Some("abc-123"));
    }

    #[test]
    fn no_cost_json_is_none() {
        assert!(parse_cost("plain output, no json").is_none());
        assert!(parse_cost("{\"no_cost\":true}").is_none());
        assert!(parse_adapter_session_id("no json").is_none());
    }
}
