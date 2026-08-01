//! The notification mechanism (ADR-0020): how the company reaches the human.
//!
//! Agents collaborate on their own. When something genuinely needs you — an
//! agent asking to convene a meeting, the decision that meeting reached — it
//! lands here, with the reason and everything you need to act.
//!
//! Two layers, on purpose: a **durable row** (nothing is lost while the app is
//! closed) and a **live push** over `/ws` (an open app reacts at once). The row
//! is the record; the push is only the fast path. A notification is written in
//! the same transaction as the thing that caused it, so you are never told
//! about something that didn't happen — and never miss something that did.

use serde_json::{Value, json};
use sqlx::sqlite::SqliteConnection;

use crate::db::AppState;

/// Notification kinds. These are contract with the UI — add, never rename.
pub mod kind {
    pub const MEETING_REQUESTED: &str = "meeting.requested";
    pub const MEETING_DECIDED: &str = "meeting.decided";
    pub const MEETING_DECLINED: &str = "meeting.declined";
    /// The room met and concluded there was nothing to decide.
    pub const MEETING_DROPPED: &str = "meeting.dropped";
    pub const MEETING_FAILED: &str = "meeting.failed";
    /// A gated agent wants to start a task and needs you (ADR-0012).
    pub const APPROVAL_REQUESTED: &str = "approval.requested";
}

/// A notification to record. `approval_id` is what makes it *actionable*: the
/// UI shows approve/reject on it.
pub struct New<'a> {
    pub kind: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    /// Who is telling you. `None` for the system itself.
    pub agent_id: Option<&'a str>,
    /// What to open when you act on it, e.g. `("meeting", id)`.
    pub subject: Option<(&'a str, &'a str)>,
    pub approval_id: Option<&'a str>,
}

/// Record a notification. Returns its JSON, which the caller pushes with
/// [`deliver`] *after* committing — never before, or a rolled-back transaction
/// would leave a phantom toast on screen.
pub async fn post(
    conn: &mut SqliteConnection,
    company_id: &str,
    n: New<'_>,
) -> Result<Value, sqlx::Error> {
    let id = uuid::Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let (subject_type, subject_id) = match n.subject {
        Some((t, i)) => (Some(t), Some(i)),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO notifications
         (id, company_id, kind, title, body, agent_id, subject_type, subject_id, approval_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(company_id)
    .bind(n.kind)
    .bind(n.title)
    .bind(n.body)
    .bind(n.agent_id)
    .bind(subject_type)
    .bind(subject_id)
    .bind(n.approval_id)
    .bind(&created_at)
    .execute(conn)
    .await?;
    Ok(json!({
        "id": id,
        "company_id": company_id,
        "kind": n.kind,
        "title": n.title,
        "body": n.body,
        "agent_id": n.agent_id,
        "subject_type": subject_type,
        "subject_id": subject_id,
        "approval_id": n.approval_id,
        "read_at": Value::Null,
        "created_at": created_at,
    }))
}

/// Push a recorded notification to connected clients, so an open app surfaces
/// it immediately instead of waiting for the next poll.
pub fn deliver(state: &AppState, company_id: &str, notification: &Value) {
    state.push(json!({
        "type": "notification",
        "company_id": company_id,
        "notification": notification,
    }));
}

/// Record and push in one go, outside any caller transaction.
pub async fn send(state: &AppState, company_id: &str, n: New<'_>) -> Result<(), sqlx::Error> {
    let mut conn = state.pool.acquire().await?;
    let value = post(&mut conn, company_id, n).await?;
    drop(conn);
    deliver(state, company_id, &value);
    Ok(())
}
