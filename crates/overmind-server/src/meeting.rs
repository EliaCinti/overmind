//! ADR-0020 (builds on ADR-0019): inter-agent meetings.
//!
//! Agents collaborate on their own — a specialist opens a task for a teammate,
//! escalates to the leader, works its own tasks. Sometimes that collaboration
//! hits a call none of them should make alone. Then one of them **asks for a
//! meeting**: it names the room and says why, in its own words.
//!
//! Nothing runs on that alone. The request raises a notification and an
//! approval; the human decides. On approval — and only then — the agents
//! deliberate among themselves: round-robin, at most `turn_cap` turns, always
//! ending in a **recorded decision** (the chair must call it if nobody else
//! does). The decision is audited, stored to organizational memory, and comes
//! back as a notification.
//!
//! What this is *not*: a free-form group chat, and not something that spends a
//! single token before you have said yes.

use serde_json::{Value, json};

use crate::audit;
use crate::ceo::{CeoError, leader_id, resolve_teammate, run_adapter};
use crate::db::AppState;
use crate::domain::event_kind;
use crate::notify;

/// However long a caller asks for, a meeting never exceeds this many turns.
const MAX_TURN_CAP: i64 = 12;

/// The default when an agent asks for a meeting without saying how long.
const DEFAULT_TURN_CAP: i64 = 6;

/// An agent may have **one** meeting request waiting on the human at a time.
/// Autonomy is not the right to interrupt without limit: until you have
/// answered the last one, the agent works with what it has.
const MAX_PENDING_PER_AGENT: i64 = 1;

/// And the company as a whole may not queue more than this. Ten agents each
/// asking once is still ten interruptions.
const MAX_PENDING_PER_COMPANY: i64 = 3;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// What an agent asks for when it wants colleagues in a room. The agent names
/// people the way it knows them; the server resolves them against the roster.
#[derive(Debug, Clone)]
pub struct Request {
    pub topic: String,
    /// Why the room is needed — this is what the human reads first.
    pub reason: String,
    pub participants: Vec<String>,
    pub turn_cap: i64,
}

impl Request {
    /// Parse the `meeting` object an agent emits — in a chat plan (ADR-0019) or
    /// as `MEETING_REQUEST.json` in its working directory. Returns `None` for
    /// anything without a topic: a meeting about nothing is not a request.
    pub fn from_json(v: &Value) -> Option<Request> {
        let topic = crate::ceo::clamp_agent_text(v.get("topic").and_then(Value::as_str)?);
        if topic.is_empty() {
            return None;
        }
        let reason =
            crate::ceo::clamp_agent_text(v.get("reason").and_then(Value::as_str).unwrap_or(""));
        let participants = v
            .get("participants")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let turn_cap = v
            .get("turn_cap")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_TURN_CAP);
        Some(Request {
            topic,
            reason,
            participants,
            turn_cap,
        })
    }
}

