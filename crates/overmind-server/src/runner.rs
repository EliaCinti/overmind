//! Agent execution (M2): atomic task checkout, isolated git worktrees,
//! process supervision, output capture and cost recording.
//!
//! Design follows Paperclip's session model (`agent_task_sessions`,
//! `cost_events`) and Vibe Kanban's worktree-per-run isolation (ADR-0008).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tokio::process::Command;

use crate::audit;
use crate::db::AppState;
use crate::domain::event_kind;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("task is not available for checkout")]
    Conflict,
    #[error("git error: {0}")]
    Git(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub struct StartOutcome {
    pub session_id: String,
    pub branch: String,
    pub workspace_path: String,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
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
/// wins; the others get `Conflict`. Checkout, session creation and audit
/// events commit in one transaction; the process itself runs afterwards in a
/// background task that finalizes the session.
pub async fn start_task(
    state: &AppState,
    task_id: &str,
    agent_id: &str,
) -> Result<StartOutcome, RunnerError> {
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

    let agent: Option<(String, String)> = sqlx::query_as(
        "SELECT name, traits FROM agents WHERE id = ? AND company_id = ? AND status = 'active'",
    )
    .bind(agent_id)
    .bind(&company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((_agent_name, agent_traits)) = agent else {
        return Err(RunnerError::NotFound("agent"));
    };

    let session_id = uuid::Uuid::now_v7().to_string();
    let branch = format!("overmind/task-{}-{}", short(task_id), short(&session_id));
    let worktree_dir = state
        .config
        .data_dir
        .join("worktrees")
        .join(&session_id)
        .to_string_lossy()
        .into_owned();

    // Atomic checkout: exactly one concurrent caller wins this UPDATE.
    let mut tx = state.pool.begin().await?;
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
        "INSERT INTO agent_task_sessions (id, task_id, agent_id, adapter_type, status, branch, workspace_path, created_at)
         VALUES (?, ?, ?, 'claude_code', 'queued', ?, ?, ?)",
    )
    .bind(&session_id)
    .bind(task_id)
    .bind(agent_id)
    .bind(&branch)
    .bind(&worktree_dir)
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

    // Run the agent in the background; it finalizes the session and the task.
    let ctx = SessionContext {
        state: state.clone(),
        session_id: session_id.clone(),
        task_id: task_id.to_string(),
        company_id,
        repo_cwd: PathBuf::from(repo_cwd),
        default_ref,
        branch: branch.clone(),
        worktree_dir: PathBuf::from(&worktree_dir),
        title,
        description,
        agent_traits,
    };
    tokio::spawn(async move {
        run_session(ctx).await;
    });

    Ok(StartOutcome {
        session_id,
        branch,
        workspace_path: worktree_dir,
    })
}

struct SessionContext {
    state: AppState,
    session_id: String,
    task_id: String,
    company_id: String,
    repo_cwd: PathBuf,
    default_ref: Option<String>,
    branch: String,
    worktree_dir: PathBuf,
    title: String,
    description: String,
    agent_traits: String,
}

async fn run_session(ctx: SessionContext) {
    let result = execute(&ctx).await;
    if let Err(e) = finalize(&ctx, result).await {
        eprintln!("session {}: failed to finalize: {e}", ctx.session_id);
    }
}

struct ExecutionResult {
    output: String,
    exit_code: i32,
}

async fn execute(ctx: &SessionContext) -> Result<ExecutionResult, RunnerError> {
    // Isolated worktree on its own branch, from the workspace's default ref.
    if let Some(parent) = ctx.worktree_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| RunnerError::Git(format!("cannot create worktree dir: {e}")))?;
    }
    let worktree = ctx.worktree_dir.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "add", worktree.as_str(), "-b", &ctx.branch];
    if let Some(r) = &ctx.default_ref {
        args.push(r.as_str());
    }
    git(&ctx.repo_cwd, &args).await?;
    let base_sha = git(&ctx.worktree_dir, &["rev-parse", "HEAD"]).await?;

    sqlx::query(
        "UPDATE agent_task_sessions SET status = 'running', base_sha = ?, started_at = ? WHERE id = ?",
    )
    .bind(&base_sha)
    .bind(now())
    .bind(&ctx.session_id)
    .execute(&ctx.state.pool)
    .await?;

    // The adapter command is configurable (tests use a stub); the default
    // drives the Claude Code CLI headless with a JSON result.
    let agent_cmd =
        ctx.state.config.agent_cmd.clone().unwrap_or_else(|| {
            "claude -p \"$OVERMIND_TASK_PROMPT\" --output-format json".to_string()
        });
    let prompt = format!(
        "You are working on the task \"{}\".\n\n{}\n\nWork in the current directory. When done, leave the changes uncommitted.",
        ctx.title, ctx.description
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(&agent_cmd)
        .current_dir(&ctx.worktree_dir)
        .env("OVERMIND_TASK_PROMPT", &prompt)
        .env("OVERMIND_TASK_TITLE", &ctx.title)
        .env("OVERMIND_TASK_DESCRIPTION", &ctx.description)
        .env("OVERMIND_AGENT_TRAITS", &ctx.agent_traits)
        .output()
        .await
        .map_err(|e| RunnerError::Git(format!("failed to spawn agent: {e}")))?;

    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        output.push_str("\n--- stderr ---\n");
        output.push_str(stderr.trim());
    }
    Ok(ExecutionResult {
        output,
        exit_code: out.status.code().unwrap_or(-1),
    })
}

