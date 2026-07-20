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

/// Cents currently reserved by the agent's in-flight sessions.
pub async fn reserved_cents(
    conn: &mut SqliteConnection,
    agent_id: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(reserved_cents), 0) FROM agent_task_sessions
         WHERE agent_id = ? AND status IN ('queued', 'running')",
    )
    .bind(agent_id)
    .fetch_one(conn)
    .await?;
    Ok(row.0)
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