/// An agent asks to bring colleagues together. Records the request, the
/// approval that gates it, and the notification that reaches the human —
/// atomically. Nothing deliberates until [`approve`] is called.
///
/// The convener is always in its own room. Named teammates are resolved by
/// name or title; if none resolve, the org leader is brought in, because an
/// agent asking for a meeting is at minimum asking for its boss.
pub async fn request(
    state: &AppState,
    company_id: &str,
    convener_agent_id: &str,
    req: &Request,
) -> Result<String, CeoError> {
    let topic = req.topic.trim();
    if topic.is_empty() {
        return Err(CeoError::Invalid("a meeting needs a topic".into()));
    }

    // Restraint (M13.5). Checked before any work: an agent that is already
    // waiting on you does not get to ask again, and the inbox has a ceiling.
    let (mine,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM meetings WHERE convener_agent_id = ? AND status = 'requested'",
    )
    .bind(convener_agent_id)
    .fetch_one(&state.pool)
    .await?;
    if mine >= MAX_PENDING_PER_AGENT {
        return Err(CeoError::Invalid(
            "you already have a meeting request waiting on the human; carry on with what you have until it is answered".into(),
        ));
    }
    // Paused rooms count here too (ADR-0022). The ceiling exists to stop rooms
    // piling up unnoticed, and a room paused for budget is exactly a room
    // piling up unnoticed — leaving it uncounted would reopen M13.5's hole from
    // a new direction.
    let (queued,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM meetings
         WHERE company_id = ? AND status IN ('requested', 'paused')",
    )
    .bind(company_id)
    .fetch_one(&state.pool)
    .await?;
    if queued >= MAX_PENDING_PER_COMPANY {
        return Err(CeoError::Invalid(format!(
            "the company already has {queued} meetings waiting on the human"
        )));
    }

    // Build the room: the convener, then whoever it named, de-duplicated.
    let mut room: Vec<String> = vec![convener_agent_id.to_string()];
    for name in &req.participants {
        if let Some(id) = resolve_teammate(state, company_id, name).await?
            && !room.contains(&id)
        {
            room.push(id);
        }
    }
    if room.len() < 2
        && let Some(leader) = leader_id(state, company_id).await?
        && !room.contains(&leader)
    {
        room.push(leader);
    }
    if room.len() < 2 {
        return Err(CeoError::Invalid(
            "a meeting needs at least two participants".into(),
        ));
    }
    let turn_cap = req.turn_cap.clamp(1, MAX_TURN_CAP);

    // Names for the human-readable notification.
    let convener = agent_label(state, convener_agent_id).await?;
    let mut labels = Vec::new();
    for id in &room {
        labels.push(agent_label(state, id).await?);
    }
    let roster = labels.join(", ");
    let reason = if req.reason.trim().is_empty() {
        "(no reason given)".to_string()
    } else {
        req.reason.trim().to_string()
    };
    let body = format!(
        "Topic: {topic}\n\nWhy: {reason}\n\nIn the room: {roster}\n\nUp to {turn_cap} turns, then whoever chairs it must call the decision. Nothing runs until you approve."
    );

    let meeting_id = new_id();
    let approval_id = new_id();
    let mut tx = state.pool.begin().await?;
    // The approval first: the meeting row points at it, and SQLite checks
    // foreign keys as each statement runs, not at commit.
    sqlx::query(
        "INSERT INTO approvals (id, company_id, type, status, payload, summary, created_at)
         VALUES (?, ?, 'meeting_request', 'pending', ?, ?, ?)",
    )
    .bind(&approval_id)
    .bind(company_id)
    .bind(json!({ "meeting_id": meeting_id }).to_string())
    .bind(format!("{convener} asks to meet about \"{topic}\""))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO meetings
         (id, company_id, topic, reason, convener_agent_id, turn_cap, status, approval_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 'requested', ?, ?)",
    )
    .bind(&meeting_id)
    .bind(company_id)
    .bind(topic)
    .bind(&reason)
    .bind(convener_agent_id)
    .bind(turn_cap)
    .bind(&approval_id)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    for (position, agent_id) in room.iter().enumerate() {
        sqlx::query(
            "INSERT INTO meeting_participants (meeting_id, agent_id, position) VALUES (?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(agent_id)
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MEETING_REQUESTED,
        &json!({
            "meeting_id": meeting_id,
            "convener_agent_id": convener_agent_id,
            "topic": topic,
            "turn_cap": turn_cap,
            "participants": room,
            "approval_id": approval_id,
        }),
    )
    .await?;
    let notification = notify::post(
        &mut tx,
        company_id,
        notify::New {
            kind: notify::kind::MEETING_REQUESTED,
            title: &format!("{convener} wants to convene a meeting"),
            body: &body,
            // `reason` is the convener's own words — passed through, never
            // re-worded. Everything else is scaffolding the client can phrase.
            params: json!({
                "agent": convener,
                "topic": topic,
                "reason": reason,
                "roster": roster,
                "turnCap": turn_cap,
            }),
            agent_id: Some(convener_agent_id),
            subject: Some(("meeting", &meeting_id)),
            approval_id: Some(&approval_id),
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    notify::deliver(state, company_id, &notification);

    Ok(meeting_id)
}

/// Convene a meeting directly, without the approval gate — the human asking
/// for it *is* the approval. Agents cannot reach this path; they go through
/// [`request`].
pub async fn convene(
    state: &AppState,
    company_id: &str,
    topic: &str,
    participant_ids: &[String],
    turn_cap: i64,
) -> Result<String, CeoError> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err(CeoError::Invalid("a meeting needs a topic".into()));
    }
    let mut seen = std::collections::HashSet::new();
    let participants: Vec<String> = participant_ids
        .iter()
        .filter(|id| seen.insert((*id).clone()))
        .cloned()
        .collect();
    if participants.len() < 2 {
        return Err(CeoError::Invalid(
            "a meeting needs at least two participants".into(),
        ));
    }
    for id in &participants {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM agents WHERE id = ? AND company_id = ?")
                .bind(id)
                .bind(company_id)
                .fetch_optional(&state.pool)
                .await?;
        match row {
            None => return Err(CeoError::NotFound("agent")),
            Some((status,)) if status != "active" => {
                return Err(CeoError::Invalid(format!("agent is {status}")));
            }
            _ => {}
        }
    }
    let turn_cap = turn_cap.clamp(1, MAX_TURN_CAP);

    let meeting_id = new_id();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO meetings (id, company_id, topic, turn_cap, status, created_at)
         VALUES (?, ?, ?, ?, 'open', ?)",
    )
    .bind(&meeting_id)
    .bind(company_id)
    .bind(topic)
    .bind(turn_cap)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    for (position, agent_id) in participants.iter().enumerate() {
        sqlx::query(
            "INSERT INTO meeting_participants (meeting_id, agent_id, position) VALUES (?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(agent_id)
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MEETING_CONVENED,
        &json!({
            "meeting_id": meeting_id,
            "topic": topic,
            "turn_cap": turn_cap,
            "participants": participants,
            "convened_by": "human",
        }),
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    spawn_deliberation(state, company_id, &meeting_id);
    Ok(meeting_id)
}

/// The human approved the request: the room opens and the agents deliberate.
pub async fn approve(state: &AppState, meeting_id: &str) -> Result<(), CeoError> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT company_id, status, topic FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, status, topic)) = row else {
        return Err(CeoError::NotFound("meeting"));
    };
    if status != "requested" {
        return Err(CeoError::Invalid(format!("meeting is already {status}")));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE meetings SET status = 'open' WHERE id = ? AND status = 'requested'")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::MEETING_CONVENED,
        &json!({ "meeting_id": meeting_id, "topic": topic, "convened_by": "approval" }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    spawn_deliberation(state, &company_id, meeting_id);
    Ok(())
}

/// The human said no: the meeting never happens, and the agent that asked is
/// told so in the same place it asked.
pub async fn decline(
    state: &AppState,
    meeting_id: &str,
    note: Option<&str>,
) -> Result<(), CeoError> {
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT company_id, status, topic, convener_agent_id FROM meetings WHERE id = ?",
    )
    .bind(meeting_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((company_id, status, topic, convener)) = row else {
        return Err(CeoError::NotFound("meeting"));
    };
    if status != "requested" {
        return Err(CeoError::Invalid(format!("meeting is already {status}")));
    }
    let mut tx = state.pool.begin().await?;
    // The note is stored on the meeting, not only in the notification: it has
    // to reach the agent that asked (see `decisions_block`), or the same
    // request comes straight back on its next turn.
    sqlx::query(
        "UPDATE meetings SET status = 'declined', decline_note = ?, decided_at = ?
         WHERE id = ? AND status = 'requested'",
    )
    .bind(note.map(str::trim).filter(|n| !n.is_empty()))
    .bind(now())
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::MEETING_DECLINED,
        &json!({ "meeting_id": meeting_id, "note": note }),
    )
    .await?;
    let body = match note {
        Some(n) if !n.trim().is_empty() => {
            format!("You declined the meeting on \"{topic}\": {}", n.trim())
        }
        _ => format!("You declined the meeting on \"{topic}\". It will not run."),
    };
    let notification = notify::post(
        &mut tx,
        &company_id,
        notify::New {
            kind: notify::kind::MEETING_DECLINED,
            title: "Meeting declined",
            body: &body,
            params: json!({
                "topic": topic,
                "note": note.map(str::trim).filter(|n| !n.is_empty()),
            }),
            agent_id: convener.as_deref(),
            subject: Some(("meeting", meeting_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    notify::deliver(state, &company_id, &notification);
    Ok(())
}

/// Run the deliberation in the background; the transcript and the decision
/// land as they happen.
fn spawn_deliberation(state: &AppState, company_id: &str, meeting_id: &str) {
    let state = state.clone();
    let company = company_id.to_string();
    let id = meeting_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = run_meeting(&state, &company, &id).await {
            eprintln!("meeting {id} failed: {e}");
            // Never leave a meeting hanging open: record it and say so.
            if let Err(e) = fail_meeting(&state, &company, &id, &e.to_string()).await {
                eprintln!("could not mark meeting {id} failed: {e}");
            }
        }
    });
}

/// "Iris (Security Engineer)" — how an agent is named to the human.
async fn agent_label(state: &AppState, agent_id: &str) -> Result<String, CeoError> {
    let row: Option<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT a.name, a.title, ar.slug FROM agents a
         JOIN archetypes ar ON ar.id = a.archetype_id WHERE a.id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((name, title, slug)) = row else {
        return Err(CeoError::NotFound("agent"));
    };
    Ok(format!("{name} ({})", title.unwrap_or(slug)))
}

/// A participant, in speaking order.
struct Speaker {
    id: String,
    name: String,
    role: String,
    traits: String,
    brief: Option<String>,
    is_leader: bool,
}

/// The participants of a meeting, in speaking order.
async fn load_speakers(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
) -> Result<Vec<Speaker>, CeoError> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT a.id, a.name, a.title, ar.slug, a.traits, a.custom_brief, a.reports_to
         FROM meeting_participants mp
         JOIN agents a ON a.id = mp.agent_id
         JOIN archetypes ar ON ar.id = a.archetype_id
         WHERE mp.meeting_id = ? AND a.company_id = ?
         ORDER BY mp.position",
    )
    .bind(meeting_id)
    .bind(company_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, title, slug, traits, brief, reports_to)| Speaker {
                id,
                name,
                role: title.unwrap_or(slug),
                traits,
                brief,
                is_leader: reports_to.is_none(),
            },
        )
        .collect())
}

