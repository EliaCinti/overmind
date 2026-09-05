//! Budget accounting and config-revision snapshots (M6, ADR-0012).
//!
//! The budget "amount" is the agent's `monthly_budget_cents` trait; the window
//! is the calendar (UTC) month. Enforcement happens inside the task-checkout
//! transaction in the runner so a start that would overrun the cap is stopped
//! atomically, never after the spend.

use chrono::{Datelike, Timelike};
use serde_json::{Value, json};
use sqlx::sqlite::SqliteConnection;

/// Start of the current UTC month, RFC3339 — the budget window start.
/// RFC3339 strings with a fixed +00:00 offset compare chronologically as
/// strings, so this is directly usable in `WHERE occurred_at >= ?`.
pub fn month_window_start() -> String {
    let now = chrono::Utc::now();
    now.with_day(1)
        .and_then(|d| d.with_hour(0))
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(now)
        .to_rfc3339()
}

/// Actual spend recorded for an agent since `window_start`.
pub async fn spent_cents(
    conn: &mut SqliteConnection,
    agent_id: &str,
    window_start: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(cost_cents), 0) FROM cost_events
         WHERE agent_id = ? AND occurred_at >= ?",
    )
    .bind(agent_id)
    .bind(window_start)
    .fetch_one(conn)
    .await?;
    Ok(row.0)
}

/// Cents currently reserved by everything the agent has in flight.
///
/// Two sources since [ADR-0022](../../docs/adr/0022-conversational-spend-under-budget.md):
/// task sessions, and conversational turns. Turn reservations could not live on
/// `agent_task_sessions` — its `task_id` is `NOT NULL` — so they have their own
/// table and are summed here, where every caller already looks.
pub async fn reserved_cents(
    conn: &mut SqliteConnection,
    agent_id: &str,
) -> Result<i64, sqlx::Error> {
    let sessions: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(reserved_cents), 0) FROM agent_task_sessions
         WHERE agent_id = ? AND status IN ('queued', 'running')",
    )
    .bind(agent_id)
    .fetch_one(&mut *conn)
    .await?;
    let turns: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(reserved_cents), 0) FROM agent_turn_reservations
         WHERE agent_id = ? AND released_at IS NULL",
    )
    .bind(agent_id)
    .fetch_one(conn)
    .await?;
    Ok(sessions.0 + turns.0)
}

/// What the budget looks like for one prospective run.
#[derive(Clone, Copy, Debug)]
pub struct BudgetCheck {
    /// Whether this run fits under the cap. Always true for an uncapped agent.
    pub fits: bool,
    pub spent: i64,
    pub reserved: i64,
    pub estimate: i64,
    /// The agent's monthly cap; `0` means uncapped.
    pub cap: i64,
}

impl BudgetCheck {
    /// What the run would bring the agent to.
    pub fn observed(&self) -> i64 {
        self.spent + self.reserved + self.estimate
    }

    /// The most this run may spend before the cap is reached — everything under
    /// it that is neither already spent nor spoken for by something *else*.
    ///
    /// `None` when the agent is uncapped, because there is then no ceiling to
    /// hand the adapter and inventing one would be a limit nobody asked for.
    ///
    /// Note what is *not* subtracted: this run's own estimate. The estimate is
    /// a placeholder held at the gate so concurrent runs cannot overcommit, not
    /// a budget for the run — subtracting it here would cap every run at the
    /// difference between the flat estimate and the truth, which is the very
    /// quantity M18 named as the open gap.
    pub fn headroom(&self) -> Option<i64> {
        (self.cap > 0).then(|| (self.cap - self.spent - self.reserved).max(0))
    }
}

/// [`BudgetCheck::headroom`] for a run that has *already* reserved.
///
/// A task run reserves at checkout and spawns later, so by spawn time its own
/// reservation is in the total — and counting it against itself would hand the
/// adapter a ceiling of roughly nothing. `excluding_session` takes it back out.
pub async fn headroom_cents(
    conn: &mut SqliteConnection,
    agent_id: &str,
    cap: i64,
    excluding_session: Option<&str>,
) -> Result<Option<i64>, sqlx::Error> {
    if cap <= 0 {
        return Ok(None);
    }
    let window = month_window_start();
    let spent = spent_cents(&mut *conn, agent_id, &window).await?;
    let sessions: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(reserved_cents), 0) FROM agent_task_sessions
         WHERE agent_id = ? AND status IN ('queued', 'running') AND id IS NOT ?",
    )
    .bind(agent_id)
    .bind(excluding_session)
    .fetch_one(&mut *conn)
    .await?;
    let turns: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(reserved_cents), 0) FROM agent_turn_reservations
         WHERE agent_id = ? AND released_at IS NULL",
    )
    .bind(agent_id)
    .fetch_one(conn)
    .await?;
    Ok(Some((cap - spent - sessions.0 - turns.0).max(0)))
}

