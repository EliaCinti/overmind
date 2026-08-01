//! M15: the CEO proposes an organization, the human decides.
//!
//! Every company is founded with a CEO (see [`crate::db::CEO_ARCHETYPE`]). You
//! can then take either road, and they cost the same to support:
//!
//! - **Tell the CEO the idea.** It answers with a proposed team — who to hire,
//!   in what role, reporting to whom, and *why*. It is a proposal: nothing is
//!   hired until you accept, and you may drop members before you do.
//! - **Build it yourself.** Hire agents and set `reports_to` by hand. The
//!   proposal machinery is never in the way.
//!
//! The shape is deliberately the same as a meeting request (ADR-0020): a
//! durable object, an approval that gates it, a notification that reaches you,
//! and — if you refuse — a note fed back into the CEO's next prompt so it does
//! not propose the same team again.

use serde_json::{Value, json};

use crate::audit;
use crate::ceo::CeoError;
use crate::db::AppState;
use crate::domain::event_kind;
use crate::notify;

/// One pending proposal per company: an org chart you have not answered yet is
/// the wrong thing to build a second one on top of.
const MAX_PENDING_PER_COMPANY: i64 = 1;

/// However many the CEO dreams up, a single proposal may not exceed this.
const MAX_MEMBERS: usize = 12;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// One proposed hire, as the CEO describes it.
#[derive(Debug, Clone)]
pub struct MemberSpec {
    pub name: String,
    pub archetype: String,
    pub title: Option<String>,
    /// The **name** of another member, or `None` to report to the CEO.
    pub reports_to: Option<String>,
    pub brief: Option<String>,
    pub rationale: Option<String>,
}

/// A whole proposed organization.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub summary: String,
    pub members: Vec<MemberSpec>,
}

impl Proposal {
    /// Parse the `team` object the CEO emits in its plan. `None` for anything
    /// without members: a team of nobody is not a proposal.
    pub fn from_json(v: &Value) -> Option<Proposal> {
        let members: Vec<MemberSpec> = v
            .get("members")
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|m| {
                let name = m.get("name").and_then(Value::as_str)?.trim().to_string();
                let archetype = m
                    .get("archetype")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_string();
                if name.is_empty() || archetype.is_empty() {
                    return None;
                }
                let text = |k: &str| {
                    m.get(k)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                };
                Some(MemberSpec {
                    name,
                    archetype,
                    title: text("title"),
                    reports_to: text("reports_to"),
                    brief: text("brief"),
                    rationale: text("why").or_else(|| text("rationale")),
                })
            })
            .take(MAX_MEMBERS)
            .collect();
        if members.is_empty() {
            return None;
        }
        Some(Proposal {
            summary: v
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            members,
        })
    }
}

