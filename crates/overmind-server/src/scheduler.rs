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
use crate::notify;
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
            refusals,
            taken,
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
                // The repetition detector's input (ADR-0050 §3): which task
                // was refused and why, as data -- the variant, and for a
                // remedy its kind -- and which task was taken, if any.
                "refusals": refusals
                    .iter()
                    .map(|r| json!({ "task_id": r.task_id, "cause": r.cause }))
                    .collect::<Vec<_>>(),
                "taken": taken,
            }),
        )
        .await?;
        let mut told = Vec::new();
        if let Some(company_id) = &company_id {
            let mut constrained = Vec::new();
            for refusal in &refusals {
                if let Some(c) =
                    brake_if_repeated(state, &mut tx, company_id, &agent_id, refusal).await?
                {
                    constrained.push(c);
                }
            }
            told = tell_constrained(&mut tx, company_id, &agent_id, constrained).await?;
        }
        tx.commit().await?;
        if let Some(company_id) = &company_id {
            for n in &told {
                notify::deliver(state, company_id, n);
            }
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
    refusals: Vec<Refusal>,
    /// The task the wakeup started, or filed an approval for.
    taken: Option<String>,
}

impl WakeupOutcome {
    fn new(company_id: Option<String>, summary: impl Into<String>) -> Self {
        WakeupOutcome {
            company_id,
            summary: summary.into(),
            remedies: Vec::new(),
            refusals: Vec::new(),
            taken: None,
        }
    }
}

/// A start refused for the task's sake, not the agent's.
struct Refusal {
    task_id: String,
    /// What the detector compares: the variant, and for a remedy its kind
    /// (`invalid`, `conflict`, `remediable:grant_multimodal`). Never the
    /// sentence, which may name the files or the repository that changed.
    cause: String,
    message: String,
    remedy: Option<Value>,
}

/// The cause a `agent.wakeup_processed` payload records for refusing
/// `task_id`, if it refused it at all (a wakeup that took it did not).
fn refused_cause(payload: &str, task_id: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(payload).ok()?;
    payload
        .get("refusals")?
        .as_array()?
        .iter()
        .find(|r| r.get("task_id").and_then(Value::as_str) == Some(task_id))?
        .get("cause")?
        .as_str()
        .map(str::to_string)
}