/// The transcript so far, in order. Empty for a room that has not spoken yet.
///
/// This is what makes a paused meeting resumable at all (ADR-0022): M13 has
/// persisted every contribution with its ordinal since the day it shipped, so
/// nothing has to be reconstructed or guessed.
async fn recorded_turns(
    state: &AppState,
    meeting_id: &str,
) -> Result<Vec<(String, String)>, CeoError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.name, t.content FROM meeting_turns t
         JOIN agents a ON a.id = t.agent_id
         WHERE t.meeting_id = ? ORDER BY t.ordinal",
    )
    .bind(meeting_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows)
}

/// Stop a room that has run out of budget, and tell the human what unblocks it.
///
/// Not a terminal state: the transcript stays, the turn cap is untouched, and
/// `resume` picks up at the ordinal it stopped at. The turn cap deliberately
/// does **not** refill — otherwise pausing would be the way to buy more
/// deliberation than was ever approved (ADR-0022).
async fn pause_meeting(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
    topic: &str,
    speaker: &Speaker,
    check: &crate::governance::BudgetCheck,
) -> Result<(), CeoError> {
    let note = format!(
        "{} reached its monthly budget: {} of {} spent.",
        speaker.name,
        crate::governance::euros(check.spent + check.reserved),
        crate::governance::euros(check.cap),
    );
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE meetings SET status = 'paused', paused_agent_id = ?, paused_note = ?
         WHERE id = ? AND status = 'open'",
    )
    .bind(&speaker.id)
    .bind(&note)
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    crate::audit::append(
        &mut tx,
        Some(company_id),
        None,
        crate::domain::event_kind::MEETING_PAUSED,
        &json!({
            "meeting_id": meeting_id,
            "agent_id": speaker.id,
            "limit_cents": check.cap,
            "observed_cents": check.observed(),
        }),
    )
    .await?;
    let pushed = crate::notify::post(
        &mut tx,
        company_id,
        crate::notify::New {
            kind: crate::notify::kind::MEETING_PAUSED,
            title: &format!("Meeting paused: {topic}"),
            body: &format!("{note} Raise the cap or wait for the new month, then resume."),
            params: json!({
                "agent": speaker.name,
                "topic": topic,
                "spentCents": check.spent + check.reserved,
                "limitCents": check.cap,
            }),
            agent_id: Some(&speaker.id),
            subject: Some(("meeting", meeting_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    crate::notify::deliver(state, company_id, &pushed);
    state.notify(company_id);
    Ok(())
}

/// The same waiting, for a limit nobody chose and nobody can raise.
///
/// Deliberately its own function and its own notification kind rather than a
/// parameter on the budget one. The two look identical from the outside — a
/// room stopped, a person told, a `resume` waiting — and their remedies are
/// opposite: one says "raise the cap", and here there *is* no cap. Telling
/// somebody to raise a limit that is not theirs is worse than saying nothing.
async fn pause_for_plan(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
    topic: &str,
    speaker: &Speaker,
    window: &crate::economy::PlanWindow,
) -> Result<(), CeoError> {
    let resets = crate::economy::reset_time(window);
    let pretty = window.window.replace('_', "-");
    let note =
        format!("The subscription has run out for its {pretty} window; it resets at {resets}.");
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE meetings SET status = 'paused', paused_agent_id = ?, paused_note = ?
         WHERE id = ? AND status = 'open'",
    )
    .bind(&speaker.id)
    .bind(&note)
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    crate::audit::append(
        &mut tx,
        Some(company_id),
        None,
        crate::domain::event_kind::MEETING_PAUSED,
        &json!({
            "meeting_id": meeting_id,
            "agent_id": speaker.id,
            "reason": "plan_exhausted",
            "window": window.window,
            "resets_at": window.resets_at,
        }),
    )
    .await?;
    let pushed = crate::notify::post(
        &mut tx,
        company_id,
        crate::notify::New {
            kind: crate::notify::kind::MEETING_PAUSED_PLAN,
            title: &format!("Meeting paused: {topic}"),
            body: &format!("{note} Nothing is over budget — resume when the window has reset."),
            params: json!({
                "agent": speaker.name,
                "topic": topic,
                "window": window.window,
                "resetsAt": window.resets_at,
            }),
            agent_id: Some(&speaker.id),
            subject: Some(("meeting", meeting_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    crate::notify::deliver(state, company_id, &pushed);
    state.notify(company_id);
    Ok(())
}

/// Pick a paused room back up, from the ordinal it stopped at.
///
/// Re-runs the same deliberation: if the agent still has no budget it simply
/// pauses again, saying so, rather than half-running and leaving the room in a
/// third state.
pub async fn resume(state: &AppState, company_id: &str, meeting_id: &str) -> Result<(), CeoError> {
    let reopened = sqlx::query(
        "UPDATE meetings SET status = 'open', paused_agent_id = NULL, paused_note = NULL
         WHERE id = ? AND company_id = ? AND status = 'paused'",
    )
    .bind(meeting_id)
    .bind(company_id)
    .execute(&state.pool)
    .await?;
    if reopened.rows_affected() != 1 {
        return Err(CeoError::Invalid("this meeting is not paused".into()));
    }
    state.notify(company_id);
    run_meeting(state, company_id, meeting_id).await
}

/// Round-robin turns up to the cap, then a closing turn by the chair if nobody
/// has decided yet.
async fn run_meeting(state: &AppState, company_id: &str, meeting_id: &str) -> Result<(), CeoError> {
    let meeting: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT topic, reason, turn_cap FROM meetings WHERE id = ? AND company_id = ?",
    )
    .bind(meeting_id)
    .bind(company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((topic, reason, turn_cap)) = meeting else {
        return Err(CeoError::NotFound("meeting"));
    };
    let speakers = load_speakers(state, company_id, meeting_id).await?;
    if speakers.len() < 2 {
        return Err(CeoError::Invalid(
            "a meeting needs at least two participants".into(),
        ));
    }
    // The chair: the org leader in the room, else whoever was listed first.
    let chair = speakers.iter().position(|s| s.is_leader).unwrap_or(0);

    let memory_context = state
        .memory_for(company_id)
        .await
        .get_context(&state.brain_dir(company_id).to_string_lossy(), &topic)
        .await;
    // The company's language (M16): a room must not deliberate in English
    // under an Italian interface.
    let language =
        crate::i18n::prompt_line(&crate::i18n::company_language(state, company_id).await);

    // One scratch dir for the whole meeting — agents deliberate here, they
    // don't produce files (that is what tasks are for).
    let room = state.config.data_dir.join("meetings").join(meeting_id);
    tokio::fs::create_dir_all(&room)
        .await
        .map_err(|e| CeoError::Invalid(format!("cannot create meeting dir: {e}")))?;

    let agenda = Agenda {
        language: &language,
        topic: &topic,
        reason: &reason,
        speakers: &speakers,
        memory_context: memory_context.as_deref(),
    };
    // Whatever the room already said. Empty on a first run; populated when a
    // paused meeting is resumed (ADR-0022), which is what makes resuming exact
    // rather than approximate: the speaker is `ordinal % speakers.len()`, a
    // pure function of the ordinal, so continuing from the recorded count lands
    // on the agent whose turn it actually was.
    let mut transcript = recorded_turns(state, meeting_id).await?;
    let resumed_from = transcript.len() as i64;
    for ordinal in resumed_from..turn_cap {
        let speaker = &speakers[(ordinal as usize) % speakers.len()];
        let prompt = turn_prompt(&agenda, speaker, &transcript, false);
        let output = match run_adapter(
            state,
            &crate::ceo::Turn {
                company_id,
                agent_id: &speaker.id,
                kind: "meeting",
                traits: &speaker.traits,
            },
            &room,
            &prompt,
            memory_context.as_deref(),
        )
        .await
        {
            Ok(out) => out,
            // The room ran out of money. Nothing was spent, the transcript is
            // durable, and this says nothing about the topic — so the room
            // waits for you instead of being closed or forced to a conclusion
            // it did not reach (ADR-0022).
            Err(CeoError::OverBudget(check)) => {
                return pause_meeting(state, company_id, meeting_id, &topic, speaker, &check).await;
            }
            // The *subscription* ran out, which is not the room's fault and not
            // this agent's cap. Same waiting, different sentence: there is no
            // cap to raise, only a window to wait out (ADR-0030).
            Err(CeoError::PlanExhausted(window)) => {
                return pause_for_plan(state, company_id, meeting_id, &topic, speaker, &window)
                    .await;
            }
            Err(e) => return Err(e),
        };
        let turn = turn_json(&output);
        let said = said_or_raw(turn.as_ref(), &output);
        record_turn(state, company_id, meeting_id, &speaker.id, ordinal, &said).await?;
        transcript.push((speaker.name.clone(), said.clone()));

        // The room concludes there was nothing to decide. Cheaper and more
        // honest than forcing a decision out of a meeting that should not have
        // been called — and it is the self-correction that makes asking for a
        // meeting safe: a wrong room costs one turn, not a fake settled call.
        if let Some(why) = turn
            .as_ref()
            .and_then(|v| v.get("no_decision_needed").and_then(Value::as_str))
            .map(str::trim)
            .filter(|w| !w.is_empty())
        {
            return drop_meeting(state, company_id, meeting_id, &topic, &speaker.name, why).await;
        }

        // Someone settled it: the meeting ends early, on their decision.
        if let Some(decision) = turn
            .as_ref()
            .and_then(|v| v.get("decision").and_then(Value::as_str))
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            return conclude(state, company_id, meeting_id, &topic, decision).await;
        }
    }

    // The cap is reached and nobody has called it: the chair closes.
    let speaker = &speakers[chair];
    let prompt = turn_prompt(&agenda, speaker, &transcript, true);
    let output = match run_adapter(
        state,
        &crate::ceo::Turn {
            company_id,
            agent_id: &speaker.id,
            kind: "meeting",
            traits: &speaker.traits,
        },
        &room,
        &prompt,
        memory_context.as_deref(),
    )
    .await
    {
        Ok(out) => out,
        Err(CeoError::PlanExhausted(window)) => {
            return pause_for_plan(state, company_id, meeting_id, &topic, speaker, &window).await;
        }
        Err(CeoError::OverBudget(check)) => {
            return pause_meeting(state, company_id, meeting_id, &topic, speaker, &check).await;
        }
        Err(e) => return Err(e),
    };
    let turn = turn_json(&output);
    let said = said_or_raw(turn.as_ref(), &output);
    record_turn(state, company_id, meeting_id, &speaker.id, turn_cap, &said).await?;

    // The closing turn must decide. If the chair gives only prose, that prose
    // is the call — a meeting never ends undecided while the chair spoke.
    let decision = turn
        .as_ref()
        .and_then(|v| v.get("decision").and_then(Value::as_str))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or(said);
    if decision.trim().is_empty() {
        return Err(CeoError::Invalid(
            "the chair closed the meeting without a decision".into(),
        ));
    }
    conclude(state, company_id, meeting_id, &topic, &decision).await
}

/// The agent's own contribution, picked out of its output.
///
/// Not simply the last JSON line: a real adapter prints its **own** result
/// envelope (cost, usage, session id) after whatever the agent said, so the
/// last object is usually the adapter's, not the agent's. We take the last one
/// that actually looks like a turn.
fn turn_json(output: &str) -> Option<Value> {
    // Through the envelope, not around it — the real CLI nests the agent's turn
    // inside `.result` (see `ceo::agent_text`), which is why meetings could
    // never reach a decision live while every stub-driven test passed.
    crate::ceo::find_json_object(output, |v| {
        v.get("say").is_some() || v.get("decision").is_some()
    })
}

/// What the agent said this turn, degrading to its raw output if the JSON is
/// missing or garbled — a malformed turn is still a turn, not a dead meeting.
fn said_or_raw(turn: Option<&Value>, output: &str) -> String {
    let output = &crate::ceo::agent_text(output);
    turn.and_then(|v| v.get("say").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| output.trim().to_string())
}

/// What an agent is asked at its turn. `closing` is the chair's final turn,
/// where a decision is mandatory.
///
/// The prompt does most of the work of keeping a meeting *useful*. Round-robin
/// agents drift into agreeing with each other — "+1", restating the last turn —
/// and a room that only agrees reaches a decision without ever testing it. So
/// each turn is asked for the thing only that role can see, agreement must
/// carry its cost, and a decision has to be concrete enough to act on.
/// What every turn of one meeting shares. The room, the topic and what the
/// organization remembers do not change between turns — only who is speaking
/// and what has been said so far.
struct Agenda<'a> {
    language: &'a str,
    topic: &'a str,
    reason: &'a str,
    speakers: &'a [Speaker],
    memory_context: Option<&'a str>,
}

fn turn_prompt(
    agenda: &Agenda<'_>,
    speaker: &Speaker,
    transcript: &[(String, String)],
    closing: bool,
) -> String {
    let Agenda {
        language,
        topic,
        reason,
        speakers,
        memory_context,
    } = *agenda;
    let room = speakers
        .iter()
        .map(|s| {
            if s.id == speaker.id {
                format!("- {} ({}) — you", s.name, s.role)
            } else {
                format!("- {} ({})", s.name, s.role)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let so_far = if transcript.is_empty() {
        "(you speak first)".to_string()
    } else {
        transcript
            .iter()
            .map(|(who, what)| crate::ceo::transcript_turn(who, what))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let brief_line = speaker
        .brief
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| format!("\nYour brief: {b}"))
        .unwrap_or_default();
    let memory_block = memory_context
        .map(|m| format!("\n\nWhat the organization remembers:\n{m}"))
        .unwrap_or_default();

    let why_line = if reason.trim().is_empty() {
        String::new()
    } else {
        format!("\nWhy this room was called: {}", reason.trim())
    };

    let instruction = if closing {
        "You are chairing this meeting and it has reached its turn limit. Close it. In \"say\", give the reasoning in one or two sentences: what the room actually converged on, and why it beats the alternative that was raised. In \"decision\", state the call itself — concrete and actionable, something a colleague could pick up tomorrow, not \"we should look into it\". A decision is REQUIRED — do not defer, do not ask for another meeting."
    } else if transcript.is_empty() {
        "You speak first. Do not restate the topic — everyone has read it. Frame the actual choice: the two or three real options on the table and the trade-off between them, seen from what your role is responsible for. Name the specific risk, cost or constraint that makes this a decision rather than an obvious call. Two or three sentences."
    } else {
        "Speak once, from what your role is responsible for — say the thing the others cannot see from where they sit. \
         Do not agree without adding something: if you agree, name the condition, cost or risk that agreement carries. If you disagree, say why, concretely, and give the alternative you would take instead. \
         Never restate a point already made; if you genuinely have nothing to add, say so in one line and defer. \
         Set \"decision\" ONLY if the group has genuinely converged AND you can state a concrete, actionable call in one sentence. If you are the first to speak on it, you are almost certainly too early. Otherwise omit it and let the discussion continue. \
         If this room should not have been called at all — the question answers itself, or it is one person's call to make — set \"no_decision_needed\" with one line saying why, and the meeting closes without a decision. That is a good outcome, not a failure."
    };

    format!(
        "You are {name}, the {role} at an AI company, in a meeting with your colleagues.{brief_line}\n\n\
         Meeting topic: {topic}{why_line}\n\n\
         In the room:\n{room}\n\n\
         The meeting so far:\n{so_far}{memory_block}\n\n\
         {instruction}\n\n\
         Respond with a SINGLE JSON object on the LAST line of your output, and nothing after it:\n\
         {{\"say\": \"<your contribution>\", \"decision\": \"<the group's decision — omit unless settled>\", \"no_decision_needed\": \"<omit unless this room is pointless>\"}}{language}",
        name = speaker.name,
        role = speaker.role,
    )
}

/// Append a contribution to the transcript. Turns are visible as they happen.
async fn record_turn(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
    agent_id: &str,
    ordinal: i64,
    content: &str,
) -> Result<(), CeoError> {
    sqlx::query(
        "INSERT INTO meeting_turns (id, meeting_id, agent_id, ordinal, content, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(meeting_id)
    .bind(agent_id)
    .bind(ordinal)
    .bind(content)
    .bind(now())
    .execute(&state.pool)
    .await?;
    state.notify(company_id);
    Ok(())
}

/// What past meetings mean for this agent's next piece of work: the calls it
/// must act on, and the requests you turned down.
///
/// Both halves matter. A decision is only worth the meeting if it changes what
/// happens next. And a **refusal is invisible to the agent that asked** unless
/// it is put here — the decline notification goes to the human, so without this
/// the agent re-requests the same meeting on its very next turn, forever.
///
/// Injected into the prompt of the next task run (ADR-0017) and of the next
/// conversational turn (ADR-0019).
pub async fn decisions_block(state: &AppState, agent_id: &str) -> String {
    let decided: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.topic, m.decision FROM meetings m
         JOIN meeting_participants mp ON mp.meeting_id = m.id
         WHERE mp.agent_id = ? AND m.status = 'decided' AND m.decision IS NOT NULL
         ORDER BY m.decided_at DESC LIMIT 3",
    )
    .bind(agent_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    // Only the convener is told: it is the one who would otherwise ask again.
    let refused: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT topic, decline_note FROM meetings
         WHERE convener_agent_id = ? AND status IN ('declined', 'dropped')
         ORDER BY decided_at DESC LIMIT 3",
    )
    .bind(agent_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut out = String::new();
    if !decided.is_empty() {
        let list = decided
            .iter()
            .map(|(topic, decision)| format!("- On \"{topic}\": {decision}"))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!(
            "\n\nDecisions from meetings you took part in — these are settled, act on them and do not re-litigate:\n{list}"
        ));
    }
    if !refused.is_empty() {
        let list = refused
            .iter()
            .map(
                |(topic, note)| match note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    Some(n) => format!("- \"{topic}\" — {n}"),
                    None => format!("- \"{topic}\" — no reason given"),
                },
            )
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!(
            "\n\nMeetings you asked for that did NOT happen. Do not ask again for these — take the call yourself, or say plainly that you are blocked:\n{list}"
        ));
    }
    out
}

/// Send every participant back to work with the decision in hand. The wakeup
/// is a request, not a command: the scheduler still enforces autonomy and
/// budget (ADR-0005/0012), so an agent that needs a human to start its work
/// still needs one. What is guaranteed is that the decision is in front of it
/// (see [`decisions_block`]).
async fn wake_participants(
    tx: &mut sqlx::SqliteConnection,
    company_id: &str,
    meeting_id: &str,
    topic: &str,
) -> Result<(), CeoError> {
    let participants: Vec<(String,)> =
        sqlx::query_as("SELECT agent_id FROM meeting_participants WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_all(&mut *tx)
            .await?;
    for (agent_id,) in participants {
        let request_id = new_id();
        sqlx::query(
            "INSERT INTO agent_wakeup_requests (id, agent_id, source, reason, requested_at)
             VALUES (?, ?, 'meeting', ?, ?)",
        )
        .bind(&request_id)
        .bind(&agent_id)
        .bind(format!("carry the decision on \"{topic}\" into your work"))
        .bind(now())
        .execute(&mut *tx)
        .await?;
        audit::append(
            tx,
            Some(company_id),
            None,
            event_kind::WAKEUP_REQUESTED,
            &json!({
                "request_id": request_id,
                "agent_id": agent_id,
                "source": "meeting",
                "meeting_id": meeting_id,
            }),
        )
        .await?;
    }
    Ok(())
}

/// End the meeting on a decision: recorded, audited, remembered, reported back
/// to the human who allowed it — and pushed back into the work of everyone who
/// was in the room.
async fn conclude(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
    topic: &str,
    decision: &str,
) -> Result<(), CeoError> {
    let convener: Option<(Option<String>,)> =
        sqlx::query_as("SELECT convener_agent_id FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&state.pool)
            .await?;
    let convener = convener.and_then(|(c,)| c);

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE meetings SET status = 'decided', decision = ?, decided_at = ?
         WHERE id = ? AND status = 'open'",
    )
    .bind(decision)
    .bind(now())
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MEETING_DECIDED,
        &json!({ "meeting_id": meeting_id, "decision": decision }),
    )
    .await?;
    // Everyone who spoke now goes back to work with the decision in hand.
    wake_participants(&mut tx, company_id, meeting_id, topic).await?;
    let notification = notify::post(
        &mut tx,
        company_id,
        notify::New {
            kind: notify::kind::MEETING_DECIDED,
            title: &format!("Decided: {topic}"),
            body: decision,
            // The decision is the room's own wording; only the label around it
            // is ours.
            params: json!({ "topic": topic, "decision": decision }),
            agent_id: convener.as_deref(),
            subject: Some(("meeting", meeting_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    notify::deliver(state, company_id, &notification);

    // Why the company decided this outlives the meeting (ADR-0013).
    // Best-effort: no memory server configured is a normal, silent no-op.
    let meeting_tag = format!("meeting:{meeting_id}");
    let stored = state
        .memory_for(company_id)
        .await
        .store_decision(
            decision,
            &format!("Decided in a meeting on \"{topic}\"."),
            company_id,
            &[&meeting_tag],
        )
        .await;
    // Which room decided this, so the browser can send you back to the
    // transcript rather than to a sentence with no history (ADR-0025).
    state
        .link_memory(
            company_id,
            "decision",
            stored.as_deref(),
            "meeting",
            meeting_id,
            topic,
        )
        .await;
    Ok(())
}

/// The room decided there was nothing to decide. Recorded as `dropped`, not
/// `decided`: a dropped meeting must never be injected into anyone's work as a
/// settled call. The convener is told (via `decisions_block`) so it does not
/// call the same room again.
async fn drop_meeting(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
    topic: &str,
    who: &str,
    why: &str,
) -> Result<(), CeoError> {
    let convener: Option<(Option<String>,)> =
        sqlx::query_as("SELECT convener_agent_id FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&state.pool)
            .await?;
    let convener = convener.and_then(|(c,)| c);

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE meetings SET status = 'dropped', decline_note = ?, decided_at = ?
         WHERE id = ? AND status = 'open'",
    )
    .bind(format!("{who}: {why}"))
    .bind(now())
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MEETING_DROPPED,
        &json!({ "meeting_id": meeting_id, "by": who, "reason": why }),
    )
    .await?;
    let notification = notify::post(
        &mut tx,
        company_id,
        notify::New {
            kind: notify::kind::MEETING_DROPPED,
            title: &format!("No decision needed: {topic}"),
            body: &format!("{who} closed the room: {why}"),
            params: json!({ "agent": who, "topic": topic, "why": why }),
            agent_id: convener.as_deref(),
            subject: Some(("meeting", meeting_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    notify::deliver(state, company_id, &notification);
    Ok(())
}

/// A meeting that could not run (adapter down, timeout) is closed as failed
/// rather than left open forever — and you are told, because you approved it.
async fn fail_meeting(
    state: &AppState,
    company_id: &str,
    meeting_id: &str,
    reason: &str,
) -> Result<(), CeoError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT topic, convener_agent_id FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&state.pool)
            .await?;
    let (topic, convener) = row.unwrap_or_else(|| ("the meeting".to_string(), None));

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE meetings SET status = 'failed', decided_at = ? WHERE id = ? AND status = 'open'",
    )
    .bind(now())
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::MEETING_FAILED,
        &json!({ "meeting_id": meeting_id, "reason": reason }),
    )
    .await?;
    let notification = notify::post(
        &mut tx,
        company_id,
        notify::New {
            kind: notify::kind::MEETING_FAILED,
            title: &format!("Meeting could not run: {topic}"),
            body: reason,
            params: json!({ "topic": topic, "reason": reason }),
            agent_id: convener.as_deref(),
            subject: Some(("meeting", meeting_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    notify::deliver(state, company_id, &notification);
    Ok(())
}
