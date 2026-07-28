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

/// Get the company's CEO thread, creating it on first use. The caller names
/// which agent is the CEO (the org leader, in the UI).
async fn get_or_create_conversation(
    state: &AppState,
    company_id: &str,
    ceo_agent_id: &str,
) -> Result<String, CeoError> {
    if let Some((id,)) =
        sqlx::query_as::<_, (String,)>("SELECT id FROM conversations WHERE company_id = ?")
            .bind(company_id)
            .fetch_optional(&state.pool)
            .await?
    {
        return Ok(id);
    }
    let agent: Option<(String,)> =
        sqlx::query_as("SELECT status FROM agents WHERE id = ? AND company_id = ?")
            .bind(ceo_agent_id)
            .bind(company_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((status,)) = agent else {
        return Err(CeoError::NotFound("agent"));
    };
    if status != "active" {
        return Err(CeoError::Invalid(format!("CEO agent is {status}")));
    }
    let id = new_id();
    let mut tx = state.pool.begin().await?;
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
    tx.commit().await?;
    state.notify(company_id);
    Ok(id)
}

/// Metadata for a stored attachment, returned to the client.
pub struct AttachmentMeta {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size_bytes: i64,
}

/// A path-free, filesystem-safe basename for an uploaded file.
fn safe_name(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

/// Store an uploaded file against the company's CEO thread. The bytes go to
/// disk; the row is created unlinked (`message_id` NULL) and is attached to the
/// user's message when they post it (see `post_user_message`).
pub async fn store_attachment(
    state: &AppState,
    company_id: &str,
    ceo_agent_id: &str,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<AttachmentMeta, CeoError> {
    if bytes.is_empty() {
        return Err(CeoError::Invalid("attachment is empty".into()));
    }
    let conversation_id = get_or_create_conversation(state, company_id, ceo_agent_id).await?;
    let name = safe_name(filename);
    let id = new_id();
    let dir = state
        .config
        .data_dir
        .join("attachments")
        .join(&conversation_id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| CeoError::Invalid(format!("cannot create attachments dir: {e}")))?;
    let path = dir.join(format!("{id}_{name}"));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| CeoError::Invalid(format!("cannot write attachment: {e}")))?;
    let size = bytes.len() as i64;
    sqlx::query(
        "INSERT INTO attachments (id, conversation_id, message_id, filename, mime, size_bytes, path, created_at)
         VALUES (?, ?, NULL, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&conversation_id)
    .bind(&name)
    .bind(mime)
    .bind(size)
    .bind(path.to_string_lossy().as_ref())
    .bind(now())
    .execute(&state.pool)
    .await?;
    Ok(AttachmentMeta {
        id,
        filename: name,
        mime: mime.to_string(),
        size_bytes: size,
    })
}

/// Post a user message to the company's CEO thread and launch the CEO's turn.
/// Any `attachment_ids` (already uploaded via `store_attachment`) are linked to
/// the new message and reach the CEO in its working directory. Returns the
/// conversation id; the CEO's reply and any tasks land asynchronously over `/ws`.
pub async fn post_user_message(
    state: &AppState,
    company_id: &str,
    ceo_agent_id: &str,
    content: &str,
    attachment_ids: &[String],
) -> Result<String, CeoError> {
    if content.trim().is_empty() && attachment_ids.is_empty() {
        return Err(CeoError::Invalid("message must not be empty".into()));
    }
    let conversation_id = get_or_create_conversation(state, company_id, ceo_agent_id).await?;
    let ceo_id = ceo_agent_id.to_string();

    let message_id = new_id();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, role, content, created_at)
         VALUES (?, ?, 'user', ?, ?)",
    )
    .bind(&message_id)
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
    // Link any staged attachments to this message (only ones in this thread,
    // not already linked — so ids can't be replayed across messages).
    for att_id in attachment_ids {
        let linked = sqlx::query(
            "UPDATE attachments SET message_id = ?
             WHERE id = ? AND conversation_id = ? AND message_id IS NULL",
        )
        .bind(&message_id)
        .bind(att_id)
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await?;
        if linked.rows_affected() == 1 {
            audit::append(
                &mut tx,
                Some(company_id),
                None,
                event_kind::ATTACHMENT_ADDED,
                &json!({ "conversation_id": conversation_id, "attachment_id": att_id }),
            )
            .await?;
        }
    }
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

    // Files the user attached to the thread — copied into the CEO's working
    // directory below, and listed here so it knows to open them.
    let attachments: Vec<(String, String)> = sqlx::query_as(
        "SELECT filename, path FROM attachments
         WHERE conversation_id = ? AND message_id IS NOT NULL ORDER BY created_at",
    )
    .bind(conversation_id)
    .fetch_all(&state.pool)
    .await?;
    let attach_block = if attachments.is_empty() {
        String::new()
    } else {
        let list = attachments
            .iter()
            .map(|(name, _)| format!("- {name}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nThe user attached these files, now in your working directory — open them if relevant:\n{list}"
        )
    };

    let prompt = format!(
        "You are the CEO of an AI company. The user is talking to you. Reply helpfully, and when work is needed, delegate it by proposing tasks for your team.\n\nYour team:\n{team_block}\n\nConversation so far:\n{convo_block}{memory_block}{attach_block}\n\nRespond with a SINGLE JSON object on the LAST line of your output, and nothing after it:\n{{\"reply\": \"<your message to the user>\", \"tasks\": [{{\"title\": \"...\", \"description\": \"...\", \"execution_kind\": \"knowledge\"}}]}}\nUse \"knowledge\" for research/documents and \"code\" for software changes. Propose tasks only when the user actually needs work done; otherwise return an empty tasks array."
    );

    // Run the adapter in a throwaway scratch dir.
    let scratch = state.config.data_dir.join("ceo").join(new_id());
    tokio::fs::create_dir_all(&scratch)
        .await
        .map_err(|e| CeoError::Invalid(format!("cannot create ceo scratch dir: {e}")))?;
    // Copy the attachments in so the agent can read (or see) them by filename.
    for (name, path) in &attachments {
        let _ = tokio::fs::copy(path, scratch.join(safe_name(name))).await;
    }
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