/// Record a proposed organization and put it in front of the human. Nothing is
/// hired here.
pub async fn propose(
    state: &AppState,
    company_id: &str,
    proposed_by: &str,
    proposal: &Proposal,
) -> Result<String, CeoError> {
    let (pending,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM org_proposals WHERE company_id = ? AND status = 'proposed'",
    )
    .bind(company_id)
    .fetch_one(&state.pool)
    .await?;
    if pending >= MAX_PENDING_PER_COMPANY {
        return Err(CeoError::Invalid(
            "there is already a proposed organization waiting on the human".into(),
        ));
    }

    // Every archetype must exist: a proposal you cannot accept is worse than
    // no proposal, and the CEO should learn the catalog it actually has.
    for m in &proposal.members {
        let known: Option<(String,)> = sqlx::query_as("SELECT id FROM archetypes WHERE slug = ?")
            .bind(&m.archetype)
            .fetch_optional(&state.pool)
            .await?;
        if known.is_none() {
            return Err(CeoError::Invalid(format!(
                "no such archetype: `{}` (for {})",
                m.archetype, m.name
            )));
        }
    }

    let who: Option<(String,)> = sqlx::query_as("SELECT name FROM agents WHERE id = ?")
        .bind(proposed_by)
        .fetch_optional(&state.pool)
        .await?;
    let who = who.map(|(n,)| n).unwrap_or_else(|| "The CEO".into());

    let roster = proposal
        .members
        .iter()
        .map(|m| match &m.title {
            Some(t) => format!("- {} — {t}", m.name),
            None => format!("- {}", m.name),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let summary = if proposal.summary.is_empty() {
        "(no rationale given)".to_string()
    } else {
        proposal.summary.clone()
    };
    let body = format!(
        "{who} has drawn up a team of {}:\n\n{roster}\n\nWhy: {summary}\n\nNobody is hired until you accept. You can drop anyone from the list first.",
        proposal.members.len()
    );

    let proposal_id = new_id();
    let approval_id = new_id();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO approvals (id, company_id, type, status, payload, summary, created_at)
         VALUES (?, ?, 'org_proposal', 'pending', ?, ?, ?)",
    )
    .bind(&approval_id)
    .bind(company_id)
    .bind(json!({ "proposal_id": proposal_id }).to_string())
    .bind(format!(
        "{who} proposes a team of {}",
        proposal.members.len()
    ))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO org_proposals (id, company_id, proposed_by, summary, status, approval_id, created_at)
         VALUES (?, ?, ?, ?, 'proposed', ?, ?)",
    )
    .bind(&proposal_id)
    .bind(company_id)
    .bind(proposed_by)
    .bind(&summary)
    .bind(&approval_id)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    for (position, m) in proposal.members.iter().enumerate() {
        sqlx::query(
            "INSERT INTO org_proposal_members
             (id, proposal_id, position, name, archetype, title, reports_to, brief, rationale)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&proposal_id)
        .bind(position as i64)
        .bind(&m.name)
        .bind(&m.archetype)
        .bind(&m.title)
        .bind(&m.reports_to)
        .bind(&m.brief)
        .bind(&m.rationale)
        .execute(&mut *tx)
        .await?;
    }
    audit::append(
        &mut tx,
        Some(company_id),
        None,
        event_kind::ORG_PROPOSED,
        &json!({
            "proposal_id": proposal_id,
            "proposed_by": proposed_by,
            "members": proposal.members.len(),
            "approval_id": approval_id,
        }),
    )
    .await?;
    let notification = notify::post(
        &mut tx,
        company_id,
        notify::New {
            kind: notify::kind::ORG_PROPOSED,
            title: &format!("{who} proposes a team"),
            body: &body,
            agent_id: Some(proposed_by),
            subject: Some(("org_proposal", &proposal_id)),
            approval_id: Some(&approval_id),
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    notify::deliver(state, company_id, &notification);
    Ok(proposal_id)
}

/// (id, name, archetype, title, reports_to, brief)
type MemberRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Accept: hire everyone still on the list, wiring the reporting tree.
///
/// Members name their manager by *name*, so hiring runs in passes — anyone
/// whose manager is already hired goes next. Whatever cannot be resolved after
/// the last pass (a typo, a dropped member) reports to the CEO instead of
/// being lost: an org with one root beats an org with a hole in it.
pub async fn accept(state: &AppState, proposal_id: &str) -> Result<Vec<String>, CeoError> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT company_id, status, proposed_by FROM org_proposals WHERE id = ?")
            .bind(proposal_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, status, _proposed_by)) = row else {
        return Err(CeoError::NotFound("org proposal"));
    };
    if status != "proposed" {
        return Err(CeoError::Invalid(format!("proposal is already {status}")));
    }

    let members: Vec<MemberRow> = sqlx::query_as(
        "SELECT id, name, archetype, title, reports_to, brief FROM org_proposal_members
         WHERE proposal_id = ? AND excluded = 0 ORDER BY position",
    )
    .bind(proposal_id)
    .fetch_all(&state.pool)
    .await?;
    if members.is_empty() {
        return Err(CeoError::Invalid(
            "every member was dropped: nothing to hire".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let mut by_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut hired: Vec<String> = Vec::new();
    let mut pending: Vec<&MemberRow> = members.iter().collect();

    // Passes until nothing else can be placed; then the remainder go under the
    // CEO (hire() defaults a manager-less agent to the org leader).
    loop {
        let mut placed_any = false;
        let mut still: Vec<&MemberRow> = Vec::new();
        for m in pending {
            let (member_id, name, archetype, title, reports_to, brief) = m;
            let manager = match reports_to
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                None => None,
                Some(mgr_name) => match by_name.get(mgr_name) {
                    Some(id) => Some(id.clone()),
                    // Manager not hired yet — try again next pass.
                    None if still.len() < members.len() => {
                        still.push(m);
                        continue;
                    }
                    None => None,
                },
            };
            let agent = crate::api::hire(
                &mut tx,
                &company_id,
                &crate::api::HireAgent {
                    name: name.clone(),
                    archetype: archetype.clone(),
                    traits: Default::default(),
                    custom_brief: brief.clone(),
                    title: title.clone(),
                    reports_to: manager,
                },
            )
            .await
            .map_err(|e| CeoError::Invalid(format!("cannot hire {name}: {e}")))?;
            let agent_id = agent["id"].as_str().unwrap_or_default().to_string();
            sqlx::query("UPDATE org_proposal_members SET hired_agent_id = ? WHERE id = ?")
                .bind(&agent_id)
                .bind(member_id)
                .execute(&mut *tx)
                .await?;
            by_name.insert(name.clone(), agent_id.clone());
            hired.push(agent_id);
            placed_any = true;
        }
        if still.is_empty() {
            break;
        }
        if !placed_any {
            // Nothing resolved this pass: the rest report to the CEO.
            pending = still;
            for m in pending.iter() {
                let (member_id, name, archetype, title, _rt, brief) = *m;
                let agent = crate::api::hire(
                    &mut tx,
                    &company_id,
                    &crate::api::HireAgent {
                        name: name.clone(),
                        archetype: archetype.clone(),
                        traits: Default::default(),
                        custom_brief: brief.clone(),
                        title: title.clone(),
                        reports_to: None,
                    },
                )
                .await
                .map_err(|e| CeoError::Invalid(format!("cannot hire {name}: {e}")))?;
                let agent_id = agent["id"].as_str().unwrap_or_default().to_string();
                sqlx::query("UPDATE org_proposal_members SET hired_agent_id = ? WHERE id = ?")
                    .bind(&agent_id)
                    .bind(member_id)
                    .execute(&mut *tx)
                    .await?;
                hired.push(agent_id);
            }
            break;
        }
        pending = still;
    }

    sqlx::query("UPDATE org_proposals SET status = 'accepted', decided_at = ? WHERE id = ?")
        .bind(now())
        .bind(proposal_id)
        .execute(&mut *tx)
        .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::ORG_ACCEPTED,
        &json!({ "proposal_id": proposal_id, "hired": hired.len() }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    Ok(hired)
}

/// Refuse the proposed organization. The reason reaches the CEO's next prompt.
pub async fn reject(
    state: &AppState,
    proposal_id: &str,
    note: Option<&str>,
) -> Result<(), CeoError> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT company_id, status, proposed_by FROM org_proposals WHERE id = ?")
            .bind(proposal_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((company_id, status, proposed_by)) = row else {
        return Err(CeoError::NotFound("org proposal"));
    };
    if status != "proposed" {
        return Err(CeoError::Invalid(format!("proposal is already {status}")));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE org_proposals SET status = 'rejected', decline_note = ?, decided_at = ?
         WHERE id = ? AND status = 'proposed'",
    )
    .bind(note.map(str::trim).filter(|n| !n.is_empty()))
    .bind(now())
    .bind(proposal_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        None,
        event_kind::ORG_REJECTED,
        &json!({ "proposal_id": proposal_id, "note": note }),
    )
    .await?;
    let notification = notify::post(
        &mut tx,
        &company_id,
        notify::New {
            kind: notify::kind::ORG_REJECTED,
            title: "Team proposal declined",
            body: &match note {
                Some(n) if !n.trim().is_empty() => {
                    format!("You declined the proposed team: {}", n.trim())
                }
                _ => "You declined the proposed team.".to_string(),
            },
            agent_id: Some(&proposed_by),
            subject: Some(("org_proposal", proposal_id)),
            approval_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);
    notify::deliver(state, &company_id, &notification);
    Ok(())
}

/// What the CEO must know about its past proposals, injected into its prompt:
/// what you refused and why, so it does not draw up the same chart again.
pub async fn feedback_block(state: &AppState, agent_id: &str) -> String {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT summary, decline_note FROM org_proposals
         WHERE proposed_by = ? AND status = 'rejected'
         ORDER BY decided_at DESC LIMIT 2",
    )
    .bind(agent_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    if rows.is_empty() {
        return String::new();
    }
    let list = rows
        .iter()
        .map(
            |(summary, note)| match note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => format!("- you proposed \"{summary}\" — refused: {n}"),
                None => format!("- you proposed \"{summary}\" — refused, no reason given"),
            },
        )
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\nTeams you proposed that were turned down. Do not propose the same shape again — ask what is missing, or work with the people you have:\n{list}"
    )
}
