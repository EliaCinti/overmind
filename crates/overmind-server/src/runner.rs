//! Agent execution: atomic task checkout, isolated git worktrees, process
//! supervision with timeouts, session resume, output capture and cost
//! recording.
//!
//! Design follows Paperclip's session model (`agent_task_sessions`,
//! `cost_events`) and Vibe Kanban's worktree-per-run isolation
//! (ADR-0008, ADR-0009).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::audit;
use crate::db::AppState;
use crate::domain::{ExecutionKind, event_kind};
use crate::files;
use crate::governance;

/// How a working agent asks for a meeting: a control file in its working
/// directory (ADR-0020). A file, not stdout, because the last JSON line of a
/// run belongs to the adapter's own result envelope.
const MEETING_REQUEST_FILE: &str = "MEETING_REQUEST.json";

/// Where files you attached to a task are placed, inside the run directory
/// (M17). A named directory rather than the root for two reasons: in a `code`
/// run the root is a git worktree and loose files would land in the diff, and
/// in either kind the output collector must be able to tell what the agent
/// *produced* from what it was *given*.
const INPUTS_DIR: &str = "inputs";

/// Where a `code` run puts anything that is not a code change — a report, a
/// chart, a generated file. Git-excluded, collected as artifacts. A knowledge
/// run needs no such convention: everything it writes is the deliverable.
const DELIVERABLES_DIR: &str = "deliverables";

/// A cap on how many files one run can hand back. Not a policy about what is
/// reasonable — a guard so a runaway loop writing files cannot fill the
/// database.
const MAX_ARTIFACTS: usize = 200;

/// Text kept inline in the row for preview and search; beyond this only the
/// file on disk. 256 KB is far past any document a human reads in a drawer.
const MAX_INLINE_BYTES: u64 = 256 * 1024;

/// Files attached to this task, as `(filename, mime, size, absolute path)`.
async fn task_inputs(state: &AppState, task_id: &str) -> Vec<(String, String, i64, String)> {
    // The task's own files, plus the files of the thread it was born in
    // (ADR-0038): the CEO's description says "read the sketch", so the
    // sketch has to be there. Only posted files (message_id set) ride —
    // a staged upload nobody sent is nobody's input.
    sqlx::query_as(
        "SELECT filename, mime, size_bytes, path FROM attachments
         WHERE task_id = ?1
            OR (message_id IS NOT NULL AND conversation_id IS NOT NULL
                AND conversation_id = (SELECT conversation_id FROM tasks WHERE id = ?1))
         ORDER BY created_at",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default()
}

/// Copy the task's attachments into the run directory before the agent starts.
///
/// Best-effort: a file that cannot be copied costs the agent that input, and
/// the prompt still names it — better than failing a run over one unreadable
/// upload. The agent is told what is there and where.
async fn place_inputs(ctx: &SessionContext) {
    let inputs = task_inputs(&ctx.state, &ctx.task_id).await;
    if inputs.is_empty() {
        return;
    }
    let dir = ctx.worktree_dir.join(INPUTS_DIR);
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return;
    }
    for (name, _, _, path) in &inputs {
        let _ = tokio::fs::copy(path, dir.join(files::safe_name(name))).await;
    }
}

/// Who the agent is, compiled for the prompt (ADR-0005 / M14).
///
/// The characterization is structured data — archetype, title, focus areas,
/// brief — and ADR-0005 says it must compile into the agent's *prompt context*.
/// Until M14 it only reached conversational turns: a task run got a prompt with
/// no persona at all, so a "Media & A/V quality" agent and a backend developer
/// were handed identical instructions for the same task.
pub(crate) struct Persona {
    pub name: String,
    /// Job title if set, else the archetype's human name.
    pub role: String,
    pub focus_areas: Vec<String>,
    /// One line about the field the agent works in, from its domain
    /// (ADR-0021). Empty for the general domain, which adds nothing.
    pub domain_context: String,
    pub brief: Option<String>,
    /// The line naming the tools this agent holds (ADR-0036), already
    /// worded; empty when none were granted.
    pub tools_line: String,
}

impl Persona {
    /// The "who you are" block that opens a task prompt. Empty only if the
    /// agent could not be loaded — never silently role-blind. `company` is
    /// [`crate::db::AppState::company_descriptor`]: an agent that knows whose
    /// writer it is does not reach for world knowledge about the name (M21).
    fn block(&self, company: &str) -> String {
        let mut s = format!("You are {}, the {} at {company}.", self.name, self.role);
        if !self.focus_areas.is_empty() {
            s.push_str(&format!(
                " What you are relied on for: {}.",
                self.focus_areas.join(", ")
            ));
        }
        if !self.domain_context.trim().is_empty() {
            s.push_str(&format!("\n{}", self.domain_context.trim()));
        }
        if let Some(brief) = self
            .brief
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
        {
            s.push_str(&format!("\nYour brief: {brief}"));
        }
        s.push_str(&self.tools_line);
        s.push_str("\n\nWork in role: bring the judgement your role is hired for, and say so when the task strays outside it.");
        s
    }
}

/// (name, title, archetype name, traits JSON, custom_brief, domain patch JSON)
type PersonaRow = (
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
);

/// Load an agent's characterization for the prompt. Best-effort by design:
/// a missing archetype row must not stop work, it just costs the persona.
async fn load_persona(state: &AppState, agent_id: &str) -> Option<Persona> {
    let row: Option<PersonaRow> = sqlx::query_as(
        "SELECT a.name, a.title, ar.name, a.traits, a.custom_brief, d.traits_patch
         FROM agents a
         JOIN archetypes ar ON ar.id = a.archetype_id
         LEFT JOIN domains d ON d.id = a.domain_id
         WHERE a.id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let (name, title, archetype_name, traits, brief, domain_patch) = row?;
    // A LEFT JOIN: an agent hired before ADR-0021 has no domain, and works
    // exactly as it did — the line is simply absent from its prompt.
    let domain_context = domain_patch
        .and_then(|json| serde_json::from_str::<crate::domain::DomainPatch>(&json).ok())
        .map(|p| p.context)
        .unwrap_or_default();
    let focus_areas = serde_json::from_str::<Value>(&traits)
        .ok()
        .and_then(|v| {
            v.get("focus_areas").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default();
    let tools_line = tools_line(state, &trait_tools(&traits));
    Some(Persona {
        name,
        role: title.unwrap_or(archetype_name),
        focus_areas,
        domain_context,
        brief,
        tools_line,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("task is not available for checkout")]
    Conflict,
    #[error("{0}")]
    Blocked(String),
    /// Refused, but Overmind itself knows the repair (ADR-0038 addendum):
    /// the message carries a machine-readable `remedy` the interface can
    /// offer as a button — the user approves, Overmind acts. Never a
    /// string-match on the English sentence.
    #[error("{message}")]
    Remediable {
        message: String,
        remedy: serde_json::Value,
    },
    /// Carries the two numbers a person needs to act (ADR-0046): until now it
    /// was a bare variant whose message named neither the cap nor the spend,
    /// so a refused start could not say what to raise or by how much.
    #[error("agent is over its monthly budget: {observed_cents} of {limit_cents} cents")]
    OverBudget {
        limit_cents: i64,
        observed_cents: i64,
    },
    #[error("git error: {0}")]
    Git(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// What a start attempt did: actually launched a session, or (when the agent
/// is governance-gated) filed an approval and launched nothing.
pub enum StartResult {
    Started(StartOutcome),
    ApprovalRequired { approval_id: String },
}

pub struct StartOutcome {
    pub session_id: String,
    pub branch: String,
    pub workspace_path: String,
}

/// Read one stream-json line for what the agent is doing right now
/// (ADR-0039). `assistant` events carry either a tool call or words; both are
/// a narration worth a line. Returned structured, never as an English
/// sentence — the interface words it in the person's language.
/// The text delta inside a partial-message event, if this line is one.
/// `--include-partial-messages` makes the CLI emit `stream_event` lines that
/// wrap the API's own SSE events; the reply grows one delta at a time.
pub fn text_delta_in(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("stream_event") {
        return None;
    }
    let event = v.get("event")?;
    if event.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    delta
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// What of the accumulated text is worth showing a person while it streams
/// (ADR-0039 addendum). Chat agents answer with a JSON plan whose FIRST key
/// is `reply` — so the readable part is the reply string as it grows, JSON
/// syntax and escapes stripped. Prose before any JSON or fence shows as is.
pub fn draft_reply(accum: &str) -> Option<String> {
    if let Some(i) = accum.find("\"reply\"") {
        let rest = accum[i + 7..].trim_start().strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            out.push(ch);
                        }
                    }
                    Some(other) => out.push(other),
                    None => break,
                },
                '"' => break,
                _ => out.push(c),
            }
        }
        return (!out.trim().is_empty()).then_some(out);
    }
    let cut = accum
        .find("```")
        .or_else(|| accum.find('{'))
        .unwrap_or(accum.len());
    let head = accum[..cut].trim();
    (!head.is_empty()).then(|| head.to_string())
}

pub(crate) fn activity_in(line: &str) -> Option<serde_json::Value> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let content = v.get("message")?.get("content")?.as_array()?;
    // The last block wins: a message that says a word and then calls a tool
    // is doing the tool.
    for block in content.iter().rev() {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                // MCP names arrive as `mcp__server__tool_name`.
                let (server, tool) = match name.strip_prefix("mcp__") {
                    Some(rest) => match rest.split_once("__") {
                        Some((srv, t)) => (Some(srv.to_string()), t.replace('_', " ")),
                        None => (None, rest.replace('_', " ")),
                    },
                    None => (None, name.replace('_', " ")),
                };
                let mut out = serde_json::json!({ "kind": "tool", "tool": tool });
                if let Some(srv) = server {
                    out["server"] = serde_json::json!(srv);
                }
                return Some(out);
            }
            Some("text") => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                let preview: String = text.trim().chars().take(120).collect();
                if !preview.is_empty() {
                    return Some(serde_json::json!({ "kind": "text", "preview": preview }));
                }
            }
            _ => {}
        }
    }
    None
}

