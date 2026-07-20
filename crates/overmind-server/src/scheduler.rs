//! Heartbeat scheduler (M3, ADR-0009): a periodic beat that
//! 1. recovers orphaned sessions (running in the DB, no live runner here —
//!    e.g. after a server restart) by resuming them in their worktree, and
//! 2. processes queued `agent_wakeup_requests`, letting agents whose traits
//!    grant `act_within_budget` autonomously pick up the oldest todo task.
//!
//! Paperclip's full cron-style routines are deferred; this is the substrate
//! they will sit on.

use serde_json::{Value, json};
use tokio::time::{Duration, MissedTickBehavior, interval};

use crate::audit;
use crate::db::AppState;
use crate::domain::event_kind;
use crate::runner::{self, RunnerError};

pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(state.config.heartbeat_ms));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = beat(&state).await {
                eprintln!("heartbeat error: {e}");
            }
        }
    })
}

/// One heartbeat. Public so tests can drive it deterministically.
pub async fn beat(state: &AppState) -> Result<(), RunnerError> {
    recover_orphans(state).await?;
    process_wakeups(state).await?;
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Sessions younger than this are presumed owned by an in-flight request
/// even if they are not registered yet (start commits the row before the
/// runner registers). RFC3339 strings with a fixed +00:00 offset compare
/// chronologically as strings.
fn grace_cutoff(state: &AppState) -> String {
    let grace_ms = (2 * state.config.heartbeat_ms).max(1_000);
    (chrono::Utc::now() - chrono::Duration::milliseconds(grace_ms as i64)).to_rfc3339()
}

async fn recover_orphans(state: &AppState) -> Result<(), RunnerError> {
    let cutoff = grace_cutoff(state);
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM agent_task_sessions
         WHERE status IN ('queued', 'running')
           AND COALESCE(started_at, created_at) < ?",
    )
    .bind(&cutoff)
    .fetch_all(&state.pool)
    .await?;
    for (session_id,) in rows {
        let owned = state
            .running
            .lock()
            .map(|r| r.contains(&session_id))
            .unwrap_or(true);
        if owned {
            continue;
        }
        if let Err(e) = runner::resume_session(state, &session_id).await {
            eprintln!("heartbeat: cannot recover session {session_id}: {e}");
        }
    }
    Ok(())
}

async fn process_wakeups(state: &AppState) -> Result<(), RunnerError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, agent_id FROM agent_wakeup_requests
         WHERE status = 'queued' ORDER BY requested_at",
    )
    .fetch_all(&state.pool)
    .await?;
    for (request_id, agent_id) in rows {
        sqlx::query("UPDATE agent_wakeup_requests SET claimed_at = ? WHERE id = ?")
            .bind(now())
            .bind(&request_id)
            .execute(&state.pool)
            .await?;
        let (company_id, outcome) = wakeup_outcome(state, &agent_id).await?;
        sqlx::query(
            "UPDATE agent_wakeup_requests SET status = 'done', outcome = ?, finished_at = ? WHERE id = ?",
        )
        .bind(&outcome)
        .bind(now())
        .bind(&request_id)
        .execute(&state.pool)
        .await?;
        let mut tx = state.pool.begin().await?;
        audit::append(
            &mut tx,
            company_id.as_deref(),
            None,
            event_kind::WAKEUP_PROCESSED,
            &json!({ "request_id": request_id, "agent_id": agent_id, "outcome": outcome }),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn wakeup_outcome(
    state: &AppState,
    agent_id: &str,
) -> Result<(Option<String>, String), RunnerError> {
    let agent: Option<(String, String, String)> =
        sqlx::query_as("SELECT company_id, status, traits FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, status, traits)) = agent else {
        return Ok((None, "agent not found".to_string()));
    };
    if status != "active" {
        return Ok((Some(company_id), format!("agent is {status}")));
    }

    // An interrupted session is the scheduler's job (recover_orphans), and a
    // live one means the agent is busy.
    let in_flight: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM agent_task_sessions WHERE agent_id = ? AND status IN ('queued', 'running') LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(&state.pool)
    .await?;
    if in_flight.is_some() {
        return Ok((
            Some(company_id),
            "agent has a session in flight".to_string(),
        ));
    }

    // Autonomy is enforced here (ADR-0005): only act_within_budget agents
    // may pick up work without a human starting it.
    let autonomy = serde_json::from_str::<Value>(&traits)
        .ok()
        .and_then(|v| {
            v.get("autonomy")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if autonomy != "act_within_budget" {
        return Ok((
            Some(company_id),
            format!("autonomy '{autonomy}' requires a human to start tasks"),
        ));
    }

    let next: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM tasks WHERE company_id = ? AND status = 'todo' ORDER BY created_at LIMIT 1",
    )
    .bind(&company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((task_id,)) = next else {
        return Ok((Some(company_id), "no todo tasks".to_string()));
    };
    match runner::start_task(state, &task_id, agent_id).await {
        Ok(outcome) => Ok((
            Some(company_id),
            format!("started task {task_id} (session {})", outcome.session_id),
        )),
        Err(RunnerError::Conflict) => Ok((
            Some(company_id),
            format!("task {task_id} was taken by someone else"),
        )),
        Err(RunnerError::Invalid(msg)) => Ok((Some(company_id), format!("cannot start: {msg}"))),
        Err(e) => Err(e),
    }
}
