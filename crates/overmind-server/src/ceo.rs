//! M12 / ADR-0018: the conversational CEO layer.
//!
//! Posting a user message triggers the CEO agent to run over the conversation:
//! it produces a reply and a structured plan of tasks to open. The plan is
//! applied server-side (tasks created and audited); the reply is stored as a
//! CEO message. Structured-first (ADR-0005): the CEO's actions are validated
//! JSON, never free prose that silently mutates state.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::audit;
use crate::db::AppState;
use crate::domain::event_kind;

#[derive(Debug, thiserror::Error)]
pub enum CeoError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Post a user message to the company's CEO thread and launch the CEO's turn.
/// The thread is created on first use (the caller names which agent is the CEO).
/// Returns the conversation id; the CEO's reply and any tasks land
/// asynchronously and are announced over the live channel.
pub async fn post_user_message(
    state: &AppState,
    company_id: &str,
    ceo_agent_id: &str,
    content: &str,
) -> Result<String, CeoError> {
    if content.trim().is_empty() {
        return Err(CeoError::Invalid("message must not be empty".into()));
    }

    let existing: Option<(String, String)> =
        sqlx::query_as("SELECT id, ceo_agent_id FROM conversations WHERE company_id = ?")
            .bind(company_id)
            .fetch_optional(&state.pool)
            .await?;

    let mut tx = state.pool.begin().await?;
    let (conversation_id, ceo_id) = match existing {
        Some(pair) => pair,
        None => {
            let agent: Option<(String,)> =
                sqlx::query_as("SELECT status FROM agents WHERE id = ? AND company_id = ?")
                    .bind(ceo_agent_id)
                    .bind(company_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some((status,)) = agent else {
                return Err(CeoError::NotFound("agent"));
            };
            if status != "active" {
                return Err(CeoError::Invalid(format!("CEO agent is {status}")));
            }
            let id = new_id();
            sqlx::query(
                "INSERT INTO conversations (id, company_id, ceo_agent_id, title, created_at)
                 VALUES (?, ?, ?, 'CEO', ?)",
            )
            .bind(&id)
            .bind(company_id)
            .bind(ceo_agent_id)
            .bind(now())
            .execute(&mut *tx)
            .await?;
            audit::append(
                &mut tx,
                Some(company_id),
                None,
                event_kind::CONVERSATION_CREATED,
                &json!({ "conversation_id": id, "ceo_agent_id": ceo_agent_id }),
            )
            .await?;
            (id, ceo_agent_id.to_string())
        }
    };

    sqlx::query(
        "INSERT INTO messages (id, conversation_id, role, content, created_at)
         VALUES (?, ?, 'user', ?, ?)",
    )
    .bind(new_id())
    .bind(&conversation_id)
    .bind(content)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MESSAGE_POSTED,
        &json!({ "conversation_id": conversation_id, "role": "user" }),
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);

    // The CEO's turn runs in the background; its reply + tasks land via notify.
    let state2 = state.clone();
    let company = company_id.to_string();
    let convo = conversation_id.clone();
    tokio::spawn(async move {
        if let Err(e) = run_ceo_turn(&state2, &company, &convo, &ceo_id).await {
            eprintln!("ceo turn for {convo} failed: {e}");
            // Don't leave the user hanging: surface the failure as a message.
            let _ = post_system_message(
                &state2,
                &company,
                &convo,
                &format!("The CEO could not respond: {e}"),
            )
            .await;
        }
    });

    Ok(conversation_id)
}

async fn post_system_message(
    state: &AppState,
    company_id: &str,
    conversation_id: &str,
    content: &str,
) -> Result<(), CeoError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, role, content, created_at)
         VALUES (?, ?, 'system', ?, ?)",
    )
    .bind(new_id())
    .bind(conversation_id)
    .bind(content)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MESSAGE_POSTED,
        &json!({ "conversation_id": conversation_id, "role": "system" }),
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    Ok(())
}