/// The repetition detector on refused starts (ADR-0050 §3, and its addendum
/// of 28 Oct): the same agent refused the same task for the same cause at
/// `repetition_tolerance` of its wakeups in a row is no longer waiting for
/// something to change. What the cause is *about* decides what stops. One
/// that is the task's own stops the task as `stalled`, with the reason. One
/// that is the agent's -- a remedy is a patch to the agent -- stops offering
/// that task to that agent and tells the owner once: the count is one
/// agent's, and a stall would be everyone's.
///
/// Read from the audit chain alone: only the wakeups that reached this task
/// count (a wakeup that found the agent busy says nothing about it), a
/// different outcome in between -- another cause, or the task taken -- ends
/// the run, and the count starts again when a person puts a stalled task back
/// or the agent is characterized anew, which is how a remedy is applied.
/// Returns what it constrained, for `tell_constrained`.
async fn brake_if_repeated(
    state: &AppState,
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    company_id: &str,
    agent_id: &str,
    refusal: &Refusal,
) -> Result<Option<Constrained>, RunnerError> {
    let tolerance = state.config.repetition_tolerance.max(1);
    let recent: Vec<(String, String)> = sqlx::query_as(
        "SELECT json_extract(e.payload, '$.request_id'), e.payload
         FROM audit_events e JOIN tasks t ON t.id = ?
         WHERE e.kind = ? AND e.company_id = ?
           AND json_extract(e.payload, '$.agent_id') = ?
           AND e.created_at >= COALESCE(t.attempts_since, '')
           AND e.created_at >= COALESCE((SELECT MAX(r.created_at) FROM agent_config_revisions r
                                         WHERE r.agent_id = ? AND r.changed_keys <> '[]'), '')
           AND (json_extract(e.payload, '$.taken') = t.id
                OR EXISTS (SELECT 1 FROM json_each(e.payload, '$.refusals') r
                           WHERE json_extract(r.value, '$.task_id') = t.id))
         ORDER BY e.seq DESC LIMIT ?",
    )
    .bind(&refusal.task_id)
    .bind(event_kind::WAKEUP_PROCESSED)
    .bind(company_id)
    .bind(agent_id)
    .bind(agent_id)
    .bind(tolerance)
    .fetch_all(&mut **tx)
    .await?;
    let mut wakeups: Vec<String> = recent
        .into_iter()
        .take_while(|(_, payload)| {
            refused_cause(payload, &refusal.task_id) == Some(refusal.cause.clone())
        })
        .map(|(request_id, _)| request_id)
        .collect();
    if wakeups.len() < tolerance as usize {
        return Ok(None);
    }
    wakeups.reverse();
    if refusal.cause.starts_with("remediable:") {
        return constrain(tx, company_id, agent_id, refusal, &wakeups, tolerance)
            .await
            .map(Some);
    }

    // Only a task still waiting in the queue stops: one a person or another
    // agent has moved since the refusal is not the loop's to stall.
    let moved = sqlx::query(
        "UPDATE tasks SET status = 'stalled', updated_at = ? WHERE id = ? AND status = 'todo'",
    )
    .bind(now())
    .bind(&refusal.task_id)
    .execute(&mut **tx)
    .await?;
    if moved.rows_affected() == 0 {
        return Ok(None);
    }
    audit::append(
        tx,
        Some(company_id),
        Some(&refusal.task_id),
        event_kind::TASK_TRANSITIONED,
        &json!({ "from": "todo", "to": "stalled" }),
    )
    .await?;
    audit::append(
        tx,
        Some(company_id),
        Some(&refusal.task_id),
        event_kind::TASK_STALLED,
        &json!({
            "cause": "refused",
            "refusal": refusal.cause,
            "attempts": wakeups.len(),
            "cap": tolerance,
            "reason": format!("refused {} times: {}", wakeups.len(), refusal.message),
            "agent_id": agent_id,
            "remedies": refusal.remedy.iter().collect::<Vec<_>>(),
            // No session ran: the evidence is the wakeups that were refused.
            "sessions": [],
            "wakeups": wakeups,
        }),
    )
    .await?;
    Ok(None)
}

/// The agent stops being offered the task; everyone else still is. The audit
/// row is what the wakeup's candidate query reads, so it is the brake itself,
/// not a record of one -- and it is told to the owner here, once, because
/// after it the agent is never offered the task again to be refused it.
async fn constrain(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    company_id: &str,
    agent_id: &str,
    refusal: &Refusal,
    wakeups: &[String],
    cap: u32,
) -> Result<Constrained, RunnerError> {
    audit::append(
        tx,
        Some(company_id),
        Some(&refusal.task_id),
        event_kind::LOOP_CONSTRAINED,
        &json!({
            "cause": refusal.cause,
            "agent_id": agent_id,
            "attempts": wakeups.len(),
            "cap": cap,
            "reason": format!("refused {} times: {}", wakeups.len(), refusal.message),
            "remedies": refusal.remedy.iter().collect::<Vec<_>>(),
            "wakeups": wakeups,
        }),
    )
    .await?;
    // A title is for the sentence only: a task gone since the refusal must not
    // fail the wakeup that recorded it.
    let title: Option<(String,)> = sqlx::query_as("SELECT title FROM tasks WHERE id = ?")
        .bind(&refusal.task_id)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(Constrained {
        task_id: refusal.task_id.clone(),
        title: title
            .map(|(t,)| t)
            .unwrap_or_else(|| refusal.task_id.clone()),
        attempts: wakeups.len(),
        reason: refusal.message.clone(),
        remedy: refusal.remedy.clone(),
    })
}

/// A task the loop stopped offering to an agent, waiting to be told.
struct Constrained {
    task_id: String,
    title: String,
    attempts: usize,
    reason: String,
    remedy: Option<Value>,
}

