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
use crate::domain::{ExecutionKind, event_kind, perm};
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
    process_digests(state).await?;
    Ok(())
}

/// The CEO writes back on its own (ADR-0041): for each conversation where a
/// task born in it finished *after* the person's last word there, run one
/// digest turn — debounced, never while a turn is in flight, never twice for
/// the same completions (the in-memory watermark advances first, so a SKIP
/// also settles the matter).
async fn process_digests(state: &AppState) -> Result<(), RunnerError> {
    if !state.config.ceo_digest {
        return Ok(());
    }
    let due: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT c.id, c.company_id, c.agent_id, MAX(s.finished_at), MAX(m.created_at)
         FROM conversations c
         JOIN tasks t ON t.conversation_id = c.id
         JOIN agent_task_sessions s ON s.task_id = t.id AND s.status = 'completed'
         JOIN messages m ON m.conversation_id = c.id
         WHERE s.finished_at IS NOT NULL
         GROUP BY c.id
         HAVING MAX(s.finished_at) > MAX(m.created_at)",
    )
    .fetch_all(&state.pool)
    .await?;
    for (convo, company, agent, newest_finish, last_word) in due {
        if state.is_answering(&convo) {
            continue;
        }
        // Quiet long enough? A person mid-conversation does not need an
        // unprompted update landing between their own messages.
        let debounce = state.config.digest_debounce_secs;
        if debounce > 0 {
            let quiet = chrono::DateTime::parse_from_rfc3339(&last_word)
                .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds())
                .unwrap_or(i64::MAX);
            if quiet < debounce as i64 {
                continue;
            }
        }
        if !state.digest_advance(&convo, &newest_finish) {
            continue;
        }
        // What finished since the person's last word, worded by the agents'
        // own reports (clamped: a digest reads summaries, not transcripts).
        let finished: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
            "SELECT t.title, COALESCE(s.output, '') FROM tasks t
             JOIN agent_task_sessions s ON s.task_id = t.id AND s.status = 'completed'
             WHERE t.conversation_id = ? AND s.finished_at > ?
             ORDER BY s.finished_at DESC LIMIT 6",
        )
        .bind(&convo)
        .bind(&last_word)
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(title, output)| {
            let said: String = crate::ceo::agent_text(&output)
                .trim()
                .chars()
                .take(600)
                .collect();
            (title, said)
        })
        .collect();
        if finished.is_empty() {
            continue;
        }
        let state2 = state.clone();
        tokio::spawn(async move {
            if let Err(e) =
                crate::ceo::run_digest_turn(&state2, &company, &convo, &agent, &finished).await
            {
                eprintln!("digest turn for {convo} failed: {e}");
            }
        });
    }
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
        let WakeupOutcome {
            company_id,
            summary: outcome,
            remedies,
        } = wakeup_outcome(state, &agent_id).await?;
        sqlx::query(
            "UPDATE agent_wakeup_requests SET status = 'done', outcome = ?, finished_at = ? WHERE id = ?",
        )
        .bind(&outcome)
        .bind(now())
        .bind(&request_id)
        .execute(&state.pool)
        .await?;
        let mut tx = state.write_tx().await?;
        audit::append(
            &mut tx,
            company_id.as_deref(),
            None,
            event_kind::WAKEUP_PROCESSED,
            &json!({
                "request_id": request_id,
                "agent_id": agent_id,
                "outcome": outcome,
                // The remedies a refused start named, as data: the contract on
                // `RunnerError::Remediable` is that nobody string-matches the
                // sentence, and the audit row is where a consumer would look.
                "remedies": remedies,
            }),
        )
        .await?;
        tx.commit().await?;
        if let Some(company_id) = &company_id {
            state.notify(company_id);
        }
    }
    Ok(())
}

/// What a wakeup came to: the words for the audit row and the wakeup's own
/// `outcome` column, and -- when a start was refused with a remedy in hand --
/// the remedies themselves, as data.
struct WakeupOutcome {
    company_id: Option<String>,
    summary: String,
    remedies: Vec<Value>,
}

impl WakeupOutcome {
    fn new(company_id: Option<String>, summary: impl Into<String>) -> Self {
        WakeupOutcome {
            company_id,
            summary: summary.into(),
            remedies: Vec::new(),
        }
    }
}

