//! ADR-0019 (extends ADR-0018): the conversational layer.
//!
//! The user can talk to any agent in its role. Posting a message runs that
//! agent's turn over the conversation: it produces a reply and a structured
//! plan — tasks (optionally assigned to a teammate, the "ripple") and an
//! optional escalation to the org leader. The plan is applied server-side and
//! audited. Structured-first (ADR-0005): the agent's actions are validated
//! JSON, never free prose that silently mutates state. The CEO is simply the
//! conversation with the org leader.

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

/// Get the thread with a given agent, creating it on first use. One thread per
/// (company, agent). The "CEO thread" is just the org leader's.
async fn get_or_create_conversation(
    state: &AppState,
    company_id: &str,
    agent_id: &str,
) -> Result<String, CeoError> {
    if let Some((id,)) = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM conversations WHERE company_id = ? AND agent_id = ?",
    )
    .bind(company_id)
    .bind(agent_id)
    .fetch_optional(&state.pool)
    .await?
    {
        return Ok(id);
    }
    let agent: Option<(String, String)> =
        sqlx::query_as("SELECT status, name FROM agents WHERE id = ? AND company_id = ?")
            .bind(agent_id)
            .bind(company_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((status, name)) = agent else {
        return Err(CeoError::NotFound("agent"));
    };
    if status != "active" {
        return Err(CeoError::Invalid(format!("agent is {status}")));
    }
    let id = new_id();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO conversations (id, company_id, agent_id, title, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(company_id)
    .bind(agent_id)
    .bind(&name)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::CONVERSATION_CREATED,
        &json!({ "conversation_id": id, "agent_id": agent_id }),
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

/// Store an uploaded file against an agent's thread. The bytes go to disk; the
/// row is created unlinked (`message_id` NULL) and is attached to the user's
/// message when they post it (see `post_user_message`).
pub async fn store_attachment(
    state: &AppState,
    company_id: &str,
    agent_id: &str,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<AttachmentMeta, CeoError> {
    if bytes.is_empty() {
        return Err(CeoError::Invalid("attachment is empty".into()));
    }
    let conversation_id = get_or_create_conversation(state, company_id, agent_id).await?;
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

/// Post a user message to an agent's thread and launch that agent's turn. Any
/// `attachment_ids` (already uploaded via `store_attachment`) are linked to the
/// new message and reach the agent in its working directory. Returns the
/// conversation id; the reply and any tasks land asynchronously over `/ws`.
pub async fn post_user_message(
    state: &AppState,
    company_id: &str,
    agent_id: &str,
    content: &str,
    attachment_ids: &[String],
) -> Result<String, CeoError> {
    if content.trim().is_empty() && attachment_ids.is_empty() {
        return Err(CeoError::Invalid("message must not be empty".into()));
    }
    let conversation_id = get_or_create_conversation(state, company_id, agent_id).await?;
    let agent = agent_id.to_string();

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

    // The agent's turn runs in the background; its reply + tasks land via notify.
    let state2 = state.clone();
    let company = company_id.to_string();
    let convo = conversation_id.clone();
    tokio::spawn(async move {
        if let Err(e) = run_agent_turn(&state2, &company, &convo, &agent).await {
            eprintln!("agent turn for {convo} failed: {e}");
            // Don't leave the user hanging: surface the failure as a message.
            let _ = post_system_message(
                &state2,
                &company,
                &convo,
                &format!("The agent could not respond: {e}"),
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

/// The org leader (an agent that reports to the human), if any — the "CEO".
pub(crate) async fn leader_id(
    state: &AppState,
    company_id: &str,
) -> Result<Option<String>, CeoError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM agents
         WHERE company_id = ? AND status = 'active' AND reports_to IS NULL
         ORDER BY created_at LIMIT 1",
    )
    .bind(company_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Resolve a teammate named in a plan (by name or title) to an active agent id.
pub(crate) async fn resolve_teammate(
    state: &AppState,
    company_id: &str,
    name: &str,
) -> Result<Option<String>, CeoError> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM agents
         WHERE company_id = ? AND status = 'active'
           AND (lower(name) = lower(?) OR lower(coalesce(title, '')) = lower(?))
         LIMIT 1",
    )
    .bind(company_id)
    .bind(name)
    .bind(name)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Who is speaking: (name, title, archetype slug, traits, custom_brief, reports_to).
type SpeakerRow = (
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
);

/// Run an agent over its conversation, then store its reply, open the tasks it
/// proposed (optionally assigned to a teammate), and escalate to the leader.
async fn run_agent_turn(
    state: &AppState,
    company_id: &str,
    conversation_id: &str,
    agent_id: &str,
) -> Result<(), CeoError> {
    // Who is speaking, and their role.
    let who: Option<SpeakerRow> = sqlx::query_as(
        "SELECT a.name, a.title, ar.slug, a.traits, a.custom_brief, a.reports_to
             FROM agents a JOIN archetypes ar ON ar.id = a.archetype_id
             WHERE a.id = ? AND a.company_id = ?",
    )
    .bind(agent_id)
    .bind(company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((name, title, slug, traits, custom_brief, reports_to)) = who else {
        return Err(CeoError::NotFound("agent"));
    };
    let is_leader = reports_to.is_none();
    let role = title.clone().unwrap_or_else(|| slug.clone());

    // The rest of the team (for delegation / assignment).
    let team: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT a.name, a.title, ar.slug FROM agents a
         JOIN archetypes ar ON ar.id = a.archetype_id
         WHERE a.company_id = ? AND a.status = 'active' AND a.id != ?",
    )
    .bind(company_id)
    .bind(agent_id)
    .fetch_all(&state.pool)
    .await?;
    let team_block = if team.is_empty() {
        "(no teammates hired yet)".to_string()
    } else {
        team.iter()
            .map(|(n, t, s)| format!("- {n} ({})", t.clone().unwrap_or_else(|| s.clone())))
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
        .map(|(r, c)| format!("{r}: {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    let last_user = history
        .iter()
        .rev()
        .find(|(r, _)| r == "user")
        .map(|(_, c)| c.clone())
        .unwrap_or_default();

    let memory_context = state
        .memory
        .get_context(&state.config.data_dir.to_string_lossy(), &last_user)
        .await;
    let memory_block = memory_context
        .as_deref()
        .map(|m| format!("\n\nWhat the organization remembers:\n{m}"))
        .unwrap_or_default();

    // Calls this agent sat in on are settled — it acts on them (ADR-0020).
    let decisions_block = crate::meeting::decisions_block(state, agent_id).await;
    // The company's language (M16).
    let language =
        crate::i18n::prompt_line(&crate::i18n::company_language(state, company_id).await);

    // Only the leader designs the organization (M15), and it can only propose
    // roles that exist: hand it the catalogue rather than let it invent slugs.
    let catalogue_block = if is_leader {
        let slugs: Vec<(String, String)> =
            sqlx::query_as("SELECT slug, name FROM archetypes ORDER BY slug")
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default();
        let catalogue = slugs
            .iter()
            .map(|(slug, name)| format!("  {slug} — {name}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nRoles you can hire (use the slug exactly):\n{catalogue}{}",
            crate::org::feedback_block(state, agent_id).await
        )
    } else {
        String::new()
    };

    // Files the user attached — copied into the working directory below.
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
            .map(|(n, _)| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nThe user attached these files, now in your working directory — open them if relevant:\n{list}"
        )
    };

    let persona = if is_leader {
        format!(
            "You are {name}, the CEO of an AI company. The user is talking to you. Reply helpfully, and when work is needed, delegate it by proposing tasks — assign each to the right teammate by name. \
             When the user describes an idea and the company does not yet have the people for it, your job is to design the organization: propose a team with \"team\" (see below). You are not obliged to — if the people you have can do it, say so and get on with it."
        )
    } else {
        format!(
            "You are {name}, the {role} at an AI company. The user is talking to you directly, in your role. Reply in character. Propose tasks only when work is genuinely needed: assign one to a teammate by name when it is their job, or leave it unassigned for the team. If the request affects the wider company beyond your own role, set \"escalate\" with a short note for the CEO."
        )
    };
    let brief_line = custom_brief
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| format!("\nYour brief: {b}"))
        .unwrap_or_default();

    let prompt = format!(
        "{persona}{brief_line}\n\nYour teammates:\n{team_block}\n\nConversation so far:\n{convo_block}{memory_block}{decisions_block}{catalogue_block}{attach_block}\n\nRespond with a SINGLE JSON object on the LAST line of your output, and nothing after it:\n{{\"reply\": \"<your message to the user>\", \"tasks\": [{{\"title\": \"...\", \"description\": \"...\", \"execution_kind\": \"knowledge\", \"assignee\": \"<teammate name, optional>\"}}], \"escalate\": \"<optional note for the CEO>\", \"meeting\": {{\"topic\": \"...\", \"reason\": \"why the room is needed\", \"participants\": [\"<teammate name>\"], \"turn_cap\": 6}}, \"team\": {{\"summary\": \"why this shape\", \"members\": [{{\"name\": \"...\", \"archetype\": \"<slug from the list>\", \"title\": \"...\", \"reports_to\": \"<another member's name, or omit to report to you>\", \"brief\": \"...\", \"why\": \"why this person is on the team\"}}]}}}}\nUse \"knowledge\" for research/documents and \"code\" for software changes. Omit \"assignee\" to leave a task unassigned. Ask for a \"meeting\" only when the call genuinely needs colleagues in one room — a decision you should not take alone; name who must be there and say why. The human approves it before anyone meets, you may have only ONE request waiting at a time, and every request costs them an interruption — if you can take the call yourself, take it. Propose a \"team\" only when the company lacks the people for what the user described: name each hire, pick an archetype slug from the list above, say who they report to and why they are there. Nobody is hired until the user accepts, and they can drop members first. Return an empty tasks array and omit escalate/meeting/team when nothing is needed.{language}"
    );

    // Run the adapter in a throwaway scratch dir.
    let scratch = state.config.data_dir.join("chat").join(new_id());
    tokio::fs::create_dir_all(&scratch)
        .await
        .map_err(|e| CeoError::Invalid(format!("cannot create scratch dir: {e}")))?;
    // Copy the attachments in so the agent can read (or see) them by filename.
    for (n, path) in &attachments {
        let _ = tokio::fs::copy(path, scratch.join(safe_name(n))).await;
    }
    let output = run_adapter(state, &scratch, &prompt, &traits, memory_context.as_deref()).await?;

    // Parse the plan. A missing/garbled plan degrades to "reply with the raw
    // output, open no tasks" rather than failing the turn.
    let plan = plan_json(&output);
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

    // Resolve any task assignees before opening the write transaction.
    let mut resolved: Vec<(String, String, &'static str, Option<String>)> = Vec::new();
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
        let assignee = match t.get("assignee").and_then(Value::as_str) {
            Some(n) => resolve_teammate(state, company_id, n).await?,
            None => None,
        };
        resolved.push((title.to_string(), description.to_string(), kind, assignee));
    }

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

    for (title, description, kind, assignee) in &resolved {
        let task_id = new_id();
        sqlx::query(
            "INSERT INTO tasks (id, company_id, goal_id, title, description, status, priority, execution_kind, assignee_agent_id, created_at, updated_at)
             VALUES (?, ?, NULL, ?, ?, 'todo', 'medium', ?, ?, ?, ?)",
        )
        .bind(&task_id)
        .bind(company_id)
        .bind(title)
        .bind(description)
        .bind(kind)
        .bind(assignee.as_deref())
        .bind(now())
        .bind(now())
        .execute(&mut *tx)
        .await?;
        audit::append(
            &mut tx,
            Some(company_id),
            Some(&task_id),
            event_kind::TASK_CREATED,
            &json!({
                "title": title, "execution_kind": kind, "via": name,
                "assignee_agent_id": assignee, "conversation_id": conversation_id,
            }),
        )
        .await?;
    }
    tx.commit().await?;
    state.notify(company_id);

    // Cross-impact: a specialist can escalate to the leader (its own thread).
    if !is_leader
        && let Some(escalate) = plan
            .as_ref()
            .and_then(|v| v.get("escalate").and_then(Value::as_str))
    {
        let escalate = escalate.trim();
        if !escalate.is_empty()
            && let Some(leader) = leader_id(state, company_id).await?
            && leader != agent_id
        {
            let leader_convo = get_or_create_conversation(state, company_id, &leader).await?;
            let _ = post_system_message(
                state,
                company_id,
                &leader_convo,
                &format!("Escalation from {name}: {escalate}"),
            )
            .await;
        }
    }

    // A call the agent shouldn't make alone: it asks for a meeting. Nothing
    // deliberates until the human approves it (ADR-0020). A malformed or
    // impossible request is dropped rather than failing the whole turn.
    if let Some(m) = plan.as_ref().and_then(|v| v.get("meeting"))
        && let Some(req) = crate::meeting::Request::from_json(m)
        && let Err(e) = crate::meeting::request(state, company_id, agent_id, &req).await
    {
        eprintln!("meeting requested by {name} could not be raised: {e}");
    }

    // The CEO has designed an organization (M15). Proposed, never applied:
    // hiring is the user's decision, and only the leader may draw one up.
    if is_leader
        && let Some(t) = plan.as_ref().and_then(|v| v.get("team"))
        && let Some(proposal) = crate::org::Proposal::from_json(t)
        && let Err(e) = crate::org::propose(state, company_id, agent_id, &proposal).await
    {
        eprintln!("team proposed by {name} could not be raised: {e}");
    }
    Ok(())
}

/// Run the configured agent adapter with a prompt, in `cwd`, and return its raw
/// stdout. Bounded by `session_timeout_secs` — an adapter that hangs can never
/// hold a turn (or a meeting, ADR-0020) open forever. Shared by conversational
/// turns and meetings.
pub(crate) async fn run_adapter(
    state: &AppState,
    cwd: &std::path::Path,
    prompt: &str,
    traits: &str,
    memory_context: Option<&str>,
) -> Result<String, CeoError> {
    let agent_cmd =
        state.config.agent_cmd.clone().unwrap_or_else(|| {
            "claude -p \"$OVERMIND_TASK_PROMPT\" --output-format json".to_string()
        });
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&agent_cmd)
        .current_dir(cwd)
        .env("OVERMIND_TASK_PROMPT", prompt)
        .env("OVERMIND_AGENT_TRAITS", traits)
        .env("OVERMIND_MEMORY_CONTEXT", memory_context.unwrap_or(""))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| CeoError::Invalid(format!("failed to spawn agent: {e}")))?;
    let waited = tokio::time::timeout(
        Duration::from_secs(state.config.session_timeout_secs),
        child.wait_with_output(),
    )
    .await;
    match waited {
        Ok(Ok(out)) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(Err(e)) => Err(CeoError::Invalid(format!(
            "failed to read agent output: {e}"
        ))),
        Err(_) => Err(CeoError::Invalid("agent turn timed out".into())),
    }
}

/// The agent's own plan, picked out of its output.
///
/// Not simply the last JSON object: an adapter prints its **own** result
/// envelope (cost, usage, session id) after whatever the agent said, so the
/// last object on stdout is usually the adapter's, not the agent's. We take
/// the last one that actually looks like a plan.
fn plan_json(output: &str) -> Option<Value> {
    for line in output.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line)
            && (v.get("reply").is_some() || v.get("tasks").is_some() || v.get("meeting").is_some())
        {
            return Some(v);
        }
    }
    last_json_object(output)
}

/// The last line of output that parses as a JSON object.
pub(crate) fn last_json_object(output: &str) -> Option<Value> {
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