/// Run the CEO agent over the conversation, then store its reply and open the
/// tasks it proposed.
async fn run_ceo_turn(
    state: &AppState,
    company_id: &str,
    conversation_id: &str,
    ceo_agent_id: &str,
) -> Result<(), CeoError> {
    // The team the CEO can delegate to, and the CEO's own model.
    let team: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT a.name, a.title, ar.slug FROM agents a
         JOIN archetypes ar ON ar.id = a.archetype_id
         WHERE a.company_id = ? AND a.status = 'active' AND a.id != ?",
    )
    .bind(company_id)
    .bind(ceo_agent_id)
    .fetch_all(&state.pool)
    .await?;
    let team_block = if team.is_empty() {
        "(no team hired yet)".to_string()
    } else {
        team.iter()
            .map(|(name, title, slug)| {
                format!(
                    "- {name} ({})",
                    title.clone().unwrap_or_else(|| slug.clone())
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let history: Vec<(String, String)> = sqlx::query_as(
        "SELECT role, content FROM messages WHERE conversation_id = ? ORDER BY created_at",
    )
    .bind(conversation_id)
    .fetch_all(&state.pool)
    .await?;
    let convo_block = history
        .iter()
        .map(|(role, content)| format!("{role}: {content}"))
        .collect::<Vec<_>>()
        .join("\n");
    let last_user = history
        .iter()
        .rev()
        .find(|(role, _)| role == "user")
        .map(|(_, c)| c.clone())
        .unwrap_or_default();

    let ceo_traits: Option<(String,)> = sqlx::query_as("SELECT traits FROM agents WHERE id = ?")
        .bind(ceo_agent_id)
        .fetch_optional(&state.pool)
        .await?;
    let ceo_traits = ceo_traits.map(|(t,)| t).unwrap_or_default();

    let memory_context = state
        .memory
        .get_context(&state.config.data_dir.to_string_lossy(), &last_user)
        .await;
    let memory_block = memory_context
        .as_deref()
        .map(|m| format!("\n\nWhat the organization remembers:\n{m}"))
        .unwrap_or_default();

    let prompt = format!(
        "You are the CEO of an AI company. The user is talking to you. Reply helpfully, and when work is needed, delegate it by proposing tasks for your team.\n\nYour team:\n{team_block}\n\nConversation so far:\n{convo_block}{memory_block}\n\nRespond with a SINGLE JSON object on the LAST line of your output, and nothing after it:\n{{\"reply\": \"<your message to the user>\", \"tasks\": [{{\"title\": \"...\", \"description\": \"...\", \"execution_kind\": \"knowledge\"}}]}}\nUse \"knowledge\" for research/documents and \"code\" for software changes. Propose tasks only when the user actually needs work done; otherwise return an empty tasks array."
    );

    // Run the adapter in a throwaway scratch dir.
    let scratch = state.config.data_dir.join("ceo").join(new_id());
    tokio::fs::create_dir_all(&scratch)
        .await
        .map_err(|e| CeoError::Invalid(format!("cannot create ceo scratch dir: {e}")))?;
    let agent_cmd =
        state.config.agent_cmd.clone().unwrap_or_else(|| {
            "claude -p \"$OVERMIND_TASK_PROMPT\" --output-format json".to_string()
        });
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&agent_cmd)
        .current_dir(&scratch)
        .env("OVERMIND_TASK_PROMPT", &prompt)
        .env("OVERMIND_AGENT_TRAITS", &ceo_traits)
        .env(
            "OVERMIND_MEMORY_CONTEXT",
            memory_context.as_deref().unwrap_or(""),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| CeoError::Invalid(format!("failed to spawn CEO agent: {e}")))?;
    let waited = tokio::time::timeout(
        Duration::from_secs(state.config.session_timeout_secs),
        child.wait_with_output(),
    )
    .await;
    let output = match waited {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(Err(e)) => return Err(CeoError::Invalid(format!("failed to read CEO output: {e}"))),
        Err(_) => return Err(CeoError::Invalid("CEO turn timed out".into())),
    };

    // Parse the plan. A missing/garbled plan degrades to "reply with the raw
    // output, open no tasks" rather than failing the turn.
    let plan = last_json_object(&output);
    let reply = plan
        .as_ref()
        .and_then(|v| v.get("reply").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| output.trim().to_string());
    let tasks = plan
        .as_ref()
        .and_then(|v| v.get("tasks").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, role, content, created_at)
         VALUES (?, ?, 'ceo', ?, ?)",
    )
    .bind(new_id())
    .bind(conversation_id)
    .bind(&reply)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MESSAGE_POSTED,
        &json!({ "conversation_id": conversation_id, "role": "ceo" }),
    )
    .await?;

    for t in &tasks {
        let title = t.get("title").and_then(Value::as_str).unwrap_or("").trim();
        if title.is_empty() {
            continue;
        }
        let description = t
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let kind = match t.get("execution_kind").and_then(Value::as_str) {
            Some("code") => "code",
            _ => "knowledge",
        };
        let task_id = new_id();
        sqlx::query(
            "INSERT INTO tasks (id, company_id, goal_id, title, description, status, priority, execution_kind, created_at, updated_at)
             VALUES (?, ?, NULL, ?, ?, 'todo', 'medium', ?, ?, ?)",
        )
        .bind(&task_id)
        .bind(company_id)
        .bind(title)
        .bind(description)
        .bind(kind)
        .bind(now())
        .bind(now())
        .execute(&mut *tx)
        .await?;
        audit::append(
            &mut tx,
            Some(company_id),
            Some(&task_id),
            event_kind::TASK_CREATED,
            &json!({ "title": title, "execution_kind": kind, "via": "ceo", "conversation_id": conversation_id }),
        )
        .await?;
    }
    tx.commit().await?;
    state.notify(company_id);
    Ok(())
}

/// The last line of output that parses as a JSON object.
fn last_json_object(output: &str) -> Option<Value> {
    for line in output.lines().rev() {
        let line = line.trim();
        if line.starts_with('{')
            && let Ok(v) = serde_json::from_str::<Value>(line)
        {
            return Some(v);
        }
    }
    None
}