/// What draining a narrated child produced: the same shape
/// `wait_with_output` gave, so both call sites keep their behaviour.
pub(crate) struct Drained {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Wait for the child while reading its stdout line by line, narrating each
/// line's activity under `key` (ADR-0039). On timeout the child is killed and
/// `None` is returned. The narration is cleared by the caller — the run's end
/// clears it, however it ends.
pub(crate) async fn drain_narrating(
    state: &AppState,
    key: &str,
    mut child: tokio::process::Child,
    timeout_secs: u64,
) -> std::io::Result<Option<Drained>> {
    use tokio::io::AsyncBufReadExt;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let state2 = state.clone();
    let key2 = key.to_string();
    let out_task = tokio::spawn(async move {
        let mut buf = String::new();
        // The reply as it streams (ADR-0039 addendum): text deltas accumulate
        // and the readable part rides as a `draft` activity, so the chat can
        // show the words appearing. Bounded — narration, not storage.
        let mut accum = String::new();
        if let Some(stdout) = stdout {
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(delta) = text_delta_in(&line) {
                    if accum.len() < 64_000 {
                        accum.push_str(&delta);
                    }
                    if let Some(draft) = draft_reply(&accum) {
                        let shown: String = draft.chars().take(12_000).collect();
                        state2.set_activity(
                            &key2,
                            serde_json::json!({ "kind": "draft", "text": shown }),
                        );
                    }
                } else if let Some(activity) = activity_in(&line) {
                    // A full-message text event repeats what the deltas already
                    // narrated: never let its 120-char preview shadow a live
                    // draft. Tool calls always win — they are new information.
                    let is_text = activity["kind"] == "text";
                    let is_tool = activity["kind"] == "tool";
                    if !(is_text && !accum.is_empty()) {
                        state2.set_activity(&key2, activity);
                    }
                    if is_tool {
                        accum.clear();
                    }
                }
                buf.push_str(&line);
                buf.push('\n');
            }
        }
        buf
    });
    let err_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(stderr) = stderr {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                buf.push_str(&line);
                buf.push('\n');
            }
        }
        buf
    });
    let waited =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await;
    match waited {
        Err(_elapsed) => {
            let _ = child.kill().await;
            out_task.abort();
            err_task.abort();
            Ok(None)
        }
        Ok(Err(e)) => {
            out_task.abort();
            err_task.abort();
            Err(e)
        }
        Ok(Ok(status)) => {
            let stdout = out_task.await.unwrap_or_default();
            let stderr = err_task.await.unwrap_or_default();
            Ok(Some(Drained {
                status,
                stdout,
                stderr,
            }))
        }
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Last 12 hex chars of a UUID — the random tail. UUIDv7's *leading* bytes
/// are a millisecond timestamp, so ids minted in the same millisecond share
/// their prefix; the tail is what actually distinguishes them. Used only for
/// human-readable branch names; uniqueness rides on the full session id.
fn tag(id: &str) -> &str {
    let trimmed = id.trim_end_matches('-');
    trimmed
        .get(trimmed.len().saturating_sub(12)..)
        .unwrap_or(id)
}

async fn git(cwd: &Path, args: &[&str]) -> Result<String, RunnerError> {
    // The path is declared safe for this one invocation (M23). Since the uid
    // split of ADR-0029 the agent commits as uid 10001 while the server asks
    // git questions as root, and git refuses a repository owned by another
    // user -- "dubious ownership" -- which broke every code task's diff in
    // the image. The declaration is exact, not `*`: the server only ever
    // points git at a worktree it created or a workspace the operator
    // mounted, and this narrows the exception to precisely that path.
    let safe = format!("safe.directory={}", cwd.display());
    let out = Command::new("git")
        .arg("-c")
        .arg(&safe)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| RunnerError::Git(format!("failed to run git: {e}")))?;
    if !out.status.success() {
        return Err(RunnerError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Check a task out for an agent and launch the session.
///
/// The checkout is a single conditional UPDATE (`status = 'todo'` →
/// `in_progress`): of N concurrent attempts exactly one affects a row and
/// wins; the others get `Conflict`. Budget enforcement, checkout, session
/// creation and audit events commit in one transaction; the process itself
/// runs afterwards in a background task that finalizes the session.
///
/// Governance (ADR-0012): a `requires_approval` agent files an approval and
/// launches nothing unless `bypass_approval` (i.e. the approval was granted);
/// a start that would push the agent past its monthly budget is stopped here,
/// atomically, and recorded as a budget incident.
/// The goal a company's `code` tasks attach to, or `None` when that is not
/// Overmind's to decide.
///
/// The frontend has always resolved this (`web/src/lib/repo.ts` creates it,
/// `CreateTaskDialog` passes it) and the server never did, which is how tasks
/// opened by an agent came out orphaned — see the addendum to
/// [ADR-0008](../../../docs/adr/0008-execution-sessions-and-atomic-checkout.md).
///
/// One rule, applied here and again at start: **never guess which repository,
/// filing within one is fine.** So a second repo-backed project makes this
/// `None` — which codebase an agent works in is a decision with consequences,
/// and the task is better visibly unattached than quietly attached to the wrong
/// thing. Choosing among several goals of the *same* project only decides where
/// the task is filed, so the oldest wins and a human can move it.
pub(crate) async fn default_goal(state: &AppState, company_id: &str) -> Option<String> {
    let projects: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT w.project_id FROM project_workspaces w
         JOIN projects p ON p.id = w.project_id
         WHERE p.company_id = ? AND w.is_primary = 1",
    )
    .bind(company_id)
    .fetch_all(&state.pool)
    .await
    .ok()?;
    let [project_id] = projects.as_slice() else {
        return None;
    };
    sqlx::query_scalar("SELECT id FROM goals WHERE project_id = ? ORDER BY created_at, id LIMIT 1")
        .bind(project_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
}

/// File the approval that starts a task, and tell the human (ADR-0020): an
/// approval nobody sees is an agent stuck waiting forever. Used by the
/// governance gate and by [`offer_start`] for agents that act with approval.
pub(crate) async fn file_start_approval(
    state: &AppState,
    company_id: &str,
    task_id: &str,
    agent_id: &str,
    title: &str,
) -> Result<String, RunnerError> {
    let approval_id = uuid::Uuid::now_v7().to_string();
    // The summary in the company's language, read before the write
    // transaction opens (M23, carried).
    let lang = crate::i18n::company_language(state, company_id).await;
    let mut tx = state.write_tx().await?;
    sqlx::query(
        "INSERT INTO approvals (id, company_id, type, status, payload, summary, created_at)
         VALUES (?, ?, 'task_start', 'pending', ?, ?, ?)",
    )
    .bind(&approval_id)
    .bind(company_id)
    .bind(json!({ "task_id": task_id, "agent_id": agent_id }).to_string())
    .bind(crate::i18n::start_task_summary(&lang, title))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(company_id),
        Some(task_id),
        event_kind::APPROVAL_REQUESTED,
        &json!({ "approval_id": approval_id, "agent_id": agent_id, "type": "task_start" }),
    )
    .await?;
    // Reach the human the same way a meeting request does (ADR-0020): an
    // approval nobody sees is an agent stuck waiting forever.
    let who: Option<(String,)> = sqlx::query_as("SELECT name FROM agents WHERE id = ?")
        .bind(agent_id)
        .fetch_optional(&mut *tx)
        .await?;
    let who = who.map(|(n,)| n).unwrap_or_else(|| "An agent".into());
    let notification = crate::notify::post(
        &mut tx,
        company_id,
        crate::notify::New {
            kind: crate::notify::kind::APPROVAL_REQUESTED,
            title: &format!("{who} wants to start a task"),
            body: &format!(
                "Task: {title}\n\nThis agent is gated: it starts only once you approve."
            ),
            params: serde_json::json!({ "agent": who, "task": title }),
            agent_id: Some(agent_id),
            subject: Some(("task", task_id)),
            approval_id: Some(&approval_id),
        },
    )
    .await?;
    tx.commit().await?;
    state.notify(company_id);
    crate::notify::deliver(state, company_id, &notification);
    Ok(approval_id)
}

/// A dependency delivered: hand its artifacts to every task waiting on it,
/// and offer each one a start (M30, ADR-0042). The handoff copies the
/// artifact files into the dependent's attachments — the same table the
/// run's `place_inputs` already reads — so the dependent opens with the
/// dependency's deliverables in its working directory.
pub(crate) async fn release_dependents(
    state: &AppState,
    task_id: &str,
    session_id: &str,
) -> Result<(), RunnerError> {
    let dependents: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, assignee_agent_id FROM tasks
         WHERE depends_on = ? AND status IN ('backlog', 'todo')",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await?;
    if dependents.is_empty() {
        return Ok(());
    }
    let artifacts: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT title, mime, file_path, size_bytes FROM task_artifacts
         WHERE session_id = ? AND file_path IS NOT NULL",
    )
    .bind(session_id)
    .fetch_all(&state.pool)
    .await?;
    for (dep_id, assignee) in dependents {
        for (title, mime, file_path, size) in &artifacts {
            let Some(path) = file_path else { continue };
            let mut tx = state.write_tx().await?;
            sqlx::query(
                "INSERT INTO attachments
                 (id, conversation_id, task_id, message_id, origin, filename, mime, size_bytes, path, created_at)
                 VALUES (?, NULL, ?, NULL, 'agent', ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&dep_id)
            .bind(title)
            .bind(mime)
            .bind(size)
            .bind(path)
            .bind(now())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        // The dependency is met: the row no longer gates anything, and a
        // second completion of the same task must not re-trigger.
        sqlx::query("UPDATE tasks SET depends_on = NULL WHERE id = ?")
            .bind(&dep_id)
            .execute(&state.pool)
            .await?;
        if let Some(assignee) = assignee
            && let Err(e) = boxed_offer_start(state, &dep_id, &assignee).await
        {
            eprintln!("dependent {dep_id} could not be offered: {e}");
        }
    }
    Ok(())
}

/// [`offer_start`], type-erased: the release path runs inside a session that
/// `start_task` spawned, and an opaque future that (transitively) contains
/// itself is a cycle — a boxed dyn future is not.
fn boxed_offer_start<'a>(
    state: &'a AppState,
    task_id: &'a str,
    agent_id: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Offer, RunnerError>> + Send + 'a>> {
    Box::pin(offer_start(state, task_id, agent_id))
}

/// Start (or relaunch) an existing open task on the CEO's word (M30):
/// `blocked` returns to the queue first — a relaunch is exactly that — and
/// the start goes through [`offer_start`]'s autonomy gates like any other.
pub(crate) async fn start_existing(
    state: &AppState,
    company_id: &str,
    title: &str,
) -> Result<Started, RunnerError> {
    let task: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, assignee_agent_id FROM tasks
         WHERE company_id = ? AND title = ? AND status NOT IN ('done', 'cancelled')
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(company_id)
    .bind(title)
    .fetch_optional(&state.pool)
    .await?;
    let Some((task_id, status, assignee)) = task else {
        return Ok(Started::NoSuchTask);
    };
    // Status before assignee, the same order the digest path uses: an
    // in-progress task whose assignee row is empty is *running*, and calling
    // it "nobody is on it" is the false report this milestone removes.
    if matches!(status.as_str(), "in_progress" | "in_review") {
        return Ok(Started::AlreadyRunning);
    }
    let Some(assignee) = assignee else {
        return Ok(Started::NobodyOnIt);
    };
    if status != "todo" {
        sqlx::query("UPDATE tasks SET status = 'todo', updated_at = ? WHERE id = ?")
            .bind(now())
            .bind(&task_id)
            .execute(&state.pool)
            .await?;
    }
    Ok(Started::Offered(
        offer_start(state, &task_id, &assignee).await?,
    ))
}

/// What asking for a start by title actually did (ADR-0046).
///
/// It used to be `Option<Offer>`, and `None` meant two different facts with
/// two different remedies — no open task carries that title, or one does and
/// nobody is on it. The caller cannot act on a shrug, and the CEO reported
/// both as "running".
#[derive(Debug)]
pub enum Started {
    /// No open task on the board carries that title.
    NoSuchTask,
    /// It is already under way — running, or waiting for review.
    AlreadyRunning,
    /// The task is there and has nobody assigned, so there is nobody to start.
    NobodyOnIt,
    /// It reached the autonomy gates, and this is what they said.
    Offered(Offer),
}

/// What offering a start did (ADR-0038).
#[derive(Debug)]
pub enum Offer {
    /// The agent acts within budget: the task is running.
    Started,
    /// The agent acts with approval: the start sits in the inbox.
    Asked { approval_id: String },
    /// The agent only proposes: a human starts it, nobody is asked.
    Waiting,
}

/// Send a freshly opened task to work the way its agent's autonomy says
/// (ADR-0038). Until this existed a task the CEO planned for an agent that
/// "acts with approval" asked nobody for approval, and sat in `todo` until a
/// human found the button — measured on the owner's first real brief.
pub async fn offer_start(
    state: &AppState,
    task_id: &str,
    agent_id: &str,
) -> Result<Offer, RunnerError> {
    let agent: Option<(String, String, String)> = sqlx::query_as(
        "SELECT a.company_id, a.traits, t.title FROM agents a
         JOIN tasks t ON t.id = ? AND t.company_id = a.company_id
         WHERE a.id = ?",
    )
    .bind(task_id)
    .bind(agent_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((company_id, traits, title)) = agent else {
        return Err(RunnerError::NotFound("agent"));
    };
    let autonomy = serde_json::from_str::<Value>(&traits)
        .ok()
        .and_then(|v| {
            v.get("autonomy")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    match autonomy.as_str() {
        "act_within_budget" => match start_task(state, task_id, agent_id, false).await? {
            StartResult::Started(_) => Ok(Offer::Started),
            StartResult::ApprovalRequired { approval_id } => Ok(Offer::Asked { approval_id }),
        },
        "act_with_approval" => {
            let approval_id =
                file_start_approval(state, &company_id, task_id, agent_id, &title).await?;
            Ok(Offer::Asked { approval_id })
        }
        _ => Ok(Offer::Waiting),
    }
}

pub async fn start_task(
    state: &AppState,
    task_id: &str,
    agent_id: &str,
    bypass_approval: bool,
) -> Result<StartResult, RunnerError> {
    // Resolve task -> goal -> project -> primary workspace.
    let task: Option<(String, Option<String>, String, String, String, String)> = sqlx::query_as(
        "SELECT company_id, goal_id, title, description, status, execution_kind FROM tasks WHERE id = ?",
    )
    .bind(task_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((company_id, goal_id, title, description, status, exec_kind_str)) = task else {
        return Err(RunnerError::NotFound("task"));
    };
    if status != "todo" {
        return Err(RunnerError::Conflict);
    }
    let exec_kind = ExecutionKind::parse(&exec_kind_str).unwrap_or_default();

    // Who you are asking comes before what the job needs: these checks are
    // cheap, and "this agent cannot do code work" is a more useful answer than
    // "the project has no workspace" when both are true.
    let agent: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT traits, status, requires_approval FROM agents WHERE id = ? AND company_id = ?",
    )
    .bind(agent_id)
    .bind(&company_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((agent_traits, agent_status, requires_approval)) = agent else {
        return Err(RunnerError::NotFound("agent"));
    };
    if agent_status != "active" {
        return Err(RunnerError::Blocked(format!("agent is {agent_status}")));
    }

    // Capability gate (M14 / ADR-0005). The only enforcement that is honest
    // here: we cannot police what the spawned CLI does, but we can refuse to
    // hand it work it was never characterized for. A researcher does not get
    // put on a code task — not by you, not by a teammate that assigned it one.
    let required = crate::domain::perm::for_execution_kind(exec_kind);
    if !trait_permissions(&agent_traits)
        .iter()
        .any(|p| p == required)
    {
        return Err(RunnerError::Blocked(format!(
            "this agent is not characterized for {exec_kind_str} work (missing `{required}`)"
        )));
    }

    // Multimodal gate (ADR-0021). Same shape as the capability gate above, and
    // the same honesty about what it is: not a claim that the spawned CLI
    // cannot open a PNG, but a refusal to hand an agent material it was never
    // characterized to judge. A researcher who has never been asked to look at
    // anything should not silently become the one grading your projector.
    if !trait_multimodal(&agent_traits) {
        let visual: Vec<String> = task_inputs(state, task_id)
            .await
            .into_iter()
            .filter(|(_, mime, _, _)| crate::files::is_visual(mime))
            .map(|(name, _, _, _)| name)
            .collect();
        if !visual.is_empty() {
            return Err(RunnerError::Remediable {
                message: format!(
                    "this task carries material to look at ({}) and this agent is not characterized for visual work",
                    visual.join(", ")
                ),
                // The repair Overmind can apply itself: one traits patch.
                remedy: serde_json::json!({
                    "kind": "grant_multimodal",
                    "agent_id": agent_id,
                }),
            });
        }
    }

    // A `code` run needs a git repo to branch a worktree from; a `knowledge`
    // run (ADR-0017) works in a scratch dir and needs neither goal nor workspace.
    let workspace: Option<(String, Option<String>)> = match exec_kind {
        ExecutionKind::Code => {
            let ws: Option<(String, Option<String>)> = match &goal_id {
                Some(goal_id) => {
                    sqlx::query_as(
                        "SELECT w.cwd, w.default_ref FROM project_workspaces w
                         JOIN goals g ON g.project_id = w.project_id
                         WHERE g.id = ? AND w.is_primary = 1",
                    )
                    .bind(goal_id)
                    .fetch_optional(&state.pool)
                    .await?
                }
                // An orphan — opened before a repo was connected, or by a build
                // that did not attach it. When the company has exactly one
                // repository there is nothing to decide, so decide it; when it
                // has several, the choice is genuinely the user's and guessing
                // would run an agent against the wrong codebase.
                None => {
                    let repos: Vec<(String, Option<String>)> = sqlx::query_as(
                        "SELECT w.cwd, w.default_ref FROM project_workspaces w
                         JOIN projects p ON p.id = w.project_id
                         WHERE p.company_id = ? AND w.is_primary = 1
                         ORDER BY w.cwd",
                    )
                    .bind(&company_id)
                    .fetch_all(&state.pool)
                    .await?;
                    if repos.len() > 1 {
                        return Err(RunnerError::Invalid(
                            "this task is not attached to a project, and the company has more \
                             than one repository: attach it to a goal first"
                                .into(),
                        ));
                    }
                    repos.into_iter().next()
                }
            };
            let Some(ws) = ws else {
                return Err(RunnerError::Invalid(
                    "no repository to work in: connect one to this company first".into(),
                ));
            };
            Some(ws)
        }
        ExecutionKind::Knowledge => None,
    };

    // Governance gate: file an approval and launch nothing.
    if requires_approval != 0 && !bypass_approval {
        let approval_id =
            file_start_approval(state, &company_id, task_id, agent_id, &title).await?;
        return Ok(StartResult::ApprovalRequired { approval_id });
    }

    let budget = trait_budget_cents(&agent_traits);
    // Priced by this agent's own ledger (M26, ADR-0035), read before the
    // write transaction opens: a read that can wait on nothing.
    let estimate = {
        let mut conn = state.pool.acquire().await?;
        crate::governance::estimate_cents(
            &mut conn,
            agent_id,
            crate::governance::SpendKind::Task,
            state.config.start_estimate_cents,
        )
        .await?
        .cents
    };

    // Where the brain stands before this run touches anything (ADR-0026).
    // Taken OUTSIDE the transaction on purpose: it is an MCP round-trip to
    // another process, and holding a write lock across one would let a slow or
    // hung memory provider stall every checkout on the server. `None` is
    // ordinary — memory off, tool absent, call failed — and simply means this
    // run gets no collision window.
    let brain_watermark = state
        .memory_for(&company_id)
        .await
        .watermark(&company_id)
        .await;

    let session_id = uuid::Uuid::now_v7().to_string();
    // `code`: a git branch + worktree dir. `knowledge`: no branch, a scratch dir.
    // Branch uniqueness rides on the full session id (globally unique); the task
    // tag only makes it human-recognizable.
    let (branch, work_dir): (String, PathBuf) = match exec_kind {
        ExecutionKind::Code => (
            format!("overmind/task-{}-sess-{}", tag(task_id), session_id),
            state.config.data_dir.join("worktrees").join(&session_id),
        ),
        ExecutionKind::Knowledge => (
            String::new(),
            state.config.data_dir.join("sessions").join(&session_id),
        ),
    };
    let work_dir_str = work_dir.to_string_lossy().into_owned();

    let mut tx = state.write_tx().await?;

    // Budget check, atomic with checkout. spent (this month) + reserved
    // (in-flight) + this run's estimate must fit under the cap.
    // One arithmetic, shared with conversational turns (ADR-0022) — a second
    // implementation of "does this fit" is a second thing to drift.
    let check = governance::check(&mut tx, agent_id, budget, estimate).await?;
    if !check.fits {
        // Record the incident and commit that alone; the task is untouched.
        governance::record_overrun(&mut tx, &company_id, agent_id, Some(task_id), &check).await?;
        tx.commit().await?;
        state.notify(&company_id);
        return Err(RunnerError::OverBudget {
            limit_cents: check.cap,
            observed_cents: check.observed(),
        });
    }

    // Atomic checkout: exactly one concurrent caller wins this UPDATE.
    let checked_out =
        sqlx::query("UPDATE tasks SET status = 'in_progress', assignee_agent_id = ?, updated_at = ? WHERE id = ? AND status = 'todo'")
            .bind(agent_id)
            .bind(now())
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
    if checked_out.rows_affected() != 1 {
        return Err(RunnerError::Conflict);
    }
    sqlx::query(
        "INSERT INTO agent_task_sessions (id, task_id, agent_id, adapter_type, status, branch, workspace_path, reserved_cents, brain_watermark, created_at)
         VALUES (?, ?, ?, 'claude_code', 'queued', ?, ?, ?, ?, ?)",
    )
    .bind(&session_id)
    .bind(task_id)
    .bind(agent_id)
    .bind(&branch)
    .bind(&work_dir_str)
    .bind(estimate)
    .bind(brain_watermark.as_deref())
    .bind(now())
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(task_id),
        event_kind::TASK_TRANSITIONED,
        &json!({ "from": "todo", "to": "in_progress", "assignee_agent_id": agent_id }),
    )
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(task_id),
        event_kind::SESSION_STARTED,
        &json!({ "session_id": session_id, "agent_id": agent_id, "branch": branch }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);

    let ctx = SessionContext {
        state: state.clone(),
        session_id: session_id.clone(),
        task_id: task_id.to_string(),
        company_id,
        agent_id: agent_id.to_string(),
        persona: load_persona(state, agent_id).await,
        worktree_dir: work_dir,
        title,
        description,
        agent_traits,
        exec_kind,
    };
    let fresh = match exec_kind {
        ExecutionKind::Code => {
            let (repo_cwd, default_ref) =
                workspace.expect("a code run resolved a primary workspace above");
            FreshSpec::Code(WorktreeSpec {
                repo_cwd: PathBuf::from(repo_cwd),
                default_ref,
                branch: branch.clone(),
            })
        }
        ExecutionKind::Knowledge => FreshSpec::Knowledge,
    };
    register(state, &session_id);
    tokio::spawn(async move {
        run_session(ctx, Mode::Fresh(fresh)).await;
    });

    Ok(StartResult::Started(StartOutcome {
        session_id,
        branch,
        workspace_path: work_dir_str,
    }))
}

/// The monthly budget cap from an agent's serialized traits (0 = uncapped).
pub(crate) fn trait_budget_cents(traits_json: &str) -> i64 {
    serde_json::from_str::<Value>(traits_json)
        .ok()
        .and_then(|v| v.get("monthly_budget_cents").and_then(Value::as_i64))
        .unwrap_or(0)
}

/// What the agent is allowed to do. An unreadable traits blob yields none —
/// fail closed: an agent we cannot characterize gets no work.
fn trait_permissions(traits_json: &str) -> Vec<String> {
    serde_json::from_str::<Value>(traits_json)
        .ok()
        .and_then(|v| {
            v.get("permissions").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Whether the agent is characterized to work with visual material (ADR-0021).
/// An unreadable traits blob yields `false`, on the same fail-closed grounds as
/// [`trait_permissions`].
pub(crate) fn trait_multimodal(traits_json: &str) -> bool {
    serde_json::from_str::<Value>(traits_json)
        .ok()
        .and_then(|v| v.get("multimodal").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Which model runs this agent. Falls back to the catalog default rather than
/// to nothing: a traits blob we cannot read is a reason to run the agent
/// plainly, not a reason to invoke the CLI with an empty `--model`.
/// The tools an agent holds (ADR-0036), read off its traits. Unreadable
/// traits yield none -- the same graceful reading every trait helper does.
pub(crate) fn trait_tools(traits_json: &str) -> Vec<String> {
    serde_json::from_str::<Value>(traits_json)
        .ok()
        .and_then(|v| {
            v.get("tools").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The line that tells an agent what it holds (ADR-0036): a granted tool is
/// something it is *meant* to use, not something it discovers in a menu.
/// Empty when nothing was granted.
pub(crate) fn tools_line(state: &AppState, tools: &[String]) -> String {
    let named: Vec<String> = tools
        .iter()
        .filter(|t| state.config.agent_tools.contains(t))
        .map(|t| match state.config.agent_tools.description(t) {
            Some(d) => format!("{t} — {d}"),
            None => t.clone(),
        })
        .collect();
    if named.is_empty() {
        String::new()
    } else {
        format!(
            "\nTools granted to you (MCP servers, already connected): {}. Use them when the work calls for them.",
            named.join("; ")
        )
    }
}

pub(crate) fn trait_model(traits_json: &str) -> String {
    serde_json::from_str::<Value>(traits_json)
        .ok()
        .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
        .filter(|m| crate::model::is_known(m))
        .unwrap_or_else(|| crate::model::default_model().id.to_string())
}

/// The adapter invocation (ADR-0021). Configurable — tests use a stub — and the
/// default drives the Claude Code CLI headless, on the model the agent is
/// actually characterized for. Until M14 slice 3 this string existed in two
/// copies and named no model at all, so `AgentTraits.model` was decorative.
/// Single-quote a path for the shell the agent command runs through.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub(crate) fn agent_command(
    state: &AppState,
    caged: bool,
    mcp_config: Option<&Path>,
    ceiling_cents: Option<i64>,
) -> String {
    if let Some(configured) = state.config.agent_cmd.clone() {
        return configured;
    }
    // `stream-json` rather than `json`, and the reason is not the streaming.
    //
    // A stream run emits a `rate_limit_event` carrying the subscription's
    // current window, when it resets, and whether we are still allowed inside
    // it (ADR-0030). A `-p json` run does not, and the status line that would
    // otherwise carry it is never invoked headless — measured, not assumed. So
    // this is how the plan's state **rides along with work already being done**
    // instead of costing a call of its own, the same bargain ADR-0026 made for
    // memory watermarks.
    //
    // The final `result` event is the identical envelope `json` produced, so
    // cost parsing and failure reporting read exactly what they always did.
    // `--verbose` is not optional here: the CLI refuses `stream-json` under
    // `--print` without it.
    let mut cmd = "claude -p \"$OVERMIND_TASK_PROMPT\" --model \"$OVERMIND_AGENT_MODEL\" \
                   --output-format stream-json --verbose --include-partial-messages"
        .to_string();
    // The cap, handed to the adapter itself (ADR-0030). Since M6 the gate has
    // been *around* the run: check, reserve, spawn, record. What it could not do
    // is stop a run already under way, so an agent could overrun by the whole
    // difference between the flat estimate and one turn's true cost — M18 said
    // exactly that and left it open.
    //
    // This narrows it rather than closing it, and the difference matters:
    // measured, a $0.05 ceiling recorded $0.080729, because the flag stops
    // *after* exceeding rather than pre-authorising. So the gate stays the
    // primary control and this is the second layer, which is also why it is
    // passed in every economy: under a subscription the cap is not money, but a
    // looping agent burns quota just the same and the brake is the same brake.
    //
    // Only when there is something to promise: an uncapped agent gets no
    // ceiling, and a zero one would refuse the run at the first token — the
    // gate has already decided whether it may start.
    if let Some(cents) = ceiling_cents.filter(|c| *c > 0) {
        cmd.push_str(&format!(
            " --max-budget-usd {}",
            crate::governance::dollars(cents)
        ));
    }
    // `--strict-mcp-config` matters as much as the config itself: without it the
    // CLI merges whatever MCP servers the machine's own configuration happens to
    // hold, and a caged agent would quietly inherit tools nobody granted it.
    if let Some(p) = mcp_config {
        cmd.push_str(&format!(
            " --mcp-config {} --strict-mcp-config",
            shell_quote(&p.to_string_lossy())
        ));
    }
    // Headless has nobody to ask. The CLI's permission system assumes a person
    // at a terminal, so in `-p` mode every Edit, Write and Bash is denied and
    // the agent can only read — which is how the smoke run found that no `code`
    // task had ever produced a diff against the real adapter. Stubs are shell
    // scripts and write freely, so every test was green over it.
    //
    // The flag is safe *here and only here*: ADR-0023 moved enforcement to the
    // OS, and a caged agent can write to its run directory and nowhere else,
    // with no credentials to push anything. Asking a permission question nobody
    // can answer is not a second boundary, it is a deadlock. Uncaged, we do not
    // pass it: the CLI's own prompt is then the only thing left, and a
    // read-only agent beats an unconstrained one.
    if caged {
        cmd.push_str(" --dangerously-skip-permissions");
    }
    cmd
}

/// Resume a session that is marked queued/running in the DB but has no live
/// runner in this process (server restart, crashed runner). Called by the
/// heartbeat scheduler.
pub async fn resume_session(state: &AppState, session_id: &str) -> Result<(), RunnerError> {
    let session: Option<(String, String, String)> = sqlx::query_as(
        "SELECT task_id, agent_id, workspace_path FROM agent_task_sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((task_id, agent_id, workspace_path)) = session else {
        return Err(RunnerError::NotFound("session"));
    };
    let task: Option<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT company_id, title, description, status, execution_kind FROM tasks WHERE id = ?",
    )
    .bind(&task_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((company_id, title, description, task_status, exec_kind_str)) = task else {
        return Err(RunnerError::NotFound("task"));
    };
    let exec_kind = ExecutionKind::parse(&exec_kind_str).unwrap_or_default();
    let agent_traits: Option<(String,)> = sqlx::query_as("SELECT traits FROM agents WHERE id = ?")
        .bind(&agent_id)
        .fetch_optional(&state.pool)
        .await?;
    let agent_traits = agent_traits.map(|(t,)| t).unwrap_or_default();

    let ctx = SessionContext {
        state: state.clone(),
        session_id: session_id.to_string(),
        task_id: task_id.clone(),
        company_id: company_id.clone(),
        agent_id: agent_id.clone(),
        persona: load_persona(state, &agent_id).await,
        worktree_dir: PathBuf::from(&workspace_path),
        title,
        description,
        agent_traits,
        exec_kind,
    };

    // A session whose task is no longer in progress, or whose worktree is
    // gone, cannot be resumed: fail it and release the task.
    if task_status != "in_progress" || !ctx.worktree_dir.is_dir() {
        let error = if task_status != "in_progress" {
            format!("cannot resume: task is '{task_status}'")
        } else {
            "cannot resume: worktree is missing".to_string()
        };
        let release = task_status == "in_progress";
        finalize(&ctx, Outcome::Infra { error, release }).await?;
        return Ok(());
    }

    let mut tx = state.write_tx().await?;
    sqlx::query(
        "UPDATE agent_task_sessions SET status = 'running', resumed_count = resumed_count + 1 WHERE id = ?",
    )
    .bind(session_id)
    .execute(&mut *tx)
    .await?;
    audit::append(
        &mut tx,
        Some(&company_id),
        Some(&task_id),
        event_kind::SESSION_RESUMED,
        &json!({ "session_id": session_id, "agent_id": agent_id }),
    )
    .await?;
    tx.commit().await?;
    state.notify(&company_id);

    register(state, session_id);
    tokio::spawn(async move {
        run_session(ctx, Mode::Resume).await;
    });
    Ok(())
}

fn register(state: &AppState, session_id: &str) {
    if let Ok(mut running) = state.running.lock() {
        running.insert(session_id.to_string());
    }
}

fn deregister(state: &AppState, session_id: &str) {
    if let Ok(mut running) = state.running.lock() {
        running.remove(session_id);
    }
}

pub(crate) struct SessionContext {
    state: AppState,
    session_id: String,
    task_id: String,
    company_id: String,
    agent_id: String,
    /// Who is doing the work (ADR-0005). `None` only if the agent row vanished.
    persona: Option<Persona>,
    /// The agent's cwd: a git worktree for `code`, a scratch dir for `knowledge`.
    worktree_dir: PathBuf,
    title: String,
    description: String,
    agent_traits: String,
    exec_kind: ExecutionKind,
}

pub(crate) struct WorktreeSpec {
    repo_cwd: PathBuf,
    default_ref: Option<String>,
    branch: String,
}

/// How a fresh run is prepared: `code` needs a git worktree from a repo;
/// `knowledge` just needs an empty scratch dir (ADR-0017).
pub(crate) enum FreshSpec {
    Code(WorktreeSpec),
    Knowledge,
}

pub(crate) enum Mode {
    Fresh(FreshSpec),
    Resume,
}

enum Outcome {
    Success { output: String },
    AgentFailure { output: String, exit_code: i32 },
    TimedOut { timeout_secs: u64 },
    Infra { error: String, release: bool },
}

async fn run_session(ctx: SessionContext, mode: Mode) {
    let outcome = execute(&ctx, mode).await;
    // Retire the run's token (ADR-0027) **before** the run is published as
    // over. The config file is already gone — its guard dropped when
    // `run_process` returned — but the row outlives the file, and a token that
    // still resolves is still a key. Invalidating is a write rather than an
    // expiry because "the run is over" is a fact, and a clock would be a worse
    // way to learn it.
    //
    // The order is the fix, not decoration. `finalize` is what writes the
    // terminal status, so retiring afterwards left a window in which the
    // session read as finished to everyone watching while its token still
    // authenticated. Nothing in `finalize` needs the token — the agent process
    // died when `run_process` returned — so the door is shut first and the
    // announcement made second. Found by CI on macOS the first time that
    // platform ever ran the suite; the window was always there and the loser of
    // the race was simply never observed.
    //
    // Unconditional: it runs for a completed, failed, timed-out or abandoned
    // run alike, and costs nothing when there was never a token.
    let _ = sqlx::query("UPDATE agent_task_sessions SET mcp_token = NULL WHERE id = ?")
        .bind(&ctx.session_id)
        .execute(&ctx.state.pool)
        .await;
    if let Err(e) = finalize(&ctx, outcome).await {
        eprintln!("session {}: failed to finalize: {e}", ctx.session_id);
    }
    deregister(&ctx.state, &ctx.session_id);
}

async fn execute(ctx: &SessionContext, mode: Mode) -> Outcome {
    let resume = matches!(mode, Mode::Resume);
    if let Mode::Fresh(fresh) = &mode {
        let prep = match fresh {
            FreshSpec::Code(spec) => prepare_worktree(ctx, spec).await,
            FreshSpec::Knowledge => prepare_scratch(ctx).await,
        };
        if let Err(e) = prep {
            return Outcome::Infra {
                error: e.to_string(),
                release: false,
            };
        }
    }
    run_process(ctx, resume).await
}

/// Prepare a `knowledge` run: an empty scratch dir, no git. The session goes
/// `running` with no `base_sha` — there is no diff base (ADR-0017).
async fn prepare_scratch(ctx: &SessionContext) -> Result<(), RunnerError> {
    tokio::fs::create_dir_all(&ctx.worktree_dir)
        .await
        .map_err(|e| RunnerError::Invalid(format!("cannot create scratch dir: {e}")))?;
    place_inputs(ctx).await;
    sqlx::query("UPDATE agent_task_sessions SET status = 'running', started_at = ? WHERE id = ?")
        .bind(now())
        .bind(&ctx.session_id)
        .execute(&ctx.state.pool)
        .await?;
    Ok(())
}

/// Keep Overmind's own directories out of the run's diff.
///
/// The path matters: in a worktree `.git` is a *file* pointing at
/// `<repo>/.git/worktrees/<name>`, so `<worktree>/.git/info/exclude` does not
/// exist and writing to it silently does nothing — which is exactly what
/// happened until a test caught it. `git rev-parse --git-path` resolves the
/// per-worktree location, so the user's own repo is never touched.
///
/// Best-effort: without it the worst case is a report showing up in a diff.
async fn exclude_from_git(worktree: &Path) {
    let Ok(path) = git(worktree, &["rev-parse", "--git-path", "info/exclude"]).await else {
        return;
    };
    // The path is relative to the worktree unless git returns an absolute one.
    let path = if Path::new(&path).is_absolute() {
        PathBuf::from(path)
    } else {
        worktree.join(path)
    };
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&path, format!("/{DELIVERABLES_DIR}/\n/{INPUTS_DIR}/\n")).await;
}

/// One `git worktree add` at a time per repository. Two concurrent adds on
/// the same repo race inside `.git/worktrees/` -- measured on CI (M27):
/// *"failed to read .git/worktrees/<other session>/commondir"*, the second
/// add reading the first's half-written entry -- and git offers no lock of
/// its own for it. Process-wide, keyed by the repository path: runs on
/// different repositories never wait on each other.
static WORKTREE_LOCKS: std::sync::Mutex<
    Option<std::collections::HashMap<PathBuf, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::Mutex::new(None);

fn worktree_lock(repo: &Path) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut guard = match WORKTREE_LOCKS.lock() {
        Ok(g) => g,
        // A poisoned registry costs a fresh lock, not a failed checkout --
        // the same trade the runner makes everywhere.
        Err(e) => e.into_inner(),
    };
    guard
        .get_or_insert_with(Default::default)
        .entry(repo.to_path_buf())
        .or_default()
        .clone()
}

async fn prepare_worktree(ctx: &SessionContext, spec: &WorktreeSpec) -> Result<(), RunnerError> {
    if let Some(parent) = ctx.worktree_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| RunnerError::Git(format!("cannot create worktree dir: {e}")))?;
    }
    let worktree = ctx.worktree_dir.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "add", worktree.as_str(), "-b", &spec.branch];
    if let Some(r) = &spec.default_ref {
        args.push(r.as_str());
    }
    let base_sha = {
        let lock = worktree_lock(&spec.repo_cwd);
        let _held = lock.lock().await;
        git(&spec.repo_cwd, &args).await?;
        git(&ctx.worktree_dir, &["rev-parse", "HEAD"]).await?
    };
    // A code run can also hand back documents (M17): `deliverables/` is
    // collected as artifacts and kept out of git, so one run can produce a
    // diff *and* a report without either polluting the other. Same for the
    // files the human attached — they are input, not a change.
    let _ = tokio::fs::create_dir_all(ctx.worktree_dir.join(DELIVERABLES_DIR)).await;
    exclude_from_git(&ctx.worktree_dir).await;
    place_inputs(ctx).await;
    sqlx::query(
        "UPDATE agent_task_sessions SET status = 'running', base_sha = ?, started_at = ? WHERE id = ?",
    )
    .bind(&base_sha)
    .bind(now())
    .bind(&ctx.session_id)
    .execute(&ctx.state.pool)
    .await?;
    Ok(())
}

/// A per-run MCP config file, and the token it carries, both gone when this
/// drops (ADR-0027).
///
/// The deletion is a `Drop` and not a line at the end of the happy path on
/// purpose: ADR-0015 called this out as learned the hard way, and a run can end
/// by completing, failing, timing out, or being torn down when the server
/// stops. A file holding a live bearer token that outlives its run is a key
/// left in a door.
///
/// It lives in the system temp dir rather than the run directory: for a `code`
/// task the run directory is a git worktree, and a token file there is one
/// `git add -A` away from being committed.
pub(crate) struct AgentMcpConfig {
    pub(crate) path: PathBuf,
    /// The run's memory token -- `None` when the file carries only granted
    /// tools (memory off, or a conversational turn, which has no session).
    pub(crate) token: Option<String>,
}

impl AgentMcpConfig {
    /// The file for a conversational turn (ADR-0036): granted tools only, no
    /// memory endpoint -- a turn has no session and therefore no token. `None`
    /// when the agent holds no tools, so a turn that needs nothing gets
    /// exactly what it got before.
    pub(crate) fn write_for_turn(state: &AppState, id: &str, tools: &[String]) -> Option<Self> {
        Self::write_with(state, id, tools, false)
    }

    fn write_with(state: &AppState, id: &str, tools: &[String], with_memory: bool) -> Option<Self> {
        let memory = with_memory && state.memory.is_enabled();
        let mut servers = serde_json::Map::new();
        // Granted tools (ADR-0036): the operator declared them, the company
        // granted them to this agent, and nothing else gets in -- the CLI
        // runs with --strict-mcp-config over exactly this file.
        for name in tools {
            if let Some(def) = state.config.agent_tools.servers.get(name) {
                servers.insert(name.clone(), def.clone());
            }
        }
        let mut token = None;
        if memory {
            // v4, not the v7 used for ids elsewhere: v7 encodes its creation
            // time in the leading bits, and a secret should not tell you when
            // it was minted. The entropy of v7 would have been sufficient;
            // being predictable in *any* dimension is not a property to hand
            // a token.
            let t = uuid::Uuid::new_v4().to_string();
            servers.insert(
                "overmind".into(),
                json!({
                    "type": "http",
                    "url": format!("{}/mcp", state.config.self_url.trim_end_matches('/')),
                    "headers": { "Authorization": format!("Bearer {t}") }
                }),
            );
            token = Some(t);
        }
        if servers.is_empty() {
            return None;
        }
        let path = std::env::temp_dir().join(format!("overmind-mcp-{id}.json"));
        let body = json!({ "mcpServers": servers });
        std::fs::write(&path, body.to_string()).ok()?;
        // Before anyone can read it: the token is the run's identity.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Some(Self { path, token })
    }

    /// The file for a task run: the memory endpoint (ADR-0027) plus the
    /// agent's granted tools (ADR-0036). `None` when there is neither -- the
    /// agent then gets no MCP config at all, which is the same graceful
    /// degradation as an empty `OVERMIND_MEMORY_CONTEXT` (ADR-0003, rule 6).
    fn write(state: &AppState, session_id: &str, tools: &[String]) -> Option<Self> {
        Self::write_with(state, session_id, tools, true)
    }
}

impl Drop for AgentMcpConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn run_process(ctx: &SessionContext, resume: bool) -> Outcome {
    let cage = crate::sandbox::Cage {
        run_dir: &ctx.worktree_dir,
    };
    // The agent's own door to memory (ADR-0027). Held for the whole run: when
    // this binding drops, the file goes and the token stops working.
    let mcp = AgentMcpConfig::write(&ctx.state, &ctx.session_id, &trait_tools(&ctx.agent_traits));
    if let Some(token) = mcp.as_ref().and_then(|m| m.token.as_ref()) {
        let _ = sqlx::query("UPDATE agent_task_sessions SET mcp_token = ? WHERE id = ?")
            .bind(token)
            .bind(&ctx.session_id)
            .execute(&ctx.state.pool)
            .await;
    }
    // Where the agent runs as its own uid (ADR-0029), everything the server
    // just built for this run belongs to the server until it is handed over:
    // the run directory with its inputs, and the token file — that one by
    // itself, never the directory holding every run's token.
    //
    // Loudly, because a run that cannot write its own directory produces
    // nothing, and "produced nothing" arriving as success is the defect this
    // milestone exists to end.
    if let Err(e) = crate::sandbox::hand_over(&ctx.state.config, &ctx.worktree_dir).await {
        return Outcome::Infra {
            error: format!("cannot hand the run directory to the agent: {e}"),
            release: false,
        };
    }
    if let Some(m) = &mcp
        && let Err(e) = crate::sandbox::hand_over(&ctx.state.config, &m.path).await
    {
        return Outcome::Infra {
            error: format!("cannot hand the agent its memory credentials: {e}"),
            release: false,
        };
    }
    // What is left under the agent's cap, this run's own reservation excluded —
    // it reserved at checkout, and counting that against itself would hand the
    // adapter a ceiling of nothing (ADR-0030).
    let ceiling = {
        let cap = trait_budget_cents(&ctx.agent_traits);
        match ctx.state.pool.acquire().await {
            Ok(mut conn) => crate::governance::headroom_cents(
                &mut conn,
                &ctx.agent_id,
                cap,
                Some(&ctx.session_id),
            )
            .await
            .unwrap_or(None),
            // A ceiling we cannot compute is one we do not impose. The gate
            // already let this run start, and inventing a limit from a failed
            // query would stop work for a reason nobody could explain.
            Err(_) => None,
        }
    };
    let agent_cmd = agent_command(
        &ctx.state,
        crate::sandbox::caged(&ctx.state.config, &cage),
        mcp.as_ref().map(|m| m.path.as_path()),
        ceiling,
    );
    // Load what *this company* remembers about this kind of work, and put it
    // in front of the agent (and in an env var). A no-op when memory is off.
    let memory_context = ctx
        .state
        .memory_for(&ctx.company_id)
        .await
        .get_context(
            &ctx.worktree_dir.to_string_lossy(),
            &format!("{}\n{}", ctx.title, ctx.description),
        )
        .await;
    let memory_block = memory_context
        .as_deref()
        .map(|m| {
            format!(
                "\n\nWhat the organization remembers (use it, don't repeat past mistakes):\n{m}"
            )
        })
        .unwrap_or_default();

    // Calls this agent sat in on are settled — it works from them (ADR-0020).
    let decisions_block = crate::meeting::decisions_block(&ctx.state, &ctx.agent_id).await;
    // The company's language (M16): what the agent writes must match the UI.
    let language =
        crate::i18n::prompt_line(&crate::i18n::company_language(&ctx.state, &ctx.company_id).await);

    // How the agent is expected to deliver, per execution kind (ADR-0017).
    //
    // Both kinds may hand back files of any type (M17). We say what the file
    // is and where to put it; we do not tell it what to write it *with* — the
    // adapter has its own tools, and constraining the format here would make
    // "produce a chart" impossible for no reason.
    let deliver = match ctx.exec_kind {
        ExecutionKind::Code => {
            "Work in the current directory. When done, leave the changes uncommitted. \
             Anything that is NOT a code change — a report, a chart, a generated file, a \
             standalone snippet — goes in the `deliverables/` directory instead: it is kept out \
             of git and handed back alongside your diff. Any file type is fine."
        }
        ExecutionKind::Knowledge => {
            "Write your deliverable as files in the current directory — Markdown for prose \
             (e.g. ARTIFACT.md), but any format the work calls for: CSV or JSON for data, an \
             image for a chart, a source file for code, a PDF if you can produce one. \
             Subdirectories are kept, so organise them if there is more than one. Do not use git."
        }
    };

    // What the human handed the agent (M17). Named with type and size so it
    // can decide what is worth opening before it opens anything.
    let inputs = task_inputs(&ctx.state, &ctx.task_id).await;
    let inputs_block = if inputs.is_empty() {
        String::new()
    } else {
        let list = inputs
            .iter()
            .map(|(name, mime, size, _)| {
                format!(
                    "- {INPUTS_DIR}/{} ({mime}, {})",
                    files::safe_name(name),
                    files::human_size(*size as u64)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nFiles provided with this task, already in your working directory:\n{list}\nOpen the ones that matter. If one is in a format you cannot read directly, say so in your output rather than guessing at its contents."
        )
    };
    // Collaboration is expected: an agent that hits a call it should not make
    // alone asks for a meeting instead of guessing (ADR-0020). Nothing happens
    // until the human approves, so this is cheap to ask for and safe to ignore.
    let meeting_hint = "\n\nIf finishing this needs a decision you should not take alone — it affects teammates' work, or you are blocked on a call above your role — write a file named MEETING_REQUEST.json in this directory: {\"topic\": \"...\", \"reason\": \"why the room is needed\", \"participants\": [\"<teammate name>\"], \"turn_cap\": 6}. It reaches the human for approval; do not wait for it, finish what you can.\nAsk sparingly: you may have only ONE request waiting on the human at a time, and every request costs them an interruption. If you can take the call yourself, take it.";
    // Who is doing the work (ADR-0005 / M14). Without this the prompt is
    // role-blind: every agent gets identical instructions for the same task.
    let company = ctx.state.company_descriptor(&ctx.company_id).await;
    let persona_block = ctx
        .persona
        .as_ref()
        .map(|p| format!("{}\n\n", p.block(&company)))
        .unwrap_or_default();

    let prompt = if resume {
        format!(
            "{persona_block}You are resuming interrupted work on the task \"{}\".\n\n{}{}{}{inputs_block}\n\nThe current directory may contain partial work from the interrupted run — inspect it first, then finish the task. {deliver}{meeting_hint}{language}",
            ctx.title, ctx.description, memory_block, decisions_block
        )
    } else {
        format!(
            "{persona_block}You are working on the task \"{}\".\n\n{}{}{}{inputs_block}\n\n{deliver}{meeting_hint}{language}",
            ctx.title, ctx.description, memory_block, decisions_block
        )
    };

    let adapter_session_id: Option<String> = if resume {
        sqlx::query_as::<_, (Option<String>,)>(
            "SELECT adapter_session_id FROM agent_task_sessions WHERE id = ?",
        )
        .bind(&ctx.session_id)
        .fetch_optional(&ctx.state.pool)
        .await
        .ok()
        .flatten()
        .and_then(|(s,)| s)
    } else {
        None
    };

    // Caged: the agent may write its own run directory and nothing else
    // (ADR-0023). `~/.ssh`, the browser profile and Overmind's own database
    // are unreachable from in there.
    let mut cmd = crate::sandbox::command(&ctx.state.config, &cage, &agent_cmd);
    for (k, v) in crate::sandbox::git_isolation() {
        cmd.env(k, v);
    }
    cmd.current_dir(&ctx.worktree_dir)
        .env("OVERMIND_TASK_PROMPT", &prompt)
        .env("OVERMIND_TASK_TITLE", &ctx.title)
        .env("OVERMIND_TASK_DESCRIPTION", &ctx.description)
        .env("OVERMIND_AGENT_TRAITS", &ctx.agent_traits)
        .env("OVERMIND_AGENT_MODEL", trait_model(&ctx.agent_traits))
        .env(
            "OVERMIND_MEMORY_CONTEXT",
            memory_context.as_deref().unwrap_or(""),
        )
        // Nothing is piped in, and the server's own stdin is not the agent's to
        // inherit — under a daemon it never reaches EOF and the CLI waits on it.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(sid) = &adapter_session_id {
        cmd.env("OVERMIND_RESUME_SESSION_ID", sid);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome::Infra {
                error: format!("failed to spawn agent: {e}"),
                release: false,
            };
        }
    };

    let timeout_secs = ctx.state.config.session_timeout_secs;
    // Drained line by line so the run narrates itself while it happens
    // (ADR-0039); the narration is cleared with the run, however it ends.
    let drained = drain_narrating(&ctx.state, &ctx.session_id, child, timeout_secs).await;
    ctx.state.clear_activity(&ctx.session_id);
    match drained {
        Ok(None) => Outcome::TimedOut { timeout_secs },
        Err(e) => Outcome::Infra {
            error: format!("failed to read agent output: {e}"),
            release: false,
        },
        Ok(Some(out)) => {
            let mut output = out.stdout;
            let stderr = out.stderr;
            if !stderr.trim().is_empty() {
                output.push_str("\n--- stderr ---\n");
                output.push_str(stderr.trim());
            }
            // What the run learned about the plan on its way past (ADR-0030).
            // Read from success and failure alike: a run that was *refused* for
            // running out is exactly the one whose report matters most.
            if let Some(window) = crate::economy::plan_window_in(&output) {
                ctx.state.set_plan_window(window);
            }
            let exit_code = out.status.code().unwrap_or(-1);
            if exit_code == 0 {
                Outcome::Success { output }
            } else {
                Outcome::AgentFailure { output, exit_code }
            }
        }
    }
}

/// What the adapter said went wrong, when it said anything.
///
/// "agent exited with code 1" is true and useless. The Claude Code CLI puts the
/// reason in its result envelope — `"Credit balance is too low"` is the one that
/// stopped the smoke run, and a person reading the drawer had to find it inside
/// a wall of JSON to learn their account was empty. Errors a human can act on
/// are worth more than exit codes.
fn adapter_failure(output: &str) -> Option<String> {
    let envelope: Value = output
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok())?;
    if envelope.get("is_error").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    // The ceiling we handed it (ADR-0030). Worth naming rather than passing
    // through, because this failure is *ours*: the agent did not break, it
    // reached the cap this Overmind gave it, and the person reading has a
    // specific thing they can do about that. The envelope carries no `result`
    // in this case, so without this the whole event would read "agent exited
    // with code 1".
    if envelope.get("subtype").and_then(Value::as_str) == Some("error_max_budget_usd") {
        let said = envelope
            .get("errors")
            .and_then(Value::as_array)
            .and_then(|e| e.first())
            .and_then(Value::as_str)
            .unwrap_or("the run reached its budget ceiling");
        return Some(format!(
            "stopped at this agent's budget cap — {said}. Raise the cap to let it continue."
        ));
    }
    // Otherwise whatever the adapter said, preferring its prose and falling
    // back to its error list — `Credit balance is too low` arrived in the first
    // and a ceiling arrives in the second, and both beat an exit code.
    let said = envelope
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            envelope
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|e| e.first())
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })?;
    Some(crate::ceo::clamp_agent_text(said))
}

async fn finalize(ctx: &SessionContext, outcome: Outcome) -> Result<(), RunnerError> {
    struct Final {
        session_status: &'static str,
        output: String,
        exit_code: Option<i32>,
        last_error: Option<String>,
        /// `Some(status)` moves the task; release additionally clears the assignee.
        task_to: &'static str,
        release: bool,
    }
    let f = match outcome {
        Outcome::Success { output } => Final {
            session_status: "completed",
            output,
            exit_code: Some(0),
            last_error: None,
            task_to: "in_review",
            release: false,
        },
        Outcome::AgentFailure { output, exit_code } => Final {
            session_status: "failed",
            last_error: Some(
                adapter_failure(&output)
                    .unwrap_or_else(|| format!("agent exited with code {exit_code}")),
            ),
            output,
            exit_code: Some(exit_code),
            task_to: "blocked",
            release: false,
        },
        Outcome::TimedOut { timeout_secs } => Final {
            session_status: "failed",
            output: String::new(),
            exit_code: None,
            last_error: Some(format!("session timed out after {timeout_secs}s")),
            task_to: "todo",
            release: true,
        },
        Outcome::Infra { error, release } => Final {
            session_status: "failed",
            output: String::new(),
            exit_code: None,
            last_error: Some(error),
            task_to: if release { "todo" } else { "blocked" },
            release,
        },
    };

    let mut tx = ctx.state.write_tx().await?;

    // What the run produced, besides (or instead of) a diff — M17.
    //
    // A knowledge run's deliverable is everything it wrote; a code run's is the
    // diff, plus anything it deliberately put in `deliverables/`. Either way
    // the bytes are copied somewhere durable first: the worktree is torn down,
    // and an artifact that points into a deleted directory is not an artifact.
    //
    // Best-effort throughout: the session is already recorded, and a file that
    // cannot be read must not undo that.
    let delivered: usize = if f.session_status == "completed" {
        let (root, inline_root) = match ctx.exec_kind {
            ExecutionKind::Knowledge => (ctx.worktree_dir.clone(), true),
            ExecutionKind::Code => (ctx.worktree_dir.join(DELIVERABLES_DIR), false),
        };
        let store = ctx
            .state
            .config
            .data_dir
            .join("artifacts")
            .join(&ctx.session_id);
        let mut n = 0usize;
        for (rel, size) in files::collect_files(&root, MAX_ARTIFACTS).await {
            let rel_str = files::safe_relative(&rel);
            // Control files and the inputs we placed are not deliverables.
            if rel_str.is_empty()
                || rel_str == MEETING_REQUEST_FILE
                || rel_str.starts_with(&format!("{INPUTS_DIR}/"))
                || (inline_root && rel_str.starts_with(&format!("{DELIVERABLES_DIR}/")))
            {
                continue;
            }
            let mime = files::mime_for(&rel_str);
            // Text small enough to read stays inline, so the drawer can show it
            // without a second request; everything else is served from disk.
            let content = if files::is_texty(mime) && size <= MAX_INLINE_BYTES {
                tokio::fs::read_to_string(root.join(&rel)).await.ok()
            } else {
                None
            };
            let stored = store.join(&rel_str);
            let copied = async {
                tokio::fs::create_dir_all(stored.parent()?).await.ok()?;
                tokio::fs::copy(root.join(&rel), &stored).await.ok()
            }
            .await
            .is_some();
            if !copied && content.is_none() {
                continue; // nothing survives of this one; do not record a lie
            }
            sqlx::query(
                "INSERT INTO task_artifacts
                 (id, task_id, session_id, kind, title, mime, content, file_path, size_bytes, relative_path, created_at)
                 VALUES (?, ?, ?, 'document', ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&ctx.task_id)
            .bind(&ctx.session_id)
            .bind(&rel_str)
            .bind(mime)
            .bind(&content)
            .bind(copied.then(|| stored.to_string_lossy().into_owned()))
            .bind(size as i64)
            .bind(&rel_str)
            .bind(now())
            .execute(&mut *tx)
            .await?;
            audit::append(
                &mut tx,
                Some(&ctx.company_id),
                Some(&ctx.task_id),
                event_kind::ARTIFACT_CREATED,
                &json!({ "session_id": ctx.session_id, "title": rel_str, "mime": mime, "size_bytes": size }),
            )
            .await?;
            n += 1;
        }
        // A knowledge run must leave something in the drawer even when the
        // agent wrote no file — the raw output is better than an empty panel.
        // A code run that produced only a diff is complete without one.
        if n == 0 && ctx.exec_kind == ExecutionKind::Knowledge {
            sqlx::query(
                "INSERT INTO task_artifacts
                 (id, task_id, session_id, kind, title, mime, content, file_path, size_bytes, relative_path, created_at)
                 VALUES (?, ?, ?, 'document', 'Run output', 'text/plain', ?, NULL, ?, NULL, ?)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&ctx.task_id)
            .bind(&ctx.session_id)
            .bind(&f.output)
            .bind(f.output.len() as i64)
            .bind(now())
            .execute(&mut *tx)
            .await?;
            audit::append(
                &mut tx,
                Some(&ctx.company_id),
                Some(&ctx.task_id),
                event_kind::ARTIFACT_CREATED,
                &json!({ "session_id": ctx.session_id, "title": "Run output" }),
            )
            .await?;
        }
        n
    } else {
        0
    };

    // A knowledge run's deliverable **is** what it wrote, so one that wrote
    // nothing delivered nothing — however cleanly the adapter exited. Until now
    // that was filed `in_review`, with the adapter's own transcript standing in
    // for a document.
    //
    // Measured in the container on 2026-08-15, which is where it matters most
    // because the cage does not reach there yet: session `completed`, exit 0,
    // scratch directory empty, and the only thing in the drawer the `Run
    // output` fallback — a person invited to review `ttft_ms` and
    // `permission_denials`. The fallback stays, because an empty panel tells
    // you less than a transcript does; what stops is a run that delivered only
    // its own transcript calling itself a success.
    //
    // Code runs are excluded on purpose: their deliverable is the diff, and one
    // that deliberately changed nothing is a legitimate answer.
    let empty_handed = f.session_status == "completed"
        && ctx.exec_kind == ExecutionKind::Knowledge
        && delivered == 0;
    let session_status = if empty_handed {
        "failed"
    } else {
        f.session_status
    };
    let task_to = if empty_handed { "blocked" } else { f.task_to };
    let last_error = if empty_handed {
        // What the adapter said, when it said anything: "Credit balance is too
        // low" is a better answer than any sentence written here.
        Some(adapter_failure(&f.output).unwrap_or_else(|| {
            "the run wrote no file, so there is nothing to review — only the \
             adapter's own output"
                .to_string()
        }))
    } else {
        f.last_error.clone()
    };

    // Releasing the reservation (→ 0): once the run is over, its actual cost
    // is a cost_event and counts as spent; the in-flight reservation is gone.
    sqlx::query(
        "UPDATE agent_task_sessions SET status = ?, output = ?, exit_code = ?, last_error = ?, reserved_cents = 0, finished_at = ? WHERE id = ?",
    )
    .bind(session_status)
    .bind(&f.output)
    .bind(f.exit_code)
    .bind(&last_error)
    .bind(now())
    .bind(&ctx.session_id)
    .execute(&mut *tx)
    .await?;

    if let Some(adapter_sid) = parse_adapter_session_id(&f.output) {
        sqlx::query("UPDATE agent_task_sessions SET adapter_session_id = ? WHERE id = ?")
            .bind(&adapter_sid)
            .bind(&ctx.session_id)
            .execute(&mut *tx)
            .await?;
    }

    if f.release {
        sqlx::query(
            "UPDATE tasks SET status = 'todo', assignee_agent_id = NULL, updated_at = ? WHERE id = ?",
        )
        .bind(now())
        .bind(&ctx.task_id)
        .execute(&mut *tx)
        .await?;
        audit::append(
            &mut tx,
            Some(&ctx.company_id),
            Some(&ctx.task_id),
            event_kind::TASK_RELEASED,
            &json!({ "from": "in_progress", "to": "todo", "reason": last_error }),
        )
        .await?;
    } else {
        sqlx::query("UPDATE tasks SET status = ?, updated_at = ? WHERE id = ?")
            .bind(task_to)
            .bind(now())
            .bind(&ctx.task_id)
            .execute(&mut *tx)
            .await?;
        audit::append(
            &mut tx,
            Some(&ctx.company_id),
            Some(&ctx.task_id),
            event_kind::TASK_TRANSITIONED,
            &json!({ "from": "in_progress", "to": task_to }),
        )
        .await?;
    }
    audit::append(
        &mut tx,
        Some(&ctx.company_id),
        Some(&ctx.task_id),
        event_kind::SESSION_FINISHED,
        &json!({
            "session_id": ctx.session_id,
            "status": session_status,
            "exit_code": f.exit_code,
            "error": last_error,
        }),
    )
    .await?;

    // Cost capture: the Claude Code CLI (and our stubs) print a final JSON
    // object with total_cost_usd and usage. Missing/unparseable cost is not
    // an error — the session already carries the full output.
    if let Some(cost) = parse_cost(&f.output) {
        let agent_id: Option<(String,)> =
            sqlx::query_as("SELECT agent_id FROM agent_task_sessions WHERE id = ?")
                .bind(&ctx.session_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((agent_id,)) = agent_id {
            sqlx::query(
                "INSERT INTO cost_events (id, company_id, agent_id, task_id, session_id, provider, model,
                 input_tokens, cached_input_tokens, output_tokens, cost_cents, occurred_at, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&ctx.company_id)
            .bind(&agent_id)
            .bind(&ctx.task_id)
            .bind(&ctx.session_id)
            .bind("anthropic")
            .bind(&cost.model)
            .bind(cost.input_tokens)
            .bind(cost.cached_input_tokens)
            .bind(cost.output_tokens)
            .bind(cost.cost_cents)
            .bind(now())
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    ctx.state.notify(&ctx.company_id);

    // The floor moves on its own (M30, ADR-0042): a completed run releases
    // the tasks that were waiting on this one — each inherits the
    // deliverables as inputs and is offered to start by its agent's
    // autonomy. Best-effort: the session is already recorded.
    if f.session_status == "completed" {
        // Spawned, not awaited: releasing a dependent starts a new session,
        // and a future that awaits its own descendants is a cycle the
        // compiler rightly refuses.
        let st = ctx.state.clone();
        let tid = ctx.task_id.clone();
        let sid = ctx.session_id.clone();
        tokio::spawn(async move {
            if let Err(e) = release_dependents(&st, &tid, &sid).await {
                eprintln!("could not release dependents of {tid}: {e}");
            }
        });
    }

    // Collaboration (ADR-0020): while working, the agent may have hit a call it
    // should not make alone and left a MEETING_REQUEST.json behind. Raising it
    // notifies the human and waits for approval — nobody meets on their own.
    // Best-effort: a malformed request never fails an otherwise done session.
    let request_path = ctx.worktree_dir.join(MEETING_REQUEST_FILE);
    if let Ok(text) = tokio::fs::read_to_string(&request_path).await {
        // Remove it first, so it never lands in the diff or a resumed run.
        let _ = tokio::fs::remove_file(&request_path).await;
        match serde_json::from_str::<Value>(&text)
            .ok()
            .as_ref()
            .and_then(crate::meeting::Request::from_json)
        {
            Some(req) => {
                if let Err(e) =
                    crate::meeting::request(&ctx.state, &ctx.company_id, &ctx.agent_id, &req).await
                {
                    eprintln!(
                        "meeting requested by session {} refused: {e}",
                        ctx.session_id
                    );
                }
            }
            None => eprintln!(
                "session {} left an unreadable {MEETING_REQUEST_FILE}",
                ctx.session_id
            ),
        }
    }

    // Record what the organization just learned. Best-effort; never fatal.
    if f.session_status == "completed" {
        // The task is the memory's provenance (ADR-0015 decision 3, made real
        // in ADR-0025). The tag rides along so the brain still says where this
        // came from when read outside Overmind; the link row is what the UI
        // queries.
        let task_tag = format!("task:{}", ctx.task_id);
        // Hand back the position taken at checkout (ADR-0026): the provider
        // compares what we are storing against what appeared while this run was
        // working. A run that never got one simply sends nothing.
        let watermark: Option<String> =
            sqlx::query_scalar("SELECT brain_watermark FROM agent_task_sessions WHERE id = ?")
                .bind(&ctx.session_id)
                .fetch_optional(&ctx.state.pool)
                .await
                .ok()
                .flatten();

        let stored = ctx
            .state
            .memory_for(&ctx.company_id)
            .await
            .store_memory(
                &ctx.title,
                &format!(
                    "Task \"{}\" completed by an agent.\n\n{}",
                    ctx.title, ctx.description
                ),
                &ctx.company_id,
                &["task-completed", &task_tag],
                "note",
                watermark.as_deref(),
            )
            .await;
        ctx.state
            .link_memory(
                &ctx.company_id,
                "memory",
                stored.memory_ref.as_deref(),
                "task",
                &ctx.task_id,
                &ctx.title,
            )
            .await;

        if !stored.collisions.is_empty() {
            report_collisions(ctx, &stored.collisions).await;
        }
    }

    Ok(())
}

/// Tell the human that two agents wrote about the same thing without seeing
/// each other (ADR-0026).
///
/// A notification, not an approval, and the difference is the design. An
/// approval gates an action that has not happened yet; here both writes are
/// already in the brain and the task is finished, so there is nothing left to
/// authorize. An approval whose only outcome is "seen" is a to-do list wearing
/// governance's clothes, and it would train people to click through the gate
/// that matters. This says what happened and names both sides; judging whether
/// they actually contradict is a human's job, because similarity cannot tell
/// agreement from disagreement.
async fn report_collisions(ctx: &SessionContext, collisions: &[crate::mcp::Collision]) {
    let top = &collisions[0];
    let others = collisions.len().saturating_sub(1);
    let also = if others > 0 {
        format!(" (and {others} more)")
    } else {
        String::new()
    };
    let body = format!(
        "While \"{}\" was running, something close to what it just recorded was written too:\n\n\u{2022} {}{also}\n\nBoth are stored. They may agree — this only says nobody saw the other.",
        ctx.title, top.title
    );
    let n = crate::notify::New {
        kind: crate::notify::kind::MEMORY_COLLISION,
        title: "Two agents wrote about the same thing",
        body: &body,
        params: json!({
            "task": ctx.title,
            "collisions": collisions,
        }),
        agent_id: None, // the system noticed, not an agent
        subject: Some(("task", &ctx.task_id)),
        approval_id: None, // nothing to authorize — see above
    };
    if let Err(e) = crate::notify::send(&ctx.state, &ctx.company_id, n).await {
        // Never fatal: the work is done and the memory is stored. A missing
        // notification is worth a line in the log, not a failed run.
        eprintln!("collision notification failed (ignored): {e}");
    }
}

pub(crate) struct ParsedCost {
    pub model: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub cost_cents: i64,
}

fn last_json_object(output: &str) -> Option<Value> {
    for line in output.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            return Some(v);
        }
    }
    None
}

/// The adapter's own session id (e.g. Claude Code's), used for `--resume`.
fn parse_adapter_session_id(output: &str) -> Option<String> {
    last_json_object(output)?
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Find the last line of output that is a JSON object carrying
/// `total_cost_usd`, and extract cost + usage from it.
pub(crate) fn parse_cost(output: &str) -> Option<ParsedCost> {
    let v = last_json_object(output)?;
    let usd = v.get("total_cost_usd").and_then(Value::as_f64)?;
    let usage = v.get("usage").cloned().unwrap_or_else(|| json!({}));
    let tok = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0);
    Some(ParsedCost {
        model: billed_model(&v),
        input_tokens: tok("input_tokens"),
        cached_input_tokens: tok("cache_read_input_tokens"),
        output_tokens: tok("output_tokens"),
        cost_cents: cost_cents(usd),
    })
}

/// Which model this run should be attributed to.
///
/// The real Claude Code envelope has **no top-level `model`** — measured
/// against the live CLI, not assumed — so the old `unwrap_or("unknown")` meant
/// every real cost event was filed under "unknown" while the stubs, which do
/// emit `model`, looked fine. It does carry `modelUsage`, a map of model to
/// per-model cost, and a single run can touch more than one: the CLI bills a
/// small model for its own bookkeeping alongside the one doing the work. We
/// attribute the run to whichever cost the most, which is the one the operator
/// chose and the one worth seeing in the ledger.
fn billed_model(v: &Value) -> String {
    let by_cost = v
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|m| {
            m.iter()
                .max_by(|(_, a), (_, b)| {
                    let cost = |u: &Value| u.get("costUSD").and_then(Value::as_f64).unwrap_or(0.0);
                    cost(a).total_cmp(&cost(b))
                })
                .map(|(name, usage)| {
                    // The canonical name where the CLI gives one, so a dated
                    // snapshot and its alias do not read as different models.
                    usage
                        .get("canonicalModel")
                        .and_then(Value::as_str)
                        .unwrap_or(name)
                        .to_string()
                })
        });
    by_cost
        .or_else(|| v.get("model").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Dollars to cents, never losing a run that cost money.
///
/// Plain rounding sends anything under half a cent to zero, and a cheap turn
/// is genuinely that cheap — a small-model chat turn measured at $0.004 would
/// have recorded as **0**. Spend that records as nothing is spend the cap never
/// sees, which is the whole failure M18 existed to fix, arriving by a different
/// door. A run that cost money costs at least a cent.
///
/// The bias is upward by design: the budget is a cap, reserved in flat 50-cent
/// estimates, so sub-cent precision would be false precision — and erring
/// toward the cap being respected is the safe direction for a limit.
fn cost_cents(usd: f64) -> i64 {
    if usd <= 0.0 {
        return 0;
    }
    ((usd * 100.0).round() as i64).max(1)
}

/// Diff of everything the session changed (committed or not) against the
/// commit its worktree started from.
pub async fn session_diff(state: &AppState, session_id: &str) -> Result<String, RunnerError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT workspace_path, base_sha FROM agent_task_sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((workspace_path, base_sha)) = row else {
        return Err(RunnerError::NotFound("session"));
    };
    let Some(base_sha) = base_sha else {
        return Err(RunnerError::Invalid("session has not started yet".into()));
    };
    let worktree = Path::new(&workspace_path);
    // `git diff` ignores untracked files; intent-to-add makes new files
    // created by the agent show up in the diff without staging content.
    git(worktree, &["add", "--intent-to-add", "--all"]).await?;
    git(worktree, &["diff", &base_sha]).await
}

#[cfg(test)]
mod tests {
    use super::{parse_adapter_session_id, parse_cost};

    /// A real result envelope from the Claude Code CLI, captured live during
    /// M10's smoke run. Stubs emit a tidy `{"total_cost_usd":…,"model":…}`;
    /// the real thing has no top-level `model` at all, which is how the ledger
    /// came to file every real run under "unknown" while every test passed.
    const REAL_ENVELOPE: &str = include_str!("../tests/fixtures/claude-code-result.json");

    #[test]
    fn the_real_envelope_is_attributed_to_the_model_that_did_the_work() {
        let cost = parse_cost(REAL_ENVELOPE).expect("the real CLI reports cost");
        assert_eq!(
            cost.model, "claude-haiku-4-5",
            "not `unknown`, and not the bookkeeping model the CLI bills alongside it"
        );
        assert!(cost.input_tokens > 0 || cost.cached_input_tokens > 0);
        assert!(
            cost.cost_cents >= 1,
            "the run cost money: {}",
            cost.cost_cents
        );
    }

    #[test]
    fn a_run_that_cost_money_never_records_as_free() {
        // Measured shape of a cheap small-model turn. Plain rounding sent this
        // to zero, and spend that records as nothing is spend no cap can see.
        assert_eq!(super::cost_cents(0.004), 1);
        assert_eq!(super::cost_cents(0.0001), 1);
        // Nothing is still nothing, and ordinary amounts are unchanged.
        assert_eq!(super::cost_cents(0.0), 0);
        assert_eq!(super::cost_cents(0.0558), 6);
        assert_eq!(super::cost_cents(1.20), 120);
    }

    #[test]
    fn parses_cost_from_final_json_line() {
        let output = "doing work...\n{\"model\":\"claude-sonnet\",\"session_id\":\"abc-123\",\"total_cost_usd\":0.0525,\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":10,\"output_tokens\":50}}";
        let cost = parse_cost(output).expect("cost parsed");
        assert_eq!(cost.cost_cents, 5);
        assert_eq!(cost.input_tokens, 100);
        assert_eq!(cost.cached_input_tokens, 10);
        assert_eq!(cost.output_tokens, 50);
        assert_eq!(cost.model, "claude-sonnet");
        assert_eq!(parse_adapter_session_id(output).as_deref(), Some("abc-123"));
    }

    #[test]
    fn no_cost_json_is_none() {
        assert!(parse_cost("plain output, no json").is_none());
        assert!(parse_cost("{\"no_cost\":true}").is_none());
        assert!(parse_adapter_session_id("no json").is_none());
    }
}

#[cfg(test)]
mod failure_tests {
    use super::adapter_failure;

    /// The envelope that ended the live smoke run, trimmed to the fields that
    /// matter. Kept verbatim in shape because this is the one thing a stub
    /// cannot teach us — see `tests/fixtures/`.
    const CREDIT_EXHAUSTED: &str = r#"{"is_error":true,"subtype":"success","result":"Credit balance is too low","terminal_reason":"api_error","total_cost_usd":0}"#;

    #[test]
    fn a_failed_run_reports_what_the_adapter_said() {
        assert_eq!(
            adapter_failure(CREDIT_EXHAUSTED).as_deref(),
            Some("Credit balance is too low"),
            "the reason was in the envelope all along"
        );
    }

    #[test]
    fn a_run_that_failed_without_saying_why_falls_back_to_the_exit_code() {
        // Non-JSON output, and a well-formed envelope that is not an error:
        // neither has a message worth showing instead of the exit code.
        assert_eq!(adapter_failure("Segmentation fault"), None);
        assert_eq!(
            adapter_failure(r#"{"is_error":false,"result":"all good"}"#),
            None
        );
        assert_eq!(adapter_failure(r#"{"is_error":true,"result":"  "}"#), None);
    }
}

#[cfg(test)]
mod adapter_command_tests {
    use super::agent_command;
    use crate::db::AppState;

    async fn state_with(agent_cmd: Option<String>) -> AppState {
        let config = crate::Config {
            agent_cmd,
            ..crate::Config::default()
        };
        crate::init_with("sqlite::memory:", config)
            .await
            .expect("init in-memory db")
    }

    /// The pairing this whole thing turns on. `--dangerously-skip-permissions`
    /// is only defensible because the cage is what stops the agent; letting it
    /// escape onto an uncaged run would hand an unconstrained agent the whole
    /// machine, which is precisely what ADR-0023 exists to prevent.
    #[tokio::test]
    async fn the_permission_flag_never_travels_without_the_cage() {
        let state = state_with(None).await;
        assert!(
            agent_command(&state, true, None, None).contains("--dangerously-skip-permissions"),
            "a caged agent that cannot write is a read-only agent"
        );
        assert!(
            !agent_command(&state, false, None, None).contains("--dangerously-skip-permissions"),
            "uncaged, the CLI's own prompt is the only boundary left"
        );
    }

    /// Whatever the operator configured is theirs, cage or no cage. The escape
    /// hatch has to stay literal to be worth anything.
    #[tokio::test]
    async fn a_configured_command_is_left_exactly_as_written() {
        let state = state_with(Some("sh /my/stub.sh".into())).await;
        assert_eq!(agent_command(&state, true, None, None), "sh /my/stub.sh");
        assert_eq!(agent_command(&state, false, None, None), "sh /my/stub.sh");
    }
}

#[cfg(test)]
mod ceiling_tests {
    use super::{adapter_failure, agent_command};
    use crate::db::AppState;
    use crate::governance::{BudgetCheck, dollars};

    async fn default_state() -> AppState {
        crate::db::init_with("sqlite::memory:", crate::Config::default())
            .await
            .expect("in-memory db")
    }

    /// Cents are Overmind's unit and dollars are the flag's. The join between
    /// them is one place, and it has to survive the awkward numbers.
    #[test]
    fn cents_become_the_dollars_the_flag_expects() {
        assert_eq!(dollars(4981), "49.81");
        assert_eq!(dollars(5000), "50.00");
        assert_eq!(dollars(7), "0.07");
        assert_eq!(dollars(70), "0.70");
        assert_eq!(dollars(100), "1.00");
    }

    /// A run's own reservation is what it is about to spend. Counting it against
    /// itself would hand the adapter a ceiling of roughly nothing, which is the
    /// mistake that turns a second layer into a broken product.
    #[test]
    fn a_runs_own_estimate_is_not_subtracted_from_its_ceiling() {
        let check = BudgetCheck {
            fits: true,
            spent: 1000,
            reserved: 500,
            estimate: 50,
            cap: 5000,
        };
        assert_eq!(check.headroom(), Some(3500));
    }

    /// An uncapped agent gets no ceiling: there is nothing to promise, and a
    /// number invented here would be a limit nobody asked for.
    #[test]
    fn an_uncapped_agent_gets_no_ceiling() {
        let check = BudgetCheck {
            fits: true,
            spent: 0,
            reserved: 0,
            estimate: 50,
            cap: 0,
        };
        assert_eq!(check.headroom(), None);
    }

    /// Already over is zero, not negative — and zero must not reach the flag,
    /// which is checked separately below.
    #[test]
    fn an_agent_already_at_its_cap_has_no_headroom() {
        let check = BudgetCheck {
            fits: false,
            spent: 6000,
            reserved: 0,
            estimate: 50,
            cap: 5000,
        };
        assert_eq!(check.headroom(), Some(0));
    }

    #[tokio::test]
    async fn the_ceiling_reaches_the_adapter_only_when_there_is_one() {
        let state = default_state().await;
        let with = agent_command(&state, true, None, Some(4981));
        assert!(with.contains("--max-budget-usd 49.81"), "{with}");

        // Uncapped: no flag at all.
        let without = agent_command(&state, true, None, None);
        assert!(!without.contains("--max-budget-usd"), "{without}");

        // Zero would refuse the run at its first token, and whether it may run
        // at all is the gate's decision, already made before we got here.
        let zero = agent_command(&state, true, None, Some(0));
        assert!(!zero.contains("--max-budget-usd"), "{zero}");
    }

    /// A configured adapter is left exactly as written — a stub is not a CLI
    /// that has ever heard of this flag.
    #[tokio::test]
    async fn a_configured_command_gets_no_ceiling_bolted_on() {
        let state = crate::db::init_with(
            "sqlite::memory:",
            crate::Config {
                agent_cmd: Some("sh /my/stub.sh".into()),
                ..crate::Config::default()
            },
        )
        .await
        .expect("in-memory db");
        assert_eq!(
            agent_command(&state, true, None, Some(4981)),
            "sh /my/stub.sh"
        );
    }

    /// The envelope for a ceiling stop carries no `result`, so without
    /// recognising it the whole event reads "agent exited with code 1". This is
    /// the real payload, as measured on 2026-08-17.
    #[test]
    fn a_run_stopped_by_its_ceiling_says_so_and_says_what_to_do() {
        let observed = r#"{"is_error":true,"subtype":"error_max_budget_usd","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.05)"],"type":"result"}"#;
        let said = adapter_failure(observed).expect("a ceiling stop is a reportable failure");
        assert!(said.contains("budget cap"), "{said}");
        assert!(said.contains("Reached maximum budget"), "{said}");
        assert!(said.contains("Raise the cap"), "{said}");
    }

    /// The pre-existing path still wins where the adapter wrote prose.
    #[test]
    fn an_adapters_own_words_are_preferred_when_it_has_any() {
        let said = adapter_failure(r#"{"is_error":true,"result":"Credit balance is too low"}"#)
            .expect("prose is a reportable failure");
        assert_eq!(said, "Credit balance is too low");
    }

    /// And an error list is better than an exit code when there is no prose.
    #[test]
    fn an_error_list_beats_an_exit_code() {
        let said = adapter_failure(r#"{"is_error":true,"errors":["the model refused"]}"#)
            .expect("an error list is a reportable failure");
        assert_eq!(said, "the model refused");
    }
}