async fn wakeup_outcome(state: &AppState, agent_id: &str) -> Result<WakeupOutcome, RunnerError> {
    let agent: Option<(String, String, String)> =
        sqlx::query_as("SELECT company_id, status, traits FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, status, traits)) = agent else {
        return Ok(WakeupOutcome::new(None, "agent not found"));
    };
    if status != "active" {
        return Ok(WakeupOutcome::new(
            Some(company_id),
            format!("agent is {status}"),
        ));
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
        return Ok(WakeupOutcome::new(
            Some(company_id),
            "agent has a session in flight",
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
        return Ok(WakeupOutcome::new(
            Some(company_id),
            format!("autonomy '{autonomy}' requires a human to start tasks"),
        ));
    }

    // Only work this agent is characterized for (M14). Picking the oldest todo
    // task blindly would hand a researcher a code task and stall the wakeup on
    // a capability it was never meant to have.
    let permissions = serde_json::from_str::<Value>(&traits)
        .ok()
        .and_then(|v| {
            v.get("permissions").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();
    let kinds: Vec<&str> = [ExecutionKind::Code, ExecutionKind::Knowledge]
        .into_iter()
        .filter(|k| {
            let required = perm::for_execution_kind(*k);
            permissions.iter().any(|p| p == required)
        })
        .map(|k| k.as_str())
        .collect();
    if kinds.is_empty() {
        return Ok(WakeupOutcome::new(
            Some(company_id),
            "agent is not characterized for any kind of task",
        ));
    }
    let placeholders = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    // Oldest first, and bounded: a wakeup is "take the work you can", and a
    // beat is not the place to try a whole backlog. One more than the window
    // is fetched so the outcome can say when the window was the limit.
    const WINDOW: usize = 20;
    let sql = format!(
        "SELECT id FROM tasks WHERE company_id = ? AND status = 'todo'
           AND execution_kind IN ({placeholders}) ORDER BY created_at LIMIT {}",
        WINDOW + 1
    );
    let mut query = sqlx::query_as::<_, (String,)>(&sql).bind(&company_id);
    for k in &kinds {
        query = query.bind(*k);
    }
    let mut candidates: Vec<(String,)> = query.fetch_all(&state.pool).await?;
    if candidates.is_empty() {
        return Ok(WakeupOutcome::new(
            Some(company_id),
            "no todo tasks it can take",
        ));
    }
    let more_behind = candidates.len() > WINDOW;
    candidates.truncate(WINDOW);

    // A refusal is this agent's problem, not the heartbeat's -- and not the
    // next task's either: it is recorded, and the wakeup moves on to the task
    // behind it (ADR-0009, addendum of 7 Sep 2026). Stopping at the first
    // refusal used to fail the beat for everyone (the refusal fell through as
    // an error, the row stayed queued, and the next beat refused the same
    // task again, thirty seconds on); and merely recording it would have left
    // the oldest task this agent cannot take standing in front of every one
    // it can. What is the *agent's* condition, not the task's -- its wallet,
    // its status, its characterization -- ends the wakeup at once: no other
    // task would fare better.
    let mut refused: Vec<String> = Vec::new();
    let mut remedies: Vec<Value> = Vec::new();
    let last: Option<String> = 'tasks: {
        for (task_id,) in candidates {
            match runner::start_task(state, &task_id, agent_id, false).await {
                Ok(runner::StartResult::Started(outcome)) => {
                    break 'tasks Some(format!(
                        "started task {task_id} (session {})",
                        outcome.session_id
                    ));
                }
                Ok(runner::StartResult::ApprovalRequired { approval_id }) => {
                    break 'tasks Some(format!("task {task_id} needs approval ({approval_id})"));
                }
                // The agent's condition: over its budget, paused, or not
                // characterized for this kind of work at all.
                Err(RunnerError::OverBudget { .. }) => {
                    break 'tasks Some(format!("task {task_id} blocked: over budget"));
                }
                Err(RunnerError::Blocked(msg)) => {
                    break 'tasks Some(format!("blocked: {msg}"));
                }
                // The task's condition: taken by someone else since the
                // SELECT, malformed for a start (a code task with no home
                // repository), or refused with a remedy in hand (the
                // multimodal gate, ADR-0021 -- applying the remedy is the
                // owner's call, not the heartbeat's). The next candidate may
                // well be fine.
                Err(RunnerError::Conflict) => {
                    refused.push(format!("task {task_id} was taken by someone else"));
                }
                Err(RunnerError::Invalid(msg)) => {
                    refused.push(format!("task {task_id} cannot start: {msg}"));
                }
                Err(RunnerError::Remediable { message, remedy }) => {
                    refused.push(format!("task {task_id} refused: {message}"));
                    // A remedy is about the agent, so five sketches name the
                    // same one: keep each once.
                    if !remedies.contains(&remedy) {
                        remedies.push(remedy);
                    }
                }
                // Named, not wildcarded: the next variant added to
                // RunnerError has to be decided here, or it falls through as
                // an error and the beat stalls again. NotFound heals on the
                // next beat, Git cannot happen before a spawn, Db is the
                // beat's own problem.
                Err(e @ (RunnerError::NotFound(_) | RunnerError::Git(_) | RunnerError::Db(_))) => {
                    return Err(e);
                }
            }
        }
        None
    };
    let exhausted = last.is_none();
    let mut summary = match last {
        Some(last) if refused.is_empty() => last,
        Some(last) => format!("refused: {}; {last}", refused.join("; ")),
        None => format!("refused: {}", refused.join("; ")),
    };
    if exhausted && more_behind {
        summary.push_str(&format!(" (and more behind these {WINDOW}, not tried)"));
    }
    Ok(WakeupOutcome {
        company_id: Some(company_id),
        summary,
        remedies,
    })
}