/// Cents as the adapter's `--max-budget-usd` wants them: plain dollars.
pub fn dollars(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

/// Does one more run of `estimate` cents fit under `cap`?
///
/// The rule M6 has enforced at task checkout since ADR-0012, lifted out so
/// conversational turns are measured by the same arithmetic rather than by a
/// second implementation that could drift from it (ADR-0022).
pub async fn check(
    conn: &mut SqliteConnection,
    agent_id: &str,
    cap: i64,
    estimate: i64,
) -> Result<BudgetCheck, sqlx::Error> {
    if cap <= 0 {
        return Ok(BudgetCheck {
            fits: true,
            spent: 0,
            reserved: 0,
            estimate,
            cap,
        });
    }
    let window = month_window_start();
    let spent = spent_cents(&mut *conn, agent_id, &window).await?;
    let reserved = reserved_cents(&mut *conn, agent_id).await?;
    Ok(BudgetCheck {
        fits: spent + reserved + estimate <= cap,
        spent,
        reserved,
        estimate,
        cap,
    })
}

/// Record that an agent was stopped by its cap: a durable incident plus the
/// audit event. Shared by task checkout and conversational turns so both
/// overruns are visible in the same place, described the same way.
pub async fn record_overrun(
    conn: &mut SqliteConnection,
    company_id: &str,
    agent_id: &str,
    task_id: Option<&str>,
    check: &BudgetCheck,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO budget_incidents (id, company_id, agent_id, window_start, threshold_type, amount_limit, amount_observed, status, created_at)
         VALUES (?, ?, ?, ?, 'hard', ?, ?, 'open', ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(company_id)
    .bind(agent_id)
    .bind(month_window_start())
    .bind(check.cap)
    .bind(check.observed())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await?;
    crate::audit::append(
        conn,
        Some(company_id),
        task_id,
        crate::domain::event_kind::BUDGET_BLOCKED,
        &json!({
            "agent_id": agent_id,
            "limit_cents": check.cap,
            "observed_cents": check.observed(),
        }),
    )
    .await
    .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    Ok(())
}

/// Hold `cents` against an agent's cap for the duration of one conversational
/// turn. Returns the reservation id, which the caller must release.
pub async fn reserve_turn(
    conn: &mut SqliteConnection,
    company_id: &str,
    agent_id: &str,
    kind: &str,
    cents: i64,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO agent_turn_reservations (id, company_id, agent_id, kind, reserved_cents, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(company_id)
    .bind(agent_id)
    .bind(kind)
    .bind(cents)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(conn)
    .await?;
    Ok(id)
}

/// Record what a conversational turn actually cost (ADR-0022).
///
/// `task_id` and `session_id` are null: this spend belongs to no task and no
/// session, which is exactly why it was invisible until now. Both columns have
/// been nullable since M2, so the ledger's shape already allowed for it — the
/// only thing missing was a second writer.
///
/// Best-effort: an adapter that prints no cost envelope is not an error, and
/// losing the accounting for one turn must not lose the turn.
pub async fn record_turn_cost(
    pool: &sqlx::SqlitePool,
    company_id: &str,
    agent_id: &str,
    output: &str,
) {
    let Some(cost) = crate::provider::current().cost(output) else {
        return;
    };
    let now = chrono::Utc::now().to_rfc3339();
    let _ = sqlx::query(
        "INSERT INTO cost_events (id, company_id, agent_id, task_id, session_id, provider, model,
         input_tokens, cached_input_tokens, output_tokens, cost_cents, occurred_at, created_at)
         VALUES (?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(company_id)
    .bind(agent_id)
    .bind("anthropic")
    .bind(&cost.model)
    .bind(cost.input_tokens)
    .bind(cost.cached_input_tokens)
    .bind(cost.output_tokens)
    .bind(cost.cost_cents)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await;
}

/// Release a turn's reservation, however the turn ended. Best-effort on
/// purpose: a failure here must not mask the turn's own outcome, and the worst
/// case is a reservation that reads as in-flight until it is cleaned up —
/// conservative in the direction of spending less.
pub async fn release_turn(pool: &sqlx::SqlitePool, reservation_id: &str) {
    let _ = sqlx::query("UPDATE agent_turn_reservations SET released_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(reservation_id)
        .execute(pool)
        .await;
}

/// Cents as euros, for the English `title`/`body` a notification keeps as its
/// durable record. The *translated* wording is the client's job (M16 slice D);
/// this is the fallback, and the fallback has always been English.
pub fn euros(cents: i64) -> String {
    format!("€{}.{:02}", cents / 100, (cents % 100).abs())
}

/// The full config snapshot stored in an `agent_config_revisions` row.
pub fn agent_snapshot(
    name: &str,
    title: Option<&str>,
    reports_to: Option<&str>,
    traits: &Value,
    custom_brief: Option<&str>,
    requires_approval: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "reports_to": reports_to,
        "traits": traits,
        "custom_brief": custom_brief,
        "requires_approval": requires_approval,
    })
}

/// Append a config revision. Forward-only history; never edited.
pub async fn record_revision(
    conn: &mut SqliteConnection,
    company_id: &str,
    agent_id: &str,
    source: &str,
    before: &Value,
    after: &Value,
) -> Result<(), sqlx::Error> {
    let changed: Vec<&str> = after
        .as_object()
        .map(|a| {
            a.iter()
                .filter(|(k, v)| before.get(*k) != Some(*v))
                .map(|(k, _)| k.as_str())
                .collect()
        })
        .unwrap_or_default();
    sqlx::query(
        "INSERT INTO agent_config_revisions (id, company_id, agent_id, source, changed_keys, before_config, after_config, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(company_id)
    .bind(agent_id)
    .bind(source)
    .bind(serde_json::to_string(&changed).unwrap_or_else(|_| "[]".into()))
    .bind(before.to_string())
    .bind(after.to_string())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(conn)
    .await?;
    Ok(())
}

/// What a run is expected to cost before it runs (M26, ADR-0035), and how
/// much history that number rests on. `samples == 0` below the threshold
/// means the flat default is standing in, visibly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Estimate {
    pub cents: i64,
    pub samples: usize,
}

/// The two kinds of spend the ledger tells apart by `session_id` -- a task
/// run carries a repository into the context, a conversational turn carries
/// a conversation, and they cost differently for the same agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendKind {
    Task,
    Turn,
}

/// Fewer samples than this and the flat default stands: a guess from one
/// data point is the same guess with a false precision attached.
const MIN_SAMPLES: usize = 3;
/// How far back the estimate looks: enough to smooth one odd run, short
/// enough to follow an agent whose work changed.
const WINDOW_RUNS: i64 = 10;

/// The value at three quarters of the way up the sorted costs -- at least
/// 75% of past runs cost this much or less. Above the median on purpose: a
/// reservation exists to keep the next run *inside* the cap, so it leans
/// toward the agent's dearer days. Never below one cent.
fn leaning_dear(mut costs: Vec<i64>) -> i64 {
    costs.sort_unstable();
    let idx = ((costs.len() * 3) / 4).min(costs.len().saturating_sub(1));
    costs.get(idx).copied().unwrap_or(1).max(1)
}

/// The estimate for an agent's next run of `kind`: its own last ten costs of
/// that kind, read from the ledger, leaning dear; `default` until the ledger
/// knows at least three.
pub async fn estimate_cents(
    conn: &mut SqliteConnection,
    agent_id: &str,
    kind: SpendKind,
    default: i64,
) -> Result<Estimate, sqlx::Error> {
    let sql = match kind {
        // A task run may record more than one event; the run is the sum.
        SpendKind::Task => {
            "SELECT SUM(cost_cents) AS cents FROM cost_events
             WHERE agent_id = ? AND session_id IS NOT NULL
             GROUP BY session_id ORDER BY MAX(occurred_at) DESC LIMIT ?"
        }
        SpendKind::Turn => {
            "SELECT cost_cents FROM cost_events
             WHERE agent_id = ? AND session_id IS NULL
             ORDER BY occurred_at DESC LIMIT ?"
        }
    };
    let rows: Vec<(i64,)> = sqlx::query_as(sql)
        .bind(agent_id)
        .bind(WINDOW_RUNS)
        .fetch_all(conn)
        .await?;
    let costs: Vec<i64> = rows.into_iter().map(|(c,)| c).collect();
    let samples = costs.len();
    if samples < MIN_SAMPLES {
        return Ok(Estimate {
            cents: default,
            samples,
        });
    }
    Ok(Estimate {
        cents: leaning_dear(costs),
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::leaning_dear;

    /// Four runs at 2, 3, 3, 4: the reservation is 4, not the median 3 --
    /// a number that is right half the time lets the cap be crossed half
    /// the time.
    #[test]
    fn the_estimate_leans_toward_the_dearer_days() {
        assert_eq!(leaning_dear(vec![3, 2, 4, 3]), 4);
        assert_eq!(leaning_dear(vec![70, 90, 80]), 90);
        // Ten runs: the eighth smallest, so one outlier at the top does not
        // price every run after it.
        assert_eq!(leaning_dear(vec![1, 1, 1, 1, 1, 1, 1, 2, 3, 100]), 2);
    }

    /// A run that cost nothing is not a run that costs nothing next time:
    /// the floor keeps the gate from becoming a formality.
    #[test]
    fn the_estimate_is_never_below_a_cent() {
        assert_eq!(leaning_dear(vec![0, 0, 0]), 1);
    }
}