async fn finalize(
    ctx: &SessionContext,
    result: Result<ExecutionResult, RunnerError>,
) -> Result<(), RunnerError> {
    let (session_status, task_to, output, exit_code, last_error) = match &result {
        Ok(r) if r.exit_code == 0 => (
            "completed",
            "in_review",
            r.output.clone(),
            r.exit_code,
            None,
        ),
        Ok(r) => (
            "failed",
            "blocked",
            r.output.clone(),
            r.exit_code,
            Some(format!("agent exited with code {}", r.exit_code)),
        ),
        Err(e) => ("failed", "blocked", String::new(), -1, Some(e.to_string())),
    };

    let mut tx = ctx.state.pool.begin().await?;
    sqlx::query(
        "UPDATE agent_task_sessions SET status = ?, output = ?, exit_code = ?, last_error = ?, finished_at = ? WHERE id = ?",
    )
    .bind(session_status)
    .bind(&output)
    .bind(exit_code)
    .bind(&last_error)
    .bind(now())
    .bind(&ctx.session_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE tasks SET status = ?, updated_at = ? WHERE id = ?")
        .bind(task_to)
        .bind(now())
        .bind(&ctx.task_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&ctx.company_id),
        Some(&ctx.task_id),
        event_kind::TASK_TRANSITIONED,
        &json!({ "from": "in_progress", "to": task_to }),
    )
    .await?;
    audit::append(
        &mut tx,
        Some(&ctx.company_id),
        Some(&ctx.task_id),
        event_kind::SESSION_FINISHED,
        &json!({
            "session_id": ctx.session_id,
            "status": session_status,
            "exit_code": exit_code,
            "error": last_error,
        }),
    )
    .await?;

    // Cost capture: the Claude Code CLI (and our stubs) print a final JSON
    // object with total_cost_usd and usage. Missing/unparseable cost is not
    // an error — the session already carries the full output.
    if let Some(cost) = parse_cost(&output) {
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
    Ok(())
}

struct ParsedCost {
    model: String,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    cost_cents: i64,
}

/// Find the last line of output that is a JSON object carrying
/// `total_cost_usd`, and extract cost + usage from it.
fn parse_cost(output: &str) -> Option<ParsedCost> {
    for line in output.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(usd) = v.get("total_cost_usd").and_then(Value::as_f64) else {
            continue;
        };
        let usage = v.get("usage").cloned().unwrap_or_else(|| json!({}));
        let tok = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0);
        return Some(ParsedCost {
            model: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            input_tokens: tok("input_tokens"),
            cached_input_tokens: tok("cache_read_input_tokens"),
            output_tokens: tok("output_tokens"),
            cost_cents: (usd * 100.0).round() as i64,
        });
    }
    None
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
    use super::parse_cost;

    #[test]
    fn parses_cost_from_final_json_line() {
        let output = "doing work...\n{\"model\":\"claude-sonnet\",\"total_cost_usd\":0.0525,\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":10,\"output_tokens\":50}}";
        let cost = parse_cost(output).expect("cost parsed");
        assert_eq!(cost.cost_cents, 5);
        assert_eq!(cost.input_tokens, 100);
        assert_eq!(cost.cached_input_tokens, 10);
        assert_eq!(cost.output_tokens, 50);
        assert_eq!(cost.model, "claude-sonnet");
    }

    #[test]
    fn no_cost_json_is_none() {
        assert!(parse_cost("plain output, no json").is_none());
        assert!(parse_cost("{\"no_cost\":true}").is_none());
    }
}