/// Tell the owner what one wakeup constrained: once per remedy, naming every
/// task it holds back -- one missing trait is one thing to fix, however many
/// sketches wait on it. Returns the notifications to deliver after commit.
async fn tell_constrained(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    company_id: &str,
    agent_id: &str,
    constrained: Vec<Constrained>,
) -> Result<Vec<Value>, RunnerError> {
    let mut groups: Vec<Vec<Constrained>> = Vec::new();
    for c in constrained {
        match groups.iter_mut().find(|g| g[0].remedy == c.remedy) {
            Some(g) => g.push(c),
            None => groups.push(vec![c]),
        }
    }
    if groups.is_empty() {
        return Ok(Vec::new());
    }
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM agents WHERE id = ?")
        .bind(agent_id)
        .fetch_optional(&mut **tx)
        .await?;
    let agent = name.map(|(n,)| n).unwrap_or_else(|| agent_id.to_string());
    let mut told = Vec::new();
    for group in groups {
        let first = &group[0];
        let tasks: Vec<&str> = group.iter().map(|c| c.title.as_str()).collect();
        let title = format!("{agent} is no longer offered \"{}\"", tasks.join("\", \""));
        let body = format!("Refused {} times: {}", first.attempts, first.reason);
        told.push(
            notify::post(
                tx,
                company_id,
                notify::New {
                    kind: notify::kind::LOOP_CONSTRAINED,
                    title: &title,
                    body: &body,
                    params: json!({
                        "agent": agent,
                        "task": first.title,
                        "tasks": tasks,
                        "attempts": first.attempts,
                        "reason": first.reason,
                        "remedy": first.remedy,
                    }),
                    agent_id: Some(agent_id),
                    subject: Some(("task", &first.task_id)),
                    approval_id: None,
                },
            )
            .await?,
        );
    }
    Ok(told)
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
    // A task the loop stopped offering to this agent is not a candidate
    // (`constrain`) until the agent is characterized anew -- a revision that
    // changed something. The cause was the agent's, so nothing done to the
    // task lifts it; giving the task to someone else is the other way out.
    let sql = format!(
        "SELECT id FROM tasks WHERE company_id = ? AND status = 'todo'
           AND execution_kind IN ({placeholders})
           AND NOT EXISTS (
             SELECT 1 FROM audit_events c
             WHERE c.kind = ? AND c.task_id = tasks.id
               AND json_extract(c.payload, '$.agent_id') = ?
               AND c.created_at >= COALESCE(tasks.attempts_since, '')
               AND c.created_at >= COALESCE((SELECT MAX(r.created_at) FROM agent_config_revisions r
                                             WHERE r.agent_id = ? AND r.changed_keys <> '[]'), ''))
           ORDER BY created_at LIMIT {}",
        WINDOW + 1
    );
    let mut query = sqlx::query_as::<_, (String,)>(&sql).bind(&company_id);
    for k in &kinds {
        query = query.bind(*k);
    }
    query = query
        .bind(event_kind::LOOP_CONSTRAINED)
        .bind(agent_id)
        .bind(agent_id);
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
    let mut refusals: Vec<Refusal> = Vec::new();
    let mut taken: Option<String> = None;
    let last: Option<String> = 'tasks: {
        for (task_id,) in candidates {
            match runner::start_task(state, &task_id, agent_id, false).await {
                Ok(runner::StartResult::Started(outcome)) => {
                    taken = Some(task_id.clone());
                    break 'tasks Some(format!(
                        "started task {task_id} (session {})",
                        outcome.session_id
                    ));
                }
                Ok(runner::StartResult::ApprovalRequired { approval_id }) => {
                    taken = Some(task_id.clone());
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
                    refusals.push(Refusal {
                        task_id,
                        cause: "conflict".into(),
                        message: "taken by someone else".into(),
                        remedy: None,
                    });
                }
                Err(RunnerError::Invalid(msg)) => {
                    refused.push(format!("task {task_id} cannot start: {msg}"));
                    refusals.push(Refusal {
                        task_id,
                        cause: "invalid".into(),
                        message: msg,
                        remedy: None,
                    });
                }
                Err(RunnerError::Remediable { message, remedy }) => {
                    refused.push(format!("task {task_id} refused: {message}"));
                    let kind = remedy.get("kind").and_then(Value::as_str).unwrap_or("");
                    refusals.push(Refusal {
                        task_id,
                        cause: format!("remediable:{kind}"),
                        message,
                        remedy: Some(remedy.clone()),
                    });
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
        refusals,
        taken,
    })
}
